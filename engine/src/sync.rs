//! One sync cycle: for one file, and for several files sharing one export.
//!
//! Responsibility (design §1 the five steps, §5.1 fast path, §6.4 several files in one
//! run, §7 applying, §8.3 brakes and missing files, §9 compare-and-swap write, §9.1
//! guards 1, 2, 4, 5 and the frontmatter controls, §14 step 3 `render`). This is the
//! I/O shell around the pure core: it reads and writes the Markdown files, the state
//! store and the journal, and calls the API.
//!
//! Consumes: every other module: `wfclient::Api` (export), `mdblock` (parse, `View`,
//! render), `merge`, `plan`, `apply` (`Applier`, `Journal`, `recover`), `state`
//! (`StateStore`, `Checkpoint`, history, pending deletes), `tree`.
//! Consumed by: `main` (the `render`, `sync` and `status` commands) and `daemon`.
//!
//! # The cycle for one file (`sync_once` = `sync_all` with one file)
//!
//! 1. **Read** the file. Missing: [`SyncOutcome::Paused`] with
//!    [`PauseReason::FileMissing`] (never delete anything, §8.3, §9.1 guard 5). Frontmatter
//!    unparseable: [`SyncOutcome::Skipped`] (retry later; never "everything deleted").
//!    `paused: true`: [`SyncOutcome::Paused`].
//! 2. **Export** once (shared by all files of the run) and resolve the root
//!    (`RootRef::resolve` against the export's `IdMap`; unknown: `Error::RootNotFound`).
//!    Take the root's lock (`StateStore::lock`).
//! 3. **Recover** if the root has a journal (`apply::recover`): its adoptions feed the
//!    merge and are carried into this cycle's journal.
//! 4. **Choose the base** (§9.1 guard 1): if the file's `gen` equals the checkpoint's,
//!    base = checkpoint (`base_md`, `base_wf`); if it names an older generation still in
//!    the history, base = that history entry (a rebase); otherwise (missing, unknown)
//!    base = checkpoint and `MergeConfig::safe_mode`. `prior_docs` = the other kept
//!    generations (at most the 20 newest). No checkpoint at all: `Error::NoCheckpoint`,
//!    unless `force`, which bootstraps with base_wf = remote subtree, base_md = a fresh
//!    render of it, and safe mode (UNVERIFIED design choice: local edits made since a
//!    lost checkpoint may overwrite remote edits; hence the `--force`).
//! 5. **Fast path** (§5.1): if the file text equals the base text and the remote subtree
//!    has no outline difference from base_wf (`tree::outline_diff`, `DiffScope::All`),
//!    return [`SyncOutcome::Unchanged`] without writing anything.
//! 6. **Merge** (`merge::merge`) with `CrossFile` built from the other files of the run.
//! 7. **Deferred deletes** (§9.1 guard 2, only when [`SyncConfig::delete_grace`] is set,
//!    i.e. in the daemon): record `merge.deletes` in the checkpoint's `PendingDeletes`;
//!    deletes not yet due go into `PlanConfig::deferred_deletes` and into
//!    `ViewExtras::hidden`; cancelled ones are removed.
//! 8. **Plan** (`plan::plan`) with `force = cfg.force || frontmatter.force`. A brake gives
//!    [`SyncOutcome::Braked`]: nothing is written anywhere.
//! 9. **Dry run** ([`SyncConfig::dry_run`]): return [`SyncOutcome::DryRun`] with the plan.
//! 10. **Apply** if the plan is not empty: `Journal::begin` (with the carried adoptions)
//!     at `StateStore::journal_path`, then `Applier::apply`. An error returns
//!     `Error::Apply` and leaves the journal for the next run's recovery.
//! 11. **Re-render** from the verification export (or a fresh one): `View::build` on the
//!     new remote tree with the merge's conflicts, pending deletes hidden, and `Sticky`
//!     checkboxes from the current file; the frontmatter is the file's own with `force`
//!     cleared and `gen = checkpoint gen + 1`.
//! 12. **Write** unless guard 4 says not to: if the merge had no local changes and the
//!     new render is `mdblock::equivalent` to the current file (pure normalisation),
//!     write nothing. Otherwise: save the history entry of the new generation, then
//!     **compare-and-swap** the file (§9): re-read it; if it changed since step 1, do not
//!     write, keep the journal, and rerun from step 4 with the new text and this cycle's
//!     creates as adoptions (at most [`SyncConfig::cas_retries`] times, then
//!     [`SyncOutcome::Skipped`]; the next run recovers). Otherwise write it atomically
//!     (`state::atomic_write`).
//! 13. **Commit**: save the checkpoint (new gen, base_wf = new remote subtree, base_md =
//!     the text written, ids extended with the export, config echo, pending deletes),
//!     prune the history, then `Journal::commit`.
//!
//! Deviation from §9 (recorded here deliberately): the design updates the checkpoint
//! before rerunning a merge after a failed CAS. Here the checkpoint changes only when the
//! file is written, and the uncommitted journal (via adoptions) protects the creates
//! already pushed; that keeps "the file names its own base" (guard 1) true at all times.
//!
//! # Several files (§6.4)
//!
//! `sync_all` parses every file first. Two files naming the same root: both are
//! [`SyncOutcome::Paused`] with [`PauseReason::DuplicateRoot`] (guard 5). A root inside
//! another root of the run: the inner file is skipped (`Error::NestedRoots`). Then one
//! export; then every merge (with `CrossFile.other_roots` = the other roots and
//! `claimed_elsewhere` = nodes tagged in the other files); then every plan and apply in
//! file order (cross-file moves are planned by the destination file); then one
//! re-export; then each file's render, write and commit. A file whose apply failed is
//! not rendered; the other files continue.

#![allow(unused_variables, dead_code)] // stubs; remove when implemented

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::apply::{ApplyOptions, Recovery};
use crate::error::Error;
use crate::mdblock::{ConflictMarker, DocConfig};
use crate::merge::ReportEvent;
use crate::plan::{Brake, BrakeConfig, Plan, PlanSummary};
use crate::state::HistoryLimits;
use crate::tree::{NodeId, OutlineDiff};
use crate::wfclient::Api;

/// Options of a sync run.
#[derive(Debug, Clone)]
pub struct SyncConfig {
    /// The state directory (outside the vault).
    pub state_dir: PathBuf,
    /// `--force`: skip brakes, allow placeholder edits/deletes, allow bootstrap without a
    /// checkpoint, allow `render` over an existing file.
    pub force: bool,
    /// Plan only; write nothing anywhere.
    pub dry_run: bool,
    /// Brake thresholds (§8.3).
    pub brakes: BrakeConfig,
    /// `Some(grace)`: defer deletes (§9.1 guard 2, the daemon); `None`: delete at once
    /// (the CLI, where the user is present).
    pub delete_grace: Option<Duration>,
    /// History retention (§9.1 guard 1).
    pub history: HistoryLimits,
    /// Applier options.
    pub apply: ApplyOptions,
    /// §7.2 fast path.
    pub fast_path_creates: bool,
    /// New items typed `[ ]` become `todo` (`merge::MergeConfig::todo_from_checkbox`).
    pub todo_from_checkbox: bool,
    /// Compare-and-swap retries (§9). Default 3.
    pub cas_retries: u32,
    /// The clock (unix seconds); tests inject a fixed one.
    pub clock: fn() -> i64,
}

/// The system clock in unix seconds.
pub fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

impl SyncConfig {
    /// Defaults for a CLI run with state in `state_dir`.
    pub fn new(state_dir: impl Into<PathBuf>) -> SyncConfig {
        SyncConfig {
            state_dir: state_dir.into(),
            force: false,
            dry_run: false,
            brakes: BrakeConfig::default(),
            delete_grace: None,
            history: HistoryLimits::default(),
            apply: ApplyOptions::default(),
            fast_path_creates: true,
            todo_from_checkbox: false,
            cas_retries: 3,
            clock: unix_now,
        }
    }
}

/// Why a root is paused.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PauseReason {
    /// `paused: true` in the frontmatter.
    Frontmatter,
    /// The file is gone (§9.1 guard 5: pause and report, never delete).
    FileMissing,
    /// Several files claim the root (guard 5).
    DuplicateRoot(Vec<PathBuf>),
}

/// What one file's cycle did.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SyncOutcome {
    /// Nothing changed on either side; nothing written (§5.1 fast path).
    Unchanged,
    /// Changes were pushed and/or the file re-rendered.
    Synced {
        /// What was pushed.
        pushed: PlanSummary,
        /// True if the Markdown file was rewritten.
        wrote_file: bool,
        /// The file's generation afterwards.
        generation: u64,
    },
    /// `render_once` wrote a fresh file.
    Rendered {
        /// Generation written (1 for a new root).
        generation: u64,
    },
    /// Dry run: what would be done.
    DryRun {
        /// The plan.
        plan: Plan,
    },
    /// Not synced, by request or by guard 5.
    Paused(PauseReason),
    /// A brake engaged; nothing written (§8.3).
    Braked(Brake),
    /// Skipped this cycle (unparseable file, lock held, CAS retries exhausted, …).
    Skipped {
        /// Why.
        reason: String,
    },
}

/// The report of one file's cycle.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SyncReport {
    /// The file.
    pub file: PathBuf,
    /// Its root, once resolved.
    pub root: Option<NodeId>,
    /// The outcome.
    pub outcome: SyncOutcome,
    /// Merge events.
    pub events: Vec<ReportEvent>,
    /// Conflict markers now in the file.
    pub conflicts: Vec<ConflictMarker>,
    /// Post-apply verification differences (§7.1).
    pub discrepancies: Vec<OutlineDiff>,
    /// Parse warnings and other notes.
    pub warnings: Vec<String>,
    /// What crash recovery did at the start of this cycle.
    pub recovered: Option<Recovery>,
}

/// One file's result in a multi-file run.
#[derive(Debug)]
pub struct FileResult {
    /// The file.
    pub file: PathBuf,
    /// Its report or its error.
    pub result: Result<SyncReport, Error>,
}

/// Status of one root, from local state only (no API calls), for `syncflowy status` and
/// the daemon's status note.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RootStatus {
    /// The root.
    pub root: NodeId,
    /// The file last synced.
    pub file: Option<PathBuf>,
    /// Checkpoint generation.
    pub generation: u64,
    /// Unix seconds of the last completed cycle.
    pub synced_at: i64,
    /// Pending deletes: (node, label, first seen).
    pub pending_deletes: Vec<(NodeId, String, i64)>,
    /// True if an unfinished journal exists (the next run recovers).
    pub journal_pending: bool,
    /// True if the file's current text differs from the checkpoint's `base_md`.
    pub locally_modified: Option<bool>,
}

/// `syncflowy render <root> <out.md>` (§14 step 3): export, resolve `root` (short id,
/// UUID or URL), refuse if `out` exists or the root has a checkpoint (unless `force`),
/// build the view with `doc`, render with `gen: 1`, save history entry 1, write the file,
/// save the checkpoint.
pub fn render_once(
    api: &dyn Api,
    root: &str,
    out: &Path,
    doc: &DocConfig,
    cfg: &SyncConfig,
) -> Result<SyncReport, Error> {
    todo!()
}

/// One cycle for one file (module docs).
pub fn sync_once(api: &dyn Api, file: &Path, cfg: &SyncConfig) -> Result<SyncReport, Error> {
    todo!()
}

/// One cycle for several files sharing one export (§6.4). `Err` only for run-wide
/// failures (the export); per-file failures are in the `FileResult`s, in input order.
pub fn sync_all(
    api: &dyn Api,
    files: &[PathBuf],
    cfg: &SyncConfig,
) -> Result<Vec<FileResult>, Error> {
    todo!()
}

/// Status of every root in the state directory, or of the roots of `files` if given.
pub fn status(cfg: &SyncConfig, files: &[PathBuf]) -> Result<Vec<RootStatus>, Error> {
    todo!()
}
