//! Shared helpers of the `sync*` integration tests: a mock server, a temp vault and
//! state directory, UI-route shorthands, and an `Api` wrapper that counts mutations and
//! can run a hook during an export (to race the compare-and-swap write).

#![allow(dead_code)]

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::{Value, json};
use syncflowy::mdblock::DocConfig;
use syncflowy::sync::{self, SyncConfig, SyncReport};
use syncflowy::testsupport::MockServer;
use syncflowy::tree::{Node, NodeId, Position, ShortId, Tree};
use syncflowy::wfclient::{Api, ApiError, Client, CreateRequest, UpdateRequest};

/// A fixed clock for the sync config (the mock has its own clock).
pub fn fixed_clock() -> i64 {
    1_790_000_000
}

/// One test environment: a fresh mock server, a client, a vault and a state dir.
pub struct Env {
    pub mock: MockServer,
    pub client: Client,
    pub dir: tempfile::TempDir,
    pub cfg: SyncConfig,
}

impl Env {
    pub fn new() -> Env {
        let mock = MockServer::start();
        let client = Client::new(mock.client_config());
        let dir = tempfile::tempdir().expect("tempdir");
        let mut cfg = SyncConfig::new(dir.path().join("state"));
        cfg.clock = sync::Clock::new(fixed_clock);
        Env {
            mock,
            client,
            dir,
            cfg,
        }
    }

    /// `<vault>/<name>`.
    pub fn path(&self, name: &str) -> PathBuf {
        self.dir.path().join("vault").join(name)
    }

    /// A node created through the web-UI simulation (`text` is plain text).
    pub fn ui_create(&self, parent: Option<&NodeId>, text: &str) -> NodeId {
        self.ui_create_with(parent, json!({ "text": text }))
    }

    /// A UI create with extra body fields (`note_text`, `layoutMode`, `name`, …).
    pub fn ui_create_with(&self, parent: Option<&NodeId>, mut body: Value) -> NodeId {
        body["parent_id"] = match parent {
            Some(p) => json!(p.as_str()),
            None => Value::Null,
        };
        let r = self.mock.ui("create", body);
        NodeId::parse(r["id"].as_str().expect("id")).expect("uuid")
    }

    pub fn ui(&self, action: &str, body: Value) -> Value {
        self.mock.ui(action, body)
    }

    pub fn export(&self) -> Tree {
        Tree::from_nodes(self.client.export().expect("export")).expect("tree")
    }

    /// The nodes and trash of the mock (to prove that nothing was mutated).
    pub fn snapshot(&self) -> Value {
        let d = self.mock.dump();
        json!({ "nodes": d["nodes"], "trash": d["trash"] })
    }

    pub fn render(&self, root: &NodeId, file: &Path) -> SyncReport {
        self.render_with(root, file, &DocConfig::default())
    }

    pub fn render_with(&self, root: &NodeId, file: &Path, doc: &DocConfig) -> SyncReport {
        let short = root.short().expect("short").to_string();
        sync::render_once(&self.client, &short, file, doc, &self.cfg).expect("render")
    }

    pub fn sync(&self, file: &Path) -> SyncReport {
        sync::sync_once(&self.client, file, &self.cfg).expect("sync")
    }
}

pub fn read(path: &Path) -> String {
    std::fs::read_to_string(path).expect("read file")
}

pub fn write(path: &Path, text: &str) {
    std::fs::write(path, text).expect("write file")
}

/// Replace exactly one occurrence of `from` in the file.
pub fn edit(path: &Path, from: &str, to: &str) {
    let text = read(path);
    assert_eq!(text.matches(from).count(), 1, "{from:?} once in:\n{text}");
    write(path, &text.replacen(from, to, 1));
}

/// The short id of a node, as it appears in tags.
pub fn short(id: &NodeId) -> ShortId {
    id.short().expect("short id")
}

/// The line of `text` carrying the tag of `id`.
pub fn line_of<'t>(text: &'t str, id: &NodeId) -> &'t str {
    let s = short(id).to_string();
    text.lines()
        .find(|l| l.contains(&s))
        .unwrap_or_else(|| panic!("no line tagged {s} in:\n{text}"))
}

/// The children of `parent` in sibling order, by name.
pub fn child_names(tree: &Tree, parent: &NodeId) -> Vec<String> {
    tree.children(parent)
        .iter()
        .map(|c| tree.get(c).expect("child").name.clone())
        .collect()
}

/// The single node named `name`.
pub fn by_name<'t>(tree: &'t Tree, name: &str) -> &'t Node {
    let v: Vec<&Node> = tree.iter().filter(|n| n.name == name).collect();
    assert_eq!(v.len(), 1, "exactly one node named {name:?}");
    v[0]
}

/// Replace the file's generation-independent parts for snapshots: nothing to do, the
/// mock's ids are seeded; kept as a hook.
pub fn norm(text: &str) -> String {
    text.to_owned()
}

type Hook<'a> = Box<dyn FnMut(usize) + 'a>;
type CallHook<'a> = Box<dyn FnMut(&str) + 'a>;

/// An `Api` over the real client that counts mutating requests, runs `on_export(n)`
/// at the start of the n-th export (0-based), and `on_call(kind)` before every `get`
/// and mutating request (`kind` is "get", "create", "update", "move", "complete",
/// "uncomplete" or "delete"), to race the applier's pre-write checks.
pub struct TestApi<'a> {
    pub inner: &'a Client,
    pub mutations: AtomicUsize,
    pub exports: AtomicUsize,
    pub calls: Mutex<Vec<String>>,
    on_export: RefCell<Option<Hook<'a>>>,
    on_call: RefCell<Option<CallHook<'a>>>,
}

impl<'a> TestApi<'a> {
    pub fn new(inner: &'a Client) -> TestApi<'a> {
        TestApi {
            inner,
            mutations: AtomicUsize::new(0),
            exports: AtomicUsize::new(0),
            calls: Mutex::new(Vec::new()),
            on_export: RefCell::new(None),
            on_call: RefCell::new(None),
        }
    }

    pub fn with_call_hook(inner: &'a Client, hook: impl FnMut(&str) + 'a) -> TestApi<'a> {
        let t = TestApi::new(inner);
        *t.on_call.borrow_mut() = Some(Box::new(hook));
        t
    }

    fn before(&self, kind: &str) {
        if let Some(h) = self.on_call.borrow_mut().as_mut() {
            h(kind);
        }
    }

    pub fn with_export_hook(inner: &'a Client, hook: impl FnMut(usize) + 'a) -> TestApi<'a> {
        let t = TestApi::new(inner);
        *t.on_export.borrow_mut() = Some(Box::new(hook));
        t
    }

    pub fn mutations(&self) -> usize {
        self.mutations.load(Ordering::SeqCst)
    }

    pub fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }

    fn mutate(&self, what: String) {
        self.mutations.fetch_add(1, Ordering::SeqCst);
        self.calls.lock().unwrap().push(what);
    }
}

impl Api for TestApi<'_> {
    fn export(&self) -> Result<Vec<Node>, ApiError> {
        let n = self.exports.fetch_add(1, Ordering::SeqCst);
        if let Some(h) = self.on_export.borrow_mut().as_mut() {
            h(n);
        }
        self.calls.lock().unwrap().push("export".into());
        self.inner.export()
    }
    fn get(&self, id: &NodeId) -> Result<Node, ApiError> {
        self.before("get");
        self.inner.get(id)
    }
    fn get_short(&self, id: &ShortId) -> Result<Node, ApiError> {
        self.inner.get_short(id)
    }
    fn list(&self, parent: Option<&NodeId>) -> Result<Vec<Node>, ApiError> {
        self.inner.list(parent)
    }
    fn create(&self, req: &CreateRequest) -> Result<NodeId, ApiError> {
        self.before("create");
        self.mutate(format!("create under {}", req.parent_id));
        self.inner.create(req)
    }
    fn update(&self, id: &NodeId, req: &UpdateRequest) -> Result<(), ApiError> {
        self.before("update");
        self.mutate(format!("update {id}"));
        self.inner.update(id, req)
    }
    fn move_node(&self, id: &NodeId, parent: &NodeId, position: Position) -> Result<(), ApiError> {
        self.before("move");
        self.mutate(format!("move {id} to {} of {parent}", position.as_str()));
        self.inner.move_node(id, parent, position)
    }
    fn complete(&self, id: &NodeId) -> Result<(), ApiError> {
        self.before("complete");
        self.mutate(format!("complete {id}"));
        self.inner.complete(id)
    }
    fn uncomplete(&self, id: &NodeId) -> Result<(), ApiError> {
        self.before("uncomplete");
        self.mutate(format!("uncomplete {id}"));
        self.inner.uncomplete(id)
    }
    fn delete(&self, id: &NodeId) -> Result<(), ApiError> {
        self.before("delete");
        self.mutate(format!("delete {id}"));
        self.inner.delete(id)
    }
    fn request_count(&self) -> u64 {
        self.inner.request_count()
    }
}
