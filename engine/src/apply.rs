//! Execute a plan against the API with a write-ahead journal, and recover from a crash.
//!
//! Responsibility (design §7.1 pre-write checks and post-apply verification, §7.2 the
//! two-step create, §7.3 journal and crash recovery, §8.2 guards re-checked live, §9
//! races). This is I/O: it talks to the API through [`crate::wfclient::Api`] and writes
//! the journal file.
//!
//! Consumes: [`crate::plan`] (`Plan`, `Op`, `CreateName`, `simulate_op`, `GuardReason`),
//! [`crate::wfclient`] (`Api`, requests, `ApiError`), [`crate::tree`] (`Tree`,
//! `outline_diff`), [`crate::state`] (`atomic_write`, `now_unix`).
//! Consumed by: `sync` (runs [`Applier::apply`] each cycle and [`recover`] at start-up,
//! commits the journal after the file and checkpoint are written); `merge` (consumes
//! [`Adoption`]s).
//!
//! # Executing a plan
//!
//! For each op, in order, with temp ids mapped to the real ids created so far (an op
//! naming the temp id of a create that was skipped is skipped with
//! [`SkipReason::MissingDependency`]):
//!
//! 1. Re-check the §8.2 guards against the applier's latest known tree (the remote tree
//!    passed in, updated with `plan::simulate_op` after every successful op): a move's
//!    destination must not be the node or its descendant, and must lie inside one of the
//!    applier's `sync_roots` (plus the plan's root); the moved node must lie strictly
//!    inside one; a create's parent must lie inside one; updates, completions and
//!    deletes must target nodes inside one, and a root is never deleted. A failure skips
//!    the op ([`SkipReason::Guard`]); a refused op is never sent.
//! 2. With [`ApplyOptions::pre_write_check`] (default on), `GET` the node first and skip
//!    the op if it no longer matches: `Update*` when the current value differs from
//!    `expected` ([`SkipReason::ChangedRemotely`]); `Move` when the live parent differs
//!    from `expected_parent`; `Complete` when already completed
//!    ([`SkipReason::AlreadyCompleted`]: never re-complete, DM§4.5 re-stamps
//!    `completedAt`); `Delete` when the node's own fields (name, note, layout,
//!    completion) differ from the latest known tree; any op on a 404
//!    ([`SkipReason::Gone`]). Creates are not pre-checked: a vanished parent answers the
//!    create itself with a 404, which is reported as `Gone` as well.
//! 3. **Live cycle check** for every re-parenting move (one whose node is not already a
//!    live child of the destination): `GET` the destination and its ancestors up to a
//!    sync root, and skip with `SkipReason::Guard(GuardReason::Cycle)` if the node is
//!    among them (or `OutsideSyncRoots` if the walk reaches the top level without
//!    meeting a sync root): the API would accept the move and detach the subtree
//!    (DM§4.7.5), so the cycle check must also hold against live data, not only the
//!    snapshot (§2, §8.2). This check runs even with `pre_write_check` off, because it is
//!    a hard safety requirement. Same-parent moves (the §8.1 reorders) skip the walk: a
//!    live child of `P` cannot be an ancestor of `P`.
//! 4. Append [`JournalEntry::Intent`] (fsync) immediately before sending.
//! 5. Send it. `Create` with `CreateName::Direct(name)`: one `create` with that name, the
//!    note and the layout at `Position::Bottom`. With `CreateName::Deferred(name)`: a
//!    `create` whose name is a fresh nonce `⟳syncflowy:<16 hex>` (recorded in the
//!    `Intent`; the note and layout are sent verbatim in this first request), then
//!    `update {name}` (§7.2). [`JournalEntry::Created`] is recorded as soon as the id is
//!    known (between the two steps). The plan's `Complete` ops (step 5 of §7.1) tick new
//!    items typed ticked after they exist.
//! 6. Append [`JournalEntry::Done`] or [`JournalEntry::Skipped`], and update the latest
//!    known tree.
//!
//! **Where the nonce goes.** §7.3 suggests the nonce in the note (`note: "⟳syncflowy:…"`,
//! replaced in step 2). The plan's contract (`plan::CreateName::Deferred`) puts it in
//! the *name* instead, and this module follows it: the API cannot write a `null` note
//! back (DM§6.4), so a nonce in the note would turn every note-less created node's
//! `null` into `""`; the name is overwritten in step 2 anyway. The nonce is ASCII after
//! the `⟳` and is a fixed point of the create pipeline (checked against the mock in
//! `tests/apply.rs`).
//!
//! A failed request ([`crate::wfclient::ApiError`] after the client's retries) stops the
//! apply: append [`JournalEntry::Failed`] and return [`ApplyError::Api`] carrying the
//! outcome so far. A 404 (`Item not found`, `Parent item not found`) is not a failure
//! but a race: the op is skipped as [`SkipReason::Gone`] and the apply goes on. A create
//! whose transport error says `maybe_applied` is resolved first by an export scan for its
//! nonce (or, for a direct create, for an unknown child of the parent with exactly that
//! name created after the intent's timestamp); if the scan finds nothing, the apply
//! stops with the original error (the next cycle plans the create again).
//!
//! After the last op: if [`ApplyOptions::verify`], export once and compare the root's
//! outline with the expected tree (`M` with temp ids mapped) using
//! `tree::outline_diff(.., DiffScope::Visible)`; differences are returned as
//! `discrepancies` (a concurrent edit, a skipped op or a bug; the next cycle picks them
//! up as remote changes, §7.1) and are not errors. `note: None` and `Some("")` count as
//! equal for nodes created in this apply. Append [`JournalEntry::Applied`].
//!
//! # Journal lifecycle (one per sync cycle)
//!
//! `sync` creates the journal ([`Journal::begin`], with [`JournalEntry::new_begin`])
//! before the first op and calls [`Journal::commit`] (which deletes the file) only after
//! the new Markdown file **and** the new checkpoint are durably written. A journal that
//! still exists at start-up therefore means the previous cycle did not finish, whatever
//! the crash point.
//!
//! # Recovery (§7.3)
//!
//! [`recover`] reads the journal, exports, and for every create that has an `Intent`
//! (so may have happened): finds the node (the `Created` id, else the nonce, else the
//! direct-create heuristic); records a `Created` entry in the old journal if it was
//! missing (so a crash during recovery cannot lose the node once its nonce is gone); if
//! the node still carries the nonce as its name, finishes step 2 (writes the intended
//! name); and returns it as an [`Adoption`] so that the next merge pairs it with the
//! still untagged Markdown item instead of creating a duplicate. Extra nodes carrying the
//! same nonce (not possible through this client, which never retries a create that may
//! have been applied, but cheap insurance) are deleted. Other ops need no replay: the
//! next merge sees their effects as remote changes equal to the local ones, and a
//! completed entry (`Done`, `Skipped`) is never repeated. Adoptions listed in the
//! journal's `Begin` entry (carried from an earlier recovery) are returned too while
//! their node still exists, so a crash during a recovering cycle loses nothing. Recovery
//! never deletes the journal: the caller starts a new cycle, whose `Begin` carries the
//! adoptions, and that replaces it (so the journal is cleared only once reconciled).

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::plan::{self, CreateName, GuardReason, Op, Plan};
use crate::state::{atomic_write, now_unix};
use crate::tree::{
    DEFAULT_LAYOUT, DiffScope, Node, NodeId, OutlineDiff, Position, Tree, outline_diff,
};
use crate::wfclient::{Api, ApiError, CreateRequest, UpdateRequest};

/// The prefix of a deferred create's nonce name (§7.3).
pub const NONCE_PREFIX: &str = "⟳syncflowy:";

/// How far before an `Intent`'s timestamp a node found by the direct-create heuristic
/// may have been created (clock skew between this host and the server).
// UNVERIFIED: the clock skew between a client and production is unknown; two minutes is
// generous for NTP-synced hosts and small enough that a collaborator's identically named
// new sibling is very unlikely to fall inside the window.
pub const DIRECT_CREATE_SLACK_SECS: i64 = 120;

/// The longest ancestor walk of the live cycle check (a bound against detached cycles,
/// DM§4.7.5, besides the visited set).
const MAX_LIVE_WALK: usize = 10_000;

/// A fresh nonce name: [`NONCE_PREFIX`] followed by 16 random lowercase hex digits.
pub fn new_nonce() -> String {
    let n: u64 = rand::random();
    format!("{NONCE_PREFIX}{n:016x}")
}

/// True if `name` is a nonce name written by [`new_nonce`].
pub fn is_nonce(name: &str) -> bool {
    name.strip_prefix(NONCE_PREFIX)
        .is_some_and(|h| h.len() == 16 && h.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// A node created by an interrupted apply, to be matched with its Markdown item by the
/// next merge (`merge::MergeInput::adoptions`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Adoption {
    /// The real node.
    pub node: NodeId,
    /// Its real parent.
    pub parent: NodeId,
    /// The intended canonical WF-HTML name (as written or to be written).
    pub name: String,
    /// The intended note.
    pub note: Option<String>,
    /// The layout.
    pub layout: String,
}

/// Why an op was skipped.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SkipReason {
    /// The pre-write GET found a value other than the expected one (§7.1, §9).
    ChangedRemotely,
    /// `Complete` on a node that is already completed.
    AlreadyCompleted,
    /// The node (or the parent) no longer exists (404).
    Gone,
    /// A §8.2 guard failed against the live tree.
    Guard(GuardReason),
    /// The op depends on a create that was skipped or failed.
    MissingDependency(NodeId),
}

/// One journal line (JSON Lines, one object per line, appended and fsynced).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum JournalEntry {
    /// First line: what this cycle is about to do.
    Begin {
        /// Random cycle id (hex).
        cycle: String,
        /// The sync root.
        root: NodeId,
        /// Unix seconds.
        at: i64,
        /// The plan.
        plan: Plan,
        /// Adoptions carried over from an earlier recovery (still unmatched).
        carried_adoptions: Vec<Adoption>,
    },
    /// About to send op `index` of the plan.
    Intent {
        /// Index into `plan.ops`.
        index: usize,
        /// Unix seconds.
        at: i64,
        /// The nonce name of a deferred create.
        nonce: Option<String>,
    },
    /// A create returned this id (written between the two create steps).
    Created {
        /// Index into `plan.ops`.
        index: usize,
        /// The op's temp id.
        temp: NodeId,
        /// The real id.
        id: NodeId,
    },
    /// Op finished.
    Done {
        /// Index into `plan.ops`.
        index: usize,
    },
    /// Op skipped.
    Skipped {
        /// Index into `plan.ops`.
        index: usize,
        /// Why.
        reason: SkipReason,
    },
    /// Op failed; the apply stopped.
    Failed {
        /// Index into `plan.ops`.
        index: usize,
        /// Error text.
        error: String,
    },
    /// All ops processed (and verified, if enabled).
    Applied {
        /// Unix seconds.
        at: i64,
    },
}

impl JournalEntry {
    /// A `Begin` entry for `plan` with a fresh random cycle id and the current time.
    pub fn new_begin(plan: &Plan, carried_adoptions: Vec<Adoption>) -> JournalEntry {
        let n: u64 = rand::random();
        JournalEntry::Begin {
            cycle: format!("{n:016x}"),
            root: plan.root.clone(),
            at: now_unix(),
            plan: plan.clone(),
            carried_adoptions,
        }
    }
}

/// The write-ahead journal of one root (`state::StateStore::journal_path`).
#[derive(Debug)]
pub struct Journal {
    path: PathBuf,
    file: File,
}

fn fsync_parent(path: &Path) -> io::Result<()> {
    let dir = match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => PathBuf::from("."),
    };
    File::open(dir)?.sync_all()
}

fn entry_line(entry: &JournalEntry) -> io::Result<Vec<u8>> {
    let mut line = serde_json::to_vec(entry).map_err(io::Error::other)?;
    line.push(b'\n');
    Ok(line)
}

impl Journal {
    /// Start a new journal at `path` with its `Begin` entry, atomically replacing any
    /// existing journal (temp file, fsync, rename). The caller must have run [`recover`]
    /// on an existing journal first and carried its adoptions into `begin`. Creates the
    /// parent directory if needed.
    pub fn begin(path: &Path, begin: &JournalEntry) -> io::Result<Journal> {
        if !matches!(begin, JournalEntry::Begin { .. }) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "a journal must start with a Begin entry",
            ));
        }
        if let Some(dir) = path.parent()
            && !dir.as_os_str().is_empty()
        {
            std::fs::create_dir_all(dir)?;
        }
        atomic_write(path, &entry_line(begin)?)?;
        Journal::open_append(path)
    }

    /// Open an existing journal for appending (recovery records `Created` entries).
    ///
    /// A crash mid-append can leave the file without a final newline. Appending after
    /// such a tail would glue the new entry onto it, so that [`Journal::read`] drops both
    /// as one truncated last line (or, once more entries follow, fails the whole read
    /// with `InvalidData`). So the tail is repaired first (§7.3: the journal must keep
    /// every record recovery relies on): a tail that decodes as an entry (only the
    /// newline was lost) is terminated with a newline, and anything else is cut off,
    /// which loses nothing that `read` would have returned.
    fn open_append(path: &Path) -> io::Result<Journal> {
        let mut file = OpenOptions::new().read(true).append(true).open(path)?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;
        let keep = bytes.iter().rposition(|&b| b == b'\n').map_or(0, |i| i + 1);
        let tail = &bytes[keep..];
        if !tail.is_empty() {
            if serde_json::from_slice::<JournalEntry>(tail).is_ok() {
                file.write_all(b"\n")?;
            } else {
                log::warn!(
                    "{}: cutting a truncated last journal line before appending",
                    path.display()
                );
                file.set_len(keep as u64)?;
            }
            file.sync_data()?;
        }
        Ok(Journal {
            path: path.to_path_buf(),
            file,
        })
    }

    /// Append one entry and fsync.
    pub fn append(&mut self, entry: &JournalEntry) -> io::Result<()> {
        let line = entry_line(entry)?;
        self.file.write_all(&line)?;
        self.file.sync_data()
    }

    /// The cycle finished durably: delete the journal file (and fsync the directory).
    pub fn commit(self) -> io::Result<()> {
        let Journal { path, file } = self;
        drop(file);
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        fsync_parent(&path)
    }

    /// Read a journal; `Ok(None)` if there is no file or it is empty. A truncated last
    /// line (crash mid-write) is ignored; any other undecodable line is an
    /// `InvalidData` error.
    pub fn read(path: &Path) -> io::Result<Option<Vec<JournalEntry>>> {
        let mut bytes = Vec::new();
        match File::open(path) {
            Ok(mut f) => {
                f.read_to_end(&mut bytes)?;
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e),
        }
        // A crash mid-write can only cut the last line; lossy decoding keeps a cut
        // multi-byte character from failing the whole read.
        let text = String::from_utf8_lossy(&bytes);
        let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
        let mut out = Vec::with_capacity(lines.len());
        for (i, line) in lines.iter().enumerate() {
            match serde_json::from_str::<JournalEntry>(line) {
                Ok(e) => out.push(e),
                Err(e) if i + 1 == lines.len() => {
                    log::warn!(
                        "{}: ignoring truncated last journal line: {e}",
                        path.display()
                    );
                }
                Err(e) => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("{}: line {}: {e}", path.display(), i + 1),
                    ));
                }
            }
        }
        Ok(if out.is_empty() { None } else { Some(out) })
    }

    /// The journal's path.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// Applier options.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApplyOptions {
    /// GET before each write and skip if the node changed (§7.1). Default true.
    pub pre_write_check: bool,
    /// Re-export after the last op and diff against the expected tree. Default true.
    pub verify: bool,
}

impl Default for ApplyOptions {
    fn default() -> Self {
        ApplyOptions {
            pre_write_check: true,
            verify: true,
        }
    }
}

/// What happened to one op.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum OpOutcome {
    /// Sent and acknowledged.
    Done,
    /// Not sent (or, for a create, nothing further sent).
    Skipped(SkipReason),
    /// Not reached because an earlier op failed.
    NotRun,
}

/// The result of an apply.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApplyOutcome {
    /// Temp id -> real id for every node created.
    pub created: BTreeMap<NodeId, NodeId>,
    /// One entry per plan op, same order.
    pub results: Vec<OpOutcome>,
    /// The export taken for verification (`None` if `verify` was off or the apply
    /// failed before it).
    pub export: Option<Tree>,
    /// Differences between the expected tree and the verification export.
    pub discrepancies: Vec<OutlineDiff>,
    /// HTTP requests sent (from `Api::request_count`).
    pub requests: u64,
}

impl ApplyOutcome {
    /// The nodes this apply created, as [`Adoption`]s (for `sync`, which reruns the
    /// merge with them when the file changed under it, §9).
    pub fn adoptions(&self, plan: &Plan) -> Vec<Adoption> {
        plan.ops
            .iter()
            .filter_map(|op| match op {
                Op::Create {
                    temp,
                    parent,
                    name,
                    note,
                    layout,
                } => {
                    let node = self.created.get(temp)?.clone();
                    let parent = map_id(parent, &self.created).ok()?;
                    Some(Adoption {
                        node,
                        parent,
                        name: name.value().to_owned(),
                        note: note.clone(),
                        layout: effective_layout(layout),
                    })
                }
                _ => None,
            })
            .collect()
    }

    /// The per-operation outcome log: one human-readable line per op.
    pub fn log_lines(&self, plan: &Plan) -> Vec<String> {
        plan.ops
            .iter()
            .enumerate()
            .map(|(i, op)| {
                let what = describe_op(op, &self.created);
                let result = match self.results.get(i) {
                    Some(OpOutcome::Done) => "done".to_owned(),
                    Some(OpOutcome::Skipped(r)) => format!("skipped ({})", describe_skip(r)),
                    Some(OpOutcome::NotRun) | None => "not run".to_owned(),
                };
                format!("op {i}: {what}: {result}")
            })
            .collect()
    }

    fn empty(n: usize) -> ApplyOutcome {
        ApplyOutcome {
            created: BTreeMap::new(),
            results: vec![OpOutcome::NotRun; n],
            export: None,
            discrepancies: Vec::new(),
            requests: 0,
        }
    }
}

fn describe_skip(r: &SkipReason) -> String {
    match r {
        SkipReason::ChangedRemotely => "changed remotely".to_owned(),
        SkipReason::AlreadyCompleted => "already completed".to_owned(),
        SkipReason::Gone => "gone".to_owned(),
        SkipReason::Guard(g) => format!("guard {g:?}"),
        SkipReason::MissingDependency(t) => format!("depends on {t}, which was not created"),
    }
}

fn describe_op(op: &Op, created: &BTreeMap<NodeId, NodeId>) -> String {
    let id = |n: &NodeId| match created.get(n) {
        Some(real) => format!("{n}={real}"),
        None => n.to_string(),
    };
    match op {
        Op::Create {
            temp, parent, name, ..
        } => {
            let how = match name {
                CreateName::Direct(_) => "direct",
                CreateName::Deferred(_) => "two-step",
            };
            format!("create {} under {} ({how})", id(temp), id(parent))
        }
        Op::UpdateName { node, .. } => format!("update name of {}", id(node)),
        Op::UpdateNote { node, .. } => format!("update note of {}", id(node)),
        Op::UpdateLayout { node, layout, .. } => {
            format!("set layout of {} to {layout}", id(node))
        }
        Op::Move {
            node,
            parent,
            position,
            ..
        } => format!(
            "move {} to the {} of {}",
            id(node),
            position.as_str(),
            id(parent)
        ),
        Op::Complete { node } => format!("complete {}", id(node)),
        Op::Delete {
            node,
            hidden_descendants,
        } => format!("delete {} (+{hidden_descendants} hidden)", id(node)),
    }
}

/// Apply failures.
#[derive(Debug, Error)]
pub enum ApplyError {
    /// An API call failed after retries; `outcome` holds what was done so far.
    #[error("op {index} failed: {error}")]
    Api {
        /// Index into `plan.ops`.
        index: usize,
        /// The error.
        error: ApiError,
        /// Progress so far.
        outcome: Box<ApplyOutcome>,
    },
    /// The verification or recovery export failed.
    #[error("export failed: {0}")]
    Export(ApiError),
    /// Journal I/O failed (nothing more is sent once the journal cannot be written).
    #[error("journal: {0}")]
    Journal(#[from] std::io::Error),
    /// The journal content is not understood (corrupt, unknown version).
    #[error("bad journal: {0}")]
    BadJournal(String),
}

// ---------------------------------------------------------------------------------------
// Helpers shared by apply and recover
// ---------------------------------------------------------------------------------------

/// The layout a create with `layout` produces (DM§4.3: empty means `bullets`).
fn effective_layout(layout: &str) -> String {
    if layout.is_empty() {
        DEFAULT_LAYOUT.to_owned()
    } else {
        layout.to_owned()
    }
}

/// `id` with temp ids replaced by real ids; `Err(temp)` for a temp id not created.
fn map_id(id: &NodeId, created: &BTreeMap<NodeId, NodeId>) -> Result<NodeId, NodeId> {
    if id.is_temp() {
        created.get(id).cloned().ok_or_else(|| id.clone())
    } else {
        Ok(id.clone())
    }
}

/// `op` with every referenced temp id mapped to its real id (a create keeps its own
/// temp id; only its parent is mapped). `Err(temp)` names the first missing one.
fn map_op(op: &Op, created: &BTreeMap<NodeId, NodeId>) -> Result<Op, NodeId> {
    Ok(match op {
        Op::Create {
            temp,
            parent,
            name,
            note,
            layout,
        } => Op::Create {
            temp: temp.clone(),
            parent: map_id(parent, created)?,
            name: name.clone(),
            note: note.clone(),
            layout: layout.clone(),
        },
        Op::UpdateName {
            node,
            expected,
            name,
        } => Op::UpdateName {
            node: map_id(node, created)?,
            expected: expected.clone(),
            name: name.clone(),
        },
        Op::UpdateNote {
            node,
            expected,
            note,
        } => Op::UpdateNote {
            node: map_id(node, created)?,
            expected: expected.clone(),
            note: note.clone(),
        },
        Op::UpdateLayout {
            node,
            expected,
            layout,
        } => Op::UpdateLayout {
            node: map_id(node, created)?,
            expected: expected.clone(),
            layout: layout.clone(),
        },
        Op::Move {
            node,
            parent,
            position,
            expected_parent,
        } => Op::Move {
            node: map_id(node, created)?,
            parent: map_id(parent, created)?,
            position: *position,
            expected_parent: match expected_parent {
                Some(p) => Some(map_id(p, created)?),
                None => None,
            },
        },
        Op::Complete { node } => Op::Complete {
            node: map_id(node, created)?,
        },
        Op::Delete {
            node,
            hidden_descendants,
        } => Op::Delete {
            node: map_id(node, created)?,
            hidden_descendants: *hidden_descendants,
        },
    })
}

/// True if `id` is one of `roots` or lies below one of them in `tree`.
fn within_roots(tree: &Tree, id: &NodeId, roots: &BTreeSet<NodeId>) -> bool {
    roots.contains(id) || tree.ancestors(id).iter().any(|a| roots.contains(a))
}

/// Nodes of `tree` named `nonce`, oldest first.
fn find_by_nonce(tree: &Tree, nonce: &str) -> Vec<NodeId> {
    let mut found: Vec<&Node> = tree.iter().filter(|n| n.name == nonce).collect();
    found.sort_by(|a, b| (a.created_at, &a.id).cmp(&(b.created_at, &b.id)));
    found.into_iter().map(|n| n.id.clone()).collect()
}

/// The direct-create heuristic (§7.3, module docs): the oldest child of `parent` in
/// `tree` named exactly `name`, created no earlier than `since - slack`, and not
/// excluded.
fn find_direct(
    tree: &Tree,
    parent: &NodeId,
    name: &str,
    since: i64,
    exclude: &dyn Fn(&NodeId) -> bool,
) -> Option<NodeId> {
    let mut found: Vec<&Node> = tree
        .children(parent)
        .iter()
        .filter_map(|c| tree.get(c))
        .filter(|n| {
            n.name == name && n.created_at >= since - DIRECT_CREATE_SLACK_SECS && !exclude(&n.id)
        })
        .collect();
    found.sort_by(|a, b| (a.created_at, &a.id).cmp(&(b.created_at, &b.id)));
    found.first().map(|n| n.id.clone())
}

fn export_tree(api: &dyn Api) -> Result<Tree, ApiError> {
    let nodes = api.export()?;
    Tree::from_nodes(nodes).map_err(|e| ApiError::Decode(format!("export: {e}")))
}

fn is_not_found(e: &ApiError) -> bool {
    matches!(e, ApiError::ItemNotFound | ApiError::ParentNotFound)
}

// ---------------------------------------------------------------------------------------
// The applier
// ---------------------------------------------------------------------------------------

/// Executes plans (module docs).
pub struct Applier<'a> {
    api: &'a dyn Api,
    opts: ApplyOptions,
    sync_roots: BTreeSet<NodeId>,
}

/// What one op came to.
enum Step {
    Done,
    Skip(SkipReason),
    Fail(ApiError),
}

/// The mutable state of one apply.
struct Run<'j> {
    latest: Tree,
    created: BTreeMap<NodeId, NodeId>,
    results: Vec<OpOutcome>,
    journal: &'j mut Journal,
}

impl<'a> Applier<'a> {
    /// An applier over `api`. `sync_roots` are all roots managed in this run (the
    /// boundary guard, §8.2; the same set as `plan::PlanConfig::sync_roots`). The plan's
    /// own root is always added.
    pub fn new(api: &'a dyn Api, opts: ApplyOptions, sync_roots: BTreeSet<NodeId>) -> Applier<'a> {
        Applier {
            api,
            opts,
            sync_roots,
        }
    }

    /// Execute `plan`, journaling into `journal` (which `sync` has begun with this
    /// plan). `remote` is the export the plan was made against (the applier's initial
    /// latest known tree); `expected` is the merged tree `M` for verification (temp ids
    /// allowed; mapped with `created`).
    pub fn apply(
        &self,
        plan: &Plan,
        remote: &Tree,
        expected: &Tree,
        journal: &mut Journal,
    ) -> Result<ApplyOutcome, ApplyError> {
        let start_requests = self.api.request_count();
        let mut roots = self.sync_roots.clone();
        roots.insert(plan.root.clone());
        let mut run = Run {
            latest: remote.clone(),
            created: BTreeMap::new(),
            results: vec![OpOutcome::NotRun; plan.ops.len()],
            journal,
        };

        for (index, op) in plan.ops.iter().enumerate() {
            let step = match map_op(op, &run.created) {
                Err(missing) => Step::Skip(SkipReason::MissingDependency(missing)),
                Ok(mapped) => self.run_op(index, &mapped, &roots, &mut run)?,
            };
            match step {
                Step::Done => {
                    log::info!("op {index}: {}: done", describe_op(op, &run.created));
                    run.journal.append(&JournalEntry::Done { index })?;
                    run.results[index] = OpOutcome::Done;
                }
                Step::Skip(reason) => {
                    log::warn!(
                        "op {index}: {}: skipped ({})",
                        describe_op(op, &run.created),
                        describe_skip(&reason)
                    );
                    run.journal.append(&JournalEntry::Skipped {
                        index,
                        reason: reason.clone(),
                    })?;
                    run.results[index] = OpOutcome::Skipped(reason);
                }
                Step::Fail(error) => {
                    log::error!(
                        "op {index}: {}: failed: {error}",
                        describe_op(op, &run.created)
                    );
                    run.journal.append(&JournalEntry::Failed {
                        index,
                        error: error.to_string(),
                    })?;
                    let outcome = ApplyOutcome {
                        created: run.created,
                        results: run.results,
                        export: None,
                        discrepancies: Vec::new(),
                        requests: self.api.request_count().saturating_sub(start_requests),
                    };
                    return Err(ApplyError::Api {
                        index,
                        error,
                        outcome: Box::new(outcome),
                    });
                }
            }
        }

        let mut outcome = ApplyOutcome {
            created: run.created,
            results: run.results,
            export: None,
            discrepancies: Vec::new(),
            requests: 0,
        };
        if self.opts.verify {
            // §7.1: re-export and check that the remote now equals M.
            let actual = export_tree(self.api).map_err(ApplyError::Export)?;
            outcome.discrepancies = verify(expected, &actual, &plan.root, &outcome.created);
            for d in &outcome.discrepancies {
                log::warn!("verification: {d:?}");
            }
            outcome.export = Some(actual);
        }
        run.journal
            .append(&JournalEntry::Applied { at: now_unix() })?;
        outcome.requests = self.api.request_count().saturating_sub(start_requests);
        Ok(outcome)
    }

    /// Guards, pre-write checks, journal intent, send, latest-tree update for one op
    /// (ids already mapped). Only journal I/O errors are returned as `Err`.
    fn run_op(
        &self,
        index: usize,
        op: &Op,
        roots: &BTreeSet<NodeId>,
        run: &mut Run<'_>,
    ) -> Result<Step, ApplyError> {
        // 1. §8.2 guards against the latest known tree.
        if let Some(reason) = guard(op, &run.latest, roots) {
            return Ok(Step::Skip(SkipReason::Guard(reason)));
        }
        // 2.–3. Pre-write GET and the live cycle check.
        match self.precheck(op, &run.latest, roots) {
            Ok(Some(reason)) => return Ok(Step::Skip(reason)),
            Ok(None) => {}
            Err(e) if is_not_found(&e) => return Ok(Step::Skip(SkipReason::Gone)),
            Err(e) => return Ok(Step::Fail(e)),
        }
        // 4.–5. Intent, send.
        if let Op::Create { .. } = op {
            return self.create(index, op, run);
        }
        run.journal.append(&JournalEntry::Intent {
            index,
            at: now_unix(),
            nonce: None,
        })?;
        let sent = match op {
            Op::UpdateName { node, name, .. } => self.api.update(
                node,
                &UpdateRequest {
                    name: Some(name.clone()),
                    ..UpdateRequest::default()
                },
            ),
            Op::UpdateNote { node, note, .. } => self.api.update(
                node,
                &UpdateRequest {
                    note: Some(note.clone()),
                    ..UpdateRequest::default()
                },
            ),
            Op::UpdateLayout { node, layout, .. } => self.api.update(
                node,
                &UpdateRequest {
                    layout: Some(layout.clone()),
                    ..UpdateRequest::default()
                },
            ),
            Op::Move {
                node,
                parent,
                position,
                ..
            } => self.api.move_node(node, parent, *position),
            Op::Complete { node } => self.api.complete(node),
            Op::Delete { node, .. } => self.api.delete(node),
            Op::Create { .. } => unreachable!("handled above"),
        };
        match sent {
            Ok(()) => {
                update_latest(op, &mut run.latest);
                Ok(Step::Done)
            }
            Err(e) if is_not_found(&e) => Ok(Step::Skip(SkipReason::Gone)),
            Err(e) => Ok(Step::Fail(e)),
        }
    }

    /// §7.2: the create, in one or two requests.
    fn create(&self, index: usize, op: &Op, run: &mut Run<'_>) -> Result<Step, ApplyError> {
        let Op::Create {
            temp,
            parent,
            name,
            note,
            layout,
        } = op
        else {
            unreachable!("only called for creates")
        };
        let nonce = match name {
            CreateName::Direct(_) => None,
            CreateName::Deferred(_) => Some(new_nonce()),
        };
        let at = now_unix();
        run.journal.append(&JournalEntry::Intent {
            index,
            at,
            nonce: nonce.clone(),
        })?;
        let first_name = nonce.clone().unwrap_or_else(|| name.value().to_owned());
        let req = CreateRequest {
            parent_id: parent.clone(),
            name: first_name.clone(),
            // §7.2 step 1: note and layoutMode are verbatim on create.
            note: note.clone(),
            layout: (!layout.is_empty()).then(|| layout.clone()),
            position: Position::Bottom,
        };
        let id = match self.api.create(&req) {
            Ok(id) => id,
            Err(e) if is_not_found(&e) => return Ok(Step::Skip(SkipReason::Gone)),
            Err(
                e @ ApiError::Transport {
                    maybe_applied: true,
                    ..
                },
            ) => {
                // §7.3: the request may have been applied; look for the node before
                // deciding.
                match self.find_maybe_created(parent, &first_name, nonce.is_some(), at, run) {
                    Some(id) => {
                        log::warn!("create {temp}: found {id} after a lost response");
                        id
                    }
                    None => return Ok(Step::Fail(e)),
                }
            }
            Err(e) => return Ok(Step::Fail(e)),
        };
        run.journal.append(&JournalEntry::Created {
            index,
            temp: temp.clone(),
            id: id.clone(),
        })?;
        run.created.insert(temp.clone(), id.clone());
        // Known state after step 1 (the final name is written below).
        let mut node = Node::new(id.clone(), Some(parent.clone()), first_name);
        node.note = note.clone();
        node.layout = effective_layout(layout);
        if let Err(e) = run.latest.place(node, Some(parent), Position::Bottom) {
            log::warn!("latest tree: cannot place created node {id}: {e}");
        }
        if nonce.is_some() {
            // §7.2 step 2: the exact WF-HTML name, through update (verbatim, DM§6.1).
            let req = UpdateRequest {
                name: Some(name.value().to_owned()),
                ..UpdateRequest::default()
            };
            match self.api.update(&id, &req) {
                Ok(()) => {}
                Err(e) if is_not_found(&e) => return Ok(Step::Skip(SkipReason::Gone)),
                Err(e) => return Ok(Step::Fail(e)),
            }
            let _ = run.latest.set_name(&id, name.value());
        }
        Ok(Step::Done)
    }

    /// After a create's transport error with `maybe_applied`: an export scan for the
    /// nonce, or for an unknown child of `parent` named `name` (direct creates).
    fn find_maybe_created(
        &self,
        parent: &NodeId,
        name: &str,
        by_nonce: bool,
        since: i64,
        run: &Run<'_>,
    ) -> Option<NodeId> {
        let tree = match export_tree(self.api) {
            Ok(t) => t,
            Err(e) => {
                log::warn!("export after a lost create response failed: {e}");
                return None;
            }
        };
        if by_nonce {
            find_by_nonce(&tree, name).into_iter().next()
        } else {
            let known: BTreeSet<&NodeId> = run.created.values().collect();
            find_direct(&tree, parent, name, since, &|id| {
                run.latest.contains(id) || known.contains(id)
            })
        }
    }

    /// Steps 2 and 3 of the module docs: `Ok(Some(reason))` skips the op.
    fn precheck(
        &self,
        op: &Op,
        latest: &Tree,
        roots: &BTreeSet<NodeId>,
    ) -> Result<Option<SkipReason>, ApiError> {
        let check = self.opts.pre_write_check;
        match op {
            Op::Create { .. } => Ok(None),
            Op::UpdateName { node, expected, .. } if check => {
                let live = self.api.get(node)?;
                Ok((&live.name != expected).then_some(SkipReason::ChangedRemotely))
            }
            Op::UpdateNote { node, expected, .. } if check => {
                let live = self.api.get(node)?;
                Ok((&live.note != expected).then_some(SkipReason::ChangedRemotely))
            }
            Op::UpdateLayout { node, expected, .. } if check => {
                let live = self.api.get(node)?;
                Ok((&live.layout != expected).then_some(SkipReason::ChangedRemotely))
            }
            Op::UpdateName { .. } | Op::UpdateNote { .. } | Op::UpdateLayout { .. } => Ok(None),
            Op::Complete { node } => {
                let completed = if check {
                    self.api.get(node)?.completed
                } else {
                    latest.get(node).is_some_and(|n| n.completed)
                };
                Ok(completed.then_some(SkipReason::AlreadyCompleted))
            }
            Op::Delete { node, .. } => {
                if !check {
                    return Ok(None);
                }
                // UNVERIFIED: the design only asks for a pre-write check before updates.
                // A collaborator's edit of a node we are about to delete would only be
                // recoverable from the Trash, so the delete is skipped instead and the
                // next cycle's merge sees the edit.
                let live = self.api.get(node)?;
                let changed = latest.get(node).is_none_or(|n| {
                    n.name != live.name
                        || n.note != live.note
                        || n.layout != live.layout
                        || n.completed != live.completed
                });
                Ok(changed.then_some(SkipReason::ChangedRemotely))
            }
            Op::Move {
                node,
                parent,
                expected_parent,
                ..
            } => {
                let live_parent = if check {
                    let live = self.api.get(node)?;
                    if let Some(exp) = expected_parent
                        && live.parent_id.as_ref() != Some(exp)
                    {
                        return Ok(Some(SkipReason::ChangedRemotely));
                    }
                    live.parent_id
                } else {
                    latest.parent(node).cloned()
                };
                if live_parent.as_ref() == Some(parent) {
                    // A reorder: the node is a child of the destination, so the
                    // destination cannot be its descendant.
                    return Ok(None);
                }
                self.live_cycle_check(node, parent, roots)
            }
        }
    }

    /// §8.2 against live data: walk from `dest` up to a sync root with GETs.
    fn live_cycle_check(
        &self,
        node: &NodeId,
        dest: &NodeId,
        roots: &BTreeSet<NodeId>,
    ) -> Result<Option<SkipReason>, ApiError> {
        let mut cur = dest.clone();
        let mut seen = BTreeSet::new();
        for _ in 0..MAX_LIVE_WALK {
            if &cur == node {
                return Ok(Some(SkipReason::Guard(GuardReason::Cycle)));
            }
            if roots.contains(&cur) {
                return Ok(None);
            }
            if !seen.insert(cur.clone()) {
                // A detached cycle (DM§4.7.5) that does not contain the node: the
                // destination is not inside any sync root.
                return Ok(Some(SkipReason::Guard(GuardReason::OutsideSyncRoots)));
            }
            let live = match self.api.get(&cur) {
                Ok(n) => n,
                Err(e) if is_not_found(&e) => return Ok(Some(SkipReason::Gone)),
                Err(e) => return Err(e),
            };
            match live.parent_id {
                Some(p) => cur = p,
                None => return Ok(Some(SkipReason::Guard(GuardReason::OutsideSyncRoots))),
            }
        }
        Ok(Some(SkipReason::Guard(GuardReason::OutsideSyncRoots)))
    }
}

/// §8.2 guards of one (mapped) op against the latest known tree.
fn guard(op: &Op, latest: &Tree, roots: &BTreeSet<NodeId>) -> Option<GuardReason> {
    match op {
        Op::Create { parent, .. } => {
            if !latest.contains(parent) {
                Some(GuardReason::UnknownNode)
            } else if !within_roots(latest, parent, roots) {
                Some(GuardReason::OutsideSyncRoots)
            } else {
                None
            }
        }
        Op::Move { node, parent, .. } => {
            if !latest.contains(node) || !latest.contains(parent) {
                Some(GuardReason::UnknownNode)
            } else if parent == node || latest.is_descendant(parent, node) {
                Some(GuardReason::Cycle)
            } else if !within_roots(latest, parent, roots)
                || roots.contains(node)
                || !within_roots(latest, node, roots)
            {
                Some(GuardReason::OutsideSyncRoots)
            } else {
                None
            }
        }
        Op::UpdateName { node, .. }
        | Op::UpdateNote { node, .. }
        | Op::UpdateLayout { node, .. }
        | Op::Complete { node } => {
            if !latest.contains(node) {
                Some(GuardReason::UnknownNode)
            } else if !within_roots(latest, node, roots) {
                Some(GuardReason::OutsideRoot)
            } else {
                None
            }
        }
        Op::Delete { node, .. } => {
            if !latest.contains(node) {
                Some(GuardReason::UnknownNode)
            } else if roots.contains(node) || !within_roots(latest, node, roots) {
                Some(GuardReason::OutsideRoot)
            } else {
                None
            }
        }
    }
}

/// Keep the latest known tree in step after a successful (mapped, non-create) op.
fn update_latest(op: &Op, latest: &mut Tree) {
    if let Err(e) = plan::simulate_op(op, latest) {
        // The latest tree is only an approximation of the server; a mismatch here is
        // logged and the guards keep checking against what is known.
        log::warn!("latest tree: cannot simulate {op:?}: {e}");
    }
}

/// §7.1 verification: `expected` (temp ids mapped with `created`) against the export
/// `actual`, over the nodes visible from `root`. A `None`/`Some("")` note difference on a
/// node created in this apply is not reported. Public so that `sync` can verify several
/// files' applies against one shared re-export (§6.4) instead of one export per file.
pub fn verify(
    expected: &Tree,
    actual: &Tree,
    root: &NodeId,
    created: &BTreeMap<NodeId, NodeId>,
) -> Vec<OutlineDiff> {
    let mut exp = expected.clone();
    if let Err(e) = exp.remap_ids(created) {
        log::warn!("verification: cannot map temp ids: {e}");
    }
    let new_ids: BTreeSet<&NodeId> = created.values().collect();
    outline_diff(&exp, actual, root, DiffScope::Visible)
        .into_iter()
        .filter(|d| match d {
            OutlineDiff::Note {
                id,
                expected,
                actual,
            } => {
                let empty = |n: &Option<String>| n.as_deref().is_none_or(str::is_empty);
                !(new_ids.contains(id) && empty(expected) && empty(actual))
            }
            _ => true,
        })
        .collect()
}

// ---------------------------------------------------------------------------------------
// Recovery
// ---------------------------------------------------------------------------------------

/// What [`recover`] found and fixed.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Recovery {
    /// The interrupted cycle's root.
    pub root: Option<NodeId>,
    /// Nodes created (fully or half) by the interrupted cycle, plus carried adoptions.
    pub adoptions: Vec<Adoption>,
    /// Nodes whose second create step recovery finished.
    pub finished: Vec<NodeId>,
    /// Human-readable notes for the report.
    pub notes: Vec<String>,
}

/// What the journal says about one op.
#[derive(Debug, Default)]
struct OpRecord {
    intent: Option<(i64, Option<String>)>,
    created: Option<NodeId>,
}

/// Reconcile a non-empty journal at `path` against a fresh export (module docs).
/// `Ok(None)` if there is no journal. Does not delete the journal: the caller begins a
/// new cycle (whose `Begin` carries `adoptions`), which replaces it.
pub fn recover(path: &Path, api: &dyn Api) -> Result<Option<Recovery>, ApplyError> {
    let entries = match Journal::read(path) {
        Ok(Some(e)) => e,
        Ok(None) => return Ok(None),
        Err(e) if e.kind() == io::ErrorKind::InvalidData => {
            return Err(ApplyError::BadJournal(e.to_string()));
        }
        Err(e) => return Err(ApplyError::Journal(e)),
    };
    let mut iter = entries.into_iter();
    let Some(JournalEntry::Begin {
        root,
        plan,
        carried_adoptions,
        ..
    }) = iter.next()
    else {
        return Err(ApplyError::BadJournal(
            "the first entry is not a begin entry".to_owned(),
        ));
    };
    let n = plan.ops.len();
    let mut records: Vec<OpRecord> = (0..n).map(|_| OpRecord::default()).collect();
    let mut applied = false;
    for e in iter {
        let index = match &e {
            JournalEntry::Begin { .. } => {
                return Err(ApplyError::BadJournal("a second begin entry".to_owned()));
            }
            JournalEntry::Applied { .. } => {
                applied = true;
                continue;
            }
            JournalEntry::Intent { index, .. }
            | JournalEntry::Created { index, .. }
            | JournalEntry::Done { index }
            | JournalEntry::Skipped { index, .. }
            | JournalEntry::Failed { index, .. } => *index,
        };
        if index >= n {
            return Err(ApplyError::BadJournal(format!(
                "entry for op {index} but the plan has {n} ops"
            )));
        }
        match e {
            JournalEntry::Intent { at, nonce, .. } => records[index].intent = Some((at, nonce)),
            JournalEntry::Created { temp, id, .. } => {
                let matches_plan =
                    matches!(&plan.ops[index], Op::Create { temp: t, .. } if *t == temp);
                if !matches_plan {
                    return Err(ApplyError::BadJournal(format!(
                        "created entry for op {index} does not match the plan"
                    )));
                }
                records[index].created = Some(id);
            }
            _ => {}
        }
    }

    let tree = export_tree(api).map_err(ApplyError::Export)?;
    let mut rec = Recovery {
        root: Some(root.clone()),
        ..Recovery::default()
    };
    if applied {
        rec.notes
            .push("the interrupted cycle had applied its whole plan".to_owned());
    }
    let mut journal: Option<Journal> = None;
    let mut map: BTreeMap<NodeId, NodeId> = BTreeMap::new();
    for (index, r) in records.iter().enumerate() {
        if let (Op::Create { temp, .. }, Some(id)) = (&plan.ops[index], &r.created) {
            map.insert(temp.clone(), id.clone());
        }
    }
    // Ids the plan names (existing nodes): never taken by the direct heuristic.
    let planned: BTreeSet<NodeId> = plan
        .ops
        .iter()
        .flat_map(|op| match op {
            Op::Create { parent, .. } => vec![parent.clone()],
            Op::Move { node, parent, .. } => vec![node.clone(), parent.clone()],
            other => vec![other.node().clone()],
        })
        .filter(|id| !id.is_temp())
        .collect();

    for (index, r) in records.iter().enumerate() {
        let Op::Create {
            temp,
            parent,
            name,
            note,
            layout,
        } = &plan.ops[index]
        else {
            continue;
        };
        let Some((at, nonce)) = &r.intent else {
            continue; // never sent
        };
        let found: Option<NodeId> = if let Some(id) = &r.created {
            if tree.contains(id) {
                Some(id.clone())
            } else {
                rec.notes.push(format!(
                    "node {id} created for {temp} no longer exists; nothing to adopt"
                ));
                None
            }
        } else if let Some(nonce) = nonce {
            let mut matches = find_by_nonce(&tree, nonce);
            if matches.len() > 1 {
                for extra in matches.split_off(1) {
                    match api.delete(&extra) {
                        Ok(()) => {}
                        Err(e) if is_not_found(&e) => {}
                        Err(e) => {
                            let mut outcome = ApplyOutcome::empty(n);
                            outcome.created = map;
                            return Err(ApplyError::Api {
                                index,
                                error: e,
                                outcome: Box::new(outcome),
                            });
                        }
                    }
                    rec.notes
                        .push(format!("deleted duplicate half-created node {extra}"));
                }
            }
            matches.into_iter().next()
        } else {
            match map_id(parent, &map) {
                Ok(p) => {
                    let claimed: BTreeSet<&NodeId> = map.values().collect();
                    find_direct(&tree, &p, name.value(), *at, &|id| {
                        planned.contains(id) || claimed.contains(id)
                    })
                }
                Err(_) => None,
            }
        };
        let Some(id) = found else {
            if r.created.is_none() {
                rec.notes.push(format!(
                    "create of {temp} was not applied (not found in the export)"
                ));
            }
            continue;
        };
        if r.created.is_none() {
            // Record it before touching the node, so that a second recovery finds it
            // by id once the nonce is gone.
            let j = match &mut journal {
                Some(j) => j,
                None => journal.insert(Journal::open_append(path)?),
            };
            j.append(&JournalEntry::Created {
                index,
                temp: temp.clone(),
                id: id.clone(),
            })?;
            rec.notes
                .push(format!("found node {id} of interrupted create {temp}"));
        }
        map.insert(temp.clone(), id.clone());
        let node = tree.get(&id).expect("found in this tree");
        if nonce.as_deref().is_some_and(|nc| node.name == nc) {
            // §7.3: finish the pending step 2.
            let req = UpdateRequest {
                name: Some(name.value().to_owned()),
                ..UpdateRequest::default()
            };
            match api.update(&id, &req) {
                Ok(()) => {
                    rec.finished.push(id.clone());
                    rec.notes
                        .push(format!("finished the two-step create of {id}"));
                }
                Err(e) if is_not_found(&e) => {
                    rec.notes
                        .push(format!("node {id} vanished during recovery"));
                    continue;
                }
                Err(e) => {
                    let mut outcome = ApplyOutcome::empty(n);
                    outcome.created = map;
                    return Err(ApplyError::Api {
                        index,
                        error: e,
                        outcome: Box::new(outcome),
                    });
                }
            }
        }
        let real_parent = node
            .parent_id
            .clone()
            .or_else(|| map_id(parent, &map).ok())
            .unwrap_or_else(|| parent.clone());
        rec.adoptions.push(Adoption {
            node: id,
            parent: real_parent,
            name: name.value().to_owned(),
            note: note.clone(),
            layout: effective_layout(layout),
        });
    }
    for a in carried_adoptions {
        if rec.adoptions.iter().any(|x| x.node == a.node) {
            continue;
        }
        if tree.contains(&a.node) {
            rec.adoptions.push(a);
        } else {
            rec.notes.push(format!(
                "carried adoption {} no longer exists; dropped",
                a.node
            ));
        }
    }
    Ok(Some(rec))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::PlanSummary;

    fn id(i: u32) -> NodeId {
        NodeId::from_api(format!("00000000-0000-4000-8000-{i:012x}"))
    }

    fn tree(nodes: &[(u32, Option<u32>)]) -> Tree {
        Tree::from_nodes(nodes.iter().enumerate().map(|(k, (i, p))| {
            let mut n = Node::new(id(*i), p.map(id), format!("n{i}"));
            n.priority = k as i64 * 100;
            n
        }))
        .unwrap()
    }

    fn scratch(name: &str) -> PathBuf {
        let n: u64 = rand::random();
        std::env::temp_dir().join(format!(
            "syncflowy-apply-unit-{}-{n:016x}/{name}",
            std::process::id()
        ))
    }

    #[test]
    fn nonces() {
        let a = new_nonce();
        assert!(is_nonce(&a), "{a}");
        assert_ne!(a, new_nonce());
        assert!(!is_nonce("⟳syncflowy:xyz"));
        assert!(!is_nonce("syncflowy:0123456789abcdef"));
    }

    #[test]
    fn map_op_rewrites_temp_ids() {
        let created = BTreeMap::from([(NodeId::temp(1), id(9))]);
        let mv = Op::Move {
            node: NodeId::temp(1),
            parent: id(1),
            position: Position::Top,
            expected_parent: Some(NodeId::temp(1)),
        };
        assert_eq!(
            map_op(&mv, &created).unwrap(),
            Op::Move {
                node: id(9),
                parent: id(1),
                position: Position::Top,
                expected_parent: Some(id(9)),
            }
        );
        let c = Op::Create {
            temp: NodeId::temp(2),
            parent: NodeId::temp(1),
            name: CreateName::Direct("x".into()),
            note: None,
            layout: String::new(),
        };
        match map_op(&c, &created).unwrap() {
            Op::Create { temp, parent, .. } => {
                assert_eq!(temp, NodeId::temp(2));
                assert_eq!(parent, id(9));
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(
            map_op(
                &Op::Complete {
                    node: NodeId::temp(3)
                },
                &created
            ),
            Err(NodeId::temp(3))
        );
    }

    #[test]
    fn guards_against_the_latest_tree() {
        // 1 = root, 2 and 3 children, 4 under 3; 8 is outside.
        let t = tree(&[
            (1, None),
            (2, Some(1)),
            (3, Some(1)),
            (4, Some(3)),
            (8, None),
        ]);
        let roots = BTreeSet::from([id(1)]);
        let mv = |n, p| Op::Move {
            node: id(n),
            parent: id(p),
            position: Position::Bottom,
            expected_parent: None,
        };
        assert_eq!(guard(&mv(3, 4), &t, &roots), Some(GuardReason::Cycle));
        assert_eq!(guard(&mv(3, 3), &t, &roots), Some(GuardReason::Cycle));
        assert_eq!(
            guard(&mv(2, 8), &t, &roots),
            Some(GuardReason::OutsideSyncRoots)
        );
        assert_eq!(guard(&mv(1, 2), &t, &roots), Some(GuardReason::Cycle));
        assert_eq!(
            guard(&mv(8, 2), &t, &roots),
            Some(GuardReason::OutsideSyncRoots)
        );
        assert_eq!(guard(&mv(4, 2), &t, &roots), None);
        assert_eq!(guard(&mv(4, 1), &t, &roots), None);
        assert_eq!(
            guard(&mv(4, 99), &t, &roots),
            Some(GuardReason::UnknownNode)
        );
        let del = |n| Op::Delete {
            node: id(n),
            hidden_descendants: 0,
        };
        assert_eq!(guard(&del(1), &t, &roots), Some(GuardReason::OutsideRoot));
        assert_eq!(guard(&del(8), &t, &roots), Some(GuardReason::OutsideRoot));
        assert_eq!(guard(&del(4), &t, &roots), None);
        let create = |p| Op::Create {
            temp: NodeId::temp(1),
            parent: id(p),
            name: CreateName::Direct("x".into()),
            note: None,
            layout: String::new(),
        };
        assert_eq!(guard(&create(1), &t, &roots), None);
        assert_eq!(
            guard(&create(8), &t, &roots),
            Some(GuardReason::OutsideSyncRoots)
        );
    }

    #[test]
    fn verification_ignores_null_vs_empty_notes_of_new_nodes_only() {
        let mut expected = tree(&[(1, None), (2, Some(1))]);
        let mut n = Node::new(NodeId::temp(1), Some(id(1)), "new");
        n.priority = 500;
        expected.insert(n).unwrap();
        let mut actual = tree(&[(1, None), (2, Some(1))]);
        let mut n = Node::new(id(7), Some(id(1)), "new");
        n.note = Some(String::new());
        n.priority = 900;
        actual.insert(n).unwrap();
        actual.set_note(&id(2), Some(String::new())).unwrap();
        let created = BTreeMap::from([(NodeId::temp(1), id(7))]);
        let d = verify(&expected, &actual, &id(1), &created);
        assert_eq!(
            d,
            vec![OutlineDiff::Note {
                id: id(2),
                expected: None,
                actual: Some(String::new())
            }]
        );
    }

    #[test]
    fn journal_round_trip_and_truncated_tail() {
        let path = scratch("journal.jsonl");
        assert_eq!(Journal::read(&path).unwrap(), None);
        let plan = Plan {
            root: id(1),
            ops: vec![Op::Complete { node: id(2) }],
            summary: PlanSummary::default(),
        };
        let begin = JournalEntry::new_begin(&plan, Vec::new());
        let mut j = Journal::begin(&path, &begin).unwrap();
        let intent = JournalEntry::Intent {
            index: 0,
            at: 5,
            nonce: Some("⟳syncflowy:0123456789abcdef".into()),
        };
        j.append(&intent).unwrap();
        j.append(&JournalEntry::Done { index: 0 }).unwrap();
        assert_eq!(
            Journal::read(&path).unwrap().unwrap(),
            vec![
                begin.clone(),
                intent.clone(),
                JournalEntry::Done { index: 0 }
            ]
        );
        // A crash mid-append leaves a cut line: ignored.
        let mut f = OpenOptions::new().append(true).open(&path).unwrap();
        f.write_all(br#"{"type":"skipped","ind"#).unwrap();
        assert_eq!(Journal::read(&path).unwrap().unwrap().len(), 3);
        // A bad line in the middle is an error.
        f.write_all(b"\n{\"type\":\"done\",\"index\":0}\n").unwrap();
        assert_eq!(
            Journal::read(&path).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        // begin replaces an existing journal.
        let j2 = Journal::begin(&path, &begin).unwrap();
        assert_eq!(Journal::read(&path).unwrap().unwrap(), vec![begin.clone()]);
        j2.commit().unwrap();
        assert!(!path.exists());
        assert_eq!(Journal::read(&path).unwrap(), None);
        drop(j);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn appending_after_a_torn_tail_keeps_the_new_entry_readable() {
        let path = scratch("journal.jsonl");
        let plan = Plan {
            root: id(1),
            ops: vec![Op::Complete { node: id(2) }],
            summary: PlanSummary::default(),
        };
        let begin = JournalEntry::new_begin(&plan, Vec::new());
        drop(Journal::begin(&path, &begin).unwrap());
        let done = JournalEntry::Done { index: 0 };
        let raw = |bytes: &[u8]| {
            let mut f = OpenOptions::new().append(true).open(&path).unwrap();
            f.write_all(bytes).unwrap();
        };
        // A cut line is dropped, so the appended entry starts a line of its own.
        raw(br#"{"type":"skipped","ind"#);
        Journal::open_append(&path).unwrap().append(&done).unwrap();
        assert_eq!(
            Journal::read(&path).unwrap().unwrap(),
            vec![begin.clone(), done.clone()]
        );
        // Further appends stay readable (no InvalidData from a bad middle line).
        Journal::open_append(&path).unwrap().append(&done).unwrap();
        assert_eq!(Journal::read(&path).unwrap().unwrap().len(), 3);
        // A complete entry that only lost its newline is kept, not cut.
        let mut line = entry_line(&done).unwrap();
        line.pop();
        raw(&line);
        assert_eq!(Journal::read(&path).unwrap().unwrap().len(), 4);
        Journal::open_append(&path).unwrap().append(&done).unwrap();
        assert_eq!(Journal::read(&path).unwrap().unwrap().len(), 5);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn journal_must_begin_with_begin() {
        let path = scratch("journal.jsonl");
        assert!(Journal::begin(&path, &JournalEntry::Done { index: 0 }).is_err());
    }
}
