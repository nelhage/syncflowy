//! The `syncflowy` command line (design §12.1: one static binary).
//!
//! ```text
//! syncflowy [global options] render <ROOT> <OUT.md> [--headings N] [--force]
//! syncflowy [global options] sync <FILE.md>... [--force] [--dry-run]
//! syncflowy [global options] daemon <VAULT> [--debounce SECS] [--poll SECS] [--delete-grace SECS]
//! syncflowy [global options] status [FILE.md]...
//! ```
//!
//! Global options: `--api-key` (env `WORKFLOWY_API_KEY`), `--api-url` (env
//! `WORKFLOWY_API_URL`, default `https://workflowy.com`), `--state-dir` (env
//! `SYNCFLOWY_STATE_DIR`, default `$XDG_STATE_HOME/syncflowy` or
//! `~/.local/state/syncflowy`), `--request-log FILE`, `-v`/`-vv` (log level; `RUST_LOG`
//! also works).

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use clap::{Parser, Subcommand};

use syncflowy::daemon::{self, DaemonConfig};
use syncflowy::mdblock::DocConfig;
use syncflowy::sync::{self, SyncConfig, SyncOutcome};
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

    /// API base URL (the mock server for tests).
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
        /// The vault directory.
        vault: PathBuf,
        /// Quiet period before syncing a changed file, in seconds.
        #[arg(long, default_value_t = 45)]
        debounce: u64,
        /// Remote poll interval, in seconds.
        #[arg(long, default_value_t = 60)]
        poll: u64,
        /// Grace period of deferred deletes, in seconds.
        #[arg(long, default_value_t = 600)]
        delete_grace: u64,
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
        .ok_or_else(|| anyhow::anyhow!("no API key: pass --api-key or set WORKFLOWY_API_KEY"))?;
    let mut cfg = ClientConfig::new(key);
    cfg.base_url = cli.api_url.trim_end_matches('/').to_owned();
    cfg.request_log = cli.request_log.clone();
    Ok(Client::new(cfg))
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
            println!("{}: {:?}", report.file.display(), report.outcome);
            Ok(true)
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
                    Ok(report) => {
                        if matches!(
                            report.outcome,
                            SyncOutcome::Braked(_) | SyncOutcome::Skipped { .. }
                        ) {
                            ok = false;
                        }
                        println!("{}: {:?}", r.file.display(), report.outcome);
                        for w in &report.warnings {
                            println!("  warning: {w}");
                        }
                    }
                    Err(e) => {
                        ok = false;
                        eprintln!("{}: error: {e}", r.file.display());
                    }
                }
            }
            Ok(ok)
        }
        Command::Daemon {
            vault,
            debounce,
            poll,
            delete_grace,
        } => {
            let api = client(&cli)?;
            let mut dcfg = DaemonConfig::new(vault.clone(), cfg);
            dcfg.debounce = Duration::from_secs(*debounce);
            dcfg.remote_poll = Duration::from_secs(*poll);
            dcfg.delete_grace = Duration::from_secs(*delete_grace);
            daemon::run(&api, &dcfg)?;
            Ok(true)
        }
        Command::Status { files } => {
            for s in sync::status(&cfg, files)? {
                println!(
                    "{} gen {} synced_at {} file {} pending deletes {}{}",
                    s.root,
                    s.generation,
                    s.synced_at,
                    s.file
                        .as_ref()
                        .map(|p| p.display().to_string())
                        .unwrap_or_else(|| "-".into()),
                    s.pending_deletes.len(),
                    if s.journal_pending {
                        " (unfinished cycle: next run recovers)"
                    } else {
                        ""
                    },
                );
            }
            Ok(true)
        }
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
            eprintln!("error: {e:#}");
            ExitCode::from(2)
        }
    }
}
