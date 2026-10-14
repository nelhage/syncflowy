//! Watch mode: the long-running engine next to an Obsidian vault (design §9.1).
//!
//! Responsibility (§9.1 deployment, guard 4 debounce and "parse by content", guard 5
//! files identified by root, the status note and the frontmatter controls, §12.1 one
//! sequential loop with `notify`, §12.2 export at most every 60 s when polling).
//!
//! Consumes: `sync` (`sync_all`, `status`, `SyncConfig`), `mdblock::parse_frontmatter`,
//! `state`, `wfclient::Api`, `notify`.
//! Consumed by: `main` (`syncflowy daemon`).
//!
//! # Loop (single thread, blocking)
//!
//! 1. [`discover`] the synced files (every `*.md` under the vault, skipping dot
//!    directories and [`is_conflict_copy`] names, whose frontmatter has `workflowy:`).
//! 2. Watch the vault with `notify`; feed changes to a [`Debouncer`] keyed by path,
//!    using content fingerprints, not mtimes (guard 4).
//! 3. A file is **ready** when its content has been stable for
//!    [`DaemonConfig::debounce`] and its frontmatter parses with the same root (guard 4).
//! 4. Every [`DaemonConfig::remote_poll`] (and whenever files are ready) run
//!    `sync::sync_all` over all discovered files whose root is not duplicated, with
//!    `SyncConfig::delete_grace = Some(DaemonConfig::delete_grace)`. Ready files carry
//!    their local edits; the others are synced for remote changes only.
//! 5. Rewrite the status note ([`render_status`]) when its content changes.
//! 6. Never exit on a per-file error; log it and show it in the status note. Exit only on
//!    a fatal error (state directory unusable, or `Api` 401).

#![allow(unused_variables, dead_code)] // stubs; remove when implemented

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::error::Error;
use crate::mdblock::RootRef;
use crate::state::Fingerprint;
use crate::sync::SyncConfig;
use crate::wfclient::Api;

mod status;

pub use status::{
    ConflictLine, Event, EventKind, PendingLine, RootLine, RootState, StatusReport, format_time,
    render_status,
};

/// The default status note's file name (engine-owned, overwritten every cycle).
pub const STATUS_NOTE: &str = "Workflowy Sync Status.md";

/// Daemon options.
#[derive(Debug, Clone)]
pub struct DaemonConfig {
    /// The vault directory to watch.
    pub vault: PathBuf,
    /// Sync options (its `delete_grace` is overridden by `delete_grace` below).
    pub sync: SyncConfig,
    /// Quiet period before a changed file is synced (§9.1 guard 4: 30–60 s). Default 45 s.
    pub debounce: Duration,
    /// Interval between remote polls (§12.2: at most one export per 60 s). Default 60 s.
    pub remote_poll: Duration,
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
            delete_grace: Duration::from_secs(600),
            status_note: PathBuf::from(STATUS_NOTE),
        }
    }
}

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

/// Scan `vault` for synced files (module docs, step 1).
pub fn discover(vault: &Path) -> Discovery {
    todo!()
}

/// True for names that look like sync conflict copies (Obsidian Sync and common
/// tools: `name (conflicted copy …).md`, `name (Conflict …).md`, `name.sync-conflict-….md`,
/// `name (1).md`-style duplicates are **not** included). UNVERIFIED: Obsidian Sync's
/// exact naming (§13 Q7).
pub fn is_conflict_copy(path: &Path) -> bool {
    todo!()
}

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
        todo!()
    }

    /// Paths whose fingerprint has not changed for at least `quiet`, removing them from
    /// the debouncer.
    pub fn take_ready(&mut self, now: Instant, quiet: Duration) -> Vec<PathBuf> {
        todo!()
    }
}

/// Run the daemon loop until a fatal error (module docs).
pub fn run(api: &dyn Api, cfg: &DaemonConfig) -> Result<(), Error> {
    todo!()
}
