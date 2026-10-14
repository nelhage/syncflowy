//! The status note (§9.1 "control and status through the vault"): what it shows and
//! its Markdown rendering. Pure: the daemon builds a [`StatusReport`] and writes the
//! text.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::Discovery;
use crate::mdinline::ConflictField;
use crate::sync::PauseReason;

/// What kind of entry an [`Event`] is (the status note groups them).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EventKind {
    /// A file synced in safe mode (§9.1 guard 1).
    SafeMode,
    /// A merge artefact dropped from a file (guard 3, structure).
    ResurrectionDropped,
    /// A stale value not pushed (guard 3, fields).
    StaleEcho,
    /// An API failure (the export, or an op).
    ApiError,
    /// Any other failure of a file's cycle.
    Error,
    /// A brake engaged (§8.3).
    Brake,
    /// Something else worth knowing (conflicts, deletes sent, cancelled deletes, …).
    Info,
}

/// One logged event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Event {
    /// Unix seconds.
    pub at: i64,
    /// The file, if the event concerns one.
    pub file: Option<PathBuf>,
    /// The kind.
    pub kind: EventKind,
    /// Human-readable text.
    pub text: String,
}

/// The state of one root, as the status note shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RootState {
    /// Synced (or nothing to do) at the last cycle.
    Ok,
    /// The file has local edits waiting for the debounce period.
    Waiting,
    /// Not synced (guard 5 or `paused: true`).
    Paused(PauseReason),
    /// A brake engaged (§8.3): set `force: true` in the frontmatter to override once.
    Braked(String),
    /// The last cycle skipped the file (it is retried).
    Skipped(String),
    /// The last cycle failed for this file.
    Failed(String),
    /// The file names a root without a checkpoint (`syncflowy render`, or `force: true`).
    NoCheckpoint,
    /// Not synced yet by this daemon.
    Unknown,
}

/// One line of the roots table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RootLine {
    /// The root's short id (as written in the file, or of the checkpoint).
    pub short: String,
    /// The root's name (plain text), if known.
    pub name: Option<String>,
    /// The file(s) naming it.
    pub files: Vec<PathBuf>,
    /// Checkpoint generation.
    pub generation: Option<u64>,
    /// Unix seconds of the last successful cycle (unchanged included).
    pub last_sync: Option<i64>,
    /// State.
    pub state: RootState,
}

/// A conflict marker in a file (§6.1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConflictLine {
    /// The file.
    pub file: PathBuf,
    /// 1-based line of the marker.
    pub line: usize,
    /// The item it belongs to (its Markdown name; empty for the root note).
    pub item: String,
    /// Which field.
    pub field: ConflictField,
}

/// A pending delete (§9.1 guard 2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingLine {
    /// The file of its root.
    pub file: Option<PathBuf>,
    /// The node's label.
    pub label: String,
    /// Hidden descendants deleted with it.
    pub hidden: usize,
    /// When it was first seen (unix seconds).
    pub first_seen: i64,
    /// When the grace period ends (unix seconds); it is sent at the first cycle after
    /// that with the file unchanged.
    pub fires_at: i64,
}

/// What the status note shows (§9.1 "control and status through the vault").
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusReport {
    /// Unix seconds of the last cycle.
    pub at: i64,
    /// The vault (file paths are shown relative to it).
    pub vault: PathBuf,
    /// Per-root state.
    pub roots: Vec<RootLine>,
    /// Conflict markers in the files.
    pub conflicts: Vec<ConflictLine>,
    /// Pending deletes.
    pub pending: Vec<PendingLine>,
    /// True if pending deletes are held because a file has edits waiting.
    pub deletes_held: bool,
    /// Current per-file errors (until the file syncs again).
    pub errors: Vec<(PathBuf, String)>,
    /// Discovery problems (duplicates, skipped files).
    pub discovery: Discovery,
    /// Recent events, newest first.
    pub events: Vec<Event>,
}

/// `2026-10-14 12:00 UTC` for unix seconds.
pub fn format_time(unix: i64) -> String {
    let days = unix.div_euclid(86_400);
    let secs = unix.rem_euclid(86_400);
    // Howard Hinnant's civil_from_days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02} UTC",
        secs / 3600,
        (secs % 3600) / 60
    )
}

/// A file as an Obsidian link, relative to the vault.
pub(super) fn file_link(vault: &Path, path: &Path) -> String {
    let rel = path.strip_prefix(vault).unwrap_or(path);
    let s = rel
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/");
    let s = s.strip_suffix(".md").unwrap_or(&s);
    format!("[[{s}]]")
}

/// Text safe inside a table cell or a list item (no line breaks, no pipes).
fn cell(s: &str) -> String {
    s.replace(['\n', '\r'], " ").replace('|', "\\|")
}

fn describe_pause(reason: &PauseReason) -> &'static str {
    match reason {
        PauseReason::Frontmatter => "paused (`paused: true`)",
        PauseReason::FileMissing => "paused: no file names this root; nothing is deleted",
        PauseReason::DuplicateRoot(_) => "paused: several files name this root",
    }
}

fn describe_state(s: &RootState) -> String {
    match s {
        RootState::Ok => "ok".to_owned(),
        RootState::Waiting => "local edits waiting (debounce)".to_owned(),
        RootState::Paused(r) => describe_pause(r).to_owned(),
        RootState::Braked(b) => {
            format!("refused by a brake: {b}; set `force: true` to override once")
        }
        RootState::Skipped(r) => format!("skipped: {r}"),
        RootState::Failed(e) => format!("error: {e}"),
        RootState::NoCheckpoint => {
            "not initialised: run `syncflowy render`, or set `force: true` to adopt the file"
                .to_owned()
        }
        RootState::Unknown => "not synced yet".to_owned(),
    }
}

fn event_section(out: &mut String, title: &str, report: &StatusReport, kinds: &[EventKind]) {
    out.push_str(&format!("\n## {title}\n\n"));
    let events: Vec<&Event> = report
        .events
        .iter()
        .filter(|e| kinds.contains(&e.kind))
        .collect();
    if events.is_empty() {
        out.push_str("None.\n");
    }
    for e in events {
        let file = e
            .file
            .as_ref()
            .map(|f| format!("{}: ", file_link(&report.vault, f)))
            .unwrap_or_default();
        out.push_str(&format!(
            "- {} {file}{}\n",
            format_time(e.at),
            cell(&e.text)
        ));
    }
}

/// Render the status note as Markdown (no frontmatter, so it is never mistaken for a
/// synced file): last successful sync per root, conflicts awaiting resolution, pending
/// deletes and when they fire, safe-mode episodes, dropped resurrections, API errors.
pub fn render_status(report: &StatusReport) -> String {
    let link = |p: &Path| file_link(&report.vault, p);
    let mut out = String::new();
    out.push_str("# Workflowy sync status\n\n");
    out.push_str(
        "This note is written by syncflowy after every cycle; edits to it are overwritten.\n\n",
    );
    out.push_str(&format!("Last cycle: {}\n", format_time(report.at)));

    out.push_str("\n## Roots\n\n");
    if report.roots.is_empty() {
        out.push_str("No synced files.\n");
    } else {
        out.push_str("| File | Root | Last sync | Gen | State |\n|---|---|---|---|---|\n");
        for r in &report.roots {
            let files = if r.files.is_empty() {
                "(none)".to_owned()
            } else {
                r.files
                    .iter()
                    .map(|f| link(f))
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            let root = match &r.name {
                Some(n) => format!("{} `{}`", cell(n), r.short),
                None => format!("`{}`", r.short),
            };
            out.push_str(&format!(
                "| {} | {} | {} | {} | {} |\n",
                cell(&files),
                root,
                r.last_sync.map_or_else(|| "never".to_owned(), format_time),
                r.generation
                    .map_or_else(|| "-".to_owned(), |g| g.to_string()),
                cell(&describe_state(&r.state)),
            ));
        }
    }

    out.push_str("\n## Conflicts awaiting resolution\n\n");
    if report.conflicts.is_empty() {
        out.push_str("None.\n");
    }
    for c in &report.conflicts {
        let field = match c.field {
            ConflictField::Name => "name",
            ConflictField::Note => "note",
        };
        let what = if c.item.is_empty() {
            format!("the {field} of the root")
        } else {
            format!("the {field} of \"{}\"", cell(&c.item))
        };
        out.push_str(&format!(
            "- {} line {}: {what}. Workflowy's version is in the file; edit or delete the marker to resolve.\n",
            link(&c.file),
            c.line
        ));
    }

    out.push_str("\n## Pending deletes\n\n");
    if report.pending.is_empty() {
        out.push_str("None.\n");
    } else {
        out.push_str(
            "Removed from a file but not yet deleted in Workflowy (§9.1 guard 2). Put the line \
             back to cancel; paste its tag into another synced file to move it there.\n\n",
        );
        if report.deletes_held {
            out.push_str("Held: another synced file has edits waiting to be synced.\n\n");
        }
    }
    for p in &report.pending {
        let file = p
            .file
            .as_ref()
            .map(|f| link(f))
            .unwrap_or_else(|| "?".into());
        let hidden = if p.hidden > 0 {
            format!(" (with {} hidden item(s))", p.hidden)
        } else {
            String::new()
        };
        out.push_str(&format!(
            "- {file}: \"{}\"{hidden}, removed {}, deleted at the first cycle after {} with the file unchanged\n",
            cell(&p.label),
            format_time(p.first_seen),
            format_time(p.fires_at)
        ));
    }

    out.push_str("\n## Problems\n\n");
    let mut problems = 0;
    for (short, paths) in &report.discovery.duplicates {
        problems += 1;
        let files: Vec<String> = paths.iter().map(|p| link(p)).collect();
        out.push_str(&format!(
            "- {} all name root `{short}`: that root is paused until only one of them does.\n",
            files.join(", ")
        ));
    }
    for (path, reason) in &report.discovery.skipped {
        problems += 1;
        out.push_str(&format!("- {}: {}\n", link(path), cell(reason)));
    }
    for (path, error) in &report.errors {
        problems += 1;
        let file = if path.as_os_str().is_empty() {
            "All files".to_owned()
        } else {
            link(path)
        };
        out.push_str(&format!("- {file}: {}\n", cell(error)));
    }
    if problems == 0 {
        out.push_str("None.\n");
    }

    event_section(
        &mut out,
        "Safe-mode episodes",
        report,
        &[EventKind::SafeMode],
    );
    event_section(
        &mut out,
        "Dropped resurrections and stale echoes",
        report,
        &[EventKind::ResurrectionDropped, EventKind::StaleEcho],
    );
    event_section(&mut out, "API errors", report, &[EventKind::ApiError]);
    event_section(
        &mut out,
        "Other events",
        report,
        &[EventKind::Error, EventKind::Brake, EventKind::Info],
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times() {
        assert_eq!(format_time(0), "1970-01-01 00:00 UTC");
        assert_eq!(format_time(1_790_000_000), "2026-09-21 14:13 UTC");
        assert_eq!(format_time(951_782_400), "2000-02-29 00:00 UTC");
    }
}
