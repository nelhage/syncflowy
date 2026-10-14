//! The engine-level error type, aggregating the module errors.
//!
//! Each module keeps its own error type next to the code that produces it
//! (`wfclient::ApiError`, `mdblock::ParseError`, `merge::MergeError`, `plan::PlanError`,
//! `apply::ApplyError`, `state::StateError`, `tree::TreeError`). The I/O shell (`sync`,
//! `daemon`, `main`) returns [`enum@Error`], which wraps them (`?` converts) and adds the
//! run-level failures. Brakes are not errors: `sync` reports them as
//! `SyncOutcome::Braked`.

use std::path::PathBuf;

use thiserror::Error;

use crate::apply::ApplyError;
use crate::mdblock::{ParseError, ViewError};
use crate::merge::MergeError;
use crate::plan::PlanError;
use crate::state::StateError;
use crate::tree::{NodeId, TreeError};
use crate::wfclient::ApiError;

/// Any engine failure.
#[derive(Debug, Error)]
pub enum Error {
    /// API failure.
    #[error("API: {0}")]
    Api(#[from] ApiError),
    /// The Markdown file's frontmatter does not parse.
    #[error("parse: {0}")]
    Parse(#[from] ParseError),
    /// Building a view failed.
    #[error("view: {0}")]
    View(#[from] ViewError),
    /// The merge failed.
    #[error("merge: {0}")]
    Merge(#[from] MergeError),
    /// Planning failed (brakes are reported as outcomes, not through this).
    #[error("plan: {0}")]
    Plan(#[from] PlanError),
    /// Applying failed; the journal is kept for recovery.
    #[error("apply: {0}")]
    Apply(#[from] ApplyError),
    /// State store failure.
    #[error("state: {0}")]
    State(#[from] StateError),
    /// Tree construction failed (for example duplicate ids in an export).
    #[error("tree: {0}")]
    Tree(#[from] TreeError),
    /// File I/O on a Markdown file.
    #[error("{path}: {source}")]
    Io {
        /// The file.
        path: PathBuf,
        /// The error.
        source: std::io::Error,
    },
    /// The file's root is not in the export.
    #[error("sync root {0:?} not found")]
    RootNotFound(String),
    /// The root has no checkpoint (run `syncflowy render` first, or `--force`).
    #[error("no checkpoint for root {0}; run `syncflowy render` first or use --force")]
    NoCheckpoint(NodeId),
    /// The file names a different root than its checkpoint.
    #[error("{file}: root changed from {expected} to {found}")]
    RootMismatch {
        /// The file.
        file: PathBuf,
        /// Root in the checkpoint.
        expected: NodeId,
        /// Root in the file.
        found: String,
    },
    /// A sync root lies inside another sync root of the run (§6.4).
    #[error("sync root {inner} is inside sync root {outer}")]
    NestedRoots {
        /// The outer root.
        outer: NodeId,
        /// The inner root.
        inner: NodeId,
    },
    /// `render` was asked for a root that already has a checkpoint (use `--force`).
    #[error("root {0} already has a checkpoint; sync its file instead, or use --force")]
    CheckpointExists(NodeId),
    /// The output file of `render` exists (use `--force`).
    #[error("{0} exists (use --force)")]
    FileExists(PathBuf),
}

/// `Result` with [`enum@Error`].
pub type Result<T, E = Error> = std::result::Result<T, E>;
