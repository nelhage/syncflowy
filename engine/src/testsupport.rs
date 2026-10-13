//! Test support: run the Python mock Workflowy server (the test oracle, design §12.1)
//! from Rust tests.
//!
//! [`MockServer::start`] spawns `uv run syncflowy-mock --db <temp file> --port <free
//! port> --id-seed 1` with the repository root (`CARGO_MANIFEST_DIR/..`) as working
//! directory, waits until `GET /api/v1/nodes` answers (up to 120 s: the first `uv run`
//! may build the virtualenv), and kills the server's whole process group on drop.
//! docs/mock-server.md documents the server, including the `/_mock/*` admin routes and
//! the `/_mock/ui/*` routes that simulate web-UI edits.
//!
//! Each `MockServer` is an independent process with its own database, so tests may run
//! in parallel; for speed, share one server per test binary and call
//! [`MockServer::reset`] between tests. Ids are seeded (`--id-seed 1`): after a reset the
//! same sequence of creates yields the same ids.
//!
//! Usage:
//!
//! ```no_run
//! use syncflowy::testsupport::MockServer;
//! let mock = MockServer::start();
//! let (status, body) = mock.post("/api/v1/nodes", &serde_json::json!({"parent_id": "None", "name": "x"}));
//! assert_eq!(status, 200);
//! mock.reset();
//! ```

#![allow(
    unused_variables,
    unused_imports,
    dead_code,
    clippy::diverging_sub_expression,
    clippy::ptr_arg
)] // stubs; remove when implemented

use std::fs::File;
use std::io::Read;
use std::net::TcpListener;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::wfclient::ClientConfig;

/// The API key the mock accepts by default.
pub const MOCK_API_KEY: &str = "test-key";

/// How long [`MockServer::start`] waits for the server to answer.
pub const STARTUP_TIMEOUT: Duration = Duration::from_secs(120);

/// A running mock server (killed on drop).
#[derive(Debug)]
pub struct MockServer {
    child: Child,
    base_url: String,
    dir: PathBuf,
    agent: ureq::Agent,
}

/// The repository root (the parent of the crate directory).
pub fn repo_root() -> PathBuf {
    todo!()
}

fn free_port() -> u16 {
    todo!()
}

fn temp_dir() -> PathBuf {
    todo!()
}

fn read_file(path: &Path) -> String {
    todo!()
}

impl MockServer {
    /// Start a fresh mock server; panics (with the server's stderr) if it does not come
    /// up within [`STARTUP_TIMEOUT`].
    pub fn start() -> MockServer {
        todo!()
    }

    fn wait_ready(&mut self) {
        todo!()
    }

    /// `http://127.0.0.1:<port>`.
    pub fn base_url(&self) -> &str {
        todo!()
    }

    /// The accepted API key (`test-key`).
    pub fn api_key(&self) -> &str {
        todo!()
    }

    /// A client configuration pointing at this server, with no rate limiting and short
    /// backoffs (the mock never needs them).
    pub fn client_config(&self) -> ClientConfig {
        todo!()
    }

    fn send(&self, method: &str, path: &str, body: Option<&Value>, auth: bool) -> (u16, Value) {
        todo!()
    }

    /// Authenticated `GET <path>` (e.g. `/api/v1/nodes-export`); returns status and the
    /// JSON body (or the body text as a JSON string if it is not JSON).
    pub fn get(&self, path: &str) -> (u16, Value) {
        todo!()
    }

    /// Authenticated `POST <path>` with a JSON body.
    pub fn post(&self, path: &str, body: &Value) -> (u16, Value) {
        todo!()
    }

    /// Authenticated `DELETE <path>`.
    pub fn delete(&self, path: &str) -> (u16, Value) {
        todo!()
    }

    /// `POST /_mock/reset`: empty the outline (config and clock unchanged; the seeded id
    /// sequence restarts). Panics on failure.
    pub fn reset(&self) {
        todo!()
    }

    /// `POST /_mock/reset {"all": true}`: also restore the startup config and clock.
    pub fn reset_all(&self) {
        todo!()
    }

    /// `POST /_mock/ui/<action>` with `body`: apply an edit with web-UI semantics
    /// (create, edit, move, copy, complete, uncomplete, mirror, set_layout, delete,
    /// restore). Panics unless the status is 200; returns the JSON response.
    pub fn ui(&self, action: &str, body: Value) -> Value {
        todo!()
    }

    /// `GET /_mock/dump`: nodes (with hidden fields), trash, config, clock.
    pub fn dump(&self) -> Value {
        todo!()
    }

    /// `POST /_mock/config` with `body` (e.g. `{"faithful_cycles": true}`).
    pub fn config(&self, body: Value) -> Value {
        todo!()
    }

    /// `POST /_mock/clock` with `body` (`{"advance": s}`, `{"now": t}` or `{"offset": s}`).
    pub fn clock(&self, body: Value) -> Value {
        todo!()
    }

    /// The server's stderr so far (for debugging failed tests).
    pub fn stderr(&self) -> String {
        todo!()
    }
}

impl Drop for MockServer {
    fn drop(&mut self) {
        todo!()
    }
}
