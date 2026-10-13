//! Checkpoints, render history, pending deletes, fingerprints, and their on-disk store.
//!
//! Responsibility (design §3 checkpoint, §5.1 `base_md`/`base_wf`, §9.1 guard 1 render
//! generations and history, guard 2 pending deletes, §9 "one engine": a per-root lock).
//! I/O lives in [`StateStore`]; the data types are plain values.
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
//!   roots/<root uuid>/
//!     checkpoint.json        Checkpoint (pretty JSON), replaced atomically
//!     history/<gen>.json     HistoryEntry per render generation, pruned by HistoryLimits
//!     journal.jsonl          apply's write-ahead journal (exists only mid-cycle)
//!     lock                   held (File::try_lock) while a cycle runs on this root
//! ```
//!
//! # Rules
//!
//! * Every write is atomic: [`atomic_write`] (temp file in the same directory, fsync,
//!   rename, fsync the directory).
//! * [`StateStore::load_checkpoint`] rejects a `version` other than
//!   [`CHECKPOINT_VERSION`] with [`StateError::Version`] (no silent migration).
//! * Order within a cycle (crash safety, see `apply` and `sync`): history entry of the new
//!   generation first, then the Markdown file (CAS), then the checkpoint, then the journal
//!   commit.

#![allow(unused_variables, dead_code)] // stubs; remove when implemented

use std::collections::BTreeMap;
use std::fs::File;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::mdblock::DocConfig;
use crate::tree::{IdMap, NodeId, Tree};

/// Current checkpoint format version.
pub const CHECKPOINT_VERSION: u32 = 1;

/// SHA-256 of a text, lowercase hex.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Fingerprint(pub String);

/// The fingerprint of `text` (its UTF-8 bytes).
pub fn fingerprint(text: &str) -> Fingerprint {
    todo!()
}

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
    pub unchanged_cycles: u32,
    /// Hidden descendants that the delete takes along (for the status note).
    pub hidden_descendants: usize,
    /// Plain-text label for the status note.
    pub label: String,
}

/// Pending deletes of one root, keyed by node.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingDeletes {
    /// The entries.
    pub entries: BTreeMap<NodeId, PendingDelete>,
}

impl PendingDeletes {
    /// Record this cycle's local deletes `(node, hidden_descendants, label)` seen in a
    /// file with fingerprint `fp` at `now`: new nodes are added; existing ones get
    /// `unchanged_cycles += 1` if `fp` equals their stored fingerprint, else reset to 0
    /// with the new fingerprint.
    pub fn observe(&mut self, deletes: &[(NodeId, usize, String)], fp: &Fingerprint, now: i64) {
        todo!()
    }

    /// Nodes whose delete is due: older than `grace_secs` **and** at least one cycle with
    /// the file unchanged (§9.1 guard 2).
    pub fn due(&self, now: i64, grace_secs: i64) -> Vec<NodeId> {
        todo!()
    }

    /// Forget a node (its delete was sent, or cancelled because it reappeared or moved).
    pub fn remove(&mut self, node: &NodeId) -> Option<PendingDelete> {
        self.entries.remove(node)
    }

    /// True if `node` is pending.
    pub fn contains(&self, node: &NodeId) -> bool {
        self.entries.contains_key(node)
    }
}

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
    /// `fingerprint(base_md)`: the fast path "file unchanged" (§5.1).
    pub md_fingerprint: Fingerprint,
    /// Short id -> full id for every node we have seen (base and export).
    pub ids: IdMap,
    /// The config the file was rendered with.
    pub config: DocConfig,
    /// The file the root was last synced from (informational; files are found by root,
    /// §9.1 guard 5).
    pub file: Option<PathBuf>,
    /// Unix seconds of the last completed cycle.
    pub synced_at: i64,
    /// Deferred deletes (§9.1 guard 2).
    pub pending_deletes: PendingDeletes,
}

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
    pub at: i64,
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

/// A held per-root lock; released on drop.
#[derive(Debug)]
pub struct RootLock {
    file: File,
}

/// The state directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateStore {
    dir: PathBuf,
}

impl StateStore {
    /// Open (creating directories as needed) the store at `dir`.
    pub fn open(dir: &Path) -> Result<StateStore, StateError> {
        todo!()
    }

    /// The store's directory.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// `<dir>/roots/<root uuid>` (created on first save).
    pub fn root_dir(&self, root: &NodeId) -> PathBuf {
        todo!()
    }

    /// Every root that has a checkpoint.
    pub fn roots(&self) -> Result<Vec<NodeId>, StateError> {
        todo!()
    }

    /// Take the root's lock without blocking (`File::try_lock`).
    pub fn lock(&self, root: &NodeId) -> Result<RootLock, StateError> {
        todo!()
    }

    /// Load the checkpoint; `Ok(None)` if the root has none.
    pub fn load_checkpoint(&self, root: &NodeId) -> Result<Option<Checkpoint>, StateError> {
        todo!()
    }

    /// Save the checkpoint atomically.
    pub fn save_checkpoint(&self, cp: &Checkpoint) -> Result<(), StateError> {
        todo!()
    }

    /// Save a history entry atomically (overwrites the same gen).
    pub fn put_history(&self, root: &NodeId, entry: &HistoryEntry) -> Result<(), StateError> {
        todo!()
    }

    /// Load one generation; `Ok(None)` if not kept.
    pub fn get_history(
        &self,
        root: &NodeId,
        generation: u64,
    ) -> Result<Option<HistoryEntry>, StateError> {
        todo!()
    }

    /// Generations kept, ascending.
    pub fn history_gens(&self, root: &NodeId) -> Result<Vec<u64>, StateError> {
        todo!()
    }

    /// Delete generations per `limits` at time `now`; returns the pruned gens.
    pub fn prune_history(
        &self,
        root: &NodeId,
        limits: &HistoryLimits,
        now: i64,
    ) -> Result<Vec<u64>, StateError> {
        todo!()
    }

    /// `<root_dir>/journal.jsonl`.
    pub fn journal_path(&self, root: &NodeId) -> PathBuf {
        todo!()
    }
}

/// Write `bytes` to `path` atomically: a temp file in the same directory, fsync, rename
/// over `path`, fsync the directory.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    todo!()
}
