//! Execute a plan against the API with a write-ahead journal, and recover from a crash.
//!
//! Responsibility (design §7.1 pre-write checks and post-apply verification, §7.2 the
//! two-step create, §7.3 journal and crash recovery, §8.2 guards re-checked live, §9
//! races). This is I/O: it talks to the API through [`crate::wfclient::Api`] and writes
//! the journal file.
//!
//! Consumes: [`crate::plan`] (`Plan`, `Op`, `CreateName`, `simulate_op`, `GuardReason`),
//! [`crate::wfclient`] (`Api`, requests, `ApiError`), [`crate::tree`] (`Tree`,
//! `outline_diff`).
//! Consumed by: `sync` (runs [`Applier::apply`] each cycle and [`recover`] at start-up,
//! commits the journal after the file and checkpoint are written); `merge` (consumes
//! [`Adoption`]s).
//!
//! # Executing a plan
//!
//! For each op, in order, with temp ids mapped to the real ids created so far:
//!
//! 1. Re-check the §8.2 guards against the applier's latest known tree (the remote tree
//!    passed in, updated with `plan::simulate_op` after every successful op): a move's
//!    destination must not be the node or its descendant, and must lie inside one of the
//!    applier's `sync_roots`. A failure skips the op ([`SkipReason::Guard`]); a refused
//!    move is never sent.
//! 2. With [`ApplyOptions::pre_write_check`] (default on), `GET` the node first and skip
//!    the op if it no longer matches: `Update*` when the current value differs from
//!    `expected` ([`SkipReason::ChangedRemotely`]); `Move` when the live parent differs
//!    from `expected_parent`; `Complete` when already completed
//!    ([`SkipReason::AlreadyCompleted`]: never re-complete, DM§4.5 re-stamps
//!    `completedAt`); any op on a 404 ([`SkipReason::Gone`]). For a `Move` also `GET` the
//!    destination and its ancestors up to a sync root, and skip with
//!    `SkipReason::Guard(GuardReason::Cycle)` if the node is among them: the API would
//!    accept the move and detach the subtree (DM§4.7.5), so the cycle check must also
//!    hold against live data, not only the snapshot (§2, §8.2).
//! 3. Append [`JournalEntry::Intent`] (fsync) immediately before sending.
//! 4. Send it. `Create` with `CreateName::Direct(name)`: one `create` with that name, the
//!    note and the layout at `Position::Bottom`. With `CreateName::Deferred(name)`: a
//!    `create` whose name is a fresh nonce `⟳syncflowy:<16 hex>` (recorded in the
//!    `Intent`), then `update {name}`. Record [`JournalEntry::Created`] as soon as the
//!    id is known (between the two steps).
//! 5. Append [`JournalEntry::Done`] or [`JournalEntry::Skipped`].
//!
//! A failed request ([`crate::wfclient::ApiError`] after the client's retries) stops the
//! apply: append [`JournalEntry::Failed`] and return [`ApplyError::Api`] carrying the
//! outcome so far. A create whose transport error says `maybe_applied` is resolved first
//! by an export scan for its nonce (or, for a direct create, for an unknown child of the
//! parent with exactly that name created after the intent's timestamp).
//!
//! After the last op: if [`ApplyOptions::verify`], export once and compare the root's
//! outline with the expected tree (`M` with temp ids mapped) using
//! `tree::outline_diff(.., DiffScope::Visible)`; differences are returned as
//! `discrepancies` (a concurrent edit or a bug; the next cycle picks them up as remote
//! changes, §7.1) and are not errors. Treat `note: None` and `Some("")` as equal for
//! nodes created in this apply. Append [`JournalEntry::Applied`].
//!
//! # Journal lifecycle (one per sync cycle)
//!
//! `sync` creates the journal ([`Journal::begin`]) before the first op and calls
//! [`Journal::commit`] (which deletes the file) only after the new Markdown file **and**
//! the new checkpoint are durably written. A journal that still exists at start-up
//! therefore means the previous cycle did not finish, whatever the crash point.
//!
//! # Recovery (§7.3)
//!
//! [`recover`] reads the journal, exports, and for every create that may have happened:
//! finds the node (the `Created` id, else the nonce, else the direct-create heuristic);
//! if it still carries the nonce as its name, finishes step 2 (writes the intended name);
//! and returns it as an [`Adoption`] so that the next merge pairs it with the still
//! untagged Markdown item instead of creating a duplicate. Other ops need no replay: the
//! next merge sees their effects as remote changes equal to the local ones. Adoptions
//! listed in the journal's `Begin` entry (carried from an earlier recovery) are returned
//! too, so a crash during a recovering cycle loses nothing. Afterwards the caller starts
//! a new cycle, whose `Begin` carries the adoptions, and the old journal is replaced.

#![allow(unused_variables, dead_code)] // stubs; remove when implemented

use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::plan::{GuardReason, Plan};
use crate::tree::{NodeId, OutlineDiff, Tree};
use crate::wfclient::{Api, ApiError};

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

/// The write-ahead journal of one root (`state::StateStore::journal_path`).
#[derive(Debug)]
pub struct Journal {
    path: PathBuf,
    file: File,
}

impl Journal {
    /// Start a new journal at `path` with its `Begin` entry, atomically replacing any
    /// existing journal (temp file, fsync, rename). The caller must have run [`recover`]
    /// on an existing journal first and carried its adoptions into `begin`.
    pub fn begin(path: &Path, begin: &JournalEntry) -> std::io::Result<Journal> {
        todo!()
    }

    /// Append one entry and fsync.
    pub fn append(&mut self, entry: &JournalEntry) -> std::io::Result<()> {
        todo!()
    }

    /// The cycle finished durably: delete the journal file (and fsync the directory).
    pub fn commit(self) -> std::io::Result<()> {
        todo!()
    }

    /// Read a journal; `Ok(None)` if there is no file or it is empty. A truncated last
    /// line (crash mid-write) is ignored.
    pub fn read(path: &Path) -> std::io::Result<Option<Vec<JournalEntry>>> {
        todo!()
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

/// Executes plans (module docs).
pub struct Applier<'a> {
    api: &'a dyn Api,
    opts: ApplyOptions,
    sync_roots: BTreeSet<NodeId>,
}

impl<'a> Applier<'a> {
    /// An applier over `api`. `sync_roots` are all roots managed in this run (the
    /// boundary guard, §8.2; the same set as `plan::PlanConfig::sync_roots`).
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
        todo!()
    }
}

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

/// Reconcile a non-empty journal at `path` against a fresh export (module docs).
/// `Ok(None)` if there is no journal. Does not delete the journal: the caller begins a
/// new cycle (whose `Begin` carries `adoptions`), which replaces it.
pub fn recover(path: &Path, api: &dyn Api) -> Result<Option<Recovery>, ApplyError> {
    todo!()
}
