//! Tests of the blocking API client: every endpoint against the mock server (the
//! oracle), error mapping, short-id refusal, rate limiting, the request log, and retry
//! behaviour against a refused port and a scripted fake HTTP server.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use syncflowy::testsupport::MockServer;
use syncflowy::tree::{NodeId, Position, Tree};
use syncflowy::wfclient::{Api, ApiError, Client, ClientConfig, CreateRequest, UpdateRequest};

// ---------------------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------------------

fn top_level(mock: &MockServer, name: &str) -> NodeId {
    let (status, body) = mock.post(
        "/api/v1/nodes",
        &json!({"parent_id": "None", "name": name, "position": "bottom"}),
    );
    assert_eq!(status, 200, "{body}");
    NodeId::parse(body["item_id"].as_str().unwrap()).unwrap()
}

fn create(
    c: &Client,
    parent: &NodeId,
    name: &str,
    note: Option<&str>,
    layout: Option<&str>,
) -> NodeId {
    c.create(&CreateRequest {
        parent_id: parent.clone(),
        name: name.into(),
        note: note.map(Into::into),
        layout: layout.map(Into::into),
        position: Position::Bottom,
    })
    .expect("create")
}

fn temp_path(name: &str) -> std::path::PathBuf {
    let n: u64 = rand::random();
    std::env::temp_dir().join(format!(
        "syncflowy-wfclient-{}-{n:016x}-{name}",
        std::process::id()
    ))
}

// ---------------------------------------------------------------------------------------
// Against the mock. One server for all of these (startup is slow); sections reset it.
// ---------------------------------------------------------------------------------------

#[test]
fn against_the_mock() {
    let mock = MockServer::start();
    endpoints_round_trip(&mock);
    mock.reset();
    export_parses_a_tree(&mock);
    mock.reset();
    error_mapping(&mock);
    mock.reset();
    short_ids_are_refused_for_mutations(&mock);
    mock.reset();
    server_errors_are_retried_then_typed(&mock);
    mock.reset();
    request_log_is_written_without_the_key(&mock);
    mock.reset();
    rate_limit_is_measurable(&mock);
    export_min_interval_is_respected(&mock);
}

fn endpoints_round_trip(mock: &MockServer) {
    let c = Client::new(mock.client_config());
    let root = top_level(mock, "root");

    // create returns the item id; note and layout are verbatim.
    let a = create(&c, &root, "alpha", Some("line1\nline2"), Some("todo"));
    let b = c
        .create(&CreateRequest {
            parent_id: root.clone(),
            name: "beta".into(),
            note: None,
            layout: None,
            position: Position::Top,
        })
        .unwrap();
    assert!(a.is_uuid() && b.is_uuid());

    // get
    let na = c.get(&a).unwrap();
    assert_eq!(na.id, a);
    assert_eq!(na.parent_id.as_ref(), Some(&root));
    assert_eq!(na.name, "alpha");
    assert_eq!(na.note.as_deref(), Some("line1\nline2"));
    assert_eq!(na.layout, "todo");
    assert!(!na.completed);
    let nb = c.get(&b).unwrap();
    assert_eq!(nb.note, None);
    assert_eq!(nb.layout, "bullets");
    assert!(nb.priority < na.priority, "top placement");

    // get_short
    let ns = c.get_short(&a.short().unwrap()).unwrap();
    assert_eq!(ns.id, a);

    // list (child and top level)
    let mut kids: Vec<NodeId> = c
        .list(Some(&root))
        .unwrap()
        .into_iter()
        .map(|n| n.id)
        .collect();
    kids.sort();
    let mut want = vec![a.clone(), b.clone()];
    want.sort();
    assert_eq!(kids, want);
    let tops = c.list(None).unwrap();
    assert!(tops.iter().any(|n| n.id == root && n.parent_id.is_none()));

    // update: only Some fields change.
    c.update(
        &a,
        &UpdateRequest {
            name: Some("alpha &amp; <b>bold</b>".into()),
            note: Some(String::new()),
            layout: None,
        },
    )
    .unwrap();
    let na = c.get(&a).unwrap();
    assert_eq!(na.name, "alpha &amp; <b>bold</b>");
    assert_eq!(na.note.as_deref(), Some(""));
    assert_eq!(na.layout, "todo");
    c.update(
        &a,
        &UpdateRequest {
            layout: Some("h2".into()),
            ..Default::default()
        },
    )
    .unwrap();
    let na = c.get(&a).unwrap();
    assert_eq!(na.layout, "h2");
    assert_eq!(na.name, "alpha &amp; <b>bold</b>");

    // move, top and bottom
    let c1 = create(&c, &a, "c1", None, None);
    c.move_node(&b, &a, Position::Bottom).unwrap();
    assert_eq!(c.get(&b).unwrap().parent_id.as_ref(), Some(&a));
    let order = |c: &Client| {
        let mut v = c.list(Some(&a)).unwrap();
        v.sort_by_key(|n| (n.priority, n.id.clone()));
        v.into_iter().map(|n| n.id).collect::<Vec<_>>()
    };
    assert_eq!(order(&c), vec![c1.clone(), b.clone()]);
    c.move_node(&b, &a, Position::Top).unwrap();
    assert_eq!(order(&c), vec![b.clone(), c1.clone()]);

    // complete / uncomplete
    c.complete(&b).unwrap();
    let nb = c.get(&b).unwrap();
    assert!(nb.completed);
    assert!(nb.completed_at.is_some());
    c.uncomplete(&b).unwrap();
    let nb = c.get(&b).unwrap();
    assert!(!nb.completed);
    assert_eq!(nb.completed_at, None);

    // delete takes the subtree
    c.delete(&a).unwrap();
    assert_eq!(c.get(&a), Err(ApiError::ItemNotFound));
    assert_eq!(c.get(&b), Err(ApiError::ItemNotFound));
    assert_eq!(c.list(Some(&root)).unwrap(), vec![]);
    // Deleting twice: a 404 on a first attempt is an error, not success.
    assert_eq!(c.delete(&a), Err(ApiError::ItemNotFound));
}

fn export_parses_a_tree(mock: &MockServer) {
    let c = Client::new(mock.client_config());
    let root = top_level(mock, "Projects");
    let p1 = create(
        &c,
        &root,
        "Plan",
        Some("a note\nwith two lines"),
        Some("h1"),
    );
    let p2 = create(&c, &root, "Done thing", Some(""), Some("todo"));
    let p3 = create(&c, &root, "Board", None, Some("board"));
    let q1 = create(&c, &p1, "step one", None, None);
    let q2 = create(&c, &p1, "step two", Some("n"), Some("p"));
    let r1 = create(&c, &q2, "deep", None, Some("code-block"));
    c.complete(&p2).unwrap();
    c.complete(&r1).unwrap();
    c.move_node(&q2, &p1, Position::Top).unwrap();

    let nodes = c.export().unwrap();
    assert_eq!(nodes.len(), 7);
    let tree = Tree::from_nodes(nodes).expect("export forms a tree");
    assert_eq!(tree.tops(), vec![root.clone()]);
    assert_eq!(tree.children(&root), &[p1.clone(), p2.clone(), p3.clone()]);
    assert_eq!(tree.children(&p1), &[q2.clone(), q1.clone()]);
    assert_eq!(tree.children(&q2), std::slice::from_ref(&r1));

    let n = |id: &NodeId| tree.get(id).unwrap().clone();
    assert_eq!(n(&root).parent_id, None);
    assert_eq!(n(&p1).note.as_deref(), Some("a note\nwith two lines"));
    assert_eq!(n(&p1).layout, "h1");
    assert!(!n(&p1).completed);
    assert_eq!(n(&p2).note.as_deref(), Some(""));
    assert!(n(&p2).completed && n(&p2).completed_at.is_some());
    assert_eq!(n(&p2).layout, "todo");
    assert_eq!(n(&p3).note, None);
    assert_eq!(n(&p3).layout, "board");
    assert_eq!(n(&q1).layout, "bullets");
    assert_eq!(n(&q2).layout, "p");
    assert_eq!(n(&r1).layout, "code-block");
    assert!(n(&r1).completed);
    assert_eq!(n(&r1).parent_id.as_ref(), Some(&q2));
    assert!(n(&r1).created_at > 0 && n(&r1).modified_at.is_some());

    // The export agrees with GET node by node.
    for id in tree.ids() {
        assert_eq!(&c.get(id).unwrap(), tree.get(id).unwrap());
    }
}

fn error_mapping(mock: &MockServer) {
    let c = Client::new(mock.client_config());
    let root = top_level(mock, "root");
    let child = create(&c, &root, "child", None, None);
    let missing = NodeId::parse("00000000-0000-4000-8000-000000000000").unwrap();

    // Bad key: both 401 body shapes (GET vs POST/DELETE) map to Unauthorized.
    let mut bad = mock.client_config();
    bad.api_key = "wrong-key".into();
    let bad = Client::new(bad);
    assert_eq!(bad.get(&root), Err(ApiError::Unauthorized));
    assert_eq!(bad.export(), Err(ApiError::Unauthorized));
    assert_eq!(
        bad.update(&root, &UpdateRequest::default()),
        Err(ApiError::Unauthorized)
    );
    assert_eq!(bad.delete(&root), Err(ApiError::Unauthorized));
    assert_eq!(bad.request_count(), 4, "4xx is never retried");

    // Missing ids.
    let before = c.request_count();
    assert_eq!(c.get(&missing), Err(ApiError::ItemNotFound));
    assert_eq!(c.request_count(), before + 1, "404 is not retried");
    assert_eq!(
        c.update(&missing, &UpdateRequest::default()),
        Err(ApiError::ItemNotFound)
    );
    assert_eq!(c.complete(&missing), Err(ApiError::ItemNotFound));
    assert_eq!(c.delete(&missing), Err(ApiError::ItemNotFound));
    assert_eq!(c.list(Some(&missing)), Err(ApiError::ParentNotFound));
    assert_eq!(
        c.move_node(&child, &missing, Position::Top),
        Err(ApiError::ParentNotFound)
    );
    assert_eq!(
        c.create(&CreateRequest {
            parent_id: missing.clone(),
            name: "x".into(),
            note: None,
            layout: None,
            position: Position::Bottom,
        }),
        Err(ApiError::ParentNotFound)
    );
    assert_eq!(
        c.request_raw("GET", "/api/v1/nodes", &[("parent_id", "nope")], None),
        Err(ApiError::LocationNotFound("nope".into()))
    );

    // Invalid JSON body.
    let before = c.request_count();
    assert_eq!(
        c.request_raw("POST", "/api/v1/nodes", &[], Some(b"{not json")),
        Err(ApiError::InvalidJson)
    );
    assert_eq!(c.request_count(), before + 1);

    // Validation errors: all collected, in declaration order (DM§2.5).
    match c.request_raw(
        "POST",
        "/api/v1/nodes",
        &[],
        Some(br#"{"name": 1, "parent_id": "None", "position": "middle"}"#),
    ) {
        Err(ApiError::Validation(errs)) => {
            let kinds: Vec<&str> = errs.iter().map(|e| e.kind.as_str()).collect();
            assert_eq!(kinds, vec!["string_type", "enum"]);
            assert_eq!(errs[0].loc, vec![json!("name")]);
            assert_eq!(errs[0].msg, "Input should be a valid string");
            assert_eq!(errs[1].loc, vec![json!("position")]);
        }
        other => panic!("expected validation errors, got {other:?}"),
    }

    // invalid_request: the mock refuses a move under the node's own descendant.
    match c.move_node(&root, &child, Position::Top) {
        Err(ApiError::InvalidRequest(msg)) => assert!(msg.contains("descendant"), "{msg}"),
        other => panic!("expected invalid_request, got {other:?}"),
    }

    // Unknown route: HTML 404.
    assert!(matches!(
        c.request_raw("GET", "/api/v1/nope", &[], None),
        Err(ApiError::Unexpected { status: 404, .. })
    ));
}

fn short_ids_are_refused_for_mutations(mock: &MockServer) {
    let c = Client::new(mock.client_config());
    let root = top_level(mock, "root");
    let child = create(&c, &root, "child", None, None);
    let short = NodeId::from_api(child.short().unwrap().as_str());
    let upper = NodeId::from_api(child.as_str().to_ascii_uppercase());
    let before = c.request_count();

    for id in [&short, &upper, &NodeId::temp(3)] {
        let refused = ApiError::NotFullUuid(id.as_str().to_owned());
        assert_eq!(c.delete(id), Err(refused.clone()));
        assert_eq!(c.complete(id), Err(refused.clone()));
        assert_eq!(c.uncomplete(id), Err(refused.clone()));
        assert_eq!(
            c.update(id, &UpdateRequest::default()),
            Err(refused.clone())
        );
        assert_eq!(c.move_node(id, &root, Position::Top), Err(refused.clone()));
        assert_eq!(c.move_node(&child, id, Position::Top), Err(refused.clone()));
        assert_eq!(
            c.create(&CreateRequest {
                parent_id: id.clone(),
                name: "x".into(),
                note: None,
                layout: None,
                position: Position::Top,
            }),
            Err(refused.clone())
        );
        assert_eq!(c.get(id), Err(refused.clone()));
        assert_eq!(c.list(Some(id)), Err(refused));
    }
    assert_eq!(c.request_count(), before, "nothing was sent");
    // The node is untouched, and GET by short id still works.
    assert_eq!(c.get_short(&child.short().unwrap()).unwrap().id, child);
}

fn server_errors_are_retried_then_typed(mock: &MockServer) {
    let mut cfg = mock.client_config();
    cfg.max_retries = 2;
    cfg.backoff = Duration::from_millis(1);
    let c = Client::new(cfg);
    let root = top_level(mock, "root");
    // A NUL in the create note cannot be stored: 500, nothing created (DM§2.6).
    let before = c.request_count();
    let r = c.create(&CreateRequest {
        parent_id: root.clone(),
        name: "x".into(),
        note: Some("a\u{0}b".into()),
        layout: None,
        position: Position::Bottom,
    });
    match r {
        Err(ApiError::Server { status: 500, body }) => assert!(body.contains("internal_error")),
        other => panic!("expected a 500, got {other:?}"),
    }
    assert_eq!(c.request_count(), before + 3, "first attempt + 2 retries");
    assert_eq!(c.list(Some(&root)).unwrap(), vec![]);
    // The same through update's layoutMode.
    let child = create(&c, &root, "child", None, None);
    let before = c.request_count();
    assert!(matches!(
        c.update(
            &child,
            &UpdateRequest {
                layout: Some("a\u{0}".into()),
                ..Default::default()
            }
        ),
        Err(ApiError::Server { status: 500, .. })
    ));
    assert_eq!(c.request_count(), before + 3);
}

fn request_log_is_written_without_the_key(mock: &MockServer) {
    let log = temp_path("requests.jsonl");
    let mut cfg = mock.client_config();
    cfg.request_log = Some(log.clone());
    let c = Client::new(cfg);
    let root = top_level(mock, "root");
    let id = create(&c, &root, "logged", Some("n"), None);
    let _ = c.list(Some(&root)).unwrap();
    let _ = c.get(&NodeId::parse("00000000-0000-4000-8000-000000000000").unwrap());
    let text = std::fs::read_to_string(&log).unwrap();
    std::fs::remove_file(&log).ok();
    assert!(!text.contains("test-key"), "the key is never logged");
    let recs: Vec<Value> = text
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(recs.len(), 3);
    assert_eq!(recs[0]["method"], "POST");
    assert_eq!(recs[0]["path"], "/api/v1/nodes");
    assert_eq!(
        recs[0]["request_json"],
        json!({"name": "logged", "parent_id": root.as_str(), "note": "n", "position": "bottom"})
    );
    assert_eq!(recs[0]["status"], 200);
    assert_eq!(recs[0]["response"]["item_id"], id.as_str());
    assert_eq!(recs[0]["attempt"], 0);
    assert!(recs[0]["ts"].as_f64().unwrap() > 1.0e9);
    assert!(recs[0]["elapsed"].is_number());
    assert_eq!(recs[1]["method"], "GET");
    assert_eq!(recs[1]["query"], json!({"parent_id": root.as_str()}));
    assert_eq!(recs[2]["status"], 404);
    assert_eq!(recs[2]["response"]["message"], "Item not found");
}

fn rate_limit_is_measurable(mock: &MockServer) {
    let root = top_level(mock, "root");
    let mut cfg = mock.client_config();
    cfg.min_interval = Duration::from_millis(100);
    let limited = Client::new(cfg);
    let t = Instant::now();
    for _ in 0..25 {
        limited.get(&root).unwrap();
    }
    let slow = t.elapsed();
    // 25 requests at 10/s: 24 full intervals after the first.
    assert!(slow >= Duration::from_millis(2400), "{slow:?}");

    let free = Client::new(mock.client_config());
    let t = Instant::now();
    for _ in 0..25 {
        free.get(&root).unwrap();
    }
    let fast = t.elapsed();
    assert!(fast < Duration::from_millis(2000), "{fast:?}");
    assert!(fast < slow);
}

fn export_min_interval_is_respected(mock: &MockServer) {
    let mut cfg = mock.client_config();
    cfg.export_min_interval = Duration::from_millis(400);
    let c = Client::new(cfg);
    let t = Instant::now();
    c.export().unwrap();
    c.export().unwrap();
    assert!(
        t.elapsed() >= Duration::from_millis(400),
        "{:?}",
        t.elapsed()
    );
    // Other requests are not held back by it.
    let t = Instant::now();
    c.list(None).unwrap();
    assert!(t.elapsed() < Duration::from_millis(400));
}

// ---------------------------------------------------------------------------------------
// Transport errors: a port that refuses connections.
// ---------------------------------------------------------------------------------------

fn refused_base_url() -> String {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    drop(l);
    format!("http://127.0.0.1:{port}")
}

fn quick_config(base_url: String, retries: u32) -> ClientConfig {
    let mut cfg = ClientConfig::new("k");
    cfg.base_url = base_url;
    cfg.min_interval = Duration::ZERO;
    cfg.max_retries = retries;
    cfg.backoff = Duration::from_millis(5);
    cfg.timeout = Duration::from_secs(5);
    cfg
}

#[test]
fn transport_errors_are_retried_then_typed() {
    let c = Client::new(quick_config(refused_base_url(), 3));
    let id = NodeId::parse("5e2e76e4-fea4-4f1f-9c5b-a846cfce4c6d").unwrap();
    let t = Instant::now();
    match c.get(&id) {
        Err(ApiError::Transport { maybe_applied, .. }) => assert!(!maybe_applied),
        other => panic!("expected a transport error, got {other:?}"),
    }
    assert_eq!(c.request_count(), 4, "first attempt + 3 retries");
    // Backoff 5, 10, 20 ms.
    assert!(t.elapsed() >= Duration::from_millis(35));

    // A refused connection provably sent nothing, so even create is retried and
    // reported as not applied.
    let r = c.create(&CreateRequest {
        parent_id: id.clone(),
        name: "x".into(),
        note: None,
        layout: None,
        position: Position::Bottom,
    });
    assert!(matches!(
        r,
        Err(ApiError::Transport {
            maybe_applied: false,
            ..
        })
    ));
    assert_eq!(c.request_count(), 8);
    assert!(matches!(c.delete(&id), Err(ApiError::Transport { .. })));
    assert_eq!(c.request_count(), 12);
}

// ---------------------------------------------------------------------------------------
// A scripted fake HTTP server: exact wire format, retry policy per status.
// ---------------------------------------------------------------------------------------

/// What the fake server does with one connection.
#[derive(Clone)]
enum Reply {
    /// Answer with this status and body (and close).
    Status(u16, &'static str),
    /// Read the request, then close without answering.
    Hangup,
}

#[derive(Debug, Clone)]
struct Recorded {
    request_line: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Recorded {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
    fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap()
    }
}

struct Fake {
    base_url: String,
    seen: Arc<Mutex<Vec<Recorded>>>,
}

fn read_request(stream: &mut TcpStream) -> Recorded {
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut request_line = String::new();
    reader.read_line(&mut request_line).unwrap();
    let mut headers = Vec::new();
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        let (k, v) = line.split_once(':').unwrap();
        headers.push((k.trim().to_owned(), v.trim().to_owned()));
    }
    let len: usize = headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
        .map(|(_, v)| v.parse().unwrap())
        .unwrap_or(0);
    let mut body = vec![0; len];
    reader.read_exact(&mut body).unwrap();
    Recorded {
        request_line: request_line.trim_end().to_owned(),
        headers,
        body,
    }
}

fn fake_server(script: Vec<Reply>) -> Fake {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let seen = Arc::new(Mutex::new(Vec::new()));
    let seen2 = Arc::clone(&seen);
    thread::spawn(move || {
        for reply in script {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            let rec = read_request(&mut stream);
            seen2.lock().unwrap().push(rec);
            match reply {
                Reply::Status(status, body) => {
                    let resp = format!(
                        "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = stream.write_all(resp.as_bytes());
                }
                Reply::Hangup => {}
            }
            let _ = stream.shutdown(std::net::Shutdown::Both);
        }
    });
    Fake { base_url, seen }
}

const ID: &str = "5e2e76e4-fea4-4f1f-9c5b-a846cfce4c6d";
const PARENT: &str = "019daf85-288f-4404-bbf8-5f8f829dec96";

#[test]
fn wire_format_of_every_endpoint() {
    let export = r#"{"nodes": [{"id": "5e2e76e4-fea4-4f1f-9c5b-a846cfce4c6d", "name": "a", "note": null, "parent_id": null, "priority": 1, "completed": false, "data": {"layoutMode": "bullets"}, "createdAt": 1, "modifiedAt": null, "completedAt": null}]}"#;
    let node = r#"{"node": {"id": "5e2e76e4-fea4-4f1f-9c5b-a846cfce4c6d", "name": "a", "note": "x", "parent_id": "019daf85-288f-4404-bbf8-5f8f829dec96", "priority": 1, "completed": true, "data": {"layoutMode": "todo"}, "createdAt": 1, "modifiedAt": 2, "completedAt": 2}}"#;
    let ok = r#"{"status": "ok"}"#;
    let fake = fake_server(vec![
        Reply::Status(200, export),
        Reply::Status(200, node),
        Reply::Status(200, node),
        Reply::Status(200, r#"{"nodes": []}"#),
        Reply::Status(200, r#"{"nodes": []}"#),
        Reply::Status(
            200,
            r#"{"item_id": "5e2e76e4-fea4-4f1f-9c5b-a846cfce4c6d"}"#,
        ),
        Reply::Status(200, ok),
        Reply::Status(200, ok),
        Reply::Status(200, ok),
        Reply::Status(200, ok),
        Reply::Status(200, ok),
    ]);
    let c = Client::new(quick_config(fake.base_url.clone(), 0));
    let id = NodeId::parse(ID).unwrap();
    let parent = NodeId::parse(PARENT).unwrap();

    let nodes = c.export().unwrap();
    assert_eq!(nodes.len(), 1);
    assert_eq!(nodes[0].modified_at, None);
    let n = c.get(&id).unwrap();
    assert!(n.completed);
    assert_eq!(n.layout, "todo");
    assert_eq!(n.parent_id.as_ref(), Some(&parent));
    c.get_short(&id.short().unwrap()).unwrap();
    assert_eq!(c.list(Some(&parent)).unwrap(), vec![]);
    assert_eq!(c.list(None).unwrap(), vec![]);
    let new = c
        .create(&CreateRequest {
            parent_id: parent.clone(),
            name: "".into(),
            note: Some("⟳nonce".into()),
            layout: Some("todo".into()),
            position: Position::Bottom,
        })
        .unwrap();
    assert_eq!(new, id);
    c.update(
        &id,
        &UpdateRequest {
            name: Some("n".into()),
            ..Default::default()
        },
    )
    .unwrap();
    c.move_node(&id, &parent, Position::Top).unwrap();
    c.complete(&id).unwrap();
    c.uncomplete(&id).unwrap();
    c.delete(&id).unwrap();

    let seen = fake.seen.lock().unwrap().clone();
    let lines: Vec<&str> = seen.iter().map(|r| r.request_line.as_str()).collect();
    assert_eq!(
        lines,
        vec![
            "GET /api/v1/nodes-export HTTP/1.1".to_owned(),
            format!("GET /api/v1/nodes/{ID} HTTP/1.1"),
            format!("GET /api/v1/nodes/{} HTTP/1.1", id.short().unwrap()),
            format!("GET /api/v1/nodes?parent_id={PARENT} HTTP/1.1"),
            "GET /api/v1/nodes?parent_id=None HTTP/1.1".to_owned(),
            "POST /api/v1/nodes HTTP/1.1".to_owned(),
            format!("POST /api/v1/nodes/{ID} HTTP/1.1"),
            format!("POST /api/v1/nodes/{ID}/move HTTP/1.1"),
            format!("POST /api/v1/nodes/{ID}/complete HTTP/1.1"),
            format!("POST /api/v1/nodes/{ID}/uncomplete HTTP/1.1"),
            format!("DELETE /api/v1/nodes/{ID} HTTP/1.1"),
        ]
    );
    for r in &seen {
        assert_eq!(r.header("authorization"), Some("Bearer k"), "{r:?}");
    }
    // JSON bodies with an explicit content type; null is never sent.
    let create = &seen[5];
    assert_eq!(create.header("content-type"), Some("application/json"));
    assert_eq!(
        create.json(),
        json!({"name": "", "parent_id": PARENT, "note": "⟳nonce", "layoutMode": "todo", "position": "bottom"})
    );
    assert_eq!(seen[6].json(), json!({"name": "n"}));
    assert_eq!(
        seen[7].json(),
        json!({"parent_id": PARENT, "position": "top"})
    );
    // complete, uncomplete, delete and GETs carry no body.
    for i in [0, 1, 8, 9, 10] {
        assert!(seen[i].body.is_empty(), "{:?}", seen[i]);
    }
    assert_eq!(c.request_count(), 11);
}

#[test]
fn retries_5xx_and_429_but_not_4xx() {
    let fake = fake_server(vec![
        Reply::Status(503, "<html>busy</html>"),
        Reply::Status(429, ""),
        Reply::Status(200, r#"{"nodes": []}"#),
        Reply::Status(
            400,
            r#"{"code": "invalid_json", "message": "Invalid JSON format in request body"}"#,
        ),
        Reply::Status(502, "a"),
        Reply::Status(502, "b"),
        Reply::Status(502, "c"),
    ]);
    let c = Client::new(quick_config(fake.base_url.clone(), 2));
    assert_eq!(c.list(None).unwrap(), vec![]);
    assert_eq!(c.request_count(), 3);
    let id = NodeId::parse(ID).unwrap();
    assert_eq!(c.complete(&id), Err(ApiError::InvalidJson));
    assert_eq!(c.request_count(), 4, "400 is not retried");
    assert_eq!(
        c.list(None),
        Err(ApiError::Server {
            status: 502,
            body: "c".into()
        })
    );
    assert_eq!(c.request_count(), 7);
}

#[test]
fn create_is_retried_on_5xx_but_not_after_a_maybe_sent_transport_error() {
    let fake = fake_server(vec![
        Reply::Status(
            500,
            r#"{"code": "internal_error", "message": "An internal server error occurred"}"#,
        ),
        Reply::Status(
            200,
            r#"{"item_id": "5e2e76e4-fea4-4f1f-9c5b-a846cfce4c6d"}"#,
        ),
        Reply::Hangup,
        Reply::Hangup,
        Reply::Status(200, r#"{"nodes": []}"#),
    ]);
    let c = Client::new(quick_config(fake.base_url.clone(), 3));
    let req = CreateRequest {
        parent_id: NodeId::parse(PARENT).unwrap(),
        name: "x".into(),
        note: None,
        layout: None,
        position: Position::Bottom,
    };
    assert_eq!(c.create(&req).unwrap().as_str(), ID);
    assert_eq!(c.request_count(), 2);
    // The server read the request and hung up: it may have created the node (§7.3).
    match c.create(&req) {
        Err(ApiError::Transport { maybe_applied, .. }) => assert!(maybe_applied),
        other => panic!("expected a maybe-applied transport error, got {other:?}"),
    }
    assert_eq!(c.request_count(), 3, "not retried");
    // An idempotent request retries over the same failure.
    assert_eq!(c.list(None).unwrap(), vec![]);
    assert_eq!(c.request_count(), 5);
}

#[test]
fn delete_treats_404_on_a_retry_as_success() {
    let fake = fake_server(vec![
        Reply::Hangup,
        Reply::Status(404, r#"{"code": "not_found", "message": "Item not found"}"#),
    ]);
    let c = Client::new(quick_config(fake.base_url.clone(), 3));
    c.delete(&NodeId::parse(ID).unwrap()).unwrap();
    assert_eq!(c.request_count(), 2);
}

#[test]
fn decode_errors_are_typed() {
    let fake = fake_server(vec![
        Reply::Status(200, "not json"),
        Reply::Status(200, r#"{"node": 1}"#),
    ]);
    let c = Client::new(quick_config(fake.base_url.clone(), 0));
    assert!(matches!(c.export(), Err(ApiError::Decode(_))));
    assert!(matches!(
        c.get(&NodeId::parse(ID).unwrap()),
        Err(ApiError::Decode(_))
    ));
}
