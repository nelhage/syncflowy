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
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crate has a parent directory")
        .to_path_buf()
}

fn free_port() -> u16 {
    let l = TcpListener::bind("127.0.0.1:0").expect("bind an ephemeral port");
    l.local_addr().expect("local addr").port()
}

fn temp_dir() -> PathBuf {
    let n: u64 = rand::random();
    let dir = std::env::temp_dir().join(format!("syncflowy-mock-{}-{n:016x}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create mock temp dir");
    dir
}

fn read_file(path: &Path) -> String {
    let mut s = String::new();
    if let Ok(mut f) = File::open(path) {
        let _ = f.read_to_string(&mut s);
    }
    s
}

impl MockServer {
    /// Start a fresh mock server; panics (with the server's stderr) if it does not come
    /// up within [`STARTUP_TIMEOUT`].
    pub fn start() -> MockServer {
        let dir = temp_dir();
        let port = free_port();
        let db = dir.join("mock.sqlite3");
        let stdout = File::create(dir.join("stdout.log")).expect("create stdout log");
        let stderr = File::create(dir.join("stderr.log")).expect("create stderr log");
        let mut cmd = Command::new("uv");
        cmd.args(["run", "--quiet", "syncflowy-mock", "--db"])
            .arg(&db)
            .args([
                "--port",
                &port.to_string(),
                "--id-seed",
                "1",
                "--log-level",
                "warning",
            ])
            .current_dir(repo_root())
            .stdin(Stdio::null())
            .stdout(stdout)
            .stderr(stderr)
            // Own process group, so Drop can stop uv *and* the Python child it spawns.
            .process_group(0);
        let child = cmd
            .spawn()
            .unwrap_or_else(|e| panic!("cannot spawn `uv run syncflowy-mock`: {e}"));
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .proxy(None)
            .timeout_global(Some(Duration::from_secs(60)))
            .build()
            .into();
        let mut server = MockServer {
            child,
            base_url: format!("http://127.0.0.1:{port}"),
            dir,
            agent,
        };
        server.wait_ready();
        server
    }

    fn wait_ready(&mut self) {
        let deadline = Instant::now() + STARTUP_TIMEOUT;
        loop {
            if let Ok(Some(status)) = self.child.try_wait() {
                panic!(
                    "mock server exited early ({status}); stderr:\n{}",
                    read_file(&self.dir.join("stderr.log"))
                );
            }
            let probe = self
                .agent
                .get(format!("{}/api/v1/nodes", self.base_url))
                .header("Authorization", format!("Bearer {MOCK_API_KEY}"))
                .config()
                .timeout_global(Some(Duration::from_secs(2)))
                .build()
                .call();
            if let Ok(resp) = probe
                && resp.status().as_u16() == 200
            {
                return;
            }
            if Instant::now() > deadline {
                panic!(
                    "mock server did not answer within {STARTUP_TIMEOUT:?}; stderr:\n{}",
                    read_file(&self.dir.join("stderr.log"))
                );
            }
            std::thread::sleep(Duration::from_millis(200));
        }
    }

    /// `http://127.0.0.1:<port>`.
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// The accepted API key (`test-key`).
    pub fn api_key(&self) -> &str {
        MOCK_API_KEY
    }

    /// A client configuration pointing at this server, with no rate limiting and short
    /// backoffs (the mock never needs them).
    pub fn client_config(&self) -> ClientConfig {
        let mut cfg = ClientConfig::new(MOCK_API_KEY);
        cfg.base_url = self.base_url.clone();
        cfg.min_interval = Duration::ZERO;
        cfg.backoff = Duration::from_millis(10);
        cfg
    }

    fn send(&self, method: &str, path: &str, body: Option<&Value>, auth: bool) -> (u16, Value) {
        let url = format!("{}{}", self.base_url, path);
        let auth_header = format!("Bearer {MOCK_API_KEY}");
        let result = match (method, body) {
            ("GET", _) => {
                let mut r = self.agent.get(&url);
                if auth {
                    r = r.header("Authorization", &auth_header);
                }
                r.call()
            }
            ("DELETE", _) => {
                let mut r = self.agent.delete(&url);
                if auth {
                    r = r.header("Authorization", &auth_header);
                }
                r.call()
            }
            ("POST", body) => {
                let mut r = self.agent.post(&url);
                if auth {
                    r = r.header("Authorization", &auth_header);
                }
                match body {
                    Some(b) => r.send_json(b),
                    None => r.send_empty(),
                }
            }
            _ => panic!("unsupported method {method}"),
        };
        let mut resp = result.unwrap_or_else(|e| panic!("{method} {path}: {e}"));
        let status = resp.status().as_u16();
        let text = resp.body_mut().read_to_string().unwrap_or_default();
        let value = serde_json::from_str(&text).unwrap_or(Value::String(text));
        (status, value)
    }

    /// Authenticated `GET <path>` (e.g. `/api/v1/nodes-export`); returns status and the
    /// JSON body (or the body text as a JSON string if it is not JSON).
    pub fn get(&self, path: &str) -> (u16, Value) {
        self.send("GET", path, None, true)
    }

    /// Authenticated `POST <path>` with a JSON body.
    pub fn post(&self, path: &str, body: &Value) -> (u16, Value) {
        self.send("POST", path, Some(body), true)
    }

    /// Authenticated `DELETE <path>`.
    pub fn delete(&self, path: &str) -> (u16, Value) {
        self.send("DELETE", path, None, true)
    }

    /// `POST /_mock/reset`: empty the outline (config and clock unchanged; the seeded id
    /// sequence restarts). Panics on failure.
    pub fn reset(&self) {
        let (status, body) = self.send("POST", "/_mock/reset", None, false);
        assert_eq!(status, 200, "reset failed: {body}");
    }

    /// `POST /_mock/reset {"all": true}`: also restore the startup config and clock.
    pub fn reset_all(&self) {
        let (status, body) = self.send(
            "POST",
            "/_mock/reset",
            Some(&serde_json::json!({"all": true})),
            false,
        );
        assert_eq!(status, 200, "reset failed: {body}");
    }

    /// `POST /_mock/ui/<action>` with `body`: apply an edit with web-UI semantics
    /// (create, edit, move, copy, complete, uncomplete, mirror, set_layout, delete,
    /// restore). Panics unless the status is 200; returns the JSON response.
    pub fn ui(&self, action: &str, body: Value) -> Value {
        let (status, resp) = self.send("POST", &format!("/_mock/ui/{action}"), Some(&body), false);
        assert_eq!(status, 200, "/_mock/ui/{action} {body} failed: {resp}");
        resp
    }

    /// `GET /_mock/dump`: nodes (with hidden fields), trash, config, clock.
    pub fn dump(&self) -> Value {
        let (status, resp) = self.send("GET", "/_mock/dump", None, false);
        assert_eq!(status, 200, "dump failed: {resp}");
        resp
    }

    /// `POST /_mock/config` with `body` (e.g. `{"faithful_cycles": true}`).
    pub fn config(&self, body: Value) -> Value {
        let (status, resp) = self.send("POST", "/_mock/config", Some(&body), false);
        assert_eq!(status, 200, "config failed: {resp}");
        resp
    }

    /// `POST /_mock/clock` with `body` (`{"advance": s}`, `{"now": t}` or `{"offset": s}`).
    pub fn clock(&self, body: Value) -> Value {
        let (status, resp) = self.send("POST", "/_mock/clock", Some(&body), false);
        assert_eq!(status, 200, "clock failed: {resp}");
        resp
    }

    /// The server's stderr so far (for debugging failed tests).
    pub fn stderr(&self) -> String {
        read_file(&self.dir.join("stderr.log"))
    }
}

impl Drop for MockServer {
    fn drop(&mut self) {
        let group = format!("-{}", self.child.id());
        let kill = |signal: &str| {
            let _ = Command::new("kill")
                .args([signal, "--", &group])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        };
        // SIGTERM the whole process group (uv and the Python server it spawned), wait
        // briefly, then SIGKILL whatever is left.
        kill("-TERM");
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut exited = false;
        while Instant::now() < deadline {
            if let Ok(Some(_)) = self.child.try_wait() {
                exited = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        if !exited {
            kill("-KILL");
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}
