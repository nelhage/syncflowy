//! Checkpoints, render history, pending deletes, fingerprints, and their on-disk store.
//!
//! Responsibility (design §3 checkpoint, §5.1 `base_md`/`base_wf`, §9.1 guard 1 render
//! generations and history, guard 2 pending deletes, §9 "one engine": a per-root lock).
//! I/O lives in [`StateStore`]; the data types ([`Checkpoint`], [`HistoryEntry`],
//! [`History`], [`PendingDeletes`]) are plain values whose logic is pure: every function
//! that depends on time takes `now` (unix seconds) as an argument, so tests inject the
//! clock.
//!
//! Consumes: [`crate::tree`] (`Tree`, `IdMap`, `NodeId`), [`crate::mdblock::DocConfig`],
//! `sha2` + `hex` for fingerprints.
//! Consumed by: `sync` (load/save per cycle), `daemon` (status, pending deletes),
//! `apply` (only through [`StateStore::journal_path`]).
//!
//! # Layout under the state directory (keep it outside the vault, §9.1)
//!
//! ```text
//! <state_dir>/
//!   daemon-events.json       the daemon's event log (`daemon::EVENTS_FILE`, written by it)
//!   roots/<root uuid>/
//!     checkpoint.json        Checkpoint (pretty JSON), replaced atomically
//!     pending.json           PendingDeletes, replaced atomically (see below)
//!     history/<gen>.json     HistoryEntry per render generation, pruned by HistoryLimits
//!     journal.jsonl          apply's write-ahead journal (exists only mid-cycle)
//!     lock                   held (File::try_lock) while a cycle runs on this root
//! ```
//!
//! The directory name is the root id itself when it consists only of ASCII letters,
//! digits, `-` and `_` (every UUID does); any other byte is written as `%XX` so that an
//! unexpected id can never escape the state directory.
//!
//! # Rules
//!
//! * Every write is atomic: [`atomic_write`] (temp file in the same directory, fsync,
//!   rename, fsync the directory). A crash leaves at worst a stale temp file
//!   (`.<name>.tmp-<pid>-<n>`), which every reader ignores and [`StateStore::lock`]
//!   removes. A symbolic link at the written path is kept (the file it names is
//!   replaced, [`write_target`]) and a replaced file keeps its permission bits.
//! * [`StateStore::load_checkpoint`] rejects a `version` other than
//!   [`CHECKPOINT_VERSION`] with [`StateError::Version`] (no silent migration). The
//!   version is read before the rest of the file is decoded, so a newer format reports
//!   a version error rather than a confusing decoding error.
//! * Forward compatibility: unknown JSON fields are ignored, and every field that is not
//!   essential has a serde default, so a newer engine's additive fields do not break an
//!   older reader and an older file loads in a newer engine.
//! * Order within a cycle (crash safety, see `apply` and `sync`): history entry of the new
//!   generation first, then the Markdown file (CAS), then the checkpoint, then the journal
//!   commit.
//! * Pending deletes change on cycles that do not write the file (a cycle with the file
//!   unchanged is exactly what makes a delete due, §9.1 guard 2), while the checkpoint
//!   changes only when the file is written (§9.1 guard 1). They are therefore also kept
//!   in `pending.json`, written by [`StateStore::save_pending`]. `pending.json`, when it
//!   exists, is authoritative: [`StateStore::load_checkpoint`] overlays it onto
//!   [`Checkpoint::pending_deletes`], and [`StateStore::save_checkpoint`] writes it before
//!   the checkpoint so the two never disagree in a way that loses the newer set.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions, TryLockError};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::mdblock::DocConfig;
use crate::tree::{IdMap, NodeId, Tree};

/// Current checkpoint format version.
pub const CHECKPOINT_VERSION: u32 = 1;

/// File names inside a root directory.
const CHECKPOINT_FILE: &str = "checkpoint.json";
const PENDING_FILE: &str = "pending.json";
const HISTORY_DIR: &str = "history";
const JOURNAL_FILE: &str = "journal.jsonl";
const LOCK_FILE: &str = "lock";
const ROOTS_DIR: &str = "roots";
/// Marker inside temp file names written by [`atomic_write`].
const TEMP_MARKER: &str = ".tmp-";

/// The current time in unix seconds (the default clock; everything in this module that
/// depends on time takes `now` explicitly so tests can inject it).
pub fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------------------
// Fingerprints
// ---------------------------------------------------------------------------------------

/// SHA-256 of a text, lowercase hex.
#[derive(Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Fingerprint(pub String);

impl Fingerprint {
    /// The hex string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for Fingerprint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// The fingerprint of `text` (its UTF-8 bytes, exactly: no normalisation of line endings
/// or trailing whitespace, because §5.1's fast path is "byte-identical to `base_md`").
pub fn fingerprint(text: &str) -> Fingerprint {
    Fingerprint(hex::encode(Sha256::digest(text.as_bytes())))
}

// ---------------------------------------------------------------------------------------
// Pending deletes (§9.1 guard 2)
// ---------------------------------------------------------------------------------------

/// The number of cycles with the file unchanged that a pending delete must survive
/// before it is sent ("at least one more sync cycle in which that file is unchanged",
/// §9.1 guard 2).
pub const MIN_UNCHANGED_CYCLES: u32 = 1;

/// One delete waiting for its grace period (§9.1 guard 2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingDelete {
    /// The node.
    pub node: NodeId,
    /// When the delete was first seen (unix seconds).
    pub first_seen: i64,
    /// Fingerprint of the file when last seen.
    pub fingerprint: Fingerprint,
    /// Consecutive cycles in which the file was unchanged since the delete was seen.
    #[serde(default)]
    pub unchanged_cycles: u32,
    /// Hidden descendants that the delete takes along (for the status note).
    #[serde(default)]
    pub hidden_descendants: usize,
    /// Plain-text label for the status note.
    #[serde(default)]
    pub label: String,
    /// When the delete was last seen (unix seconds); equals `first_seen` at first.
    #[serde(default)]
    pub last_seen: i64,
}

impl PendingDelete {
    /// The earliest time the delete may be sent with a grace period of `grace_secs`.
    pub fn deadline(&self, grace_secs: i64) -> i64 {
        self.first_seen.saturating_add(grace_secs)
    }

    /// True if the grace period has passed at `now` and the file has been unchanged for
    /// at least `min_cycles` cycles since the delete was seen.
    pub fn is_due(&self, now: i64, grace_secs: i64, min_cycles: u32) -> bool {
        now >= self.deadline(grace_secs) && self.unchanged_cycles >= min_cycles
    }
}

/// Pending deletes of one root, keyed by node.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingDeletes {
    /// The entries.
    #[serde(default)]
    pub entries: BTreeMap<NodeId, PendingDelete>,
}

impl PendingDeletes {
    /// Record this cycle's local deletes `(node, hidden_descendants, label)` seen in a
    /// file with fingerprint `fp` at `now`: new nodes are added; existing ones get
    /// `unchanged_cycles += 1` if `fp` equals their stored fingerprint, else reset to 0
    /// with the new fingerprint. The label and hidden count are refreshed. Entries not
    /// listed are left alone: cancelling is the caller's decision ([`Self::remove`]).
    ///
    /// Call this once per cycle per file: two calls with the same `fp` count as two
    /// unchanged cycles.
    pub fn observe(&mut self, deletes: &[(NodeId, usize, String)], fp: &Fingerprint, now: i64) {
        for (node, hidden, label) in deletes {
            match self.entries.get_mut(node) {
                Some(e) => {
                    if &e.fingerprint == fp {
                        e.unchanged_cycles = e.unchanged_cycles.saturating_add(1);
                    } else {
                        e.fingerprint = fp.clone();
                        e.unchanged_cycles = 0;
                    }
                    e.hidden_descendants = *hidden;
                    e.label = label.clone();
                    e.last_seen = now;
                }
                None => {
                    self.entries.insert(
                        node.clone(),
                        PendingDelete {
                            node: node.clone(),
                            first_seen: now,
                            fingerprint: fp.clone(),
                            unchanged_cycles: 0,
                            hidden_descendants: *hidden,
                            label: label.clone(),
                            last_seen: now,
                        },
                    );
                }
            }
        }
    }

    /// Nodes whose delete is due: older than `grace_secs` **and** at least one cycle with
    /// the file unchanged (§9.1 guard 2). In node order.
    pub fn due(&self, now: i64, grace_secs: i64) -> Vec<NodeId> {
        self.due_with(now, grace_secs, MIN_UNCHANGED_CYCLES)
    }

    /// [`Self::due`] with an explicit number of unchanged cycles to survive.
    pub fn due_with(&self, now: i64, grace_secs: i64, min_cycles: u32) -> Vec<NodeId> {
        self.entries
            .values()
            .filter(|e| e.is_due(now, grace_secs, min_cycles))
            .map(|e| e.node.clone())
            .collect()
    }

    /// Forget a node (its delete was sent, or cancelled because it reappeared or moved).
    pub fn remove(&mut self, node: &NodeId) -> Option<PendingDelete> {
        self.entries.remove(node)
    }

    /// Forget every node not in `keep` (for example the deletes no longer seen this
    /// cycle); returns the removed entries.
    pub fn retain_only(&mut self, keep: &BTreeSet<NodeId>) -> Vec<PendingDelete> {
        let gone: Vec<NodeId> = self
            .entries
            .keys()
            .filter(|n| !keep.contains(*n))
            .cloned()
            .collect();
        gone.iter().filter_map(|n| self.entries.remove(n)).collect()
    }

    /// True if `node` is pending.
    pub fn contains(&self, node: &NodeId) -> bool {
        self.entries.contains_key(node)
    }

    /// The entry for `node`.
    pub fn get(&self, node: &NodeId) -> Option<&PendingDelete> {
        self.entries.get(node)
    }

    /// Number of pending deletes.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True if nothing is pending.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The entries in node order.
    pub fn iter(&self) -> impl Iterator<Item = &PendingDelete> {
        self.entries.values()
    }
}

// ---------------------------------------------------------------------------------------
// Checkpoint (§3, §5.1)
// ---------------------------------------------------------------------------------------

/// The state of one sync root after its last completed cycle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Checkpoint {
    /// [`CHECKPOINT_VERSION`].
    pub version: u32,
    /// The sync root.
    pub root: NodeId,
    /// The generation of `base_md` (§9.1 guard 1).
    pub generation: u64,
    /// The root's subtree as of the last sync, hidden nodes included (§4.5).
    pub base_wf: Tree,
    /// The exact text of the Markdown we last wrote (§5.1).
    pub base_md: String,
    /// `fingerprint(base_md)`: the fast path "file unchanged" (§5.1). Recomputed on load
    /// if missing or inconsistent with `base_md`.
    #[serde(default)]
    pub md_fingerprint: Fingerprint,
    /// Short id -> full id for every node we have seen (base and export).
    #[serde(default)]
    pub ids: IdMap,
    /// The config the file was rendered with. A value this engine cannot decode (for
    /// example one written by a newer engine with a changed shape) loads as the default
    /// with a warning rather than failing the whole checkpoint.
    #[serde(default, deserialize_with = "lenient_config")]
    pub config: DocConfig,
    /// The file the root was last synced from (informational; files are found by root,
    /// §9.1 guard 5).
    #[serde(default)]
    pub file: Option<PathBuf>,
    /// Unix seconds of the last completed cycle.
    #[serde(default)]
    pub synced_at: i64,
    /// Unix seconds of the root's first checkpoint (0 if unknown).
    #[serde(default)]
    pub created_at: i64,
    /// Deferred deletes (§9.1 guard 2). `pending.json` overrides this on load.
    #[serde(default)]
    pub pending_deletes: PendingDeletes,
}

fn lenient_config<'de, D: Deserializer<'de>>(d: D) -> Result<DocConfig, D::Error> {
    let v = serde_json::Value::deserialize(d)?;
    match serde_json::from_value::<DocConfig>(v) {
        Ok(c) => Ok(c),
        Err(e) => {
            // UNVERIFIED: falling back to the default config (rather than refusing the
            // checkpoint) is a choice; the file's frontmatter is the source of truth for
            // the config, the checkpoint only echoes it.
            log::warn!("checkpoint config not understood ({e}); using defaults");
            Ok(DocConfig::default())
        }
    }
}

impl Checkpoint {
    /// A checkpoint at `now` for generation `generation`: `md_fingerprint` is computed
    /// from `base_md` and `ids` from `base_wf`; no file, no pending deletes.
    pub fn new(
        root: NodeId,
        generation: u64,
        base_wf: Tree,
        base_md: String,
        config: DocConfig,
        now: i64,
    ) -> Checkpoint {
        let ids = IdMap::from_tree(&base_wf);
        Checkpoint {
            version: CHECKPOINT_VERSION,
            root,
            generation,
            md_fingerprint: fingerprint(&base_md),
            base_wf,
            base_md,
            ids,
            config,
            file: None,
            synced_at: now,
            created_at: now,
            pending_deletes: PendingDeletes::default(),
        }
    }

    /// Replace `base_md` and keep `md_fingerprint` consistent with it.
    pub fn set_base_md(&mut self, md: String) {
        self.md_fingerprint = fingerprint(&md);
        self.base_md = md;
    }

    /// True if `text` is byte-identical to `base_md` (§5.1 fast path), by fingerprint.
    pub fn md_unchanged(&self, text: &str) -> bool {
        fingerprint(text) == self.md_fingerprint
    }
}

// ---------------------------------------------------------------------------------------
// Render history (§9.1 guard 1, guard 3)
// ---------------------------------------------------------------------------------------

/// One render generation (§9.1 guard 1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryEntry {
    /// The generation stamped in the file.
    pub generation: u64,
    /// The exact Markdown text written.
    pub md: String,
    /// The root's subtree it was rendered from.
    pub tree: Tree,
    /// Unix seconds.
    #[serde(default)]
    pub at: i64,
}

impl HistoryEntry {
    /// The fingerprint of `md`.
    pub fn fingerprint(&self) -> Fingerprint {
        fingerprint(&self.md)
    }
}

/// How much history to keep (§9.1: "say, 30 days or 200 generations"). An entry is
/// pruned when it is beyond `max_entries` newest **or** older than `max_age_secs`, but the
/// newest entry is never pruned.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryLimits {
    /// Default 200.
    pub max_entries: usize,
    /// Default 30 days.
    pub max_age_secs: i64,
}

impl Default for HistoryLimits {
    fn default() -> Self {
        HistoryLimits {
            max_entries: 200,
            max_age_secs: 30 * 24 * 3600,
        }
    }
}

/// The generations to prune from `(generation, at)` pairs, per `limits` at `now`; never
/// the newest generation nor any in `keep`. Ascending.
pub fn gens_to_prune(
    entries: &[(u64, i64)],
    limits: &HistoryLimits,
    now: i64,
    keep: &BTreeSet<u64>,
) -> Vec<u64> {
    let mut sorted: Vec<(u64, i64)> = entries.to_vec();
    sorted.sort_unstable();
    sorted.dedup_by_key(|e| e.0);
    let Some(&(newest, _)) = sorted.last() else {
        return Vec::new();
    };
    let n = sorted.len();
    sorted
        .iter()
        .enumerate()
        .filter(|&(i, &(g, at))| {
            if g == newest || keep.contains(&g) {
                return false;
            }
            let rank_from_newest = n - 1 - i; // 0 = newest
            let too_many = rank_from_newest >= limits.max_entries;
            let too_old = now.saturating_sub(at) > limits.max_age_secs;
            too_many || too_old
        })
        .map(|(_, &(g, _))| g)
        .collect()
}

/// The values one node had in one generation of the history (for §9.1 guard 3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeVersion {
    /// The generation.
    pub generation: u64,
    /// Raw WF-HTML name.
    pub name: String,
    /// Raw WF-HTML note.
    pub note: Option<String>,
    /// `layoutMode`.
    pub layout: String,
    /// Completion flag.
    pub completed: bool,
    /// Parent id.
    pub parent_id: Option<NodeId>,
}

/// Several generations of one root's render history, in memory (a bounded view of
/// `history/`, loaded by [`StateStore::load_history`]).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct History {
    entries: BTreeMap<u64, HistoryEntry>,
}

impl History {
    /// An empty history.
    pub fn new() -> History {
        History::default()
    }

    /// Add (or replace) an entry.
    pub fn insert(&mut self, entry: HistoryEntry) {
        self.entries.insert(entry.generation, entry);
    }

    /// The entry of `generation`, if kept.
    pub fn get(&self, generation: u64) -> Option<&HistoryEntry> {
        self.entries.get(&generation)
    }

    /// The newest entry.
    pub fn latest(&self) -> Option<&HistoryEntry> {
        self.entries.values().next_back()
    }

    /// The oldest kept entry.
    pub fn oldest(&self) -> Option<&HistoryEntry> {
        self.entries.values().next()
    }

    /// Generations kept, ascending.
    pub fn gens(&self) -> Vec<u64> {
        self.entries.keys().copied().collect()
    }

    /// Number of entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True if empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Entries in ascending generation order.
    pub fn iter(&self) -> impl Iterator<Item = &HistoryEntry> {
        self.entries.values()
    }

    /// The newest entry whose Markdown text is byte-identical to `text` (a file that
    /// lost its `gen` line but is otherwise an earlier render).
    pub fn find_by_fingerprint(&self, fp: &Fingerprint) -> Option<&HistoryEntry> {
        self.entries.values().rev().find(|e| &e.fingerprint() == fp)
    }

    /// Every value `node` had in the kept generations, ascending by generation, with
    /// consecutive duplicates (same name, note, layout, completion and parent) collapsed
    /// to their first generation. For §9.1 guard 3 ("equals a value the engine rendered
    /// for that node in an earlier generation"). The trees are the remote trees each
    /// generation was rendered from, so these are WF values; whether the node was
    /// visible in that render is for the caller (`mdblock::View`) to decide.
    pub fn known_values(&self, node: &NodeId) -> Vec<NodeVersion> {
        let mut out: Vec<NodeVersion> = Vec::new();
        for e in self.entries.values() {
            let Some(n) = e.tree.get(node) else { continue };
            let v = NodeVersion {
                generation: e.generation,
                name: n.name.clone(),
                note: n.note.clone(),
                layout: n.layout.clone(),
                completed: n.completed,
                parent_id: n.parent_id.clone(),
            };
            if let Some(last) = out.last()
                && last.name == v.name
                && last.note == v.note
                && last.layout == v.layout
                && last.completed == v.completed
                && last.parent_id == v.parent_id
            {
                continue;
            }
            out.push(v);
        }
        out
    }

    /// The distinct names `node` had across the kept generations (ascending by first
    /// appearance).
    pub fn known_names(&self, node: &NodeId) -> Vec<String> {
        let mut seen = BTreeSet::new();
        self.known_values(node)
            .into_iter()
            .filter_map(|v| seen.insert(v.name.clone()).then_some(v.name))
            .collect()
    }

    /// The distinct notes `node` had across the kept generations.
    pub fn known_notes(&self, node: &NodeId) -> Vec<Option<String>> {
        let mut seen = BTreeSet::new();
        self.known_values(node)
            .into_iter()
            .filter_map(|v| seen.insert(v.note.clone()).then_some(v.note))
            .collect()
    }

    /// Remove generations per `limits` at `now` (never the newest, never one in `keep`);
    /// returns the pruned gens, ascending.
    pub fn prune(&mut self, limits: &HistoryLimits, now: i64, keep: &BTreeSet<u64>) -> Vec<u64> {
        let pairs: Vec<(u64, i64)> = self
            .entries
            .values()
            .map(|e| (e.generation, e.at))
            .collect();
        let pruned = gens_to_prune(&pairs, limits, now, keep);
        for g in &pruned {
            self.entries.remove(g);
        }
        pruned
    }
}

// ---------------------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------------------

/// State errors.
#[derive(Debug, Error)]
pub enum StateError {
    /// Filesystem error.
    #[error("{path}: {source}")]
    Io {
        /// The file or directory.
        path: PathBuf,
        /// The error.
        source: std::io::Error,
    },
    /// JSON (de)serialisation error.
    #[error("{path}: {message}")]
    Json {
        /// The file.
        path: PathBuf,
        /// Description.
        message: String,
    },
    /// Unsupported checkpoint version.
    #[error("{path}: checkpoint version {found}, expected {expected}")]
    Version {
        /// The file.
        path: PathBuf,
        /// Version found.
        found: u32,
        /// [`CHECKPOINT_VERSION`].
        expected: u32,
    },
    /// Another process holds the root's lock (§9: run exactly one engine).
    #[error("root {0} is locked by another process")]
    Locked(NodeId),
}

fn io_err(path: &Path) -> impl FnOnce(io::Error) -> StateError + '_ {
    move |source| StateError::Io {
        path: path.to_path_buf(),
        source,
    }
}

// ---------------------------------------------------------------------------------------
// The store
// ---------------------------------------------------------------------------------------

/// A held per-root lock; released on drop.
#[derive(Debug)]
pub struct RootLock {
    file: File,
}

impl Drop for RootLock {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

/// The state directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateStore {
    dir: PathBuf,
}

impl StateStore {
    /// Open (creating directories as needed) the store at `dir`.
    pub fn open(dir: &Path) -> Result<StateStore, StateError> {
        let roots = dir.join(ROOTS_DIR);
        fs::create_dir_all(&roots).map_err(io_err(&roots))?;
        Ok(StateStore {
            dir: dir.to_path_buf(),
        })
    }

    /// Open the store at `dir` without creating anything (for read-only commands such as
    /// `status`): `Ok(None)` if the directory does not exist.
    pub fn open_existing(dir: &Path) -> Result<Option<StateStore>, StateError> {
        match fs::metadata(dir) {
            Ok(_) => Ok(Some(StateStore {
                dir: dir.to_path_buf(),
            })),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(io_err(dir)(e)),
        }
    }

    /// The store's directory.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// `<dir>/roots/<root uuid>` (created on first save).
    pub fn root_dir(&self, root: &NodeId) -> PathBuf {
        self.dir
            .join(ROOTS_DIR)
            .join(encode_dir_name(root.as_str()))
    }

    /// `<root_dir>/checkpoint.json`.
    pub fn checkpoint_path(&self, root: &NodeId) -> PathBuf {
        self.root_dir(root).join(CHECKPOINT_FILE)
    }

    /// `<root_dir>/pending.json`.
    pub fn pending_path(&self, root: &NodeId) -> PathBuf {
        self.root_dir(root).join(PENDING_FILE)
    }

    /// `<root_dir>/history`.
    pub fn history_dir(&self, root: &NodeId) -> PathBuf {
        self.root_dir(root).join(HISTORY_DIR)
    }

    /// `<root_dir>/history/<gen>.json`.
    pub fn history_path(&self, root: &NodeId, generation: u64) -> PathBuf {
        self.history_dir(root).join(format!("{generation}.json"))
    }

    /// `<root_dir>/lock`.
    pub fn lock_path(&self, root: &NodeId) -> PathBuf {
        self.root_dir(root).join(LOCK_FILE)
    }

    /// `<root_dir>/journal.jsonl`.
    pub fn journal_path(&self, root: &NodeId) -> PathBuf {
        self.root_dir(root).join(JOURNAL_FILE)
    }

    fn ensure_root_dir(&self, root: &NodeId) -> Result<PathBuf, StateError> {
        let d = self.root_dir(root);
        fs::create_dir_all(&d).map_err(io_err(&d))?;
        Ok(d)
    }

    /// Every root that has a checkpoint, in id order.
    pub fn roots(&self) -> Result<Vec<NodeId>, StateError> {
        let roots_dir = self.dir.join(ROOTS_DIR);
        let rd = match fs::read_dir(&roots_dir) {
            Ok(rd) => rd,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(io_err(&roots_dir)(e)),
        };
        let mut out = Vec::new();
        for ent in rd {
            let ent = ent.map_err(io_err(&roots_dir))?;
            let Some(name) = ent.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            let Some(id) = decode_dir_name(&name) else {
                continue;
            };
            if ent.path().join(CHECKPOINT_FILE).is_file() {
                out.push(NodeId::from_api(id));
            }
        }
        out.sort();
        Ok(out)
    }

    /// Take the root's lock without blocking (`File::try_lock`). While holding it, stale
    /// temp files left by a crashed [`atomic_write`] in the root's directories are
    /// removed (nobody else can be writing them).
    pub fn lock(&self, root: &NodeId) -> Result<RootLock, StateError> {
        self.ensure_root_dir(root)?;
        let path = self.lock_path(root);
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&path)
            .map_err(io_err(&path))?;
        match file.try_lock() {
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => return Err(StateError::Locked(root.clone())),
            Err(TryLockError::Error(e)) => return Err(io_err(&path)(e)),
        }
        let lock = RootLock { file };
        for d in [self.root_dir(root), self.history_dir(root)] {
            remove_stale_temps(&d);
        }
        Ok(lock)
    }

    /// Load the checkpoint; `Ok(None)` if the root has none. `pending.json`, if present,
    /// replaces the checkpoint's pending deletes.
    pub fn load_checkpoint(&self, root: &NodeId) -> Result<Option<Checkpoint>, StateError> {
        let path = self.checkpoint_path(root);
        let Some(bytes) = read_optional(&path)? else {
            return Ok(None);
        };
        let value: serde_json::Value =
            serde_json::from_slice(&bytes).map_err(|e| StateError::Json {
                path: path.clone(),
                message: e.to_string(),
            })?;
        let found = value
            .get("version")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| StateError::Json {
                path: path.clone(),
                message: "missing or invalid \"version\"".to_owned(),
            })?;
        if found != u64::from(CHECKPOINT_VERSION) {
            return Err(StateError::Version {
                path,
                found: u32::try_from(found).unwrap_or(u32::MAX),
                expected: CHECKPOINT_VERSION,
            });
        }
        let mut cp: Checkpoint = serde_json::from_value(value).map_err(|e| StateError::Json {
            path: path.clone(),
            message: e.to_string(),
        })?;
        if &cp.root != root {
            return Err(StateError::Json {
                path,
                message: format!("checkpoint is for root {}, expected {root}", cp.root),
            });
        }
        let fp = fingerprint(&cp.base_md);
        if cp.md_fingerprint != fp {
            if !cp.md_fingerprint.0.is_empty() {
                log::warn!(
                    "{}: md_fingerprint does not match base_md; recomputed",
                    path.display()
                );
            }
            cp.md_fingerprint = fp;
        }
        if let Some(p) = self.load_pending_file(root)? {
            cp.pending_deletes = p;
        }
        Ok(Some(cp))
    }

    /// Save the checkpoint atomically (and its pending deletes to `pending.json`, first).
    pub fn save_checkpoint(&self, cp: &Checkpoint) -> Result<(), StateError> {
        self.ensure_root_dir(&cp.root)?;
        self.save_pending(&cp.root, &cp.pending_deletes)?;
        let mut cp = cp.clone();
        cp.version = CHECKPOINT_VERSION;
        cp.md_fingerprint = fingerprint(&cp.base_md);
        write_json(&self.checkpoint_path(&cp.root), &cp)
    }

    /// The root's pending deletes: `pending.json`, else the checkpoint's, else empty.
    pub fn load_pending(&self, root: &NodeId) -> Result<PendingDeletes, StateError> {
        if let Some(p) = self.load_pending_file(root)? {
            return Ok(p);
        }
        Ok(self
            .load_checkpoint(root)?
            .map(|cp| cp.pending_deletes)
            .unwrap_or_default())
    }

    fn load_pending_file(&self, root: &NodeId) -> Result<Option<PendingDeletes>, StateError> {
        read_json(&self.pending_path(root))
    }

    /// Save the root's pending deletes atomically to `pending.json` (on a cycle that does
    /// not write the checkpoint).
    pub fn save_pending(&self, root: &NodeId, pending: &PendingDeletes) -> Result<(), StateError> {
        self.ensure_root_dir(root)?;
        write_json(&self.pending_path(root), pending)
    }

    /// Save a history entry atomically (overwrites the same gen).
    pub fn put_history(&self, root: &NodeId, entry: &HistoryEntry) -> Result<(), StateError> {
        let d = self.history_dir(root);
        fs::create_dir_all(&d).map_err(io_err(&d))?;
        write_json(&self.history_path(root, entry.generation), entry)
    }

    /// Load one generation; `Ok(None)` if not kept.
    pub fn get_history(
        &self,
        root: &NodeId,
        generation: u64,
    ) -> Result<Option<HistoryEntry>, StateError> {
        let path = self.history_path(root, generation);
        let entry: Option<HistoryEntry> = read_json(&path)?;
        if let Some(e) = &entry
            && e.generation != generation
        {
            return Err(StateError::Json {
                path,
                message: format!("history entry has generation {}", e.generation),
            });
        }
        Ok(entry)
    }

    /// Generations kept, ascending.
    pub fn history_gens(&self, root: &NodeId) -> Result<Vec<u64>, StateError> {
        let d = self.history_dir(root);
        let rd = match fs::read_dir(&d) {
            Ok(rd) => rd,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(io_err(&d)(e)),
        };
        let mut out = Vec::new();
        for ent in rd {
            let ent = ent.map_err(io_err(&d))?;
            let name = ent.file_name();
            let Some(name) = name.to_str() else { continue };
            let Some(stem) = name.strip_suffix(".json") else {
                continue;
            };
            // Only canonical decimal names: "007.json" or ".7.json.tmp-…" are not ours.
            if stem.is_empty() || !stem.bytes().all(|b| b.is_ascii_digit()) {
                continue;
            }
            if let Ok(g) = stem.parse::<u64>()
                && g.to_string() == stem
            {
                out.push(g);
            }
        }
        out.sort_unstable();
        Ok(out)
    }

    /// Load every kept generation into memory.
    pub fn load_history(&self, root: &NodeId) -> Result<History, StateError> {
        let mut h = History::new();
        for g in self.history_gens(root)? {
            if let Some(e) = self.get_history(root, g)? {
                h.insert(e);
            }
        }
        Ok(h)
    }

    /// Delete generations per `limits` at time `now`; returns the pruned gens. The newest
    /// generation and the current checkpoint's generation are never pruned (the latter
    /// matters after a crash between the history entry and the checkpoint, when the
    /// newest entry is one ahead of the checkpoint).
    pub fn prune_history(
        &self,
        root: &NodeId,
        limits: &HistoryLimits,
        now: i64,
    ) -> Result<Vec<u64>, StateError> {
        let mut keep = BTreeSet::new();
        if let Some(cp) = self.load_checkpoint(root)? {
            keep.insert(cp.generation);
        }
        self.prune_history_keeping(root, limits, now, &keep)
    }

    /// [`Self::prune_history`] with an explicit set of generations to keep (and without
    /// reading the checkpoint).
    pub fn prune_history_keeping(
        &self,
        root: &NodeId,
        limits: &HistoryLimits,
        now: i64,
        keep: &BTreeSet<u64>,
    ) -> Result<Vec<u64>, StateError> {
        let mut pairs = Vec::new();
        for g in self.history_gens(root)? {
            // Only the timestamp is needed; a whole entry is small enough to parse.
            let path = self.history_path(root, g);
            let at: Option<HistoryStamp> = read_json(&path)?;
            if let Some(s) = at {
                pairs.push((g, s.at));
            }
        }
        let pruned = gens_to_prune(&pairs, limits, now, keep);
        for g in &pruned {
            let path = self.history_path(root, *g);
            match fs::remove_file(&path) {
                Ok(()) => {}
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(e) => return Err(io_err(&path)(e)),
            }
        }
        if !pruned.is_empty() {
            let d = self.history_dir(root);
            fsync_dir(&d).map_err(io_err(&d))?;
        }
        Ok(pruned)
    }
}

/// The part of a history file that pruning reads.
#[derive(Deserialize)]
struct HistoryStamp {
    #[serde(default)]
    at: i64,
}

fn read_optional(path: &Path) -> Result<Option<Vec<u8>>, StateError> {
    match fs::read(path) {
        Ok(b) => Ok(Some(b)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(io_err(path)(e)),
    }
}

fn read_json<T: DeserializeOwned>(path: &Path) -> Result<Option<T>, StateError> {
    let Some(bytes) = read_optional(path)? else {
        return Ok(None);
    };
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|e| StateError::Json {
            path: path.to_path_buf(),
            message: e.to_string(),
        })
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), StateError> {
    let mut bytes = serde_json::to_vec_pretty(value).map_err(|e| StateError::Json {
        path: path.to_path_buf(),
        message: e.to_string(),
    })?;
    bytes.push(b'\n');
    atomic_write(path, &bytes).map_err(io_err(path))
}

/// Encode a root id as a directory name (see the module docs).
fn encode_dir_name(id: &str) -> String {
    let mut s = String::with_capacity(id.len());
    for b in id.bytes() {
        if b.is_ascii_alphanumeric() || b == b'-' || b == b'_' {
            s.push(b as char);
        } else {
            s.push_str(&format!("%{b:02X}"));
        }
    }
    if s.is_empty() {
        s.push('%'); // the empty id; decodes back to ""
    }
    s
}

fn decode_dir_name(name: &str) -> Option<String> {
    if name == "%" {
        return Some(String::new());
    }
    let bytes = name.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' => {
                let h = name.get(i + 1..i + 3)?;
                out.push(u8::from_str_radix(h, 16).ok()?);
                i += 3;
            }
            b if b.is_ascii_alphanumeric() || b == b'-' || b == b'_' => {
                out.push(b);
                i += 1;
            }
            _ => return None, // temp files, stray names
        }
    }
    String::from_utf8(out).ok()
}

fn remove_stale_temps(dir: &Path) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    for ent in rd.flatten() {
        let name = ent.file_name();
        let Some(name) = name.to_str() else { continue };
        if name.starts_with('.') && name.contains(TEMP_MARKER) {
            let _ = fs::remove_file(ent.path());
        }
    }
}

// ---------------------------------------------------------------------------------------
// Atomic writes
// ---------------------------------------------------------------------------------------

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

fn fsync_dir(dir: &Path) -> io::Result<()> {
    File::open(dir)?.sync_all()
}

/// Give `tmp` (a temp file about to be renamed over `target`) the permission bits of the
/// file it replaces, so that a rewrite does not reset them to the umask default (a
/// private `0600` note would become `0644`). Nothing to copy when `target` does not
/// exist yet.
pub(crate) fn keep_permissions(target: &Path, tmp: &File) -> io::Result<()> {
    match fs::metadata(target) {
        Ok(m) => tmp.set_permissions(m.permissions()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

/// The file a write to `path` must replace: `path` itself, or, when `path` is a symbolic
/// link (or a chain of them), the file the last link names (which need not exist yet).
/// Renaming a temp file over a link would replace the link with a regular file, and the
/// file it pointed to would silently stop being the synced one (a vault file linked from
/// elsewhere: the daemon's discovery follows links to files). Directory links in the
/// parent path need no resolving: a rename goes through them.
pub fn write_target(path: &Path) -> io::Result<PathBuf> {
    let mut p = path.to_path_buf();
    // The same bound as Linux's ELOOP for path resolution.
    for _ in 0..40 {
        match fs::symlink_metadata(&p) {
            Ok(m) if m.file_type().is_symlink() => {
                let link = fs::read_link(&p)?;
                p = match p.parent() {
                    Some(dir) if link.is_relative() => dir.join(link),
                    _ => link,
                };
            }
            Ok(_) => return Ok(p),
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(p),
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::InvalidInput,
        format!("{}: too many levels of symbolic links", path.display()),
    ))
}

/// Write `bytes` to `path` atomically: a temp file in the same directory, fsync, rename
/// over `path`, fsync the directory. The temp file is `.<file name>.tmp-<pid>-<n>`; it is
/// removed if any step before the rename fails. The parent directory must exist.
///
/// A symbolic link at `path` is kept: the file it names is replaced instead
/// ([`write_target`]), with the temp file in that file's directory. A replaced file
/// keeps its permission bits (the temp file gets them before the rename); its owner is
/// not copied.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let path = &write_target(path)?;
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "path has no file name"))?;
    let dir = match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => PathBuf::from("."),
    };
    let n = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let tmp = dir.join(format!(
        ".{file_name}{TEMP_MARKER}{}-{n}",
        std::process::id()
    ));
    let result = (|| {
        let mut f = OpenOptions::new().write(true).create_new(true).open(&tmp)?;
        f.write_all(bytes)?;
        keep_permissions(path, &f)?;
        f.sync_all()?;
        drop(f);
        fs::rename(&tmp, path)
    })();
    if let Err(e) = result {
        let _ = fs::remove_file(&tmp);
        return Err(e);
    }
    fsync_dir(&dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dir_names_round_trip() {
        for id in [
            "6f1b1a2e-1111-4222-8333-944455556666",
            "",
            "a/b",
            "..",
            "temp-3",
            "ü%",
        ] {
            let enc = encode_dir_name(id);
            assert!(!enc.contains('/') && enc != "." && enc != "..", "{enc}");
            assert_eq!(decode_dir_name(&enc).as_deref(), Some(id), "{enc}");
        }
        assert_eq!(decode_dir_name(".x.tmp-1-2"), None);
    }

    #[test]
    fn prune_rule() {
        let lim = HistoryLimits {
            max_entries: 2,
            max_age_secs: 100,
        };
        let e = [(1, 0), (2, 50), (3, 90), (4, 95)];
        assert_eq!(gens_to_prune(&e, &lim, 100, &BTreeSet::new()), vec![1, 2]);
        // Everything old: the newest survives.
        assert_eq!(
            gens_to_prune(&e, &lim, 10_000, &BTreeSet::new()),
            vec![1, 2, 3]
        );
        assert_eq!(gens_to_prune(&e, &lim, 100, &BTreeSet::from([1])), vec![2]);
        assert!(gens_to_prune(&[], &lim, 0, &BTreeSet::new()).is_empty());
    }
}
