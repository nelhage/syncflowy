//! The `syncflowy` command line (design §12.1: one static binary).
//!
//! ```text
//! syncflowy [global options] render <ROOT> <OUT.md> [--headings N] [--force]
//! syncflowy [global options] sync <FILE.md>... [--force] [--dry-run]
//! syncflowy [global options] daemon --vault <DIR> [--debounce SECS] [--poll SECS]
//!           [--min-export-interval SECS] [--delete-grace SECS] [--status-note PATH]
//! syncflowy [global options] status [FILE.md]...
//! ```
//!
//! Global options: `--api-key` (env `WORKFLOWY_API_KEY`), `--api-url` (env
//! `WORKFLOWY_API_URL`, default `https://beta.workflowy.com`, the beta API: DM§10.5), `--state-dir` (env
//! `SYNCFLOWY_STATE_DIR`, default `$XDG_STATE_HOME/syncflowy` or
//! `~/.local/state/syncflowy`), `--request-log FILE`, `-v`/`-vv` (log level; `RUST_LOG`
//! also works).

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use clap::{Parser, Subcommand};

use syncflowy::apply;
use syncflowy::daemon::{self, DaemonConfig};
use syncflowy::mdblock::DocConfig;
use syncflowy::mdinline::ConflictField;
use syncflowy::merge::ReportEvent;
use syncflowy::sync::{self, PauseReason, RootStatus, SyncConfig, SyncOutcome, SyncReport};
use syncflowy::wfclient::{Client, ClientConfig, DEFAULT_BASE_URL};

#[derive(Debug, Parser)]
#[command(
    name = "syncflowy",
    version,
    about = "Sync Workflowy subtrees with Markdown files"
)]
struct Cli {
    /// Workflowy API key.
    #[arg(long, env = "WORKFLOWY_API_KEY", hide_env_values = true, global = true)]
    api_key: Option<String>,

    /// API base URL (default: the beta API). The production host is refused: it reports
    /// mirrors as empty items.
    #[arg(long, env = "WORKFLOWY_API_URL", default_value = DEFAULT_BASE_URL, global = true)]
    api_url: String,

    /// State directory (checkpoints, history, journals); keep it outside the vault.
    #[arg(long, env = "SYNCFLOWY_STATE_DIR", global = true)]
    state_dir: Option<PathBuf>,

    /// Append every API request to this JSONL file.
    #[arg(long, global = true)]
    request_log: Option<PathBuf>,

    /// More logging (-v info, -vv debug).
    #[arg(short, long, action = clap::ArgAction::Count, global = true)]
    verbose: u8,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Render a Workflowy subtree into a new Markdown file and start syncing it.
    Render {
        /// Sync root: short id, full UUID or https://workflowy.com/#/<short> URL.
        root: String,
        /// Output Markdown file.
        out: PathBuf,
        /// Render depth 1..N as headings.
        #[arg(long, default_value_t = 0)]
        headings: u8,
        /// Overwrite an existing file or checkpoint.
        #[arg(long)]
        force: bool,
    },
    /// Run one sync cycle over the given files.
    Sync {
        /// Synced Markdown files.
        #[arg(required = true)]
        files: Vec<PathBuf>,
        /// Override the mass-change brakes and placeholder guards.
        #[arg(long)]
        force: bool,
        /// Plan only; write nothing.
        #[arg(long)]
        dry_run: bool,
    },
    /// Watch a vault and sync continuously.
    Daemon {
        /// The vault directory (synced files are found by their `workflowy:` root).
        #[arg(long)]
        vault: PathBuf,
        /// Quiet period before syncing a changed file, in seconds.
        #[arg(long, default_value_t = 45)]
        debounce: u64,
        /// Interval of the periodic cycle that picks up remote changes, in seconds.
        #[arg(long, default_value_t = 60)]
        poll: u64,
        /// Minimum interval between two cycles (each exports), in seconds.
        #[arg(long, default_value_t = 60)]
        min_export_interval: u64,
        /// Grace period of deferred deletes, in seconds.
        #[arg(long, default_value_t = 600)]
        delete_grace: u64,
        /// The status note, relative to the vault.
        #[arg(long, default_value = daemon::STATUS_NOTE)]
        status_note: PathBuf,
    },
    /// Show the local sync state (no API calls).
    Status {
        /// Restrict to these files' roots.
        files: Vec<PathBuf>,
    },
}

fn default_state_dir() -> PathBuf {
    if let Some(x) = std::env::var_os("XDG_STATE_HOME") {
        return PathBuf::from(x).join("syncflowy");
    }
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    home.join(".local/state/syncflowy")
}

fn client(cli: &Cli) -> anyhow::Result<Client> {
    let key = cli
        .api_key
        .clone()
        .filter(|k| !k.trim().is_empty())
        .ok_or_else(|| anyhow::anyhow!("no API key: pass --api-key or set WORKFLOWY_API_KEY"))?;
    let base_url = cli.api_url.trim_end_matches('/').to_owned();
    if is_production_host(&base_url) {
        anyhow::bail!(
            "{base_url} is the production API, which reports mirrors as empty nodes; \
             syncflowy needs the beta API ({DEFAULT_BASE_URL}), which serves the same data"
        );
    }
    let mut cfg = ClientConfig::new(key);
    cfg.base_url = base_url;
    cfg.request_log = cli.request_log.clone();
    Ok(Client::new(cfg))
}

/// True for the production Workflowy host (`workflowy.com` or `www.workflowy.com`, any
/// scheme, port or path).
fn is_production_host(base_url: &str) -> bool {
    let rest = base_url.split_once("://").map_or(base_url, |(_, r)| r);
    let host = rest.split(['/', ':']).next().unwrap_or("");
    let host = host.to_ascii_lowercase();
    host == "workflowy.com" || host == "www.workflowy.com"
}

/// The CLI's remedy for an error whose message names none (the daemon words its own).
fn cli_hint(e: &syncflowy::Error) -> &'static str {
    match e {
        syncflowy::Error::NoCheckpoint(_) => "; run `syncflowy render` first or use --force",
        _ => "",
    }
}

/// One line for an outcome.
fn describe_outcome(outcome: &SyncOutcome) -> String {
    match outcome {
        SyncOutcome::Unchanged => "unchanged".to_owned(),
        SyncOutcome::Synced {
            pushed,
            wrote_file,
            generation,
        } => format!(
            "synced: {} created, {} updated, {} moved, {} reordered, {} completed, {} deleted \
             (+{} hidden); {} (gen {generation})",
            pushed.creates,
            pushed.updates,
            pushed.moves,
            pushed.reorders,
            pushed.completes,
            pushed.deletes,
            pushed.hidden_deleted,
            if *wrote_file {
                "file rewritten"
            } else {
                "file unchanged"
            },
        ),
        SyncOutcome::Rendered { generation } => format!("rendered (gen {generation})"),
        SyncOutcome::DryRun { plan } => {
            let mut s = format!("dry run: {} op(s)", plan.ops.len());
            for line in apply::plan_lines(plan) {
                s.push_str(&format!("\n  {line}"));
            }
            s
        }
        SyncOutcome::Paused(PauseReason::Frontmatter) => {
            "paused (paused: true in the frontmatter)".to_owned()
        }
        SyncOutcome::Paused(PauseReason::FileMissing) => {
            "paused: the file is missing; nothing was changed in Workflowy".to_owned()
        }
        SyncOutcome::Paused(PauseReason::DuplicateRoot(paths)) => format!(
            "paused: several files claim this root: {}",
            paths
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        ),
        SyncOutcome::Braked(b) => format!("refused: {b} (use --force)"),
        SyncOutcome::Skipped { reason } => format!("skipped: {reason}"),
    }
}

/// Print a report; false if the outcome needs the user's attention.
fn print_report(report: &SyncReport) -> bool {
    println!(
        "{}: {}",
        report.file.display(),
        describe_outcome(&report.outcome)
    );
    for w in &report.warnings {
        println!("  warning: {w}");
    }
    for m in &report.messages {
        println!("  {m}");
    }
    // A conflict recorded this run is already among the messages, by name; the
    // others are markers from earlier runs, still unresolved.
    for c in &report.conflicts {
        let new = report.events.iter().any(|e| {
            matches!(e, ReportEvent::Conflict { node, field } if *node == c.node && *field == c.field)
        });
        if !new {
            let field = match c.field {
                ConflictField::Name => "name",
                ConflictField::Note => "note",
            };
            let node = c
                .node
                .short()
                .map_or_else(|| c.node.to_string(), |s| s.to_string());
            println!(
                "  unresolved conflict on the {field} of {node}: resolve the marker in the file"
            );
        }
    }
    for d in &report.discrepancies {
        println!("  verification: {d}");
    }
    !matches!(
        report.outcome,
        SyncOutcome::Braked(_)
            | SyncOutcome::Skipped { .. }
            | SyncOutcome::Paused(PauseReason::FileMissing | PauseReason::DuplicateRoot(_))
    )
}

fn run(cli: Cli) -> anyhow::Result<bool> {
    let state_dir = cli.state_dir.clone().unwrap_or_else(default_state_dir);
    let mut cfg = SyncConfig::new(state_dir);
    match &cli.command {
        Command::Render {
            root,
            out,
            headings,
            force,
        } => {
            let api = client(&cli)?;
            cfg.force = *force;
            let doc = DocConfig {
                headings: *headings,
                ..DocConfig::default()
            };
            let report = sync::render_once(&api, root, out, &doc, &cfg)?;
            Ok(print_report(&report))
        }
        Command::Sync {
            files,
            force,
            dry_run,
        } => {
            let api = client(&cli)?;
            cfg.force = *force;
            cfg.dry_run = *dry_run;
            let results = sync::sync_all(&api, files, &cfg)?;
            let mut ok = true;
            for r in results {
                match r.result {
                    Ok(report) => ok &= print_report(&report),
                    Err(e) => {
                        ok = false;
                        eprintln!("{}: error: {e}{}", r.file.display(), cli_hint(&e));
                    }
                }
            }
            Ok(ok)
        }
        Command::Daemon {
            vault,
            debounce,
            poll,
            min_export_interval,
            delete_grace,
            status_note,
        } => {
            let api = client(&cli)?;
            let mut dcfg = DaemonConfig::new(vault.clone(), cfg);
            dcfg.debounce = Duration::from_secs(*debounce);
            dcfg.remote_poll = Duration::from_secs(*poll);
            dcfg.min_export_interval = Duration::from_secs(*min_export_interval);
            dcfg.delete_grace = Duration::from_secs(*delete_grace);
            dcfg.status_note = status_note.clone();
            daemon::run(&api, &dcfg)?;
            Ok(true)
        }
        Command::Status { files } => {
            if files.is_empty() {
                let roots = sync::status(&cfg)?;
                if roots.is_empty() {
                    println!("no synced roots in {}", cfg.state_dir.display());
                }
                roots.iter().for_each(print_root_status);
                return Ok(true);
            }
            let mut ok = true;
            for f in sync::status_of_files(&cfg, files)? {
                match &f.status {
                    Ok(s) => print_root_status(s),
                    Err(problem) => {
                        ok = false;
                        println!("{}: {problem}", f.file.display());
                    }
                }
            }
            Ok(ok)
        }
    }
}

fn print_root_status(s: &RootStatus) {
    println!(
        "{}: gen {}, last synced at {} (unix), file {}",
        s.root,
        s.generation,
        s.synced_at,
        s.file
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "-".into()),
    );
    match s.locally_modified {
        Some(true) => println!("  the file has local changes not yet synced"),
        Some(false) => println!("  the file is as last written"),
        None => println!("  the file cannot be read"),
    }
    if s.journal_pending {
        println!("  an unfinished cycle was interrupted: the next sync recovers it");
    }
    for (node, label, first_seen) in &s.pending_deletes {
        println!("  pending delete: {label:?} ({node}), first seen at {first_seen}");
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let level = match cli.verbose {
        0 => "warn",
        1 => "info",
        _ => "debug",
    };
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or(level)).init();
    match run(cli) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(1),
        Err(e) => {
            // Every message already names its cause (`Error`'s variants print their
            // source), so the chain is not appended again.
            eprintln!("error: {e}");
            ExitCode::from(2)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn production_host_is_recognised() {
        for url in [
            "https://workflowy.com",
            "https://WorkFlowy.com/",
            "http://www.workflowy.com:443/api",
            "workflowy.com",
        ] {
            assert!(is_production_host(url), "{url}");
        }
        for url in [
            DEFAULT_BASE_URL,
            "http://127.0.0.1:8080",
            "https://notworkflowy.com",
        ] {
            assert!(!is_production_host(url), "{url}");
        }
    }
}
