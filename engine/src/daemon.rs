//! Watch mode: the long-running engine next to an Obsidian vault (design §9.1).
//!
//! Responsibility (§9.1 deployment, guard 2 deferred deletes, guard 3 reporting, guard 4
//! debounce and "parse by content", guard 5 files identified by root, guard 6 Tasks
//! fields, the status note and the frontmatter controls, §12.1 one sequential loop with
//! `notify`, §12.2 export at most every 60 s, §14 step 6).
//!
//! Consumes: `sync` (`sync_all`, `SyncConfig`, `Clock`), `mdblock` (`parse`,
//! `parse_frontmatter`), `state` (`StateStore`, checkpoints, pending deletes),
//! `wfclient::Api`, `notify`.
//! Consumed by: `main` (`syncflowy daemon`), and `sync` ([`strip_tasks_fields`]).
//!
//! # Loop (single thread, blocking)
//!
//! [`run`] is a thin shell around [`Daemon::step`], which does one iteration and takes
//! its time from a [`Time`] source, so tests drive it with [`FakeTime`] instead of
//! sleeping. `run` uses [`RealTime`] and waits for `notify` events or
//! [`Daemon::next_wakeup`] between steps.
//!
//! 1. **Scan** (at the first step, when files changed, and at every poll): [`discover`]
//!    the synced files (every `*.md` under the vault, skipping dot directories, whose
//!    frontmatter has a `workflowy:` root; [`is_conflict_copy`] names are reported and
//!    never synced). Files are identified by root, never by path, so a rename needs no
//!    handling (guard 5). Each file is read and classified **by content**, never by
//!    mtime (guard 4): *clean* if its text is the checkpoint's `base_md` (nothing local
//!    to push), *paused* (`paused: true`), or *dirty*: then its content fingerprint is
//!    fed to the [`Debouncer`]. A dirty file whose content was already handed to a cycle
//!    that did not settle it (a brake, a skip, an error) is retried at every cycle
//!    without a new debounce, until its content changes; repeated identical failures
//!    are logged once.
//! 2. A dirty file is **ready** when its content has been stable for
//!    [`DaemonConfig::debounce`] (default 45 s, guard 4: no half-typed lines pushed, no
//!    write into a burst of edits). Its frontmatter parsed with a root at every scan,
//!    otherwise it would not have been discovered.
//! 3. A **cycle** runs when a file is ready or [`DaemonConfig::remote_poll`] has passed
//!    since the last one (remote changes are picked up without file events), but never
//!    sooner than [`DaemonConfig::min_export_interval`] after the previous cycle (§12.2;
//!    each cycle exports). It runs `sync::sync_all` over every clean file and every ready
//!    file (one export shared by all, §6.4), with `SyncConfig::delete_grace` set (guard
//!    2) and `strip_tasks_fields` on (guard 6). Dirty files that are not ready yet are
//!    left out entirely: their local edits are not pushed and they are not re-rendered.
//! 4. Guard 5: a root claimed by several files is not synced (none of the files) and is
//!    reported. A root with a checkpoint but no file is paused and reported; nothing is
//!    sent and the file is not re-rendered, because deleting the file on a client is how
//!    a user stops syncing it.
//! 5. The **status note** ([`render_status`], default [`STATUS_NOTE`] in the vault) is
//!    rewritten when its content changes, which is after every cycle (it carries the
//!    cycle's time). It has no frontmatter, so it is never mistaken for a synced file.
//! 6. Never exit on a per-file error; log it and show it in the status note. Exit only on
//!    a fatal error: the state directory is unusable, or the API answers 401.
//! 7. **Stop** on SIGTERM or SIGINT: [`run`] lets the step in progress finish (a cycle
//!    is never cut short, so no journal is left behind), then [`Daemon::stop`] writes
//!    `Stopped at ...` to the status note, and `run` returns `Ok` (exit 0). A second
//!    signal while stopping ends the process at once (exit 1; the next run recovers an
//!    interrupted cycle from its journal).
//!
//! The event log (the status note's event sections, at most [`MAX_EVENTS`]) is saved in
//! the state directory ([`EVENTS_FILE`]) after every step that changed it, and loaded
//! by [`Daemon::new`], so a restart keeps it.
//!
//! # Guards (§9.1) and where they live
//!
//! * Guard 1 (render generations) and guard 3 (stale echoes, dropped resurrections) are
//!   applied by `sync` and `merge` against `state`'s render history; the daemon logs
//!   their [`ReportEvent`]s ([`ReportEvent::StaleEcho`],
//!   [`ReportEvent::ResurrectionDropped`], [`ReportEvent::SafeMode`]) and shows them in
//!   the status note.
//! * Guard 2 (deferred deletes): `sync` records the deletes in the checkpoint's
//!   `PendingDeletes`, hides them from the render and sends them once the grace period
//!   ([`DaemonConfig::delete_grace`], default 10 min) is over **and** the file was
//!   unchanged for one more cycle; the periodic cycles provide those cycles. A tag that
//!   shows up in another file of the run cancels the delete and becomes a cross-file
//!   move; a line that comes back cancels it. While another synced file has edits
//!   waiting for its debounce, no pending delete is sent in that cycle (they may be the
//!   paste half of a cut and paste, a split arrival). UNVERIFIED design choice: the
//!   design only asks for the grace period; holding deletes for a waiting file costs at
//!   most one debounce period.
//! * Guard 4: `sync` writes the file only for remote changes or new tags (pure
//!   normalisation is deferred); the daemon syncs a file only once its content is
//!   stable; a frontmatter that does not parse is skipped and retried, never read as
//!   "everything deleted".
//! * Guard 5: [`discover`] and the missing-root check above.
//! * Guard 6: [`strip_tasks_fields`], applied by `sync` when
//!   `SyncConfig::strip_tasks_fields` is set.
//! * Frontmatter controls: `paused: true` is honoured (by `sync` and by the scan);
//!   `force: true` makes the file dirty, so it is synced after the debounce with the
//!   brakes off, and `sync` clears it in the rewritten file.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

use regex::Regex;
use serde::{Deserialize, Serialize};
use signal_hook::consts::{SIGINT, SIGTERM};

use crate::apply::ApplyError;
use crate::error::Error;
use crate::mdblock::{self, ItemKind, LocalDoc, LocalItem, ParseError, RootRef};
use crate::merge::ReportEvent;
use crate::state::{self, Checkpoint, Fingerprint, StateStore, fingerprint};
use crate::sync::{self, Clock, PauseReason, SyncConfig, SyncOutcome, SyncReport};
use crate::tree::{NodeId, ShortId};
use crate::wfclient::{Api, ApiError};

mod status;
mod tasks;

pub use status::{
    ConflictLine, Event, EventKind, PendingLine, RootLine, RootState, StatusReport, format_time,
    render_status,
};
pub use tasks::{strip_done_fields, strip_tasks_fields};

use status::file_link;

/// The default status note's file name (engine-owned, overwritten every cycle).
pub const STATUS_NOTE: &str = "Workflowy Sync Status.md";

/// How many events the status note keeps (newest first).
pub const MAX_EVENTS: usize = 50;

/// The event log's file in the state directory (JSON, newest first).
pub const EVENTS_FILE: &str = "daemon-events.json";

/// While waiting between steps, how often [`run`] checks for a stop signal.
const STOP_POLL: Duration = Duration::from_millis(200);

/// The shortest wait between two steps of [`run`] (no busy loop).
const MIN_WAIT: Duration = Duration::from_millis(500);

/// After a file event, how long [`run`] keeps collecting events before the next step.
const COALESCE: Duration = Duration::from_secs(1);

// ---------------------------------------------------------------------------------------
// Configuration and time
// ---------------------------------------------------------------------------------------

/// Daemon options.
#[derive(Debug, Clone)]
pub struct DaemonConfig {
    /// The vault directory to watch.
    pub vault: PathBuf,
    /// Sync options (its `delete_grace`, `clock` and `strip_tasks_fields` are set by the
    /// daemon).
    pub sync: SyncConfig,
    /// Quiet period before a changed file is synced (§9.1 guard 4: 30–60 s). Default 45 s.
    pub debounce: Duration,
    /// Interval of the periodic cycle that picks up remote changes without file events.
    /// Default 60 s.
    pub remote_poll: Duration,
    /// Minimum interval between two cycles, each of which exports (§12.2: at most one
    /// export per 60 s). Default 60 s.
    pub min_export_interval: Duration,
    /// Grace period of deferred deletes (§9.1 guard 2). Default 10 minutes.
    pub delete_grace: Duration,
    /// Status note path relative to the vault. Default [`STATUS_NOTE`].
    pub status_note: PathBuf,
}

impl DaemonConfig {
    /// Defaults for `vault` with sync options `sync`.
    pub fn new(vault: impl Into<PathBuf>, sync: SyncConfig) -> DaemonConfig {
        DaemonConfig {
            vault: vault.into(),
            sync,
            debounce: Duration::from_secs(45),
            remote_poll: Duration::from_secs(60),
            min_export_interval: Duration::from_secs(60),
            delete_grace: Duration::from_secs(600),
            status_note: PathBuf::from(STATUS_NOTE),
        }
    }
}

/// Where the daemon takes its time from: a monotonic instant for debouncing and
/// intervals, and unix seconds for the state (pending deletes, checkpoints, the note).
pub trait Time: Send + Sync {
    /// The monotonic now.
    fn now(&self) -> Instant;
    /// The wall-clock now in unix seconds.
    fn unix(&self) -> i64;
}

/// The real clocks (used by [`run`], i.e. by `main`).
#[derive(Debug, Clone, Copy, Default)]
pub struct RealTime;

impl Time for RealTime {
    fn now(&self) -> Instant {
        Instant::now()
    }
    fn unix(&self) -> i64 {
        sync::unix_now()
    }
}

/// A clock that moves only when told to ([`FakeTime::advance`]), for tests.
#[derive(Debug)]
pub struct FakeTime {
    start: Instant,
    unix0: i64,
    elapsed: Mutex<Duration>,
}

impl FakeTime {
    /// A fake clock reading `unix0` unix seconds.
    pub fn new(unix0: i64) -> FakeTime {
        FakeTime {
            start: Instant::now(),
            unix0,
            elapsed: Mutex::new(Duration::ZERO),
        }
    }

    /// Move the clock forward.
    pub fn advance(&self, d: Duration) {
        let mut e = self.elapsed.lock().unwrap_or_else(|p| p.into_inner());
        *e += d;
    }

    fn elapsed(&self) -> Duration {
        *self.elapsed.lock().unwrap_or_else(|p| p.into_inner())
    }
}

impl Time for FakeTime {
    fn now(&self) -> Instant {
        self.start + self.elapsed()
    }
    fn unix(&self) -> i64 {
        self.unix0
            .saturating_add(i64::try_from(self.elapsed().as_secs()).unwrap_or(i64::MAX))
    }
}

// ---------------------------------------------------------------------------------------
// Discovery (guard 5)
// ---------------------------------------------------------------------------------------

/// A file that names a sync root.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncedFile {
    /// Path.
    pub path: PathBuf,
    /// Its `workflowy:` root.
    pub root: RootRef,
}

/// Result of scanning a vault (§9.1 guard 5).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Discovery {
    /// Files with a unique root, sorted by path.
    pub files: Vec<SyncedFile>,
    /// Roots claimed by several files (short id -> paths); none of them is synced.
    pub duplicates: BTreeMap<String, Vec<PathBuf>>,
    /// Markdown files with a `workflowy:` key whose frontmatter does not parse, and
    /// conflict copies, with the reason.
    pub skipped: Vec<(PathBuf, String)>,
}

/// What a file's frontmatter says about syncing.
enum Claim {
    /// A synced file of this root.
    Root(RootRef),
    /// Not a synced file (no frontmatter, or no `workflowy:` key).
    None,
    /// It has a `workflowy:` key but cannot be read as a synced file.
    Bad(String),
}

/// True if one of the first lines of `text` is a top-level `workflowy:` key (used to
/// tell a broken synced file from an unrelated note when the frontmatter fails).
fn mentions_root_key(text: &str) -> bool {
    text.lines()
        .take(100)
        .any(|l| l.trim_end().starts_with("workflowy:"))
}

fn claim(bytes: &[u8]) -> Claim {
    let text = match std::str::from_utf8(bytes) {
        Ok(t) => t,
        Err(_) => {
            let lossy = String::from_utf8_lossy(bytes);
            return if mentions_root_key(&lossy) && mdblock::parse_frontmatter(&lossy).is_ok() {
                Claim::Bad("the file is not valid UTF-8".to_owned())
            } else {
                Claim::None
            };
        }
    };
    match mdblock::parse_frontmatter(text) {
        Ok(fm) => Claim::Root(fm.root),
        Err(ParseError::NoFrontmatter | ParseError::NoRoot) => Claim::None,
        Err(
            e @ (ParseError::UnterminatedFrontmatter
            | ParseError::BadRoot(_)
            | ParseError::BadConfig { .. }),
        ) => {
            if mentions_root_key(text) {
                Claim::Bad(format!(
                    "the frontmatter does not parse ({e}); retried later"
                ))
            } else {
                Claim::None
            }
        }
    }
}

/// Every `*.md` file under `dir`, skipping dot files and dot directories (`.obsidian`,
/// `.trash`, `.git`). Symbolic links to files are followed, to directories not.
fn markdown_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) => {
            log::warn!("{}: cannot list: {e}", dir.display());
            return;
        }
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        if name.to_string_lossy().starts_with('.') {
            continue;
        }
        let path = entry.path();
        let Ok(ft) = entry.file_type() else { continue };
        if ft.is_dir() {
            markdown_files(&path, out);
        } else if is_markdown(&path) && (ft.is_file() || path.is_file()) {
            out.push(path);
        }
    }
}

fn is_markdown(path: &Path) -> bool {
    path.extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("md"))
}

/// Scan `vault` for synced files (module docs, step 1).
pub fn discover(vault: &Path) -> Discovery {
    let mut paths = Vec::new();
    markdown_files(vault, &mut paths);
    paths.sort();
    let mut candidates: Vec<SyncedFile> = Vec::new();
    let mut skipped = Vec::new();
    for path in paths {
        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(e) => {
                log::debug!("{}: cannot read: {e}", path.display());
                continue;
            }
        };
        match claim(&bytes) {
            Claim::None => {}
            Claim::Bad(reason) => skipped.push((path, reason)),
            Claim::Root(_) if is_conflict_copy(&path) => skipped.push((
                path,
                "looks like a sync conflict copy; never synced (merge it by hand, then delete it)"
                    .to_owned(),
            )),
            Claim::Root(root) => candidates.push(SyncedFile { path, root }),
        }
    }
    let mut by_root: BTreeMap<ShortId, Vec<usize>> = BTreeMap::new();
    for (i, f) in candidates.iter().enumerate() {
        by_root.entry(f.root.short.clone()).or_default().push(i);
    }
    let mut duplicates = BTreeMap::new();
    let mut files = Vec::new();
    for (short, idx) in by_root {
        if idx.len() == 1 {
            files.push(candidates[idx[0]].clone());
        } else {
            duplicates.insert(
                short.to_string(),
                idx.iter().map(|i| candidates[*i].path.clone()).collect(),
            );
        }
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    Discovery {
        files,
        duplicates,
        skipped,
    }
}

static CONFLICT_COPY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(\.sync-conflict-|conflicted copy|\([^()]*\bconflict[^()]*\))")
        .expect("conflict-copy pattern")
});

/// True for names that look like sync conflict copies (Obsidian Sync and common
/// tools: `name (conflicted copy …).md`, `name (Conflict …).md`, `name.sync-conflict-….md`,
/// `name (1).md`-style duplicates are **not** included). UNVERIFIED: Obsidian Sync's
/// exact naming (§13 Q7); any parenthesised group containing the word "conflict" counts.
pub fn is_conflict_copy(path: &Path) -> bool {
    path.file_name()
        .is_some_and(|n| CONFLICT_COPY.is_match(&n.to_string_lossy()))
}

// ---------------------------------------------------------------------------------------
// Debouncing (guard 4)
// ---------------------------------------------------------------------------------------

/// Content-based debouncing (§9.1 guard 4). Pure: the caller passes the time.
#[derive(Debug, Clone, Default)]
pub struct Debouncer {
    seen: BTreeMap<PathBuf, (Fingerprint, Instant)>,
}

impl Debouncer {
    /// An empty debouncer.
    pub fn new() -> Debouncer {
        Debouncer::default()
    }

    /// Record the current content fingerprint of `path` at `now`. The stability timer
    /// restarts only when the fingerprint differs from the last one recorded.
    pub fn observe(&mut self, path: &Path, fp: Fingerprint, now: Instant) {
        match self.seen.get_mut(path) {
            Some((old, _)) if *old == fp => {}
            Some(entry) => *entry = (fp, now),
            None => {
                self.seen.insert(path.to_path_buf(), (fp, now));
            }
        }
    }

    /// Paths whose fingerprint has not changed for at least `quiet`, removing them from
    /// the debouncer.
    pub fn take_ready(&mut self, now: Instant, quiet: Duration) -> Vec<PathBuf> {
        let ready: Vec<PathBuf> = self
            .seen
            .iter()
            .filter(|(_, (_, since))| now.saturating_duration_since(*since) >= quiet)
            .map(|(p, _)| p.clone())
            .collect();
        for p in &ready {
            self.seen.remove(p);
        }
        ready
    }

    /// The earliest time at which a path becomes ready, if any is tracked.
    pub fn next_ready(&self, quiet: Duration) -> Option<Instant> {
        self.seen.values().map(|(_, since)| *since + quiet).min()
    }

    /// True if some path is tracked and not yet ready at `now`.
    pub fn has_waiting(&self, now: Instant, quiet: Duration) -> bool {
        self.seen
            .values()
            .any(|(_, since)| now.saturating_duration_since(*since) < quiet)
    }

    /// Stop tracking `path` (it is clean again, paused, or gone).
    pub fn forget(&mut self, path: &Path) {
        self.seen.remove(path);
    }

    /// Keep only the paths for which `keep` is true.
    pub fn retain(&mut self, mut keep: impl FnMut(&Path) -> bool) {
        self.seen.retain(|p, _| keep(p));
    }

    /// True if `path` is tracked.
    pub fn contains(&self, path: &Path) -> bool {
        self.seen.contains_key(path)
    }
}

// ---------------------------------------------------------------------------------------
// The daemon
// ---------------------------------------------------------------------------------------

/// How a scan classified a synced file.
#[derive(Debug, Clone, PartialEq, Eq)]
enum FileState {
    /// Its text is the checkpoint's `base_md`.
    Clean,
    /// Local edits (or no checkpoint): debounced.
    Dirty,
    /// Local edits already handed to a cycle that did not settle them (a brake, a
    /// skip, an error): retried at every cycle without a new debounce, until the text
    /// changes again.
    Retry,
    /// `paused: true`.
    Paused,
    /// Cannot be read now (retried at the next scan).
    Unreadable(String),
}

/// A synced file as of the last scan.
#[derive(Debug, Clone)]
struct Tracked {
    path: PathBuf,
    root: RootRef,
    /// The full root id, when known (from the file or a checkpoint).
    node: Option<NodeId>,
    state: FileState,
    has_checkpoint: bool,
    conflicts: Vec<ConflictLine>,
    /// Content fingerprint (when the file could be read).
    fp: Option<Fingerprint>,
}

/// Each synced file's report, or its error as text.
pub type FileResults = Vec<(PathBuf, Result<SyncReport, String>)>;

/// What one [`Daemon::step`] did.
#[derive(Debug, Default)]
pub struct StepReport {
    /// True if the vault was scanned.
    pub scanned: bool,
    /// The files synced, if a cycle ran.
    pub synced: Option<Vec<PathBuf>>,
    /// Each synced file's report, or its error as text.
    pub results: FileResults,
    /// True if the status note was written.
    pub status_written: bool,
}

/// The daemon's state between steps (module docs).
pub struct Daemon {
    cfg: DaemonConfig,
    time: Arc<dyn Time>,
    store: StateStore,
    debouncer: Debouncer,
    tracked: Vec<Tracked>,
    discovery: Discovery,
    last_scan: Option<Instant>,
    last_cycle: Option<Instant>,
    last_cycle_unix: i64,
    /// Last successful cycle per root (unix seconds).
    last_ok: BTreeMap<NodeId, i64>,
    /// The last outcome per file.
    outcomes: BTreeMap<PathBuf, SyncOutcome>,
    /// Current errors per file (`""` for the run-wide one).
    errors: BTreeMap<PathBuf, StoredError>,
    events: VecDeque<Event>,
    /// True if `events` changed since it was last saved.
    events_dirty: bool,
    /// Set by [`Daemon::stop`].
    stopped_at: Option<i64>,
    deletes_held: bool,
    status_text: Option<String>,
    /// The content of each dirty file when it was last handed to a cycle.
    attempted: BTreeMap<PathBuf, Fingerprint>,
}

/// A current error of a file (or of the whole run), kept until the file syncs again.
#[derive(Debug, Clone, PartialEq, Eq)]
struct StoredError {
    /// The message, with the daemon's own remedy ([`daemon_message`]).
    text: String,
    /// `Error::NoCheckpoint`: shown as [`RootState::NoCheckpoint`].
    no_checkpoint: bool,
}

impl StoredError {
    fn of(e: &Error) -> StoredError {
        StoredError {
            text: daemon_message(e),
            no_checkpoint: matches!(e, Error::NoCheckpoint(_)),
        }
    }
}

/// An error's message as the daemon shows it: the engine's messages name no remedy where
/// the CLI's (`--force`) and the daemon's (the frontmatter) differ, so the daemon adds
/// its own.
fn daemon_message(e: &Error) -> String {
    match e {
        Error::NoCheckpoint(_) => format!(
            "{e}; run `syncflowy render`, or set `force: true` in the frontmatter to adopt the file"
        ),
        _ => e.to_string(),
    }
}

/// True for errors after which the daemon must stop (module docs, step 6): the API
/// rejects the credentials. (A failing state directory is fatal too, but only when the
/// whole run fails on it; see [`Daemon::step`].)
pub fn is_fatal(e: &Error) -> bool {
    matches!(
        e,
        Error::Api(ApiError::Unauthorized)
            | Error::Apply(
                ApplyError::Api {
                    error: ApiError::Unauthorized,
                    ..
                } | ApplyError::Export(ApiError::Unauthorized)
            )
    )
}

fn is_api_error(e: &Error) -> bool {
    matches!(
        e,
        Error::Api(_) | Error::Apply(ApplyError::Api { .. } | ApplyError::Export(_))
    )
}

/// Conflict markers of a parsed file, with the item each belongs to.
fn conflicts_of(file: &Path, doc: &LocalDoc) -> Vec<ConflictLine> {
    fn rec(file: &Path, parent: &str, items: &[LocalItem], out: &mut Vec<ConflictLine>) {
        for it in items {
            if let ItemKind::ConflictMarker(field) = it.kind {
                out.push(ConflictLine {
                    file: file.to_path_buf(),
                    line: it.line,
                    item: parent.to_owned(),
                    field,
                });
            }
            rec(file, &it.name_md, &it.children, out);
        }
    }
    let mut out = Vec::new();
    rec(file, "", &doc.items, &mut out);
    out
}

/// The kind under which a merge event is logged, or `None` for routine events.
fn event_kind(e: &ReportEvent) -> Option<EventKind> {
    match e {
        ReportEvent::SafeMode { .. } => Some(EventKind::SafeMode),
        ReportEvent::ResurrectionDropped { .. } => Some(EventKind::ResurrectionDropped),
        ReportEvent::StaleEcho { .. } => Some(EventKind::StaleEcho),
        ReportEvent::Resurrected { .. }
        | ReportEvent::VanishedWork { .. }
        | ReportEvent::DroppedMove { .. }
        | ReportEvent::DeleteLost { .. }
        | ReportEvent::ForeignTag { .. }
        | ReportEvent::DuplicateTag { .. }
        | ReportEvent::ReadOnlyEditRefused { .. }
        | ReportEvent::PlaceholderDeleteRefused { .. }
        | ReportEvent::MirrorDeleteRefused { .. }
        | ReportEvent::Conflict { .. }
        | ReportEvent::MovedOutConflict { .. }
        | ReportEvent::ConflictLost { .. }
        | ReportEvent::LossyWrite { .. }
        | ReportEvent::PendingDeleteCancelled { .. } => Some(EventKind::Info),
        ReportEvent::MovedIn { from_file_root, .. } => {
            from_file_root.as_ref().map(|_| EventKind::Info)
        }
        ReportEvent::Appeared { .. }
        | ReportEvent::Disappeared { .. }
        | ReportEvent::MovedOut { .. }
        | ReportEvent::Rematched { .. }
        | ReportEvent::Adopted { .. }
        | ReportEvent::AlreadyCompleted { .. } => None,
    }
}

impl Daemon {
    /// A daemon over `cfg.vault` taking its time from `time`. Opens the state directory
    /// (fatal if unusable) and checks the vault.
    pub fn new(mut cfg: DaemonConfig, time: Arc<dyn Time>) -> Result<Daemon, Error> {
        if !cfg.vault.is_dir() {
            return Err(Error::Io {
                path: cfg.vault.clone(),
                source: std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "the vault is not a directory",
                ),
            });
        }
        let store = StateStore::open(&cfg.sync.state_dir)?;
        let events = load_events(&store);
        if let (Ok(state), Ok(vault)) = (
            std::fs::canonicalize(&cfg.sync.state_dir),
            std::fs::canonicalize(&cfg.vault),
        ) && state.starts_with(&vault)
        {
            log::warn!(
                "the state directory {} is inside the vault; keep it outside (§9.1)",
                state.display()
            );
        }
        let t = Arc::clone(&time);
        cfg.sync.clock = Clock::new(move || t.unix());
        cfg.sync.delete_grace = Some(cfg.delete_grace);
        cfg.sync.strip_tasks_fields = true;
        cfg.sync.dry_run = false;
        Ok(Daemon {
            cfg,
            time,
            store,
            debouncer: Debouncer::new(),
            tracked: Vec::new(),
            discovery: Discovery::default(),
            last_scan: None,
            last_cycle: None,
            last_cycle_unix: 0,
            last_ok: BTreeMap::new(),
            outcomes: BTreeMap::new(),
            errors: BTreeMap::new(),
            events,
            events_dirty: false,
            stopped_at: None,
            deletes_held: false,
            status_text: None,
            attempted: BTreeMap::new(),
        })
    }

    /// The configuration in effect.
    pub fn config(&self) -> &DaemonConfig {
        &self.cfg
    }

    /// The status note's path.
    pub fn status_path(&self) -> PathBuf {
        self.cfg.vault.join(&self.cfg.status_note)
    }

    /// True if a file event for `path` can matter (a Markdown file outside dot
    /// directories, not the status note).
    pub fn is_relevant(&self, path: &Path) -> bool {
        if !is_markdown(path) || path == self.status_path() {
            return false;
        }
        let rel = path.strip_prefix(&self.cfg.vault).unwrap_or(path);
        !rel.components()
            .any(|c| c.as_os_str().to_string_lossy().starts_with('.'))
    }

    /// When the next step has something to do (a file becomes ready, or the poll).
    pub fn next_wakeup(&self) -> Instant {
        let now = self.time.now();
        let Some(last) = self.last_cycle else {
            return now;
        };
        let poll = last + self.cfg.remote_poll.max(self.cfg.min_export_interval);
        let ready = self
            .debouncer
            .next_ready(self.cfg.debounce)
            .map(|t| t.max(last + self.cfg.min_export_interval));
        ready.map_or(poll, |r| r.min(poll))
    }

    fn log_event(&mut self, file: Option<&Path>, kind: EventKind, text: String) {
        let at = self.time.unix();
        match kind {
            EventKind::ApiError | EventKind::Error | EventKind::Brake => {
                log::warn!(
                    "{}{text}",
                    file.map(|f| format!("{}: ", f.display()))
                        .unwrap_or_default()
                )
            }
            _ => log::info!(
                "{}{text}",
                file.map(|f| format!("{}: ", f.display()))
                    .unwrap_or_default()
            ),
        }
        self.events.push_front(Event {
            at,
            file: file.map(Path::to_path_buf),
            kind,
            text,
        });
        self.events.truncate(MAX_EVENTS);
        self.events_dirty = true;
    }

    /// Save the event log if it changed. A failure is logged, not fatal.
    fn save_events(&mut self) {
        if !self.events_dirty {
            return;
        }
        let path = self.store.dir().join(EVENTS_FILE);
        let result = serde_json::to_vec_pretty(&self.events)
            .map_err(std::io::Error::other)
            .and_then(|bytes| state::atomic_write(&path, &bytes));
        match result {
            Ok(()) => self.events_dirty = false,
            Err(e) => log::warn!("{}: cannot save the event log: {e}", path.display()),
        }
    }

    /// Stop (module docs, step 7): log it, write `Stopped at ...` to the status note and
    /// save the event log. Call it between steps.
    pub fn stop(&mut self) -> Result<(), Error> {
        let at = self.time.unix();
        self.stopped_at = Some(at);
        self.log_event(None, EventKind::Info, "the daemon stopped".to_owned());
        self.write_status()?;
        self.save_events();
        Ok(())
    }

    /// Step 1: discover and classify the synced files.
    fn scan(&mut self, now: Instant) -> Result<(), Error> {
        let disc = discover(&self.cfg.vault);
        let mut by_short: BTreeMap<ShortId, NodeId> = BTreeMap::new();
        for r in self.store.roots()? {
            if let Some(s) = r.short() {
                by_short.insert(s, r);
            }
        }
        let mut tracked = Vec::new();
        for f in &disc.files {
            let node = f
                .root
                .full
                .clone()
                .or_else(|| by_short.get(&f.root.short).cloned());
            let checkpoint = match &node {
                Some(n) => match self.store.load_checkpoint(n) {
                    Ok(cp) => cp,
                    Err(e) => {
                        log::warn!("checkpoint of {n}: {e}");
                        None
                    }
                },
                None => None,
            };
            let mut t = Tracked {
                path: f.path.clone(),
                root: f.root.clone(),
                node,
                state: FileState::Dirty,
                has_checkpoint: checkpoint.is_some(),
                conflicts: Vec::new(),
                fp: None,
            };
            let text = match std::fs::read(&f.path).map(String::from_utf8) {
                Ok(Ok(text)) => text,
                Ok(Err(_)) => {
                    t.state = FileState::Unreadable("not valid UTF-8".to_owned());
                    self.debouncer.forget(&f.path);
                    tracked.push(t);
                    continue;
                }
                Err(e) => {
                    t.state = FileState::Unreadable(e.to_string());
                    self.debouncer.forget(&f.path);
                    tracked.push(t);
                    continue;
                }
            };
            let doc = match mdblock::parse(&text) {
                Ok(d) => d,
                Err(e) => {
                    // Guard 4: never "everything deleted"; retried at the next scan.
                    t.state = FileState::Unreadable(e.to_string());
                    self.debouncer.forget(&f.path);
                    tracked.push(t);
                    continue;
                }
            };
            let fp = fingerprint(&text);
            t.conflicts = conflicts_of(&f.path, &doc);
            t.state = if doc.frontmatter.paused {
                FileState::Paused
            } else if checkpoint.as_ref().is_some_and(|c| c.md_unchanged(&text)) {
                FileState::Clean
            } else if self.attempted.get(&f.path) == Some(&fp) {
                FileState::Retry
            } else {
                FileState::Dirty
            };
            match t.state {
                FileState::Dirty => self.debouncer.observe(&f.path, fp.clone(), now),
                FileState::Retry => self.debouncer.forget(&f.path),
                FileState::Clean | FileState::Paused | FileState::Unreadable(_) => {
                    self.debouncer.forget(&f.path);
                    self.attempted.remove(&f.path);
                }
            }
            t.fp = Some(fp);
            tracked.push(t);
        }
        let live: BTreeSet<PathBuf> = tracked
            .iter()
            .filter(|t| t.state == FileState::Dirty)
            .map(|t| t.path.clone())
            .collect();
        self.debouncer.retain(|p| live.contains(p));
        let known: BTreeSet<&Path> = tracked.iter().map(|t| t.path.as_path()).collect();
        self.attempted.retain(|p, _| known.contains(p.as_path()));
        // A file that was deleted or renamed takes its error and outcome with it (its
        // root is reported under the new path, or as missing: guard 5); the run-wide
        // error (`""`) stays until a cycle succeeds.
        self.errors
            .retain(|p, _| p.as_os_str().is_empty() || known.contains(p.as_path()));
        self.outcomes.retain(|p, _| known.contains(p.as_path()));
        for (path, reason) in &disc.skipped {
            if !self
                .discovery
                .skipped
                .iter()
                .any(|(p, r)| p == path && r == reason)
            {
                log::warn!("{}: {reason}", path.display());
            }
        }
        for (short, paths) in &disc.duplicates {
            if self.discovery.duplicates.get(short) != Some(paths) {
                let files: Vec<String> = paths
                    .iter()
                    .map(|p| file_link(&self.cfg.vault, p))
                    .collect();
                self.log_event(
                    None,
                    EventKind::Error,
                    format!(
                        "{} all name root {short}; that root is paused",
                        files.join(", ")
                    ),
                );
            }
        }
        self.tracked = tracked;
        self.discovery = disc;
        self.last_scan = Some(now);
        Ok(())
    }

    /// Record one file's report: outcome, last success, events.
    fn record(&mut self, file: &Path, report: &SyncReport) {
        let unix = self.time.unix();
        self.errors.remove(file);
        let previous = self
            .outcomes
            .insert(file.to_path_buf(), report.outcome.clone());
        if matches!(
            report.outcome,
            SyncOutcome::Braked(_) | SyncOutcome::Skipped { .. }
        ) && previous.as_ref() == Some(&report.outcome)
        {
            // A retry that failed the same way: already in the log.
            return;
        }
        let tree = report
            .root
            .as_ref()
            .and_then(|r| self.store.load_checkpoint(r).ok().flatten())
            .map(|c: Checkpoint| c.base_wf)
            .unwrap_or_default();
        match &report.outcome {
            SyncOutcome::Unchanged | SyncOutcome::Synced { .. } | SyncOutcome::Rendered { .. } => {
                if let Some(r) = &report.root {
                    self.last_ok.insert(r.clone(), unix);
                }
            }
            SyncOutcome::Braked(b) => self.log_event(
                Some(file),
                EventKind::Brake,
                format!(
                    "refused by a brake: {b}; set `force: true` in the frontmatter to override once"
                ),
            ),
            SyncOutcome::Skipped { reason } => {
                self.log_event(Some(file), EventKind::Error, format!("skipped: {reason}"))
            }
            SyncOutcome::Paused(_) | SyncOutcome::DryRun { .. } => {}
        }
        if let SyncOutcome::Synced { pushed, .. } = &report.outcome
            && pushed.deletes > 0
        {
            self.log_event(
                Some(file),
                EventKind::Info,
                format!(
                    "deleted {} item(s) in Workflowy (with {} hidden)",
                    pushed.deletes, pushed.hidden_deleted
                ),
            );
        }
        for w in &report.warnings {
            if w.contains("safe mode") {
                self.log_event(Some(file), EventKind::SafeMode, w.clone());
            }
        }
        for e in &report.events {
            if matches!(e, ReportEvent::SafeMode { suppressed: 0 }) {
                continue; // the warning above already says so
            }
            if let Some(kind) = event_kind(e) {
                self.log_event(Some(file), kind, e.describe(&tree));
            }
        }
        for d in &report.discrepancies {
            self.log_event(
                Some(file),
                EventKind::Error,
                format!("verification after the push: {d:?}"),
            );
        }
    }

    /// Step 3: one sync cycle over the clean and ready files.
    fn cycle(&mut self, api: &dyn Api, now: Instant) -> Result<(Vec<PathBuf>, FileResults), Error> {
        let ready: BTreeSet<PathBuf> = self
            .debouncer
            .take_ready(now, self.cfg.debounce)
            .into_iter()
            .collect();
        // Split arrivals (guard 2): hold due deletes while a file with a checkpoint has
        // edits that are still settling.
        let held = self.tracked.iter().any(|t| {
            t.state == FileState::Dirty
                && t.has_checkpoint
                && !ready.contains(&t.path)
                && self.debouncer.contains(&t.path)
        });
        let files: Vec<PathBuf> = self
            .tracked
            .iter()
            .filter(|t| match t.state {
                FileState::Clean | FileState::Retry => true,
                FileState::Dirty => ready.contains(&t.path),
                FileState::Paused | FileState::Unreadable(_) => false,
            })
            .map(|t| t.path.clone())
            .collect();
        for t in &self.tracked {
            if let (true, Some(fp)) = (ready.contains(&t.path), &t.fp) {
                self.attempted.insert(t.path.clone(), fp.clone());
            }
        }
        self.last_cycle = Some(now);
        self.last_cycle_unix = self.time.unix();
        self.deletes_held = held;
        let mut results = Vec::new();
        if files.is_empty() {
            return Ok((files, results));
        }
        let mut cfg = self.cfg.sync.clone();
        if held {
            // A grace period that never ends: nothing is due this cycle.
            cfg.delete_grace = Some(Duration::from_secs(i64::MAX as u64));
        }
        match sync::sync_all(api, &files, &cfg) {
            // `sync_all` fails as a whole only on the state directory or the export.
            Err(e) if is_fatal(&e) || matches!(e, Error::State(_)) => return Err(e),
            Err(e) => {
                let kind = if is_api_error(&e) {
                    EventKind::ApiError
                } else {
                    EventKind::Error
                };
                let err = StoredError::of(&e);
                self.log_event(None, kind, format!("the cycle failed: {}", err.text));
                for f in &files {
                    results.push((f.clone(), Err(err.text.clone())));
                }
                self.errors.insert(PathBuf::new(), err);
            }
            Ok(file_results) => {
                self.errors.remove(Path::new(""));
                for fr in file_results {
                    match fr.result {
                        Ok(report) => {
                            self.record(&fr.file, &report);
                            results.push((fr.file, Ok(report)));
                        }
                        Err(e) if is_fatal(&e) => return Err(e),
                        Err(e) => {
                            let kind = if is_api_error(&e) {
                                EventKind::ApiError
                            } else {
                                EventKind::Error
                            };
                            let err = StoredError::of(&e);
                            if self.errors.get(&fr.file) != Some(&err) {
                                self.log_event(Some(&fr.file), kind, err.text.clone());
                            }
                            self.outcomes.remove(&fr.file);
                            results.push((fr.file.clone(), Err(err.text.clone())));
                            self.errors.insert(fr.file, err);
                        }
                    }
                }
            }
        }
        Ok((files, results))
    }

    /// The status as of now (from the last scan, the last cycle and the checkpoints).
    pub fn status(&self) -> Result<StatusReport, Error> {
        let grace = i64::try_from(self.cfg.delete_grace.as_secs()).unwrap_or(i64::MAX);
        let mut roots: Vec<RootLine> = Vec::new();
        let mut pending: Vec<PendingLine> = Vec::new();
        let mut seen: BTreeSet<NodeId> = BTreeSet::new();
        let checkpoints: Vec<NodeId> = self.store.roots()?;
        let load = |n: &NodeId| self.store.load_checkpoint(n).ok().flatten();
        let name_of = |cp: &Checkpoint| {
            cp.base_wf
                .get(&cp.root)
                .map(|n| crate::wfhtml::to_plain_text(&n.name))
        };

        for t in &self.tracked {
            let cp = t.node.as_ref().and_then(load);
            if let Some(n) = &t.node {
                seen.insert(n.clone());
            }
            // A file without an error of its own shows the run-wide one (an export that
            // failed, a rejected key), if any.
            let error = self
                .errors
                .get(&t.path)
                .or_else(|| self.errors.get(Path::new("")));
            let state = match (&t.state, error, self.outcomes.get(&t.path)) {
                (FileState::Paused, ..) => RootState::Paused(PauseReason::Frontmatter),
                (FileState::Unreadable(e), ..) => RootState::Skipped(e.clone()),
                (_, Some(e), _) if e.no_checkpoint => RootState::NoCheckpoint,
                (_, Some(e), _) => RootState::Failed(e.text.clone()),
                (FileState::Dirty, ..) if self.debouncer.contains(&t.path) => RootState::Waiting,
                (_, None, Some(o)) => match o {
                    SyncOutcome::Unchanged
                    | SyncOutcome::Synced { .. }
                    | SyncOutcome::Rendered { .. }
                    | SyncOutcome::DryRun { .. } => RootState::Ok,
                    SyncOutcome::Paused(r) => RootState::Paused(r.clone()),
                    SyncOutcome::Braked(b) => RootState::Braked(b.to_string()),
                    SyncOutcome::Skipped { reason } => RootState::Skipped(reason.clone()),
                },
                _ if !t.has_checkpoint => RootState::NoCheckpoint,
                _ => RootState::Unknown,
            };
            if let Some(cp) = &cp {
                for p in cp.pending_deletes.iter() {
                    pending.push(PendingLine {
                        file: Some(t.path.clone()),
                        label: p.label.clone(),
                        hidden: p.hidden_descendants,
                        first_seen: p.first_seen,
                        fires_at: p.deadline(grace),
                    });
                }
            }
            roots.push(RootLine {
                short: t.root.short.to_string(),
                name: cp.as_ref().and_then(name_of),
                files: vec![t.path.clone()],
                generation: cp.as_ref().map(|c| c.generation),
                last_sync: t
                    .node
                    .as_ref()
                    .and_then(|n| self.last_ok.get(n).copied())
                    .or_else(|| cp.as_ref().map(|c| c.synced_at)),
                state,
            });
        }
        for (short, paths) in &self.discovery.duplicates {
            let node = checkpoints
                .iter()
                .find(|n| n.short().is_some_and(|s| s.to_string() == *short));
            let cp = node.and_then(load);
            if let Some(n) = node {
                seen.insert(n.clone());
            }
            roots.push(RootLine {
                short: short.clone(),
                name: cp.as_ref().and_then(name_of),
                files: paths.clone(),
                generation: cp.as_ref().map(|c| c.generation),
                last_sync: node
                    .and_then(|n| self.last_ok.get(n).copied())
                    .or_else(|| cp.as_ref().map(|c| c.synced_at)),
                state: RootState::Paused(PauseReason::DuplicateRoot(paths.clone())),
            });
        }
        for n in &checkpoints {
            if seen.contains(n) {
                continue;
            }
            // Guard 5: a root without a file is paused; nothing is deleted.
            let cp = load(n);
            roots.push(RootLine {
                short: n.short().map_or_else(|| n.to_string(), |s| s.to_string()),
                name: cp.as_ref().and_then(name_of),
                files: Vec::new(),
                generation: cp.as_ref().map(|c| c.generation),
                last_sync: self
                    .last_ok
                    .get(n)
                    .copied()
                    .or_else(|| cp.as_ref().map(|c| c.synced_at)),
                state: RootState::Paused(PauseReason::FileMissing),
            });
        }
        roots.sort_by(|a, b| (&a.files, &a.short).cmp(&(&b.files, &b.short)));
        pending
            .sort_by(|a, b| (a.fires_at, &a.file, &a.label).cmp(&(b.fires_at, &b.file, &b.label)));
        let conflicts = self
            .tracked
            .iter()
            .flat_map(|t| t.conflicts.iter().cloned())
            .collect();
        Ok(StatusReport {
            at: self.last_cycle_unix,
            vault: self.cfg.vault.clone(),
            roots,
            conflicts,
            pending,
            deletes_held: self.deletes_held,
            errors: self
                .errors
                .iter()
                .map(|(p, e)| (p.clone(), e.text.clone()))
                .collect(),
            discovery: self.discovery.clone(),
            events: self.events.iter().cloned().collect(),
            stopped_at: self.stopped_at,
        })
    }

    /// Step 5: rewrite the status note if its content changed. A failure to write it is
    /// logged, not fatal.
    fn write_status(&mut self) -> Result<bool, Error> {
        let text = render_status(&self.status()?);
        let path = self.status_path();
        let on_disk = std::fs::read_to_string(&path).ok();
        if self.status_text.as_deref() == Some(text.as_str()) && on_disk.as_deref() == Some(&text) {
            return Ok(false);
        }
        if on_disk.as_deref() == Some(text.as_str()) {
            self.status_text = Some(text);
            return Ok(false);
        }
        if let Some(dir) = path.parent()
            && let Err(e) = std::fs::create_dir_all(dir)
        {
            log::warn!("{}: {e}", dir.display());
            return Ok(false);
        }
        match state::atomic_write(&path, text.as_bytes()) {
            Ok(()) => {
                self.status_text = Some(text);
                Ok(true)
            }
            Err(e) => {
                log::warn!("{}: cannot write the status note: {e}", path.display());
                Ok(false)
            }
        }
    }

    /// One iteration of the loop (module docs): scan if this is the first step, files
    /// changed (`changed` lists the paths of file events; any entry triggers a scan) or
    /// the poll is due; run a cycle if a file is ready or the poll is due and the
    /// minimum export interval has passed; rewrite the status note. `Err` only for fatal
    /// errors ([`is_fatal`], or the state directory failing).
    pub fn step(&mut self, api: &dyn Api, changed: &[PathBuf]) -> Result<StepReport, Error> {
        let now = self.time.now();
        let mut report = StepReport::default();
        let since_cycle = self.last_cycle.map(|t| now.saturating_duration_since(t));
        let poll_due = since_cycle.is_none_or(|d| d >= self.cfg.remote_poll);
        if self.last_scan.is_none() || poll_due || !changed.is_empty() {
            self.scan(now)?;
            report.scanned = true;
        }
        let export_ok = since_cycle.is_none_or(|d| d >= self.cfg.min_export_interval);
        let ready_due = self
            .debouncer
            .next_ready(self.cfg.debounce)
            .is_some_and(|t| t <= now);
        if export_ok && (poll_due || ready_due) {
            match self.cycle(api, now) {
                Ok((files, results)) => {
                    // The cycle may have rewritten files (new tags, conflict markers):
                    // classify them again so the status note and the debouncer see the
                    // text now on disk. The clock is read again: a cycle can take tens
                    // of seconds (export, rate-limited writes), and an edit saved
                    // meanwhile must get its whole quiet period from when it was seen,
                    // not from the cycle's start (guard 4).
                    if !files.is_empty() {
                        let after = self.time.now();
                        self.scan(after)?;
                    }
                    report.synced = Some(files);
                    report.results = results;
                }
                Err(e) => {
                    self.log_event(None, EventKind::ApiError, format!("fatal: {e}; stopping"));
                    let _ = self.write_status();
                    self.save_events();
                    return Err(e);
                }
            }
        }
        report.status_written = self.write_status()?;
        self.save_events();
        Ok(report)
    }
}

/// The saved event log ([`EVENTS_FILE`]), or an empty one if there is none or it does
/// not decode (logged).
fn load_events(store: &StateStore) -> VecDeque<Event> {
    let path = store.dir().join(EVENTS_FILE);
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return VecDeque::new(),
        Err(e) => {
            log::warn!("{}: {e}", path.display());
            return VecDeque::new();
        }
    };
    match serde_json::from_slice::<VecDeque<Event>>(&bytes) {
        Ok(mut events) => {
            events.truncate(MAX_EVENTS);
            events
        }
        Err(e) => {
            log::warn!("{}: cannot read the event log: {e}", path.display());
            VecDeque::new()
        }
    }
}

/// Run the daemon loop until a fatal error, or until SIGTERM or SIGINT (module docs,
/// step 7: then `Ok`). Uses the real clock.
pub fn run(api: &dyn Api, cfg: &DaemonConfig) -> Result<(), Error> {
    let stop = Arc::new(AtomicBool::new(false));
    for sig in [SIGTERM, SIGINT] {
        let registered =
            signal_hook::flag::register_conditional_shutdown(sig, 1, Arc::clone(&stop))
                .and_then(|_| signal_hook::flag::register(sig, Arc::clone(&stop)));
        if let Err(e) = registered {
            return Err(Error::Io {
                path: cfg.vault.clone(),
                source: std::io::Error::other(format!("cannot handle signal {sig}: {e}")),
            });
        }
    }
    let mut daemon = Daemon::new(cfg.clone(), Arc::new(RealTime))?;
    let watch_error = |e: notify::Error| Error::Io {
        path: cfg.vault.clone(),
        source: std::io::Error::other(e.to_string()),
    };
    let (tx, rx) = mpsc::channel::<notify::Result<notify::Event>>();
    let mut watcher = notify::recommended_watcher(move |res| {
        let _ = tx.send(res);
    })
    .map_err(watch_error)?;
    notify::Watcher::watch(&mut watcher, &cfg.vault, notify::RecursiveMode::Recursive)
        .map_err(watch_error)?;
    log::info!("watching {}", cfg.vault.display());

    let mut changed: Vec<PathBuf> = Vec::new();
    let stopping = || stop.load(Ordering::SeqCst);
    'steps: loop {
        daemon.step(api, &changed)?;
        changed.clear();
        let mut deadline = daemon.next_wakeup().max(Instant::now() + MIN_WAIT);
        loop {
            if stopping() {
                break 'steps;
            }
            let now = Instant::now();
            if now >= deadline {
                break;
            }
            match rx.recv_timeout((deadline - now).min(STOP_POLL)) {
                Ok(Ok(event)) => {
                    let before = changed.len();
                    changed.extend(event.paths.into_iter().filter(|p| daemon.is_relevant(p)));
                    if changed.len() > before {
                        deadline = deadline.min(Instant::now() + COALESCE);
                    }
                }
                Ok(Err(e)) => {
                    // A lost event: rescan everything at the next step.
                    log::warn!("file watcher: {e}");
                    changed.push(cfg.vault.clone());
                    deadline = deadline.min(Instant::now() + COALESCE);
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(Error::Io {
                        path: cfg.vault.clone(),
                        source: std::io::Error::other("the file watcher stopped"),
                    });
                }
            }
        }
    }
    log::info!("stopping on a signal");
    daemon.stop()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conflict_copies() {
        for n in [
            "Plan (conflicted copy 2026-01-01).md",
            "Plan (Bob's conflicted copy).md",
            "Plan (Conflict 2026-10-13 12.00.00).md",
            "Plan.sync-conflict-20261013-120000-ABCDEFG.md",
        ] {
            assert!(is_conflict_copy(Path::new(n)), "{n}");
        }
        for n in ["Plan.md", "Plan (1).md", "Conflicts.md", "Plan (draft).md"] {
            assert!(!is_conflict_copy(Path::new(n)), "{n}");
        }
    }

    #[test]
    fn debouncer_restarts_only_on_change() {
        let t0 = Instant::now();
        let s = Duration::from_secs;
        let mut d = Debouncer::new();
        let p = Path::new("a.md");
        d.observe(p, fingerprint("1"), t0);
        d.observe(p, fingerprint("1"), t0 + s(30));
        assert_eq!(d.next_ready(s(45)), Some(t0 + s(45)));
        assert!(d.take_ready(t0 + s(44), s(45)).is_empty());
        d.observe(p, fingerprint("2"), t0 + s(40));
        assert!(d.take_ready(t0 + s(80), s(45)).is_empty());
        assert!(d.has_waiting(t0 + s(80), s(45)));
        assert_eq!(d.take_ready(t0 + s(85), s(45)), vec![p.to_path_buf()]);
        assert!(!d.contains(p));
        assert_eq!(d.next_ready(s(45)), None);
    }

    #[test]
    fn fake_time_moves_both_clocks() {
        let t = FakeTime::new(100);
        let i0 = t.now();
        t.advance(Duration::from_secs(5));
        assert_eq!(t.unix(), 105);
        assert_eq!(t.now() - i0, Duration::from_secs(5));
    }
}
