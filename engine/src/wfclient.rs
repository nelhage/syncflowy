//! Blocking Workflowy API client.
//!
//! Responsibility (design §3 "wfclient", §2 constraints, §12.1 "synchronous by design"):
//! a thin, blocking client over `ureq` 3 that ports the behaviour of
//! `src/syncflowy/prod_client.py` **minus its scope guard**: a rate limit, retries with
//! backoff, full-UUID enforcement for mutations, typed errors, and an optional JSONL
//! request log. All I/O to Workflowy goes through the [`Api`] trait so that `apply`,
//! `sync` and `daemon` can be tested against the mock server (`testsupport`) or a
//! fault-injecting wrapper.
//!
//! Consumes: [`crate::tree`] (`Node`, `ApiNode`, `NodeId`, `ShortId`, `Position`).
//! Consumed by: `apply`, `sync`, `daemon`, `main`.
//!
//! # Wire format (DM§1–§4)
//!
//! * Base URL + `/api/v1/...`; header `Authorization: Bearer <key>`; JSON bodies with
//!   `Content-Type: application/json` (the server parses any body as JSON, DM§2.4).
//! * `GET /api/v1/nodes-export` -> `{"nodes": [ApiNode…]}` (whole account, completed
//!   included, order unspecified, DM§4.10).
//! * `GET /api/v1/nodes/<id>` -> `{"node": ApiNode}`; `<id>` may be a full UUID or a
//!   12-hex short id (DM§3.1).
//! * `GET /api/v1/nodes?parent_id=<uuid|None>` -> `{"nodes": [...]}`, depth 1, creation
//!   order (callers sort by priority).
//! * `POST /api/v1/nodes` `{"parent_id", "name", "note"?, "layoutMode"?, "position"}` ->
//!   `{"item_id": "<uuid>"}`. Note: the create path transforms `name` (DM§6.2); `note`
//!   and `layoutMode` are verbatim.
//! * `POST /api/v1/nodes/<uuid>` `{"name"?, "note"?, "layoutMode"?}` (only the fields
//!   being changed; never send `null`) -> `{"status": "ok"}`; always bumps `modifiedAt`.
//! * `POST /api/v1/nodes/<uuid>/move` `{"parent_id": "<uuid>", "position"}`;
//!   `POST …/complete`, `POST …/uncomplete` (no body), `DELETE /api/v1/nodes/<uuid>`.
//!
//! # Rules
//!
//! * **Full UUIDs for mutations** (DM§3.1): every mutating method checks that every id
//!   it sends (path id and `parent_id`) satisfies `NodeId::is_uuid`, and fails with
//!   [`ApiError::NotFullUuid`] **before** sending otherwise (this also stops temp ids).
//!   `parent_id` is always sent explicitly (§8.2: an omitted one is accepted with unknown
//!   behaviour).
//! * **Rate limit**: at most one request per [`ClientConfig::min_interval`] (default
//!   100 ms = 10 req/s, §2), process-wide for this client, enforced before every attempt
//!   including retries. Exports additionally respect
//!   [`ClientConfig::export_min_interval`].
//! * **Retries**: on 5xx, 429 and transport errors, retry up to
//!   [`ClientConfig::max_retries`] times with exponential backoff
//!   (`backoff * 2^attempt`, capped at 30 s). Exception: `create` is not idempotent, so
//!   it is retried only on 5xx/429 (the server changes nothing on those, DM§2.6) and
//!   **never** on a transport error after the request may have been sent; such a failure
//!   is returned as [`ApiError::Transport`] with `maybe_applied: true` so `apply` can
//!   recover through the nonce (§7.3). A transport error that provably happened before
//!   anything was sent (connection refused, DNS failure, connect timeout) is retried even
//!   for create, and reported with `maybe_applied: false`. `delete` treats a 404 on a
//!   retry as success. Other 4xx responses are never retried. 429 is retried although
//!   no 429 was ever observed (DM§2.7), as `prod_client` does.
//! * **Typed errors** from the response shapes of DM§2.3, §2.5, §2.6 ([`ApiError`],
//!   [`map_error`]).
//! * **Request log**: if [`ClientConfig::request_log`] is set, append one JSON object per
//!   attempt (`ts`, `elapsed`, `method`, `path`, `query`, `request_json`, `status`,
//!   `response` (parsed JSON or text truncated to 5000 chars), `error` (transport error
//!   text or null), `attempt`); never the `Authorization` header.
//! * [`Client::request_raw`] sends an arbitrary (even malformed) body through the same
//!   machinery, for diagnostics and error-mapping tests.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;

use crate::tree::{ApiNode, Node, NodeId, Position, ShortId};

/// The production base URL.
pub const DEFAULT_BASE_URL: &str = "https://workflowy.com";

/// Client configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientConfig {
    /// Base URL without trailing slash (default [`DEFAULT_BASE_URL`]; tests use the mock).
    pub base_url: String,
    /// API key (`WORKFLOWY_API_KEY`).
    pub api_key: String,
    /// Minimum time between requests. Default 100 ms (10 req/s, §2).
    pub min_interval: Duration,
    /// Minimum time between exports. Default zero: the documented 1/min limit was not
    /// enforced (DM§4.10); §12.2 suggests 60 s for polling, which the daemon applies.
    pub export_min_interval: Duration,
    /// Retries after the first attempt. Default 4.
    pub max_retries: u32,
    /// Base backoff. Default 500 ms.
    pub backoff: Duration,
    /// Per-request timeout. Default 60 s (an export of 13k nodes took ~1 s).
    pub timeout: Duration,
    /// Optional JSONL request log.
    pub request_log: Option<PathBuf>,
}

impl ClientConfig {
    /// Production defaults for `api_key`.
    pub fn new(api_key: impl Into<String>) -> ClientConfig {
        ClientConfig {
            base_url: DEFAULT_BASE_URL.to_owned(),
            api_key: api_key.into(),
            min_interval: Duration::from_millis(100),
            export_min_interval: Duration::ZERO,
            max_retries: 4,
            backoff: Duration::from_millis(500),
            timeout: Duration::from_secs(60),
            request_log: None,
        }
    }
}

/// One item of a 400 validation error body (DM§2.5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidationError {
    /// `string_type`, `missing`, `enum`, `model_type`, …
    #[serde(rename = "type")]
    pub kind: String,
    /// Location, e.g. `["name"]`.
    pub loc: Vec<serde_json::Value>,
    /// Message.
    pub msg: String,
}

/// API errors, by response shape (DM§2).
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ApiError {
    /// 401 (either body shape, DM§2.3).
    #[error("invalid credentials")]
    Unauthorized,
    /// 404 `Item not found` (also for deleted nodes and short ids on mutations).
    #[error("item not found")]
    ItemNotFound,
    /// 404 `Parent item not found`.
    #[error("parent item not found")]
    ParentNotFound,
    /// 404 `Location '<raw>' not found`.
    #[error("location {0:?} not found")]
    LocationNotFound(String),
    /// 400 `{"errors": [...]}`.
    #[error("validation failed: {0:?}")]
    Validation(Vec<ValidationError>),
    /// 400 `invalid_json`.
    #[error("invalid JSON body")]
    InvalidJson,
    /// 400 `invalid_request` (and the mock-specific cycle refusal).
    #[error("invalid request: {0}")]
    InvalidRequest(String),
    /// 429 after the retries.
    #[error("rate limited")]
    RateLimited,
    /// 5xx after the retries.
    #[error("server error {status}: {body}")]
    Server {
        /// HTTP status.
        status: u16,
        /// Response body (truncated).
        body: String,
    },
    /// Any other status or an unrecognised body.
    #[error("unexpected response {status}: {body}")]
    Unexpected {
        /// HTTP status.
        status: u16,
        /// Response body (truncated).
        body: String,
    },
    /// Network failure after the retries. `maybe_applied` is true when the request may
    /// have reached the server (only reported for non-idempotent requests: create).
    #[error("transport error: {message}")]
    Transport {
        /// Description.
        message: String,
        /// The server may have executed the request.
        maybe_applied: bool,
    },
    /// A 200 response whose body did not decode.
    #[error("cannot decode response: {0}")]
    Decode(String),
    /// Refused locally: a mutation was given an id that is not a full lowercase UUID.
    #[error("not a full UUID: {0:?}")]
    NotFullUuid(String),
}

/// A create request (§7.2). `parent_id` is always explicit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateRequest {
    /// Parent (full UUID).
    pub parent_id: NodeId,
    /// Name (transformed by the server's create pipeline, DM§6.2).
    pub name: String,
    /// Note, verbatim; omitted when `None`.
    pub note: Option<String>,
    /// `layoutMode`, verbatim; omitted when `None` (server default `bullets`).
    pub layout: Option<String>,
    /// Placement.
    pub position: Position,
}

/// An update request; only `Some` fields are sent (never `null`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateRequest {
    /// New name (verbatim minus C0 controls, DM§6.1).
    pub name: Option<String>,
    /// New note.
    pub note: Option<String>,
    /// New layout.
    pub layout: Option<String>,
}

/// The API surface the engine uses. Implemented by [`Client`]; tests may wrap it.
pub trait Api {
    /// `GET /nodes-export`: every node of the account.
    fn export(&self) -> Result<Vec<Node>, ApiError>;
    /// `GET /nodes/<uuid>`.
    fn get(&self, id: &NodeId) -> Result<Node, ApiError>;
    /// `GET /nodes/<short>` (GET is the only endpoint that accepts short ids).
    fn get_short(&self, id: &ShortId) -> Result<Node, ApiError>;
    /// `GET /nodes?parent_id=…` (`None` = top level, sent as `None`). Depth 1.
    fn list(&self, parent: Option<&NodeId>) -> Result<Vec<Node>, ApiError>;
    /// `POST /nodes`; returns the new node's id.
    fn create(&self, req: &CreateRequest) -> Result<NodeId, ApiError>;
    /// `POST /nodes/<uuid>`.
    fn update(&self, id: &NodeId, req: &UpdateRequest) -> Result<(), ApiError>;
    /// `POST /nodes/<uuid>/move` with an explicit full-UUID parent.
    fn move_node(&self, id: &NodeId, parent: &NodeId, position: Position) -> Result<(), ApiError>;
    /// `POST /nodes/<uuid>/complete`.
    fn complete(&self, id: &NodeId) -> Result<(), ApiError>;
    /// `POST /nodes/<uuid>/uncomplete`.
    fn uncomplete(&self, id: &NodeId) -> Result<(), ApiError>;
    /// `DELETE /nodes/<uuid>`.
    fn delete(&self, id: &NodeId) -> Result<(), ApiError>;
    /// Number of HTTP requests sent so far (attempts, including retries).
    fn request_count(&self) -> u64;
}

/// Error bodies (and logged text responses) are truncated to this many characters.
const BODY_TRUNCATE: usize = 5000;

/// The longest backoff between two attempts.
const MAX_BACKOFF: Duration = Duration::from_secs(30);

/// Responses larger than this are refused (an export of 13k nodes was 4.4 MB, §2; ureq's
/// default limit of 10 MB would be too tight for large accounts).
const MAX_RESPONSE_BYTES: u64 = 1 << 30;

/// How a request may be retried.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Retry {
    /// Idempotent: retry on 5xx, 429 and every transport error.
    Idempotent,
    /// Not idempotent (create): retry on 5xx and 429 (the server changed nothing,
    /// DM§2.6), and on transport errors only when the request provably never left
    /// (connection refused, DNS failure, connect timeout).
    NonIdempotent,
    /// DELETE: idempotent, and a 404 on a retry means an earlier attempt succeeded.
    Delete,
}

/// One logical request.
struct Request<'a> {
    method: &'a str,
    path: String,
    query: Vec<(&'a str, String)>,
    /// Raw body bytes, sent with `Content-Type: application/json` (DM§2.4).
    body: Option<Vec<u8>>,
    retry: Retry,
}

/// The outcome of one attempt that reached the HTTP layer.
struct RawResponse {
    status: u16,
    text: String,
}

/// The `ureq`-based client.
///
/// `Client` is `Send + Sync`; the rate limiter and the request counter are shared by all
/// threads using one client.
#[derive(Debug)]
pub struct Client {
    cfg: ClientConfig,
    agent: ureq::Agent,
    /// Earliest instant at which the next attempt may start (rate limit, §2).
    next_slot: Mutex<Option<Instant>>,
    /// When the last export started (§12.2).
    last_export: Mutex<Option<Instant>>,
    /// Serialises appends to the request log.
    log_lock: Mutex<()>,
    requests: AtomicU64,
}

impl Client {
    /// Build a client (no request is sent).
    ///
    /// The HTTP proxy is taken from the environment (`HTTPS_PROXY`, `NO_PROXY`, … as
    /// `ureq` reads them), except that a loopback base URL (the mock) never uses one.
    pub fn new(cfg: ClientConfig) -> Client {
        let mut builder = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_global(Some(cfg.timeout));
        if is_loopback(&cfg.base_url) {
            builder = builder.proxy(None);
        }
        let agent: ureq::Agent = builder.build().into();
        Client {
            cfg,
            agent,
            next_slot: Mutex::new(None),
            last_export: Mutex::new(None),
            log_lock: Mutex::new(()),
            requests: AtomicU64::new(0),
        }
    }

    /// The configuration.
    pub fn config(&self) -> &ClientConfig {
        &self.cfg
    }

    /// Low-level escape hatch: send `GET`, `POST` or `DELETE` `path?query` with an
    /// optional raw body (sent as `application/json`, even if it is not valid JSON)
    /// through the same rate limiter, retry policy, request log and error mapping as the
    /// typed methods, and return the decoded JSON body of a 2xx response. No id
    /// validation is done. `GET` and `DELETE` are retried as idempotent; `POST` is
    /// treated like create (non-idempotent). Meant for diagnostics and for tests of the
    /// error mapping.
    pub fn request_raw(
        &self,
        method: &str,
        path: &str,
        query: &[(&str, &str)],
        body: Option<&[u8]>,
    ) -> Result<Value, ApiError> {
        let (method, retry) = match method {
            "GET" => ("GET", Retry::Idempotent),
            "DELETE" => ("DELETE", Retry::Delete),
            "POST" => ("POST", Retry::NonIdempotent),
            other => {
                return Err(ApiError::InvalidRequest(format!(
                    "unsupported method {other:?}"
                )));
            }
        };
        self.execute(&Request {
            method,
            path: path.to_owned(),
            query: query.iter().map(|(k, v)| (*k, (*v).to_owned())).collect(),
            body: body.map(<[u8]>::to_vec),
            retry,
        })
    }

    // ----- rate limiting ------------------------------------------------------------

    /// Block until the next request slot (§2: at most one request per `min_interval`,
    /// i.e. a token bucket of capacity one). The lock is held while sleeping so that
    /// concurrent callers queue up in order.
    fn acquire_slot(&self) {
        let interval = self.cfg.min_interval;
        let mut next = self.next_slot.lock().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();
        let start = match *next {
            Some(t) if t > now => {
                std::thread::sleep(t - now);
                t
            }
            _ => now,
        };
        *next = Some(start + interval);
    }

    /// Block until an export is allowed (§12.2), and record this one.
    fn acquire_export_slot(&self) {
        let interval = self.cfg.export_min_interval;
        let mut last = self.last_export.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(t) = *last {
            let ready = t + interval;
            let now = Instant::now();
            if ready > now {
                std::thread::sleep(ready - now);
            }
        }
        *last = Some(Instant::now());
    }

    fn backoff(&self, attempt: u32) -> Duration {
        let factor = 1u32.checked_shl(attempt.min(31)).unwrap_or(u32::MAX);
        self.cfg
            .backoff
            .checked_mul(factor)
            .unwrap_or(MAX_BACKOFF)
            .min(MAX_BACKOFF)
    }

    // ----- one attempt ----------------------------------------------------------------

    fn send_once(&self, req: &Request<'_>) -> Result<RawResponse, ureq::Error> {
        let url = format!("{}{}", self.cfg.base_url.trim_end_matches('/'), req.path);
        let auth = format!("Bearer {}", self.cfg.api_key);
        let result = match req.method {
            "GET" => {
                let mut r = self.agent.get(&url).header("Authorization", &auth);
                for (k, v) in &req.query {
                    r = r.query(*k, v);
                }
                r.call()
            }
            "DELETE" => {
                let mut r = self.agent.delete(&url).header("Authorization", &auth);
                for (k, v) in &req.query {
                    r = r.query(*k, v);
                }
                r.call()
            }
            _ => {
                let mut r = self.agent.post(&url).header("Authorization", &auth);
                for (k, v) in &req.query {
                    r = r.query(*k, v);
                }
                match &req.body {
                    Some(b) => r.header("Content-Type", "application/json").send(&b[..]),
                    None => r.send_empty(),
                }
            }
        };
        let mut resp = result?;
        read_response(&mut resp)
    }

    // ----- the retry loop -------------------------------------------------------------

    fn execute(&self, req: &Request<'_>) -> Result<Value, ApiError> {
        let mut attempt: u32 = 0;
        loop {
            self.acquire_slot();
            self.requests.fetch_add(1, Ordering::SeqCst);
            let t0 = SystemTime::now();
            let started = Instant::now();
            let result = self.send_once(req);
            self.log_attempt(req, attempt, t0, started.elapsed(), &result);
            let can_retry = attempt < self.cfg.max_retries;
            match result {
                Ok(resp) => {
                    let status = resp.status;
                    if (200..300).contains(&status) {
                        return serde_json::from_str(&resp.text).map_err(|e| {
                            ApiError::Decode(format!("{e}: {}", truncate(&resp.text)))
                        });
                    }
                    if status == 404 && req.retry == Retry::Delete && attempt > 0 {
                        // An earlier attempt may have deleted the node before failing.
                        return Ok(json!({"status": "ok"}));
                    }
                    // Retry 5xx and 429 (prod_client retries 429 too); never other 4xx.
                    if (status >= 500 || status == 429) && can_retry {
                        std::thread::sleep(self.backoff(attempt));
                        attempt += 1;
                        continue;
                    }
                    return Err(map_error(status, &resp.text));
                }
                Err(e) => {
                    let not_sent = provably_not_sent(&e);
                    if req.retry == Retry::NonIdempotent && !not_sent {
                        // §7.3: the create may have been applied; `apply` recovers it
                        // through the nonce instead of risking a duplicate.
                        return Err(ApiError::Transport {
                            message: e.to_string(),
                            maybe_applied: true,
                        });
                    }
                    if can_retry {
                        std::thread::sleep(self.backoff(attempt));
                        attempt += 1;
                        continue;
                    }
                    return Err(ApiError::Transport {
                        message: e.to_string(),
                        maybe_applied: false,
                    });
                }
            }
        }
    }

    fn log_attempt(
        &self,
        req: &Request<'_>,
        attempt: u32,
        t0: SystemTime,
        elapsed: Duration,
        result: &Result<RawResponse, ureq::Error>,
    ) {
        let Some(path) = &self.cfg.request_log else {
            return;
        };
        let ts = t0
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0);
        let query: serde_json::Map<String, Value> = req
            .query
            .iter()
            .map(|(k, v)| ((*k).to_owned(), Value::String(v.clone())))
            .collect();
        let request_json = req.body.as_ref().map(|b| {
            serde_json::from_slice::<Value>(b)
                .unwrap_or_else(|_| Value::String(truncate(&String::from_utf8_lossy(b))))
        });
        let (status, response, error) = match result {
            Ok(r) => (
                Some(r.status),
                serde_json::from_str::<Value>(&r.text)
                    .unwrap_or_else(|_| Value::String(truncate(&r.text))),
                None,
            ),
            Err(e) => (None, Value::Null, Some(e.to_string())),
        };
        // The Authorization header is never logged.
        let rec = json!({
            "ts": ts,
            "elapsed": (elapsed.as_secs_f64() * 10_000.0).round() / 10_000.0,
            "method": req.method,
            "path": req.path,
            "query": if query.is_empty() { Value::Null } else { Value::Object(query) },
            "request_json": request_json,
            "status": status,
            "response": response,
            "error": error,
            "attempt": attempt,
        });
        let _guard = self.log_lock.lock().unwrap_or_else(|e| e.into_inner());
        // A failure to log must not fail the request.
        if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(path) {
            let _ = writeln!(f, "{rec}");
        }
    }

    /// `POST /api/v1/nodes/<id><suffix>` with an optional JSON body, retried as
    /// idempotent (update, move, complete and uncomplete all are).
    fn simple_post(&self, id: &NodeId, suffix: &str, body: Option<Value>) -> Result<(), ApiError> {
        require_uuid(id)?;
        self.execute(&Request {
            method: "POST",
            path: format!("/api/v1/nodes/{}{suffix}", id.as_str()),
            query: Vec::new(),
            body: body.map(|b| b.to_string().into_bytes()),
            retry: Retry::Idempotent,
        })?;
        Ok(())
    }
}

/// Mutations accept only exact lowercase UUIDs (DM§3.1); refuse anything else locally,
/// before sending (a short id would be a silent 404, a temp id a bug).
fn require_uuid(id: &NodeId) -> Result<(), ApiError> {
    if id.is_uuid() {
        Ok(())
    } else {
        Err(ApiError::NotFullUuid(id.as_str().to_owned()))
    }
}

fn read_response(resp: &mut ureq::http::Response<ureq::Body>) -> Result<RawResponse, ureq::Error> {
    let status = resp.status().as_u16();
    let text = resp
        .body_mut()
        .with_config()
        .limit(MAX_RESPONSE_BYTES)
        .read_to_string()?;
    Ok(RawResponse { status, text })
}

/// True if the error happened before any byte of the request could reach the server, so
/// that even a non-idempotent request may be retried safely.
fn provably_not_sent(e: &ureq::Error) -> bool {
    use ureq::Timeout;
    match e {
        ureq::Error::HostNotFound
        | ureq::Error::ConnectionFailed
        | ureq::Error::BadUri(_)
        | ureq::Error::InvalidProxyUrl
        | ureq::Error::ConnectProxyFailed(_)
        | ureq::Error::Http(_) => true,
        ureq::Error::Timeout(t) => matches!(t, Timeout::Resolve | Timeout::Connect),
        ureq::Error::Io(io) => matches!(
            io.kind(),
            std::io::ErrorKind::ConnectionRefused
                | std::io::ErrorKind::AddrNotAvailable
                | std::io::ErrorKind::HostUnreachable
                | std::io::ErrorKind::NetworkUnreachable
        ),
        _ => false,
    }
}

fn is_loopback(base_url: &str) -> bool {
    let rest = base_url
        .strip_prefix("http://")
        .or_else(|| base_url.strip_prefix("https://"))
        .unwrap_or(base_url);
    let authority = rest.split('/').next().unwrap_or("");
    let host = if let Some(stripped) = authority.strip_prefix('[') {
        stripped.split(']').next().unwrap_or("")
    } else {
        authority.split(':').next().unwrap_or("")
    };
    host == "localhost" || host == "::1" || host.starts_with("127.")
}

fn truncate(s: &str) -> String {
    match s.char_indices().nth(BODY_TRUNCATE) {
        Some((i, _)) => s[..i].to_owned(),
        None => s.to_owned(),
    }
}

/// Map a non-2xx response to its typed error by status and body shape (DM§2.3, §2.5,
/// §2.6). Pure; public so that wrappers and tests can reuse it.
pub fn map_error(status: u16, body: &str) -> ApiError {
    let parsed: Option<Value> = serde_json::from_str(body).ok();
    let code = parsed
        .as_ref()
        .and_then(|v| v.get("code"))
        .and_then(Value::as_str);
    let message = parsed
        .as_ref()
        .and_then(|v| v.get("message"))
        .and_then(Value::as_str);
    let unexpected = || ApiError::Unexpected {
        status,
        body: truncate(body),
    };
    match status {
        // Both 401 shapes (GET: `{"errors": "..."}`, POST/DELETE: `{"code":
        // "invalid_credentials", ...}`) mean the same thing (DM§2.3).
        401 => ApiError::Unauthorized,
        404 => match message {
            Some("Item not found") => ApiError::ItemNotFound,
            Some("Parent item not found") => ApiError::ParentNotFound,
            Some(m) if m.starts_with("Location '") && m.ends_with("' not found") => {
                let raw = &m["Location '".len()..m.len() - "' not found".len()];
                ApiError::LocationNotFound(raw.to_owned())
            }
            // An unknown route gives an HTML 404 (DM§2.1).
            _ => unexpected(),
        },
        400 => {
            if code == Some("invalid_json") {
                return ApiError::InvalidJson;
            }
            if code == Some("invalid_request") {
                return ApiError::InvalidRequest(message.unwrap_or_default().to_owned());
            }
            match parsed.as_ref().and_then(|v| v.get("errors")) {
                Some(errs @ Value::Array(_)) => {
                    match serde_json::from_value::<Vec<ValidationError>>(errs.clone()) {
                        Ok(v) => ApiError::Validation(v),
                        Err(_) => unexpected(),
                    }
                }
                _ => unexpected(),
            }
        }
        // UNVERIFIED: no 429 was ever observed and its body shape is unknown (DM§2.7);
        // any 429 is mapped here regardless of body.
        429 => ApiError::RateLimited,
        500..=599 => ApiError::Server {
            status,
            body: truncate(body),
        },
        _ => unexpected(),
    }
}

#[derive(Deserialize)]
struct NodesResponse {
    nodes: Vec<ApiNode>,
}

#[derive(Deserialize)]
struct NodeResponse {
    node: ApiNode,
}

#[derive(Deserialize)]
struct CreateResponse {
    item_id: String,
}

fn decode<T: serde::de::DeserializeOwned>(v: Value) -> Result<T, ApiError> {
    serde_json::from_value(v).map_err(|e| ApiError::Decode(e.to_string()))
}

impl Client {
    fn get_path(&self, id: &str) -> Result<Node, ApiError> {
        let v = self.execute(&Request {
            method: "GET",
            path: format!("/api/v1/nodes/{id}"),
            query: Vec::new(),
            body: None,
            retry: Retry::Idempotent,
        })?;
        let r: NodeResponse = decode(v)?;
        Ok(r.node.into())
    }
}

impl Api for Client {
    fn export(&self) -> Result<Vec<Node>, ApiError> {
        // §12.2: optional minimum interval between exports (default zero).
        self.acquire_export_slot();
        let v = self.execute(&Request {
            method: "GET",
            path: "/api/v1/nodes-export".to_owned(),
            query: Vec::new(),
            body: None,
            retry: Retry::Idempotent,
        })?;
        // DM§4.10: a flat list in unspecified order; callers build a `Tree`.
        let r: NodesResponse = decode(v)?;
        Ok(r.nodes.into_iter().map(Node::from).collect())
    }

    fn get(&self, id: &NodeId) -> Result<Node, ApiError> {
        // Every id the API hands out is a lowercase UUID (DM§1.1); anything else (a temp
        // id) is a caller bug, and would be a 404 anyway.
        require_uuid(id)?;
        self.get_path(id.as_str())
    }

    fn get_short(&self, id: &ShortId) -> Result<Node, ApiError> {
        // DM§3.1: GET is the only endpoint that resolves short ids.
        self.get_path(id.as_str())
    }

    fn list(&self, parent: Option<&NodeId>) -> Result<Vec<Node>, ApiError> {
        let parent_id = match parent {
            Some(p) => {
                require_uuid(p)?;
                p.as_str().to_owned()
            }
            // DM§3.2: the literal string `None` is the root.
            None => "None".to_owned(),
        };
        let v = self.execute(&Request {
            method: "GET",
            path: "/api/v1/nodes".to_owned(),
            query: vec![("parent_id", parent_id)],
            body: None,
            retry: Retry::Idempotent,
        })?;
        let r: NodesResponse = decode(v)?;
        Ok(r.nodes.into_iter().map(Node::from).collect())
    }

    fn create(&self, req: &CreateRequest) -> Result<NodeId, ApiError> {
        require_uuid(&req.parent_id)?;
        // Key order follows CreateItemRequest's declaration (DM§2.5); `parent_id` is
        // always explicit (§8.2); absent optional fields are omitted, never null.
        let mut body = serde_json::Map::new();
        body.insert("name".into(), Value::String(req.name.clone()));
        body.insert(
            "parent_id".into(),
            Value::String(req.parent_id.as_str().to_owned()),
        );
        if let Some(note) = &req.note {
            body.insert("note".into(), Value::String(note.clone()));
        }
        if let Some(layout) = &req.layout {
            body.insert("layoutMode".into(), Value::String(layout.clone()));
        }
        body.insert(
            "position".into(),
            Value::String(req.position.as_str().to_owned()),
        );
        let v = self.execute(&Request {
            method: "POST",
            path: "/api/v1/nodes".to_owned(),
            query: Vec::new(),
            body: Some(Value::Object(body).to_string().into_bytes()),
            retry: Retry::NonIdempotent,
        })?;
        let r: CreateResponse = decode(v)?;
        Ok(NodeId::from_api(r.item_id))
    }

    fn update(&self, id: &NodeId, req: &UpdateRequest) -> Result<(), ApiError> {
        // Only the fields being changed; `null` is never sent (DM§4.4). An empty request
        // still sends `{}`, which bumps `modifiedAt`.
        let mut body = serde_json::Map::new();
        if let Some(name) = &req.name {
            body.insert("name".into(), Value::String(name.clone()));
        }
        if let Some(note) = &req.note {
            body.insert("note".into(), Value::String(note.clone()));
        }
        if let Some(layout) = &req.layout {
            body.insert("layoutMode".into(), Value::String(layout.clone()));
        }
        self.simple_post(id, "", Some(Value::Object(body)))
    }

    fn move_node(&self, id: &NodeId, parent: &NodeId, position: Position) -> Result<(), ApiError> {
        require_uuid(id)?;
        require_uuid(parent)?;
        // Retrying a move is safe: it lands the node at the same end again (DM§4.7; only
        // its priority value may differ).
        self.simple_post(
            id,
            "/move",
            Some(json!({"parent_id": parent.as_str(), "position": position.as_str()})),
        )
    }

    fn complete(&self, id: &NodeId) -> Result<(), ApiError> {
        // The body is ignored by the server (DM§4.5); none is sent.
        self.simple_post(id, "/complete", None)
    }

    fn uncomplete(&self, id: &NodeId) -> Result<(), ApiError> {
        self.simple_post(id, "/uncomplete", None)
    }

    fn delete(&self, id: &NodeId) -> Result<(), ApiError> {
        require_uuid(id)?;
        self.execute(&Request {
            method: "DELETE",
            path: format!("/api/v1/nodes/{}", id.as_str()),
            query: Vec::new(),
            body: None,
            retry: Retry::Delete,
        })?;
        Ok(())
    }

    fn request_count(&self) -> u64 {
        self.requests.load(Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_error_shapes() {
        assert_eq!(
            map_error(401, r#"{"errors": "Invalid Credentials, try again."}"#),
            ApiError::Unauthorized
        );
        assert_eq!(
            map_error(
                401,
                r#"{"code": "invalid_credentials", "message": "Invalid Credentials, try again."}"#
            ),
            ApiError::Unauthorized
        );
        assert_eq!(
            map_error(404, r#"{"code": "not_found", "message": "Item not found"}"#),
            ApiError::ItemNotFound
        );
        assert_eq!(
            map_error(
                404,
                r#"{"code": "not_found", "message": "Parent item not found"}"#
            ),
            ApiError::ParentNotFound
        );
        assert_eq!(
            map_error(
                404,
                r#"{"code": "not_found", "message": "Location 'x'y' not found"}"#
            ),
            ApiError::LocationNotFound("x'y".into())
        );
        assert!(matches!(
            map_error(404, "<!DOCTYPE html><title>404</title>"),
            ApiError::Unexpected { status: 404, .. }
        ));
        assert_eq!(
            map_error(
                400,
                r#"{"code": "invalid_json", "message": "Invalid JSON format in request body"}"#
            ),
            ApiError::InvalidJson
        );
        assert_eq!(
            map_error(
                400,
                r#"{"code": "invalid_request", "message": "Node is not a mirror"}"#
            ),
            ApiError::InvalidRequest("Node is not a mirror".into())
        );
        assert_eq!(
            map_error(
                400,
                r#"{"errors": [{"type": "string_type", "loc": ["name"], "msg": "Input should be a valid string", "input": 1}]}"#
            ),
            ApiError::Validation(vec![ValidationError {
                kind: "string_type".into(),
                loc: vec![json!("name")],
                msg: "Input should be a valid string".into(),
            }])
        );
        assert_eq!(map_error(429, ""), ApiError::RateLimited);
        assert!(matches!(
            map_error(502, "bad"),
            ApiError::Server { status: 502, .. }
        ));
        assert!(matches!(
            map_error(418, ""),
            ApiError::Unexpected { status: 418, .. }
        ));
    }

    #[test]
    fn truncates_on_char_boundaries() {
        let s = "é".repeat(BODY_TRUNCATE + 10);
        assert_eq!(truncate(&s).chars().count(), BODY_TRUNCATE);
        assert_eq!(truncate("abc"), "abc");
    }

    #[test]
    fn loopback_detection() {
        assert!(is_loopback("http://127.0.0.1:8080"));
        assert!(is_loopback("http://localhost:1/x"));
        assert!(is_loopback("http://[::1]:5"));
        assert!(!is_loopback("https://workflowy.com"));
    }

    #[test]
    fn backoff_doubles_and_caps() {
        let mut cfg = ClientConfig::new("k");
        cfg.backoff = Duration::from_millis(500);
        let c = Client::new(cfg);
        assert_eq!(c.backoff(0), Duration::from_millis(500));
        assert_eq!(c.backoff(2), Duration::from_secs(2));
        assert_eq!(c.backoff(10), MAX_BACKOFF);
        assert_eq!(c.backoff(40), MAX_BACKOFF);
    }

    #[test]
    fn mutations_refuse_short_and_temp_ids_without_sending() {
        let mut cfg = ClientConfig::new("k");
        // Nothing listens here; a request would be a transport error, not NotFullUuid.
        cfg.base_url = "http://127.0.0.1:9".into();
        cfg.max_retries = 0;
        let c = Client::new(cfg);
        let short = NodeId::from_api("a846cfce4c6d");
        let temp = NodeId::temp(1);
        let good = NodeId::parse("5e2e76e4-fea4-4f1f-9c5b-a846cfce4c6d").unwrap();
        assert_eq!(
            c.delete(&short),
            Err(ApiError::NotFullUuid("a846cfce4c6d".into()))
        );
        assert_eq!(
            c.complete(&temp),
            Err(ApiError::NotFullUuid("temp-1".into()))
        );
        assert!(matches!(
            c.move_node(&good, &short, Position::Top),
            Err(ApiError::NotFullUuid(_))
        ));
        assert!(matches!(
            c.create(&CreateRequest {
                parent_id: temp.clone(),
                name: "x".into(),
                note: None,
                layout: None,
                position: Position::Bottom,
            }),
            Err(ApiError::NotFullUuid(_))
        ));
        assert_eq!(c.request_count(), 0);
    }
}
