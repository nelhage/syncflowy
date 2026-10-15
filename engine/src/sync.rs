//! One sync cycle: for one file, and for several files sharing one export.
//!
//! Responsibility (design §1 the five steps, §5.1 fast path, §6.4 several files in one
//! run, §7 applying, §8.3 brakes and missing files, §9 compare-and-swap write, §9.1
//! guards 1, 2, 4, 5 and the frontmatter controls, §14 step 3 `render`). This is the
//! I/O shell around the pure core: it reads and writes the Markdown files, the state
//! store and the journal, and calls the API.
//!
//! Consumes: every other module: `wfclient::Api` (export), `mdblock` (parse, `View`,
//! render), `merge`, `plan`, `apply` (`Applier`, `Journal`, `recover`, `verify`), `state`
//! (`StateStore`, `Checkpoint`, history, pending deletes), `tree`.
//! Consumed by: `main` (the `render`, `sync` and `status` commands) and `daemon`.
//!
//! # The cycle for one file (`sync_once` = `sync_all` with one file)
//!
//! 1. **Read** the file. Missing: [`SyncOutcome::Paused`] with
//!    [`PauseReason::FileMissing`] (never delete anything, §8.3, §9.1 guard 5), before
//!    any API call. Not UTF-8, or frontmatter unparseable (guard 4 "parse by content"):
//!    [`SyncOutcome::Skipped`] (retry later; never "everything deleted").
//!    `paused: true`: [`SyncOutcome::Paused`].
//! 2. **Export** once (shared by all files of the run) and resolve the root
//!    (`RootRef::resolve` against the export's `IdMap`; unknown: `Error::RootNotFound`).
//!    Take the root's lock (`StateStore::lock`; held elsewhere: `Skipped`).
//! 3. **Recover** if the root has a journal (`apply::recover`): its adoptions feed the
//!    merge and are carried into this cycle's journal. After any recovery the export is
//!    taken again (recovery may have written names).
//! 4. **Choose the base** (§9.1 guard 1): if the file's `gen` equals the checkpoint's,
//!    base = checkpoint (`base_md`, `base_wf`); if it names another generation still in
//!    the history, base = that history entry (a rebase; this also covers a crash between
//!    the file write and the checkpoint, when the file is one generation ahead);
//!    otherwise (missing, unknown) base = checkpoint and `MergeConfig::safe_mode`.
//!    `prior_docs` = the other kept generations (at most [`PRIOR_DOCS_MAX`] newest). The
//!    merge uses the base file's config (visibility must agree with the base); the
//!    render uses the current file's. No checkpoint at all: `Error::NoCheckpoint`,
//!    unless `force`, which bootstraps with base_wf = remote subtree, base_md = a fresh
//!    render of it, and safe mode (UNVERIFIED design choice: local edits made since a
//!    lost checkpoint may overwrite remote edits; hence the `--force`).
//! 5. **Fast path** (§5.1): if the base is the checkpoint, the file text equals
//!    `base_md`, the remote subtree has no outline difference from base_wf
//!    (`tree::outline_diff`, `DiffScope::All`), and there is no journal, no pending
//!    delete and no force: return [`SyncOutcome::Unchanged`] without writing anything.
//! 6. **Merge** (`merge::merge`) with `CrossFile` built from the other files of the run,
//!    `force = cfg.force || frontmatter.force`. With [`SyncConfig::strip_tasks_fields`]
//!    (the daemon), the local doc first loses the Tasks-plugin done dates that are new
//!    on ticked lines (§9.1 guard 6, `daemon::strip_tasks_fields`).
//! 7. **Deferred deletes** (§9.1 guard 2, only when [`SyncConfig::delete_grace`] is set,
//!    i.e. in the daemon): record `merge.deletes` in the checkpoint's `PendingDeletes`;
//!    deletes not yet due go into `PlanConfig::deferred_deletes` and into
//!    `ViewExtras::hidden`; cancelled ones are removed. Without a grace period every
//!    delete (including ones a daemon left pending) is sent now.
//! 8. **Plan** (`plan::plan`) with the same `force`. A brake gives
//!    [`SyncOutcome::Braked`]: nothing is written anywhere (§8.3).
//! 9. **Dry run** ([`SyncConfig::dry_run`]): return [`SyncOutcome::DryRun`] with the plan.
//!    A dry run skips step 3 (recovery writes to Workflowy and to the journal) and warns
//!    that a journal is pending instead; the next real run recovers it.
//! 10. **Apply** if the plan is not empty: `Journal::begin` (with the carried adoptions)
//!     at `StateStore::journal_path`, then `Applier::apply` without its own verification
//!     export. An error returns `Error::Apply` and leaves the journal for the next run's
//!     recovery.
//! 11. **Re-export and verify** once for the run (`apply::verify` per file against the
//!     shared export, §7.1). If the applier skipped local changes because a
//!     collaborator changed or deleted their nodes after our export (a name, note or
//!     layout edit failing the pre-write GET, a create whose parent is gone, …:
//!     `unpushed_local_changes`), the render below would drop them from the file with no
//!     marker. Instead the cycle reruns from step 4 exactly as after a CAS conflict
//!     (step 12): the base keeps only the ops that were applied, so the next merge sees
//!     the skipped edit as a local change against a remote one (§6.1 conflict marker)
//!     or a local edit of a remotely deleted node (§6.2 resurrection). When the retries
//!     run out, nothing is committed ([`SyncOutcome::Skipped`]; the next run recovers
//!     the journal and merges again), so a `base_md` never drops unpushed local
//!     content. Otherwise **re-render** from the new export: `View::build` on the new remote
//!     tree with the merge's conflicts, pending deletes hidden, and `Sticky` checkboxes
//!     from the current file; the frontmatter is the file's own with `force` cleared.
//! 12. **Write** unless guard 4 says not to: if the base is the checkpoint, the file has
//!     no `force: true`, and the new render (with the current `gen`) is byte-identical or
//!     `mdblock::equivalent` to the current file, nothing the user needs to see changed
//!     (pure normalisation is deferred): the file is not written and `gen` stays.
//!     Otherwise: `gen` = one more than both the checkpoint's and every kept history
//!     generation; **compare-and-swap** (§9): re-read the file; if it changed since step
//!     1, do not write, keep the journal, and rerun from step 4 with the new text, the
//!     latest export and this cycle's creates as adoptions (at most
//!     [`SyncConfig::cas_retries`] times, then [`SyncOutcome::Skipped`]; the next run
//!     recovers). The history entry of the new generation is saved *before* that
//!     re-read, and the new text is staged in a temp file, so that only the re-read and
//!     a rename separate the check from the write (`write_if_unchanged`); the entry of a
//!     generation that lost the CAS is removed again.
//! 13. **Commit**: save the checkpoint (gen, base_wf = new remote subtree, base_md = the
//!     text now in the file (written or kept), ids extended, config echo, pending
//!     deletes) and, when the file was not written, the history entry of the unchanged
//!     generation (so that history[gen] always matches the checkpoint); prune the
//!     history; then `Journal::commit` (or remove a recovered journal).
//!
//! Deviation from §9 (recorded here deliberately): the design saves the checkpoint
//! before rerunning a merge after a failed CAS. Here the checkpoint is saved only at the
//! end of a cycle, together with the file it describes, so "the file names its own
//! base" (guard 1) holds at all times. The rerun instead keeps an in-memory base: the
//! pass's `base_md` with a `base_wf` updated by the field writes and re-parentings that
//! were applied (so a user's further edit of a line we just pushed is a plain local
//! edit, not a conflict with our own push), and this cycle's creates as adoptions; the
//! uncommitted journal protects those creates if the process dies before the commit.
//! Nodes moved in from another file are not added to that base: the rerun keeps the
//! run's `CrossFile`, and the merge still compares their pasted lines with the source
//! file's base (`CrossFile::bases`, §6.4), so a skipped edit of a pasted line is a
//! conflict like any other.
//!
//! # Mirrors (§15.1, `crate::mirrors`)
//!
//! On an export with the beta API's mirror metadata (DM§10.5), every file works on its
//! own **view tree** (`mirrors::view_tree` over the run's export, with the full
//! placements of its base as the sticky choice): the merge's remote, the base of the
//! fast path and of the next checkpoint, and the render's tree are all views. The
//! planner and the applier work on the **real tree** (`mirrors::real_tree`), and the
//! merge result is translated between the two with `mirrors::to_real` (the view-space
//! result keeps the conflicts, report and deletes for the file; the real-space one
//! goes to `plan`, `apply` and the verification). A CAS rerun maps the applied real
//! ops back onto the view base (`rebase_after`). Without metadata both trees are the
//! export and nothing changes.
//!
//! # Several files (§6.4)
//!
//! `sync_all` parses every file first. Two files naming the same root: both are
//! [`SyncOutcome::Paused`] with [`PauseReason::DuplicateRoot`] (guard 5). A root inside
//! another root of the run: the inner file fails with `Error::NestedRoots` (the outer
//! one is synced). Then one export; then every merge (with `CrossFile.other_roots` = the
//! other roots, `claimed_elsewhere` = nodes tagged in the other files that are not in
//! those files' own view scope (a node mirrored into two files is in both scopes and
//! claimed by neither), resolved with one global id map, and `bases` = for the nodes of each other root that this file
//! pastes, that file's base as its own merge chooses it (step 4) and its older renders,
//! so the move merges the pasted line against what was cut, `merge::cross_bases`);
//! then every plan and apply in file order (cross-file moves are
//! planned by the destination file); then one re-export; then each file's render, write
//! and commit. A file whose apply failed is not rendered; the other files continue.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::apply::{
    self, Adoption, ApplyError, ApplyOptions, ApplyOutcome, Journal, JournalEntry, Recovery,
};
use crate::error::Error;
use crate::mdblock::{
    self, Checkbox, ConflictMarker, DocConfig, Frontmatter, LocalDoc, RootRef, Sticky, View,
    ViewExtras,
};
use crate::merge::{self, CrossBase, CrossFile, MergeConfig, MergeInput, MergeResult, ReportEvent};
use crate::mirrors;
use crate::plan::{self, Brake, BrakeConfig, Op, Plan, PlanConfig, PlanError, PlanSummary};
use crate::state::{
    self, Checkpoint, HistoryEntry, HistoryLimits, PendingDeletes, RootLock, StateError,
    StateStore, fingerprint,
};
use crate::tree::{DiffScope, IdMap, NodeId, OutlineDiff, Tree, outline_diff};
use crate::wfclient::Api;

/// The number of older generations parsed as `prior_docs` for guard 3.
pub const PRIOR_DOCS_MAX: usize = 20;

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
    /// The clock (unix seconds); tests and the daemon inject their own.
    pub clock: Clock,
    /// §9.1 guard 6: strip known Tasks-plugin fields (`✅ YYYY-MM-DD`) that are new on
    /// ticked lines before the merge sees them (`daemon::strip_tasks_fields`). The
    /// daemon sets it; the CLI default is off.
    pub strip_tasks_fields: bool,
}

/// The system clock in unix seconds.
pub fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// A source of unix seconds ([`SyncConfig::clock`]). The real clock is used only by
/// `main`; tests and the daemon's fake time inject their own.
#[derive(Clone)]
pub struct Clock(Arc<dyn Fn() -> i64 + Send + Sync>);

impl Clock {
    /// The system clock ([`unix_now`]).
    pub fn system() -> Clock {
        Clock(Arc::new(unix_now))
    }

    /// A clock reading `f`.
    pub fn new(f: impl Fn() -> i64 + Send + Sync + 'static) -> Clock {
        Clock(Arc::new(f))
    }

    /// A clock that always reads `t`.
    pub fn fixed(t: i64) -> Clock {
        Clock::new(move || t)
    }

    /// The current time in unix seconds.
    pub fn now(&self) -> i64 {
        (self.0)()
    }
}

impl std::fmt::Debug for Clock {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Clock(now = {})", self.now())
    }
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
            clock: Clock::system(),
            strip_tasks_fields: false,
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
    /// Human-readable log: the merge events described with node names, and one line
    /// per applied op (for the CLI and the status note).
    #[serde(default)]
    pub messages: Vec<String>,
}

impl SyncReport {
    fn new(file: &Path, root: Option<NodeId>, outcome: SyncOutcome) -> SyncReport {
        SyncReport {
            file: file.to_path_buf(),
            root,
            outcome,
            events: Vec::new(),
            conflicts: Vec::new(),
            discrepancies: Vec::new(),
            warnings: Vec::new(),
            recovered: None,
            messages: Vec::new(),
        }
    }
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

// ---------------------------------------------------------------------------------------
// Small I/O helpers
// ---------------------------------------------------------------------------------------

/// Read a Markdown file: `Ok(None)` if it does not exist.
fn read_text(path: &Path) -> std::io::Result<Option<String>> {
    match std::fs::read(path) {
        Ok(bytes) => String::from_utf8(bytes)
            .map(Some)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

fn io_error(path: &Path) -> impl FnOnce(std::io::Error) -> Error + '_ {
    move |source| Error::Io {
        path: path.to_path_buf(),
        source,
    }
}

fn export_tree(api: &dyn Api) -> Result<Tree, Error> {
    Ok(Tree::from_nodes(api.export()?)?)
}

/// The path recorded in the checkpoint (absolute when possible; informational only,
/// files are found by root, §9.1 guard 5).
fn recorded_path(file: &Path) -> PathBuf {
    std::fs::canonicalize(file).unwrap_or_else(|_| file.to_path_buf())
}

/// The generation after `current`, also above every kept history entry, so that a
/// generation a file might already carry (a crash after the history entry) is never
/// reused for different text.
fn next_generation(store: &StateStore, root: &NodeId, current: Option<u64>) -> Result<u64, Error> {
    let newest = store.history_gens(root)?.last().copied();
    Ok(current.max(newest).unwrap_or(0) + 1)
}

fn parse_warnings(doc: &LocalDoc) -> Vec<String> {
    doc.warnings
        .iter()
        .map(|w| format!("line {}: {}", w.line, w.message))
        .collect()
}

fn add_summary(acc: &mut PlanSummary, s: &PlanSummary) {
    acc.creates += s.creates;
    acc.updates += s.updates;
    acc.moves += s.moves;
    acc.reorders += s.reorders;
    acc.completes += s.completes;
    acc.deletes += s.deletes;
    acc.hidden_deleted += s.hidden_deleted;
    acc.requests += s.requests;
}

/// Sticky checkboxes (§9.1 guard 4): the box each tagged item has in the current file.
fn sticky_from(doc: &LocalDoc, ids: &IdMap) -> Sticky {
    let mut sticky = Sticky::default();
    for (_, item) in doc.walk() {
        if item.is_marker() || item.checkbox == Checkbox::Ticked {
            continue;
        }
        if let Some(id) = item.tag.as_ref().and_then(|t| ids.get(&t.short)) {
            sticky.checkbox.insert(id.clone(), item.checkbox);
        }
    }
    sticky
}

/// Nodes tagged in `doc` (winning tags of non-marker items), resolved through `ids`.
fn tagged_nodes(doc: &LocalDoc, ids: &IdMap) -> BTreeSet<NodeId> {
    doc.walk()
        .into_iter()
        .filter(|(_, it)| !it.is_marker())
        .filter_map(|(_, it)| it.tag.as_ref().and_then(|t| ids.get(&t.short)).cloned())
        .collect()
}

// ---------------------------------------------------------------------------------------
// render
// ---------------------------------------------------------------------------------------

/// `syncflowy render <root> <out.md>` (§14 step 3): export, resolve `root` (short id,
/// UUID or URL), refuse if `out` exists or the root has a checkpoint (unless `force`),
/// build the view with `doc`, render with `gen: 1` (or one past the generations a
/// forced re-render finds), save the history entry, write the file, save the
/// checkpoint. A leftover journal of the root is recovered first (so half-created nodes
/// get their names) and then removed.
pub fn render_once(
    api: &dyn Api,
    root: &str,
    out: &Path,
    doc: &DocConfig,
    cfg: &SyncConfig,
) -> Result<SyncReport, Error> {
    if !cfg.force && std::fs::symlink_metadata(out).is_ok() {
        return Err(Error::FileExists(out.to_path_buf()));
    }
    let store = StateStore::open(&cfg.state_dir)?;
    let root_ref = RootRef::parse(root)?;
    let mut remote = export_tree(api)?;
    let root_id = root_ref
        .resolve(&IdMap::from_tree(&remote))
        .filter(|r| remote.contains(r))
        .ok_or_else(|| Error::RootNotFound(root.to_owned()))?;
    let _lock = store.lock(&root_id)?;

    let existing = match store.load_checkpoint(&root_id) {
        Ok(cp) => cp,
        Err(e) if cfg.force => {
            log::warn!("ignoring the unreadable checkpoint of {root_id}: {e}");
            None
        }
        Err(e) => return Err(e.into()),
    };
    if existing.is_some() && !cfg.force {
        return Err(Error::CheckpointExists(root_id));
    }

    let mut report = SyncReport::new(out, Some(root_id.clone()), SyncOutcome::Unchanged);
    let journal = store.journal_path(&root_id);
    if journal.exists() {
        report.recovered = apply::recover(&journal, api)?;
        remote = export_tree(api)?;
    }

    let now = cfg.clock.now();
    let generation = next_generation(&store, &root_id, existing.as_ref().map(|c| c.generation))?;
    // §15.1: the root's view of the export (sticky to a forced re-render's base).
    let previous = existing
        .as_ref()
        .map(|cp| mirrors::full_placements(&cp.base_wf))
        .unwrap_or_default();
    let remote = mirrors::view_tree(&remote, &root_id, &previous);
    let view = View::build(&remote, &root_id, doc, &ViewExtras::default())?;
    if view.root_completed {
        report
            .warnings
            .push("the sync root is completed in Workflowy".to_owned());
    }
    let mut fm = Frontmatter::new(root_ref);
    fm.config = doc.clone();
    fm.generation = Some(generation);
    let md = mdblock::render(&view, &fm);
    let subtree = remote
        .subtree(&root_id)
        .ok_or_else(|| Error::RootNotFound(root.to_owned()))?;

    store.put_history(
        &root_id,
        &HistoryEntry {
            generation,
            md: md.clone(),
            tree: subtree.clone(),
            at: now,
        },
    )?;
    if let Some(dir) = out.parent()
        && !dir.as_os_str().is_empty()
    {
        std::fs::create_dir_all(dir).map_err(io_error(dir))?;
    }
    state::atomic_write(out, md.as_bytes()).map_err(io_error(out))?;

    let mut cp = Checkpoint::new(root_id.clone(), generation, subtree, md, doc.clone(), now);
    if let Some(old) = &existing {
        cp.created_at = old.created_at;
        for (_, id) in old.ids.iter() {
            cp.ids.insert(id);
        }
    }
    cp.file = Some(recorded_path(out));
    store.save_checkpoint(&cp)?;
    store.prune_history(&root_id, &cfg.history, now)?;
    if journal.exists() {
        std::fs::remove_file(&journal).map_err(io_error(&journal))?;
    }
    report.outcome = SyncOutcome::Rendered { generation };
    Ok(report)
}

// ---------------------------------------------------------------------------------------
// The cycle
// ---------------------------------------------------------------------------------------

/// One cycle for one file (module docs).
pub fn sync_once(api: &dyn Api, file: &Path, cfg: &SyncConfig) -> Result<SyncReport, Error> {
    let mut results = sync_all(api, &[file.to_path_buf()], cfg)?;
    match results.pop() {
        Some(r) => r.result,
        None => unreachable!("sync_all returns one result per file"),
    }
}

/// A file being synced in this run.
struct Active {
    /// Index into the run's files.
    index: usize,
    file: PathBuf,
    /// The text the current pass is based on (re-read on a CAS rerun).
    text: String,
    doc: LocalDoc,
    root: NodeId,
    _lock: RootLock,
    report: SyncReport,
    /// Adoptions for the merge, carried into the journal: from recovery and from the
    /// creates of earlier passes of this cycle (CAS reruns).
    carried: Vec<Adoption>,
    /// This cycle's journal, once begun.
    journal: Option<Journal>,
    /// A journal existed at the start (recovered); it must be cleared at commit.
    had_journal: bool,
    /// Plan summaries summed over the passes.
    pushed: PlanSummary,
    /// The base of a CAS rerun (§9): the pass's base with the pushes it applied.
    rebase: Option<Base>,
    /// §15.1: content -> full placement of this root's latest view (from the
    /// checkpoint's base at first), the sticky input of the next `mirrors::view_tree`.
    full: BTreeMap<NodeId, NodeId>,
}

/// The base of a merge: `base_md` and `base_wf` (§5.1) and where they came from.
#[derive(Debug, Clone)]
struct Base {
    kind: BaseKind,
    md: String,
    wf: Tree,
}

/// Which base the merge used (§9.1 guard 1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BaseKind {
    /// The file's gen is the checkpoint's.
    Checkpoint,
    /// The file names an older (or crashed-ahead) generation kept in the history.
    History(u64),
    /// gen missing or unknown: checkpoint base, safe mode.
    Safe,
    /// No checkpoint (`--force`): base = the remote, safe mode.
    Bootstrap,
}

/// A file whose merge produced a plan.
struct Planned {
    base: Base,
    checkpoint: Option<Checkpoint>,
    /// The merge on the view tree (conflicts, report, deletes for the file).
    merge: MergeResult,
    /// The same on the real tree (`mirrors::to_real`): what `apply` and the
    /// verification compare against.
    real: MergeResult,
    plan: Plan,
    ids: IdMap,
    /// Deletes deferred this cycle (hidden from the render).
    deferred: BTreeSet<NodeId>,
    /// Pending deletes to save with the checkpoint (sent ones removed after apply).
    pending: PendingDeletes,
}

enum Prepared {
    /// The cycle ends here for this file.
    Done(SyncOutcome),
    /// Merge and plan done; apply and render follow.
    Ready(Box<Planned>),
}

enum Finished {
    Done,
    /// The file changed while we synced (§9): rerun.
    CasConflict,
}

/// Steps 4–9 of the cycle for one file.
#[allow(clippy::too_many_arguments)]
fn prepare(
    store: &StateStore,
    cfg: &SyncConfig,
    a: &mut Active,
    remote: &Tree,
    real: &Tree,
    global_ids: &IdMap,
    cross: &CrossFile,
    roots: &BTreeSet<NodeId>,
) -> Result<Prepared, Error> {
    let now = cfg.clock.now();
    let root = a.root.clone();
    let force = cfg.force || a.doc.frontmatter.force;
    let checkpoint = store.load_checkpoint(&root)?;

    // Step 4: the base (§9.1 guard 1); a CAS rerun keeps its pass's base.
    let (kind, base_md, base_wf) = match (&a.rebase, &checkpoint) {
        (Some(b), _) => (b.kind, b.md.clone(), b.wf.clone()),
        (None, None) if !force => return Err(Error::NoCheckpoint(root)),
        (None, None) => {
            // UNVERIFIED: bootstrapping without a checkpoint takes the remote as the base
            // (module docs); safe mode limits the damage.
            let mut fm = a.doc.frontmatter.clone();
            fm.force = false;
            fm.generation = None;
            let view = View::build(remote, &root, &fm.config, &ViewExtras::default())?;
            let tree = remote
                .subtree(&root)
                .ok_or_else(|| Error::RootNotFound(root.to_string()))?;
            (BaseKind::Bootstrap, mdblock::render(&view, &fm), tree)
        }
        (None, Some(cp)) => {
            let (kind, md, wf, warning) =
                base_for_gen(store, &root, a.doc.frontmatter.generation, cp);
            a.report.warnings.extend(warning);
            (kind, md, wf)
        }
    };
    match kind {
        _ if a.rebase.is_some() => {}
        BaseKind::Safe => a.report.warnings.push(format!(
            "the file's gen ({}) is unknown: safe mode (no deletes, moves or reorders)",
            a.doc
                .frontmatter
                .generation
                .map_or_else(|| "missing".to_owned(), |g| g.to_string())
        )),
        BaseKind::Bootstrap => a
            .report
            .warnings
            .push("no checkpoint: bootstrapped from Workflowy in safe mode (--force)".to_owned()),
        BaseKind::History(g) => {
            log::info!("{}: rebasing onto generation {g}", a.file.display());
        }
        BaseKind::Checkpoint => {}
    }
    let base_doc = mdblock::parse(&base_md)?;
    if base_doc.frontmatter.root.short != a.doc.frontmatter.root.short {
        return Err(Error::RootMismatch {
            file: a.file.clone(),
            expected: root,
            found: a.doc.frontmatter.root.raw.clone(),
        });
    }

    // §9.1 guard 6: Tasks-plugin fields added to ticked lines are not the user's text.
    let stripped = if cfg.strip_tasks_fields {
        crate::daemon::strip_tasks_fields(&a.doc, &base_doc)
    } else {
        None
    };
    let local_doc = match &stripped {
        Some((doc, notes)) => {
            for n in notes {
                log::info!("{}: {n}", a.file.display());
            }
            a.report.messages.extend(notes.iter().cloned());
            doc
        }
        None => &a.doc,
    };

    let mut pending = checkpoint
        .as_ref()
        .map(|c| c.pending_deletes.clone())
        .unwrap_or_default();

    // Step 5: the fast path (§5.1).
    if kind == BaseKind::Checkpoint
        && !force
        && !a.had_journal
        && a.carried.is_empty()
        && pending.is_empty()
        && a.text == base_md
        && outline_diff(&base_wf, remote, &root, DiffScope::All).is_empty()
    {
        return Ok(Prepared::Done(SyncOutcome::Unchanged));
    }

    // The id map: the whole export, the base, and every id the checkpoint remembers.
    let mut ids = global_ids.clone();
    ids.extend_from_tree(&base_wf);
    if let Some(cp) = &checkpoint {
        for (_, id) in cp.ids.iter() {
            ids.insert(id);
        }
    }

    // Guard 3 material: older renders.
    let base_gen = match kind {
        BaseKind::History(g) => Some(g),
        _ => checkpoint.as_ref().map(|c| c.generation),
    };
    let prior_docs = load_prior_docs(store, &root, base_gen)?;

    // Step 6: merge.
    let mcfg = MergeConfig {
        doc: base_doc.frontmatter.config.clone(),
        force,
        safe_mode: matches!(kind, BaseKind::Safe | BaseKind::Bootstrap),
        todo_from_checkbox: cfg.todo_from_checkbox,
    };
    let pending_set: BTreeSet<NodeId> = pending.entries.keys().cloned().collect();
    let merged = merge::merge(&MergeInput {
        root: &root,
        base: &base_wf,
        remote,
        base_doc: &base_doc,
        local_doc,
        ids: &ids,
        cfg: &mcfg,
        cross,
        pending_deletes: &pending_set,
        adoptions: &a.carried,
        prior_docs: &prior_docs,
    })?;
    a.report.events.extend(merged.report.iter().cloned());

    // Step 7: deferred deletes (§9.1 guard 2).
    for n in &merged.cancelled_pending {
        pending.remove(n);
    }
    let mut deferred = BTreeSet::new();
    match cfg.delete_grace {
        Some(grace) => {
            let deletes: Vec<(NodeId, usize, String)> = merged
                .deletes
                .iter()
                .map(|d| (d.node.clone(), d.hidden_descendants, d.label.clone()))
                .collect();
            let listed: BTreeSet<NodeId> = deletes.iter().map(|d| d.0.clone()).collect();
            pending.retain_only(&listed);
            pending.observe(&deletes, &fingerprint(&a.text), now);
            let grace = i64::try_from(grace.as_secs()).unwrap_or(i64::MAX);
            let due: BTreeSet<NodeId> = pending.due(now, grace).into_iter().collect();
            deferred = listed.difference(&due).cloned().collect();
        }
        None => pending = PendingDeletes::default(),
    }

    // Step 8: plan, with the brakes (§8.3).
    let pcfg = PlanConfig {
        sync_roots: roots.clone(),
        brakes: cfg.brakes.clone(),
        force,
        fast_path_creates: cfg.fast_path_creates,
        deferred_deletes: deferred.clone(),
    };
    // §15.1: the planner and the applier work on real nodes.
    let real_merge = mirrors::to_real(&merged, remote, real);
    let plan = match plan::plan(&real_merge, real, &pcfg) {
        Ok(p) => p,
        Err(PlanError::Brake(b)) => {
            a.report.warnings.push(format!("brake: {b}"));
            return Ok(Prepared::Done(SyncOutcome::Braked(b)));
        }
        Err(e) => return Err(e.into()),
    };

    // Step 9: dry run.
    if cfg.dry_run {
        a.report.conflicts = merged.conflicts.clone();
        a.report
            .messages
            .extend(describe_events(&merged.report, remote));
        return Ok(Prepared::Done(SyncOutcome::DryRun { plan }));
    }
    Ok(Prepared::Ready(Box::new(Planned {
        base: Base {
            kind,
            md: base_md,
            wf: base_wf,
        },
        checkpoint,
        merge: merged,
        real: real_merge,
        plan,
        ids,
        deferred,
        pending,
    })))
}

/// §9.1 guard 1: the base of a file's merge (outside a CAS rerun) given its `gen`: the
/// checkpoint if `gen` is the checkpoint's, that history entry if `gen` names one,
/// otherwise the checkpoint in safe mode. Also returns a warning for the report.
fn base_for_gen(
    store: &StateStore,
    root: &NodeId,
    generation: Option<u64>,
    cp: &Checkpoint,
) -> (BaseKind, String, Tree, Option<String>) {
    let safe = || (BaseKind::Safe, cp.base_md.clone(), cp.base_wf.clone());
    match generation {
        Some(g) if g == cp.generation => (
            BaseKind::Checkpoint,
            cp.base_md.clone(),
            cp.base_wf.clone(),
            None,
        ),
        Some(g) => match store.get_history(root, g) {
            Ok(Some(h)) => (BaseKind::History(g), h.md, h.tree, None),
            Ok(None) => {
                let (k, md, wf) = safe();
                (k, md, wf, None)
            }
            Err(e) => {
                let (k, md, wf) = safe();
                let w = format!("history entry {g} unreadable ({e}); safe mode");
                (k, md, wf, Some(w))
            }
        },
        None => {
            let (k, md, wf) = safe();
            (k, md, wf, None)
        }
    }
}

/// Guard 3 material: the [`PRIOR_DOCS_MAX`] newest kept renders other than the base's.
fn load_prior_docs(
    store: &StateStore,
    root: &NodeId,
    base_gen: Option<u64>,
) -> Result<Vec<LocalDoc>, Error> {
    let mut prior_docs = Vec::new();
    let gens: Vec<u64> = store
        .history_gens(root)?
        .into_iter()
        .filter(|g| Some(*g) != base_gen)
        .collect();
    for g in gens.iter().rev().take(PRIOR_DOCS_MAX) {
        match store.get_history(root, *g) {
            Ok(Some(h)) => match mdblock::parse(&h.md) {
                Ok(d) => prior_docs.push(d),
                Err(e) => log::warn!("history {g} of {root} does not parse: {e}"),
            },
            Ok(None) => {}
            Err(e) => log::warn!("history {g} of {root} unreadable: {e}"),
        }
    }
    Ok(prior_docs)
}

/// §6.4: this file's base for the nodes of its root that other files of the run paste
/// (`wanted`): the base its own merge would choose (guard 1) and its older renders
/// (guard 3). Problems only cost the entries (those fields are then not pushed).
fn source_bases(
    store: &StateStore,
    a: &Active,
    ids: &IdMap,
    wanted: &BTreeSet<NodeId>,
) -> BTreeMap<NodeId, CrossBase> {
    if wanted.is_empty() {
        return BTreeMap::new();
    }
    let cp = match store.load_checkpoint(&a.root) {
        Ok(Some(cp)) => cp,
        Ok(None) => return BTreeMap::new(),
        Err(e) => {
            log::warn!("{}: checkpoint unreadable ({e})", a.file.display());
            return BTreeMap::new();
        }
    };
    let (kind, md, wf, _) = base_for_gen(store, &a.root, a.doc.frontmatter.generation, &cp);
    let base_doc = match mdblock::parse(&md) {
        Ok(d) => d,
        Err(e) => {
            log::warn!("{}: base does not parse ({e})", a.file.display());
            return BTreeMap::new();
        }
    };
    let base_gen = match kind {
        BaseKind::History(g) => g,
        _ => cp.generation,
    };
    let priors = load_prior_docs(store, &a.root, Some(base_gen)).unwrap_or_else(|e| {
        log::warn!("{}: history unreadable ({e})", a.file.display());
        Vec::new()
    });
    merge::cross_bases(&wf, &base_doc, &priors, ids, wanted)
}

fn describe_events(events: &[ReportEvent], tree: &Tree) -> Vec<String> {
    events.iter().map(|e| e.describe(tree)).collect()
}

/// Step 10: apply the plan with a journal. `Ok(None)` for an empty plan. `remote` is the
/// real tree the plan was made against.
fn apply_planned(
    api: &dyn Api,
    store: &StateStore,
    cfg: &SyncConfig,
    a: &mut Active,
    p: &mut Planned,
    remote: &Tree,
    roots: &BTreeSet<NodeId>,
) -> Result<Option<ApplyOutcome>, Error> {
    if p.plan.is_empty() {
        return Ok(None);
    }
    let path = store.journal_path(&a.root);
    let begin = JournalEntry::new_begin(&p.plan, a.carried.clone());
    let journal = Journal::begin(&path, &begin).map_err(ApplyError::Journal)?;
    let journal = a.journal.insert(journal);
    let opts = ApplyOptions {
        verify: false,
        ..cfg.apply.clone()
    };
    let outcome = apply::Applier::new(api, opts, roots.clone()).apply(
        &p.plan,
        remote,
        &p.real.merged,
        journal,
    );
    let outcome = match outcome {
        Ok(o) => o,
        Err(e) => {
            // The journal stays on disk for the next run's recovery.
            a.journal = None;
            return Err(e.into());
        }
    };
    a.report.messages.extend(outcome.log_lines(&p.plan));
    add_summary(&mut a.pushed, &p.plan.summary);
    // Sent deletes are no longer pending (a skipped one comes back, or is cancelled, at
    // the next merge).
    for op in &p.plan.ops {
        if let Op::Delete { node, .. } = op {
            p.pending.remove(node);
        }
    }
    a.carried.extend(outcome.adoptions(&p.plan));
    Ok(Some(outcome))
}

/// Steps 11–13 for one file: render from `remote` (the export after the applies),
/// guard 4, compare-and-swap, commit.
fn finish(
    store: &StateStore,
    cfg: &SyncConfig,
    a: &mut Active,
    p: &Planned,
    remote: &Tree,
) -> Result<Finished, Error> {
    let now = cfg.clock.now();
    let root = a.root.clone();
    let mut fm = a.doc.frontmatter.clone();
    fm.force = false;
    let extras = ViewExtras {
        conflicts: p.merge.conflicts.clone(),
        hidden: p.deferred.clone(),
        sticky: sticky_from(&a.doc, &p.ids),
    };
    let view = View::build(remote, &root, &fm.config, &extras)?;
    let subtree = remote
        .subtree(&root)
        .ok_or_else(|| Error::RootNotFound(root.to_string()))?;
    let current_gen = p.checkpoint.as_ref().map(|c| c.generation);

    // Guard 4: write only when the user needs to see something.
    let keep_file = p.base.kind == BaseKind::Checkpoint && !a.doc.frontmatter.force && {
        let mut same = fm.clone();
        same.generation = current_gen;
        let text = mdblock::render(&view, &same);
        text == a.text
            || mdblock::parse(&text)
                .is_ok_and(|d| mdblock::equivalent(&d, &a.doc, &fm.config.inline))
    };

    let (md, generation, wrote) = match (keep_file, current_gen) {
        (true, Some(g)) => (a.text.clone(), g, false),
        _ => {
            let generation = next_generation(store, &root, current_gen)?;
            fm.generation = Some(generation);
            let md = mdblock::render(&view, &fm);
            // The history entry first (state -> sync commit order), so that nothing slow
            // sits between the compare-and-swap's re-read and its rename.
            store.put_history(
                &root,
                &HistoryEntry {
                    generation,
                    md: md.clone(),
                    tree: subtree.clone(),
                    at: now,
                },
            )?;
            cas_hook("history");
            // §9 compare-and-swap: re-read just before writing.
            if !write_if_unchanged(&a.file, &a.text, md.as_bytes()).map_err(io_error(&a.file))? {
                log::info!(
                    "{}: changed during the sync; merging again",
                    a.file.display()
                );
                // The generation was never written to the file; forget its entry so
                // that guard 3 does not count a render nobody saw (next_generation
                // would skip it anyway).
                let path = store.history_path(&root, generation);
                if let Err(e) = std::fs::remove_file(&path)
                    && e.kind() != std::io::ErrorKind::NotFound
                {
                    log::warn!("cannot remove the unused {}: {e}", path.display());
                }
                return Ok(Finished::CasConflict);
            }
            (md, generation, true)
        }
    };
    if !wrote {
        // Keep history[gen] equal to the checkpoint it names.
        store.put_history(
            &root,
            &HistoryEntry {
                generation,
                md: md.clone(),
                tree: subtree.clone(),
                at: now,
            },
        )?;
    }

    // Step 13: commit.
    let mut cp = match &p.checkpoint {
        Some(c) => c.clone(),
        None => Checkpoint::new(
            root.clone(),
            generation,
            subtree.clone(),
            md.clone(),
            fm.config.clone(),
            now,
        ),
    };
    cp.generation = generation;
    cp.ids.extend_from_tree(&subtree);
    cp.base_wf = subtree;
    cp.set_base_md(md);
    cp.config = fm.config.clone();
    cp.file = Some(recorded_path(&a.file));
    cp.synced_at = now;
    cp.pending_deletes = p.pending.clone();
    // Guard 2 counts cycles "in which that file is unchanged": unchanged from the text
    // that is in the file now (written or kept), not from the text the deletes were seen
    // in, which our own write replaces.
    let written = fingerprint(&cp.base_md);
    for e in cp.pending_deletes.entries.values_mut() {
        e.fingerprint = written.clone();
    }
    store.save_checkpoint(&cp)?;
    store.prune_history(&root, &cfg.history, now)?;
    commit_journal(store, a)?;

    if view.root_completed {
        a.report
            .warnings
            .push("the sync root is completed in Workflowy".to_owned());
    }
    let shown: BTreeSet<&ConflictMarker> = view.dropped_conflicts.iter().collect();
    a.report.conflicts = p
        .merge
        .conflicts
        .iter()
        .filter(|c| !shown.contains(c))
        .cloned()
        .collect();
    a.report
        .messages
        .extend(describe_events(&p.merge.report, remote));
    a.report.outcome = SyncOutcome::Synced {
        pushed: a.pushed,
        wrote_file: wrote,
        generation,
    };
    Ok(Finished::Done)
}

/// The cycle is durable: delete the journal (begun this cycle or recovered).
fn commit_journal(store: &StateStore, a: &mut Active) -> Result<(), Error> {
    if let Some(j) = a.journal.take() {
        j.commit().map_err(ApplyError::Journal)?;
    } else if a.had_journal {
        let path = store.journal_path(&a.root);
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(io_error(&path)(e)),
        }
    }
    a.had_journal = false;
    Ok(())
}

/// The base for a CAS rerun (§9 "the checkpoint is updated"): the pass's base with the
/// field updates and re-parentings it applied, so that the rerun sees them as neither a
/// local nor a remote change. Without this, a user's further edit of a line we just
/// pushed would conflict with our own push. Creates are matched through adoptions,
/// deletes and ticks show up on both sides as before; reorders are left to the order
/// merge.
fn rebase_after(p: &Planned, applied: Option<&ApplyOutcome>) -> Base {
    let mut wf = p.base.wf.clone();
    if let Some(outcome) = applied {
        for (op, result) in p.plan.ops.iter().zip(&outcome.results) {
            if *result != apply::OpOutcome::Done {
                continue;
            }
            // The ops name real nodes; the base is a view (§15.1): a field update
            // reaches every placement of the content, a move's destination is the
            // placement that renders the parent's children.
            let r = match op {
                Op::UpdateName { node, name, .. } => mirrors::placements_of(&wf, node)
                    .iter()
                    .try_for_each(|pl| wf.set_name(pl, name.clone())),
                Op::UpdateNote { node, note, .. } => mirrors::placements_of(&wf, node)
                    .iter()
                    .try_for_each(|pl| wf.set_note(pl, Some(note.clone()))),
                Op::UpdateLayout { node, layout, .. } => mirrors::placements_of(&wf, node)
                    .iter()
                    .try_for_each(|pl| wf.set_layout(pl, layout.clone())),
                Op::Move {
                    node,
                    parent,
                    position,
                    ..
                } if wf.contains(node) => match mirrors::full_placement_of(&wf, parent) {
                    Some(dest) if wf.parent(node) != Some(&dest) => {
                        wf.move_node(node, Some(&dest), *position)
                    }
                    _ => Ok(()),
                },
                _ => Ok(()),
            };
            if let Err(e) = r {
                log::warn!("rebase after a CAS conflict: {op:?}: {e}");
            }
        }
    }
    Base {
        kind: p.base.kind,
        md: p.base.md.clone(),
        wf,
    }
}

/// The ops of `plan` that the applier skipped because a collaborator changed or deleted
/// their nodes between our export and the write (§7.1 pre-write check, §9), and whose
/// local content a render of the new export would therefore drop from the file without
/// a trace: a name, note or layout edit (`ChangedRemotely`: §6.1 makes that a conflict
/// whose local side stays in the Markdown; `Gone`: §6.2 resurrects the edited node), a
/// create (`Gone`: its parent was deleted, §6.2 resurrects the parent with the new
/// item; `MissingDependency`: a child of such a create) and a re-parenting move whose
/// node or destination vanished (§6.2 "move vs. delete: the move wins").
///
/// Not listed: completions and deletes (nothing local is lost when their node changed
/// or vanished: a tick of a deleted node is a no-op, a skipped delete loses to the
/// edit, §6.2), moves skipped because the node moved remotely (§6.2 "move vs. move:
/// remote wins"), and §8.2 guard refusals, which are decided on the plan's own
/// snapshot and would only repeat on a rerun.
fn unpushed_local_changes(plan: &Plan, outcome: &ApplyOutcome) -> Vec<usize> {
    use apply::{OpOutcome, SkipReason};
    plan.ops
        .iter()
        .zip(&outcome.results)
        .enumerate()
        .filter(|(_, (op, result))| {
            let OpOutcome::Skipped(reason) = result else {
                return false;
            };
            match (op, reason) {
                (_, SkipReason::AlreadyCompleted | SkipReason::Guard(_)) => false,
                (Op::Complete { .. } | Op::Delete { .. }, _) => false,
                (Op::Move { .. }, SkipReason::ChangedRemotely) => false,
                (
                    Op::Create { .. }
                    | Op::UpdateName { .. }
                    | Op::UpdateNote { .. }
                    | Op::UpdateLayout { .. }
                    | Op::Move { .. },
                    SkipReason::ChangedRemotely
                    | SkipReason::Gone
                    | SkipReason::MissingDependency(_),
                ) => true,
            }
        })
        .map(|(i, _)| i)
        .collect()
}

/// After an apply: if local changes could not be pushed ([`unpushed_local_changes`]),
/// note it in the report and return the reason to merge again instead of rendering.
fn needs_remerge(a: &mut Active, p: &Planned, applied: Option<&ApplyOutcome>) -> Option<String> {
    let outcome = applied?;
    let skipped = unpushed_local_changes(&p.plan, outcome);
    if skipped.is_empty() {
        return None;
    }
    let reason = format!(
        "{} local change(s) could not be pushed because their nodes changed in \
         Workflowy during the sync (ops {skipped:?})",
        skipped.len()
    );
    log::info!("{}: {reason}; merging again", a.file.display());
    a.report
        .messages
        .push(format!("{reason}; merged again so that they are kept"));
    Some(reason)
}

/// The §9 compare-and-swap write of a new generation: stage `bytes` in a temp file next
/// to `path` (written and fsynced), then re-read `path` and rename the temp file over it
/// only if it still holds exactly `expected`; fsync the directory. Returns `false`, and
/// removes the temp file, if the file changed (or vanished). Everything slow happens
/// before the re-read, so the window in which another writer's change (Obsidian Sync,
/// §9.1) could still be replaced unseen is the re-read and one rename: a rename cannot
/// be made conditional.
///
/// Like `state::atomic_write`, a symbolic link at `path` is kept: the file it names is
/// re-read and replaced (`state::write_target`, temp file in that file's directory), and
/// the replaced file keeps its permission bits (`state::keep_permissions`).
fn write_if_unchanged(path: &Path, expected: &str, bytes: &[u8]) -> std::io::Result<bool> {
    use std::io::Write;
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let path = &state::write_target(path)?;
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| std::io::Error::other("path has no file name"))?;
    let dir = match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => PathBuf::from("."),
    };
    // A dot file: the daemon's discovery ignores it; the same `.tmp-` shape as
    // `state::atomic_write`.
    let tmp = dir.join(format!(
        ".{name}.tmp-{}-cas{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let staged = (|| {
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)?;
        f.write_all(bytes)?;
        state::keep_permissions(path, &f)?;
        f.sync_all()
    })();
    let result = staged.and_then(|()| {
        cas_hook("prepared");
        if read_text(path)?.as_deref() != Some(expected) {
            return Ok(false);
        }
        std::fs::rename(&tmp, path)?;
        Ok(true)
    });
    match result {
        Ok(true) => {
            std::fs::File::open(&dir)?.sync_all()?;
            Ok(true)
        }
        other => {
            let _ = std::fs::remove_file(&tmp);
            other
        }
    }
}

#[cfg(test)]
type CasHook = Box<dyn FnMut(&'static str)>;

#[cfg(test)]
thread_local! {
    /// Unit tests: run at named points of the compare-and-swap write, to race it.
    static CAS_HOOK: std::cell::RefCell<Option<CasHook>> =
        const { std::cell::RefCell::new(None) };
}

/// A test seam inside the compare-and-swap write (no-op outside unit tests).
fn cas_hook(stage: &'static str) {
    #[cfg(test)]
    CAS_HOOK.with(|h| {
        if let Some(f) = h.borrow_mut().as_mut() {
            f(stage)
        }
    });
    #[cfg(not(test))]
    let _ = stage;
}

/// Re-read the file for a CAS rerun. `Err(outcome)` ends the cycle.
fn reread(a: &mut Active) -> Result<Result<(), SyncOutcome>, Error> {
    let text = match read_text(&a.file) {
        Ok(Some(t)) => t,
        Ok(None) => return Ok(Err(SyncOutcome::Paused(PauseReason::FileMissing))),
        Err(e) => {
            return Ok(Err(SyncOutcome::Skipped {
                reason: format!("cannot read the file: {e}"),
            }));
        }
    };
    let doc = match mdblock::parse(&text) {
        Ok(d) => d,
        Err(e) => {
            return Ok(Err(SyncOutcome::Skipped {
                reason: format!("the file does not parse: {e}"),
            }));
        }
    };
    if doc.frontmatter.root.short != a.doc.frontmatter.root.short {
        return Err(Error::RootMismatch {
            file: a.file.clone(),
            expected: a.root.clone(),
            found: doc.frontmatter.root.raw.clone(),
        });
    }
    if doc.frontmatter.paused {
        return Ok(Err(SyncOutcome::Paused(PauseReason::Frontmatter)));
    }
    a.text = text;
    a.doc = doc;
    a.report.warnings = parse_warnings(&a.doc);
    Ok(Ok(()))
}

/// The §9 CAS loop after a conflict (or after local changes that could not be pushed,
/// [`needs_remerge`]): merge the current text against the latest remote, apply,
/// re-export, render and try the write again. `reason` says why the first pass did not
/// finish.
#[allow(clippy::too_many_arguments)]
fn rerun(
    api: &dyn Api,
    store: &StateStore,
    cfg: &SyncConfig,
    a: &mut Active,
    mut remote: Tree,
    global_ids: &IdMap,
    cross: &CrossFile,
    roots: &BTreeSet<NodeId>,
    mut reason: String,
) -> Result<(), Error> {
    for attempt in 1..=cfg.cas_retries {
        log::info!("{}: CAS rerun {attempt}", a.file.display());
        if let Err(outcome) = reread(a)? {
            a.report.outcome = outcome;
            return Ok(());
        }
        let mut ids = global_ids.clone();
        ids.extend_from_tree(&remote);
        // §15.1: the rerun's view keeps the pass's full placements.
        let view = mirrors::view_tree(&remote, &a.root, &a.full);
        let real = mirrors::real_tree(&remote);
        let mut planned = match prepare(store, cfg, a, &view, &real, &ids, cross, roots)? {
            Prepared::Done(outcome) => {
                a.report.outcome = outcome;
                return Ok(());
            }
            Prepared::Ready(p) => p,
        };
        let applied = apply_planned(api, store, cfg, a, &mut planned, &real, roots)?;
        let after_view = if applied.is_some() {
            remote = export_tree(api)?;
            if cfg.apply.verify
                && let Some(outcome) = &applied
            {
                a.report.discrepancies.extend(apply::verify(
                    &planned.real.merged,
                    &mirrors::real_tree(&remote),
                    &a.root,
                    &outcome.created,
                ));
            }
            mirrors::view_tree(&remote, &a.root, &mirrors::full_placements(&view))
        } else {
            view
        };
        a.full = mirrors::full_placements(&after_view);
        match needs_remerge(a, &planned, applied.as_ref()) {
            Some(r) => reason = r,
            None => {
                if let Finished::Done = finish(store, cfg, a, &planned, &after_view)? {
                    return Ok(());
                }
                reason = "the file kept changing during the sync".to_owned();
            }
        }
        a.rebase = Some(rebase_after(&planned, applied.as_ref()));
    }
    // Nothing is committed: the file keeps the local text, the checkpoint its base, and
    // the journal (if any) is recovered by the next run, whose merge sees the remote
    // changes as such.
    a.report.outcome = SyncOutcome::Skipped {
        reason: format!(
            "{reason} ({} retries); the next run continues",
            cfg.cas_retries
        ),
    };
    Ok(())
}

/// One cycle for several files sharing one export (§6.4). `Err` only for run-wide
/// failures (the state directory, the export); per-file failures are in the
/// `FileResult`s, in input order.
pub fn sync_all(
    api: &dyn Api,
    files: &[PathBuf],
    cfg: &SyncConfig,
) -> Result<Vec<FileResult>, Error> {
    let store = StateStore::open(&cfg.state_dir)?;
    let mut results: Vec<Option<Result<SyncReport, Error>>> = files.iter().map(|_| None).collect();

    // Step 1: read and parse every file (no API call yet).
    let mut parsed: Vec<(usize, String, LocalDoc)> = Vec::new();
    for (i, file) in files.iter().enumerate() {
        let done = |outcome| Some(Ok(SyncReport::new(file, None, outcome)));
        match read_text(file) {
            Ok(None) => {
                log::warn!(
                    "{}: missing; root paused (nothing is deleted)",
                    file.display()
                );
                results[i] = done(SyncOutcome::Paused(PauseReason::FileMissing));
            }
            Err(e) => {
                results[i] = done(SyncOutcome::Skipped {
                    reason: format!("cannot read the file: {e}"),
                });
            }
            Ok(Some(text)) => match mdblock::parse(&text) {
                Err(e) => {
                    results[i] = done(SyncOutcome::Skipped {
                        reason: format!("the file does not parse: {e}"),
                    });
                }
                Ok(doc) if doc.frontmatter.paused => {
                    results[i] = done(SyncOutcome::Paused(PauseReason::Frontmatter));
                }
                Ok(doc) => parsed.push((i, text, doc)),
            },
        }
    }

    let mut active: Vec<Active> = Vec::new();
    if !parsed.is_empty() {
        // Step 2: one export, roots resolved with it.
        let mut remote = export_tree(api)?;
        let mut ids = IdMap::from_tree(&remote);
        let mut resolved: Vec<(usize, String, LocalDoc, NodeId)> = Vec::new();
        for (i, text, doc) in parsed {
            match doc.frontmatter.root.resolve(&ids) {
                Some(r) if remote.contains(&r) => resolved.push((i, text, doc, r)),
                _ => {
                    results[i] = Some(Err(Error::RootNotFound(doc.frontmatter.root.raw.clone())));
                }
            }
        }
        // Guard 5: one file per root.
        let mut by_root: BTreeMap<NodeId, Vec<usize>> = BTreeMap::new();
        for (i, _, _, r) in &resolved {
            by_root.entry(r.clone()).or_default().push(*i);
        }
        for (r, idx) in &by_root {
            if idx.len() > 1 {
                let paths: Vec<PathBuf> = idx.iter().map(|i| files[*i].clone()).collect();
                log::warn!("root {r} is claimed by several files: {paths:?}; not synced");
                for i in idx {
                    results[*i] = Some(Ok(SyncReport::new(
                        &files[*i],
                        Some(r.clone()),
                        SyncOutcome::Paused(PauseReason::DuplicateRoot(paths.clone())),
                    )));
                }
            }
        }
        resolved.retain(|(i, ..)| results[*i].is_none());
        // §6.4: sync roots must not nest.
        let all_roots: Vec<NodeId> = resolved.iter().map(|(.., r)| r.clone()).collect();
        for (i, _, _, r) in &resolved {
            if let Some(outer) = all_roots.iter().find(|o| remote.is_descendant(r, o)) {
                results[*i] = Some(Err(Error::NestedRoots {
                    outer: outer.clone(),
                    inner: r.clone(),
                }));
            }
        }
        resolved.retain(|(i, ..)| results[*i].is_none());

        // Locks and recovery (step 3).
        let mut recovered_any = false;
        for (i, text, doc, root) in resolved {
            let file = &files[i];
            let lock = match store.lock(&root) {
                Ok(l) => l,
                Err(StateError::Locked(_)) => {
                    results[i] = Some(Ok(SyncReport::new(
                        file,
                        Some(root.clone()),
                        SyncOutcome::Skipped {
                            reason: "another sync holds the root's lock".to_owned(),
                        },
                    )));
                    continue;
                }
                Err(e) => {
                    results[i] = Some(Err(e.into()));
                    continue;
                }
            };
            let mut report = SyncReport::new(file, Some(root.clone()), SyncOutcome::Unchanged);
            report.warnings = parse_warnings(&doc);
            let journal = store.journal_path(&root);
            let had_journal = journal.exists();
            let mut carried = Vec::new();
            if had_journal && cfg.dry_run {
                // A dry run writes nothing anywhere (`SyncConfig::dry_run`), and recovery
                // writes: it finishes two-step creates, deletes duplicate nonce nodes
                // (§7.3) and appends to the journal. So the journal is left for the next
                // real run, and the plan below ignores the interrupted cycle (it may
                // propose creating again what that cycle created).
                report.warnings.push(
                    "an interrupted cycle left a journal; a real sync recovers it first, \
                     this dry run did not, so its plan may differ"
                        .to_owned(),
                );
            } else if had_journal {
                match apply::recover(&journal, api) {
                    Ok(rec) => {
                        if let Some(rec) = &rec {
                            if rec.root.as_ref().is_some_and(|r| r != &root) {
                                report.warnings.push(format!(
                                    "the journal of {root} names root {:?}",
                                    rec.root
                                ));
                            }
                            carried = rec.adoptions.clone();
                            report.messages.extend(rec.notes.iter().cloned());
                        }
                        report.recovered = rec;
                        recovered_any = true;
                    }
                    Err(e) => {
                        results[i] = Some(Err(e.into()));
                        continue;
                    }
                }
            }
            active.push(Active {
                index: i,
                file: file.clone(),
                text,
                doc,
                root,
                _lock: lock,
                report,
                carried,
                journal: None,
                had_journal,
                pushed: PlanSummary::default(),
                rebase: None,
                full: BTreeMap::new(),
            });
        }
        if recovered_any {
            remote = export_tree(api)?;
            ids = IdMap::from_tree(&remote);
        }

        // §15.1: each root's view of the export (sticky to its checkpoint's base) and
        // the real tree the plans run on.
        let real = mirrors::real_tree(&remote);
        let mut views: Vec<Tree> = Vec::with_capacity(active.len());
        for a in active.iter_mut() {
            let previous = match store.load_checkpoint(&a.root) {
                Ok(Some(cp)) => mirrors::full_placements(&cp.base_wf),
                _ => BTreeMap::new(),
            };
            let view = mirrors::view_tree(&remote, &a.root, &previous);
            a.full = mirrors::full_placements(&view);
            views.push(view);
        }

        // Steps 4–9: every merge and plan against the one export.
        let roots: BTreeSet<NodeId> = active.iter().map(|a| a.root.clone()).collect();
        // A tag in another file claims its node only when that node is not in the other
        // file's own scope: a node mirrored into both files belongs to both.
        let tagged: Vec<BTreeSet<NodeId>> = active
            .iter()
            .zip(&views)
            .map(|(a, view)| {
                let scope: BTreeSet<NodeId> = view.descendants(&a.root).into_iter().collect();
                tagged_nodes(&a.doc, &ids)
                    .into_iter()
                    .filter(|n| !scope.contains(n))
                    .collect()
            })
            .collect();
        let claimed_elsewhere = |k: usize| -> BTreeSet<NodeId> {
            tagged
                .iter()
                .enumerate()
                .filter(|(j, _)| *j != k)
                .flat_map(|(_, s)| s.iter().cloned())
                .collect()
        };
        // §6.4: each file's base for its nodes that another file pastes (a cut and
        // paste between files), so the move merges the pasted line against what the
        // user cut, not against the current remote value.
        let sources: Vec<BTreeMap<NodeId, CrossBase>> = active
            .iter()
            .enumerate()
            .map(|(k, a)| {
                let wanted: BTreeSet<NodeId> = claimed_elsewhere(k)
                    .into_iter()
                    .filter(|n| remote.is_descendant(n, &a.root))
                    .collect();
                source_bases(&store, a, &ids, &wanted)
            })
            .collect();
        let crosses: Vec<CrossFile> = (0..active.len())
            .map(|k| CrossFile {
                other_roots: roots
                    .iter()
                    .filter(|r| **r != active[k].root)
                    .cloned()
                    .collect(),
                claimed_elsewhere: claimed_elsewhere(k),
                bases: sources
                    .iter()
                    .enumerate()
                    .filter(|(j, _)| *j != k)
                    .flat_map(|(_, b)| b.iter().map(|(n, c)| (n.clone(), c.clone())))
                    .collect(),
            })
            .collect();
        let mut planned: Vec<Option<Box<Planned>>> = Vec::new();
        for (k, a) in active.iter_mut().enumerate() {
            match prepare(&store, cfg, a, &views[k], &real, &ids, &crosses[k], &roots) {
                Ok(Prepared::Ready(p)) => planned.push(Some(p)),
                Ok(Prepared::Done(outcome)) => {
                    a.report.outcome = outcome;
                    planned.push(None);
                }
                Err(e) => {
                    results[a.index] = Some(Err(e));
                    planned.push(None);
                }
            }
        }

        // Step 10: every apply, in file order.
        let mut any_applied = false;
        let mut applied: Vec<Option<ApplyOutcome>> = Vec::new();
        for (a, p) in active.iter_mut().zip(planned.iter_mut()) {
            let Some(p) = p else {
                applied.push(None);
                continue;
            };
            match apply_planned(api, &store, cfg, a, p, &real, &roots) {
                Ok(o) => {
                    any_applied |= o.is_some();
                    applied.push(o);
                }
                Err(e) => {
                    results[a.index] = Some(Err(e));
                    applied.push(None);
                }
            }
        }

        // Step 11: one re-export, verification, then each file's render and write.
        let after = if any_applied {
            export_tree(api)?
        } else {
            remote.clone()
        };
        let after_real = mirrors::real_tree(&after);
        for (k, a) in active.iter_mut().enumerate() {
            if results[a.index].is_some() {
                continue;
            }
            let Some(p) = &planned[k] else { continue };
            if cfg.apply.verify
                && let Some(outcome) = &applied[k]
            {
                a.report.discrepancies =
                    apply::verify(&p.real.merged, &after_real, &a.root, &outcome.created);
                for d in &a.report.discrepancies {
                    log::warn!("{}: verification: {d:?}", a.file.display());
                }
            }
            // §15.1: this root's view of the new export, sticky to the pass's choice.
            let after_view = if any_applied {
                mirrors::view_tree(&after, &a.root, &a.full)
            } else {
                views[k].clone()
            };
            a.full = mirrors::full_placements(&after_view);
            // Local changes the applier had to skip are merged again against the new
            // export (§6.1 conflict, §6.2 resurrection) instead of being rendered away.
            let first = match needs_remerge(a, p, applied[k].as_ref()) {
                Some(reason) => Ok(Err(reason)),
                None => finish(&store, cfg, a, p, &after_view).map(|f| match f {
                    Finished::Done => Ok(()),
                    Finished::CasConflict => {
                        Err("the file kept changing during the sync".to_owned())
                    }
                }),
            };
            let r = match first {
                Ok(Ok(())) => Ok(()),
                Ok(Err(reason)) => {
                    a.rebase = Some(rebase_after(p, applied[k].as_ref()));
                    rerun(
                        api,
                        &store,
                        cfg,
                        a,
                        after.clone(),
                        &ids,
                        &crosses[k],
                        &roots,
                        reason,
                    )
                }
                Err(e) => Err(e),
            };
            if let Err(e) = r {
                results[a.index] = Some(Err(e));
            }
        }
    }

    for a in active {
        if results[a.index].is_none() {
            results[a.index] = Some(Ok(a.report));
        }
    }
    Ok(files
        .iter()
        .zip(results)
        .map(|(file, r)| FileResult {
            file: file.clone(),
            result: r.unwrap_or_else(|| {
                Ok(SyncReport::new(
                    file,
                    None,
                    SyncOutcome::Skipped {
                        reason: "not processed".to_owned(),
                    },
                ))
            }),
        })
        .collect())
}

// ---------------------------------------------------------------------------------------
// status
// ---------------------------------------------------------------------------------------

/// Why [`status_of_files`] cannot report on a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileProblem {
    /// The file does not exist.
    Missing,
    /// The file cannot be read, or its frontmatter does not parse.
    Unreadable(String),
    /// The file's root (as written in its frontmatter) has no checkpoint in this state
    /// directory.
    NoCheckpoint(String),
}

impl std::fmt::Display for FileProblem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FileProblem::Missing => f.write_str("missing"),
            FileProblem::Unreadable(e) => f.write_str(e),
            FileProblem::NoCheckpoint(root) => {
                write!(f, "no checkpoint for root {root} in this state directory")
            }
        }
    }
}

/// [`status_of_files`]'s result for one file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileStatus {
    /// The file as given.
    pub file: PathBuf,
    /// Its root's status, or why there is none.
    pub status: Result<RootStatus, FileProblem>,
}

/// Status of every root in the state directory. Reads local state only (no API call)
/// and creates nothing: a missing state directory has no roots.
pub fn status(cfg: &SyncConfig) -> Result<Vec<RootStatus>, Error> {
    let Some(store) = StateStore::open_existing(&cfg.state_dir)? else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for root in store.roots()? {
        out.extend(root_status(&store, &root, None)?);
    }
    Ok(out)
}

/// Status of the roots of `files`, one entry per file in order. A file that is missing,
/// unreadable, or whose root has no checkpoint here gets a [`FileProblem`] and the other
/// files go on. Like [`status`], reads local state only and creates nothing.
pub fn status_of_files(cfg: &SyncConfig, files: &[PathBuf]) -> Result<Vec<FileStatus>, Error> {
    let store = StateStore::open_existing(&cfg.state_dir)?;
    let known = match &store {
        Some(store) => store.roots()?,
        None => Vec::new(),
    };
    let mut ids = IdMap::new();
    for r in &known {
        ids.insert(r);
    }
    let mut out = Vec::new();
    for file in files {
        let status = match read_text(file) {
            Ok(None) => Err(FileProblem::Missing),
            Err(e) => Err(FileProblem::Unreadable(io_error(file)(e).to_string())),
            Ok(Some(text)) => match mdblock::parse_frontmatter(&text) {
                Err(e) => Err(FileProblem::Unreadable(Error::from(e).to_string())),
                Ok(fm) => {
                    let found = match (&store, fm.root.resolve(&ids)) {
                        (Some(store), Some(root)) if known.contains(&root) => {
                            root_status(store, &root, Some(file.clone()))?
                        }
                        _ => None,
                    };
                    found.ok_or_else(|| FileProblem::NoCheckpoint(fm.root.raw.clone()))
                }
            },
        };
        out.push(FileStatus {
            file: file.clone(),
            status,
        });
    }
    Ok(out)
}

/// One root's status, `None` if it has no checkpoint. `file` overrides the checkpoint's
/// recorded file.
fn root_status(
    store: &StateStore,
    root: &NodeId,
    file: Option<PathBuf>,
) -> Result<Option<RootStatus>, Error> {
    let Some(cp) = store.load_checkpoint(root)? else {
        return Ok(None);
    };
    let file = file.or_else(|| cp.file.clone());
    let locally_modified = file
        .as_ref()
        .and_then(|f| read_text(f).ok().flatten())
        .map(|t| !cp.md_unchanged(&t));
    Ok(Some(RootStatus {
        root: root.clone(),
        file,
        generation: cp.generation,
        synced_at: cp.synced_at,
        pending_deletes: cp
            .pending_deletes
            .iter()
            .map(|p| (p.node.clone(), p.label.clone(), p.first_seen))
            .collect(),
        journal_pending: store.journal_path(root).exists(),
        locally_modified,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testsupport::MockServer;
    use crate::wfclient::Client;
    use serde_json::json;

    fn ui_create(mock: &MockServer, parent: Option<&NodeId>, text: &str) -> NodeId {
        let parent = parent.map_or(serde_json::Value::Null, |p| json!(p.as_str()));
        let r = mock.ui("create", json!({"parent_id": parent, "text": text}));
        NodeId::parse(r["id"].as_str().expect("id")).expect("uuid")
    }

    /// §9: an edit that reaches the file while the engine writes its new generation
    /// (here: right after the history entry, and right after the new text is staged) is
    /// seen by the compare-and-swap and merged, never overwritten by the rename.
    #[test]
    fn a_file_edit_during_the_generation_write_is_merged_not_overwritten() {
        for stage in ["history", "prepared"] {
            let mock = MockServer::start();
            let client = Client::new(mock.client_config());
            let dir = tempfile::tempdir().expect("tempdir");
            let mut cfg = SyncConfig::new(dir.path().join("state"));
            cfg.clock = Clock::fixed(1_790_000_000);
            let root = ui_create(&mock, None, "Root");
            let alpha = ui_create(&mock, Some(&root), "Alpha");
            let beta = ui_create(&mock, Some(&root), "Beta");
            let file = dir.path().join("vault").join("root.md");
            let short = root.short().expect("short").to_string();
            render_once(&client, &short, &file, &DocConfig::default(), &cfg).expect("render");
            // A remote edit, so that the cycle writes the file.
            mock.ui("edit", json!({"id": beta.as_str(), "text": "Beta remote"}));

            let fired = std::rc::Rc::new(std::cell::Cell::new(false));
            let (f2, fired2) = (file.clone(), fired.clone());
            CAS_HOOK.with(|h| {
                *h.borrow_mut() = Some(Box::new(move |s| {
                    if s == stage && !fired2.get() {
                        fired2.set(true);
                        // Obsidian Sync delivers an edit made on another device.
                        let text = std::fs::read_to_string(&f2).unwrap();
                        std::fs::write(&f2, text.replacen("Alpha", "Alpha typed", 1)).unwrap();
                    }
                }))
            });
            let r = sync_once(&client, &file, &cfg);
            CAS_HOOK.with(|h| *h.borrow_mut() = None);
            let r = r.expect("sync");
            assert!(fired.get(), "{stage}: hook not reached");
            assert!(
                matches!(r.outcome, SyncOutcome::Synced { .. }),
                "{stage}: {r:#?}"
            );
            let text = std::fs::read_to_string(&file).unwrap();
            assert!(text.contains("Alpha typed"), "{stage}: edit lost:\n{text}");
            assert!(text.contains("Beta remote"), "{stage}:\n{text}");
            let tree = Tree::from_nodes(client.export().unwrap()).unwrap();
            assert_eq!(tree.get(&alpha).unwrap().name, "Alpha typed", "{stage}");
            // No staged temp file is left behind in the vault.
            let names: Vec<_> = std::fs::read_dir(file.parent().unwrap())
                .unwrap()
                .map(|e| e.unwrap().file_name())
                .collect();
            assert_eq!(names, ["root.md"], "{stage}");
            // Settled: the next cycle has nothing to do.
            let r = sync_once(&client, &file, &cfg).expect("sync");
            assert_eq!(r.outcome, SyncOutcome::Unchanged, "{stage}: {r:#?}");
        }
    }
}
