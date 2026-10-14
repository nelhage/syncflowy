//! The mock-server harness works: it starts, serves the API, simulates UI edits, and
//! resets.

use serde_json::json;
use syncflowy::testsupport::MockServer;
use syncflowy::tree::{ApiNode, NodeId, Tree};

#[test]
fn mock_server_starts_creates_and_resets() {
    let mock = MockServer::start();
    assert!(mock.base_url().starts_with("http://127.0.0.1:"));
    assert_eq!(mock.api_key(), "test-key");

    // Create a top-level node through the API.
    let (status, body) = mock.post(
        "/api/v1/nodes",
        &json!({"parent_id": "None", "name": "hello", "position": "bottom"}),
    );
    assert_eq!(status, 200, "{body}");
    let id = NodeId::parse(body["item_id"].as_str().expect("item_id")).expect("full UUID");

    // It is visible through GET and the export, and parses into a Tree.
    let (status, body) = mock.get(&format!("/api/v1/nodes/{id}"));
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["node"]["name"], "hello");
    let (status, body) = mock.get("/api/v1/nodes-export");
    assert_eq!(status, 200);
    let nodes: Vec<ApiNode> =
        serde_json::from_value(body["nodes"].clone()).expect("export decodes");
    let tree = Tree::from_api(nodes).expect("tree");
    assert_eq!(tree.get(&id).map(|n| n.name.as_str()), Some("hello"));

    // A simulated UI edit escapes text like the web UI (UI:E1).
    mock.ui("edit", json!({"id": id.as_str(), "text": "AT&T"}));
    let (_, body) = mock.get(&format!("/api/v1/nodes/{id}"));
    assert_eq!(body["node"]["name"], "AT&amp;T");

    // Short ids work on GET only (DM§3.1).
    let short = id.short().expect("short id");
    let (status, _) = mock.get(&format!("/api/v1/nodes/{short}"));
    assert_eq!(status, 200);
    let (status, _) = mock.delete(&format!("/api/v1/nodes/{short}"));
    assert_eq!(status, 404);

    // The dump shows the node; reset empties the outline.
    let dump = mock.dump();
    assert_eq!(dump["nodes"].as_array().map(Vec::len), Some(1));
    mock.reset();
    let (status, body) = mock.get("/api/v1/nodes-export");
    assert_eq!(status, 200);
    assert_eq!(body["nodes"], json!([]));

    // Seeded ids: the same create after a reset yields the same id.
    let (_, body) = mock.post(
        "/api/v1/nodes",
        &json!({"parent_id": "None", "name": "hello", "position": "bottom"}),
    );
    assert_eq!(body["item_id"].as_str(), Some(id.as_str()));
}
