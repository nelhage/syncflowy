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
//!   recover through the nonce (§7.3). `delete` treats a 404 on a retry as success.
//! * **Typed errors** from the response shapes of DM§2.3, §2.5, §2.6 ([`ApiError`]).
//! * **Request log**: if [`ClientConfig::request_log`] is set, append one JSON object per
//!   attempt (`ts`, `elapsed`, `method`, `path`, `query`, `request_json`, `status`,
//!   `response` (parsed JSON or text truncated to 5000 chars), `attempt`); never the
//!   `Authorization` header.

#![allow(unused_variables, dead_code)] // stubs; remove when implemented

use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::tree::{Node, NodeId, Position, ShortId};

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

/// The `ureq`-based client.
#[derive(Debug)]
pub struct Client {
    cfg: ClientConfig,
}

impl Client {
    /// Build a client (no request is sent).
    pub fn new(cfg: ClientConfig) -> Client {
        Client { cfg }
    }

    /// The configuration.
    pub fn config(&self) -> &ClientConfig {
        &self.cfg
    }
}

impl Api for Client {
    fn export(&self) -> Result<Vec<Node>, ApiError> {
        todo!()
    }
    fn get(&self, id: &NodeId) -> Result<Node, ApiError> {
        todo!()
    }
    fn get_short(&self, id: &ShortId) -> Result<Node, ApiError> {
        todo!()
    }
    fn list(&self, parent: Option<&NodeId>) -> Result<Vec<Node>, ApiError> {
        todo!()
    }
    fn create(&self, req: &CreateRequest) -> Result<NodeId, ApiError> {
        todo!()
    }
    fn update(&self, id: &NodeId, req: &UpdateRequest) -> Result<(), ApiError> {
        todo!()
    }
    fn move_node(&self, id: &NodeId, parent: &NodeId, position: Position) -> Result<(), ApiError> {
        todo!()
    }
    fn complete(&self, id: &NodeId) -> Result<(), ApiError> {
        todo!()
    }
    fn uncomplete(&self, id: &NodeId) -> Result<(), ApiError> {
        todo!()
    }
    fn delete(&self, id: &NodeId) -> Result<(), ApiError> {
        todo!()
    }
    fn request_count(&self) -> u64 {
        todo!()
    }
}
