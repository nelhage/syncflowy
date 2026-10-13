//! The state store: checkpoints, render history, pending deletes, atomic writes,
//! fingerprints and the per-root lock (design §3, §5.1, §9.1 guards 1–3).

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use proptest::prelude::*;
use serde_json::{Value, json};
use syncflowy::mdblock::DocConfig;
use syncflowy::state::{
    CHECKPOINT_VERSION, Checkpoint, Fingerprint, History, HistoryEntry, HistoryLimits,
    PendingDeletes, StateError, StateStore, atomic_write, fingerprint,
};
use syncflowy::testsupport::MockServer;
use syncflowy::tree::{ApiNode, IdMap, Node, NodeId, Tree};

const ROOT: &str = "6f1b1a2e-1111-4222-8333-944455556666";

fn id(n: u32) -> NodeId {
    NodeId::parse(&format!("00000000-0000-4000-8000-{n:012x}")).unwrap()
}

fn root() -> NodeId {
    NodeId::parse(ROOT).unwrap()
}

/// A small tree under ROOT with WF-HTML, notes, a todo, a completed node with a hidden
/// child (hidden nodes must be kept, §4.5) and a node with `note: Some("")`.
fn sample_tree() -> Tree {
    let r = root();
    let mut nodes = vec![Node::new(r.clone(), None, "Projects")];
    let mut a = Node::new(id(1), Some(r.clone()), "<b>Plan</b> &amp; ship");
    a.note = Some("line 1\nline 2 &lt;x&gt;".into());
    a.priority = 100;
    a.created_at = 1_700_000_000;
    a.modified_at = Some(1_700_000_100);
    let mut b = Node::new(id(2), Some(r.clone()), "Done thing");
    b.completed = true;
    b.completed_at = Some(1_700_000_200);
    b.priority = 200;
    let c = Node::new(id(3), Some(id(2)), "hidden child");
    let mut d = Node::new(id(4), Some(r.clone()), "Todo list ✓ ü");
    d.layout = "todo".into();
    d.priority = 300;
    d.note = Some(String::new());
    let mut e = Node::new(
        id(5),
        Some(id(4)),
        "<a href=\"https://x.test/?a=1&amp;b=2\">link</a>",
    );
    e.priority = 5;
    nodes.extend([a, b, c, d, e]);
    Tree::from_nodes(nodes).unwrap()
}

fn sample_md() -> String {
    "---\nworkflowy: 944455556666\ngen: 7\n---\n\n- **Plan** & ship %%wf:000000000001%%\n  line 1\n- [ ] Todo list ✓ ü\n".to_owned()
}

fn sample_checkpoint(generation: u64) -> Checkpoint {
    let mut cp = Checkpoint::new(
        root(),
        generation,
        sample_tree(),
        sample_md(),
        DocConfig::default(),
        1_700_000_500,
    );
    cp.file = Some("vault/Projects.md".into());
    cp
}

fn store() -> (tempfile::TempDir, StateStore) {
    let dir = tempfile::tempdir().unwrap();
    let st = StateStore::open(&dir.path().join("state")).unwrap();
    (dir, st)
}

fn entry(generation: u64, at: i64, tree: Tree) -> HistoryEntry {
    HistoryEntry {
        generation,
        md: format!("render {generation}\n"),
        tree,
        at,
    }
}

// ---------------------------------------------------------------------------------------
// Checkpoints
// ---------------------------------------------------------------------------------------

#[test]
fn checkpoint_round_trip() {
    let (_d, st) = store();
    assert_eq!(st.load_checkpoint(&root()).unwrap(), None);
    assert!(st.roots().unwrap().is_empty());

    let cp = sample_checkpoint(7);
    assert_eq!(cp.version, CHECKPOINT_VERSION);
    assert_eq!(cp.md_fingerprint, fingerprint(&cp.base_md));
    assert_eq!(cp.ids, IdMap::from_tree(&cp.base_wf));
    st.save_checkpoint(&cp).unwrap();

    let loaded = st.load_checkpoint(&root()).unwrap().unwrap();
    assert_eq!(loaded, cp);
    assert!(loaded.md_unchanged(&sample_md()));
    assert!(!loaded.md_unchanged(&(sample_md() + " ")));
    // Hidden nodes survive (§4.5).
    assert!(loaded.base_wf.contains(&id(3)));
    assert_eq!(st.roots().unwrap(), vec![root()]);

    // The layout of §"Layout".
    let rd = st.root_dir(&root());
    assert_eq!(rd, st.dir().join("roots").join(ROOT));
    assert_eq!(st.checkpoint_path(&root()), rd.join("checkpoint.json"));
    assert_eq!(st.pending_path(&root()), rd.join("pending.json"));
    assert_eq!(st.journal_path(&root()), rd.join("journal.jsonl"));
    assert_eq!(
        st.history_path(&root(), 12),
        rd.join("history").join("12.json")
    );
    assert!(st.checkpoint_path(&root()).is_file());

    // Pretty JSON with the documented fields.
    let text = fs::read_to_string(st.checkpoint_path(&root())).unwrap();
    assert!(text.contains("\n  \"version\": 1,"), "{text}");
    let v: Value = serde_json::from_str(&text).unwrap();
    for k in [
        "root",
        "generation",
        "base_wf",
        "base_md",
        "md_fingerprint",
        "ids",
        "config",
    ] {
        assert!(v.get(k).is_some(), "missing {k}");
    }

    // Saving again replaces it.
    let mut cp2 = cp.clone();
    cp2.generation = 8;
    cp2.set_base_md("changed\n".into());
    st.save_checkpoint(&cp2).unwrap();
    let loaded = st.load_checkpoint(&root()).unwrap().unwrap();
    assert_eq!(loaded.generation, 8);
    assert_eq!(loaded.md_fingerprint, fingerprint("changed\n"));
}

#[test]
fn checkpoint_round_trip_with_exported_tree() {
    // A realistic tree: what the API's export returns, built through the mock (with
    // UI-escaped text, notes, a completed node with a hidden child, and a todo list).
    let mock = MockServer::start();
    let post = |parent: &str, name: &str, note: Option<&str>| -> String {
        let mut body = json!({"parent_id": parent, "name": name, "position": "bottom"});
        if let Some(n) = note {
            body["note"] = json!(n);
        }
        let (status, b) = mock.post("/api/v1/nodes", &body);
        assert_eq!(status, 200, "{b}");
        b["item_id"].as_str().unwrap().to_owned()
    };
    let r = post("None", "Sync root", None);
    let a = post(
        &r,
        "<b>bold</b> and <i>it</i>",
        Some("a note\nwith two lines"),
    );
    let b = post(&r, "will be completed", None);
    let _c = post(&b, "hidden under completed", None);
    let d = post(&r, "todo list", Some(""));
    let _e = post(&d, "item", None);
    let _ = post("None", "outside the root", None);
    mock.ui("edit", json!({"id": a, "text": "AT&T <tag> \u{1F600}"}));
    let (status, _) = mock.post(&format!("/api/v1/nodes/{b}/complete"), &json!({}));
    assert_eq!(status, 200);
    let (status, _) = mock.post(
        &format!("/api/v1/nodes/{d}"),
        &json!({"layoutMode": "todo"}),
    );
    assert_eq!(status, 200);

    let (status, body) = mock.get("/api/v1/nodes-export");
    assert_eq!(status, 200);
    let nodes: Vec<ApiNode> = serde_json::from_value(body["nodes"].clone()).unwrap();
    let full = Tree::from_api(nodes).unwrap();
    let rid = NodeId::parse(&r).unwrap();
    let sub = full.subtree(&rid).unwrap();
    assert_eq!(sub.len(), 6);
    assert!(sub.get(&NodeId::parse(&b).unwrap()).unwrap().completed);

    let (_d, st) = store();
    let cp = Checkpoint::new(
        rid.clone(),
        1,
        sub,
        "# x\n".into(),
        DocConfig::default(),
        42,
    );
    st.save_checkpoint(&cp).unwrap();
    let loaded = st.load_checkpoint(&rid).unwrap().unwrap();
    assert_eq!(loaded, cp);
    assert_eq!(loaded.base_wf.to_api(), cp.base_wf.to_api());
    let short = NodeId::parse(&a).unwrap().short().unwrap();
    assert_eq!(loaded.ids.get(&short), Some(&NodeId::parse(&a).unwrap()));
}

#[test]
fn version_mismatch_is_a_clear_error() {
    let (_d, st) = store();
    st.save_checkpoint(&sample_checkpoint(1)).unwrap();
    let path = st.checkpoint_path(&root());

    // A newer format, whose other fields this engine could not even decode.
    let mut v: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    v["version"] = json!(2);
    v["base_wf"] = json!("a new encoding");
    fs::write(&path, serde_json::to_vec(&v).unwrap()).unwrap();
    match st.load_checkpoint(&root()) {
        Err(StateError::Version {
            path: p,
            found,
            expected,
        }) => {
            assert_eq!((found, expected), (2, CHECKPOINT_VERSION));
            assert_eq!(p, path);
        }
        other => panic!("expected a version error, got {other:?}"),
    }
    let msg = st.load_checkpoint(&root()).unwrap_err().to_string();
    assert!(msg.contains("checkpoint version 2, expected 1"), "{msg}");

    // An older format is rejected too (no silent migration).
    v["version"] = json!(0);
    fs::write(&path, serde_json::to_vec(&v).unwrap()).unwrap();
    assert!(matches!(
        st.load_checkpoint(&root()),
        Err(StateError::Version { found: 0, .. })
    ));

    // No version at all, or not JSON: a JSON error naming the file.
    v.as_object_mut().unwrap().remove("version");
    fs::write(&path, serde_json::to_vec(&v).unwrap()).unwrap();
    assert!(matches!(
        st.load_checkpoint(&root()),
        Err(StateError::Json { .. })
    ));
    fs::write(&path, b"{ not json").unwrap();
    let err = st.load_checkpoint(&root()).unwrap_err();
    assert!(matches!(err, StateError::Json { .. }));
    assert!(err.to_string().contains("checkpoint.json"), "{err}");
}

#[test]
fn checkpoint_for_another_root_is_rejected() {
    let (_d, st) = store();
    st.save_checkpoint(&sample_checkpoint(1)).unwrap();
    let other = id(99);
    fs::create_dir_all(st.root_dir(&other)).unwrap();
    fs::copy(st.checkpoint_path(&root()), st.checkpoint_path(&other)).unwrap();
    assert!(matches!(
        st.load_checkpoint(&other),
        Err(StateError::Json { .. })
    ));
}

#[test]
fn checkpoint_is_forward_compatible() {
    let (_d, st) = store();
    let cp = sample_checkpoint(3);
    st.save_checkpoint(&cp).unwrap();
    let path = st.checkpoint_path(&root());
    let mut v: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    let o = v.as_object_mut().unwrap();
    // Fields a newer engine might add are ignored.
    o.insert("future_field".into(), json!({"x": [1, 2]}));
    // Optional fields an older engine did not write get defaults.
    for k in [
        "md_fingerprint",
        "ids",
        "file",
        "synced_at",
        "created_at",
        "pending_deletes",
    ] {
        o.remove(k);
    }
    // A config this engine cannot decode falls back to the default.
    o.insert("config".into(), json!({"headings": "not a number"}));
    fs::write(&path, serde_json::to_vec(&v).unwrap()).unwrap();
    fs::remove_file(st.pending_path(&root())).unwrap();

    let loaded = st.load_checkpoint(&root()).unwrap().unwrap();
    assert_eq!(loaded.base_wf, cp.base_wf);
    assert_eq!(loaded.base_md, cp.base_md);
    assert_eq!(loaded.md_fingerprint, fingerprint(&cp.base_md)); // recomputed
    assert_eq!(loaded.config, DocConfig::default());
    assert_eq!(loaded.file, None);
    assert!(loaded.ids.is_empty());
    assert!(loaded.pending_deletes.is_empty());

    // A stale fingerprint is recomputed, not trusted.
    let mut v: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    v["md_fingerprint"] = json!("00");
    fs::write(&path, serde_json::to_vec(&v).unwrap()).unwrap();
    let loaded = st.load_checkpoint(&root()).unwrap().unwrap();
    assert_eq!(loaded.md_fingerprint, fingerprint(&cp.base_md));
}

// ---------------------------------------------------------------------------------------
// Atomic writes
// ---------------------------------------------------------------------------------------

fn temp_files(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.contains(".tmp-"))
        .collect();
    v.sort();
    v
}

#[test]
fn stale_temp_files_from_a_crash_are_harmless() {
    let (_d, st) = store();
    let cp = sample_checkpoint(5);
    st.save_checkpoint(&cp).unwrap();
    st.put_history(&root(), &entry(5, 100, sample_tree()))
        .unwrap();
    let rd = st.root_dir(&root());
    let hd = st.history_dir(&root());
    assert!(temp_files(&rd).is_empty());
    assert!(temp_files(&hd).is_empty());

    // A crash mid-write: a truncated temp file next to each target, never renamed.
    let full = fs::read(st.checkpoint_path(&root())).unwrap();
    fs::write(
        rd.join(".checkpoint.json.tmp-99999-0"),
        &full[..full.len() / 2],
    )
    .unwrap();
    fs::write(rd.join(".pending.json.tmp-99999-1"), b"{\"entr").unwrap();
    fs::write(
        hd.join(".6.json.tmp-99999-2"),
        b"{\"generation\": 6, \"md\"",
    )
    .unwrap();
    // A crash in another, unrelated root directory name must not confuse roots().
    fs::create_dir_all(st.dir().join("roots").join(".tmpdir.tmp-1-1")).unwrap();

    assert_eq!(st.load_checkpoint(&root()).unwrap().unwrap(), cp);
    assert_eq!(st.history_gens(&root()).unwrap(), vec![5]);
    assert_eq!(st.get_history(&root(), 6).unwrap(), None);
    assert_eq!(st.roots().unwrap(), vec![root()]);
    assert!(st.load_pending(&root()).unwrap().is_empty());

    // Writing again works despite the leftovers, and taking the lock clears them.
    st.save_checkpoint(&sample_checkpoint(6)).unwrap();
    assert_eq!(temp_files(&rd).len(), 2);
    let lock = st.lock(&root()).unwrap();
    assert!(temp_files(&rd).is_empty());
    assert!(temp_files(&hd).is_empty());
    drop(lock);
    assert_eq!(st.load_checkpoint(&root()).unwrap().unwrap().generation, 6);
}

#[test]
fn atomic_write_replaces_whole_files() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("f.json");
    atomic_write(&p, b"first").unwrap();
    assert_eq!(fs::read(&p).unwrap(), b"first");
    atomic_write(&p, b"second, longer").unwrap();
    assert_eq!(fs::read(&p).unwrap(), b"second, longer");
    atomic_write(&p, b"").unwrap();
    assert_eq!(fs::read(&p).unwrap(), b"");
    assert!(temp_files(dir.path()).is_empty());

    // A failing write (missing directory) leaves nothing behind.
    let missing = dir.path().join("nope").join("f.json");
    assert!(atomic_write(&missing, b"x").is_err());
    assert!(!missing.exists());

    // Concurrent writers never produce a torn file: the result is one of the inputs.
    let p2 = dir.path().join("g.json");
    let payloads: Vec<Vec<u8>> = (0..8u8).map(|i| vec![b'a' + i; 64 * 1024]).collect();
    std::thread::scope(|s| {
        for pl in &payloads {
            let p2 = &p2;
            s.spawn(move || {
                for _ in 0..5 {
                    atomic_write(p2, pl).unwrap();
                }
            });
        }
    });
    let got = fs::read(&p2).unwrap();
    assert!(payloads.contains(&got));
    assert!(temp_files(dir.path()).is_empty());
}

// ---------------------------------------------------------------------------------------
// History
// ---------------------------------------------------------------------------------------

#[test]
fn history_put_get_and_list() {
    let (_d, st) = store();
    assert!(st.history_gens(&root()).unwrap().is_empty());
    assert_eq!(st.get_history(&root(), 1).unwrap(), None);
    for g in [3, 1, 10, 2] {
        st.put_history(&root(), &entry(g, g as i64, sample_tree()))
            .unwrap();
    }
    // Overwrite the same generation.
    let mut e = entry(2, 2, sample_tree());
    e.md = "rewritten\n".into();
    st.put_history(&root(), &e).unwrap();
    // Non-canonical names are not generations.
    fs::write(st.history_dir(&root()).join("007.json"), b"{}").unwrap();
    fs::write(st.history_dir(&root()).join("notes.txt"), b"").unwrap();

    assert_eq!(st.history_gens(&root()).unwrap(), vec![1, 2, 3, 10]);
    assert_eq!(st.get_history(&root(), 2).unwrap(), Some(e.clone()));
    let h = st.load_history(&root()).unwrap();
    assert_eq!(h.gens(), vec![1, 2, 3, 10]);
    assert_eq!(h.latest().unwrap().generation, 10);
    assert_eq!(h.oldest().unwrap().generation, 1);
    assert_eq!(h.get(2), Some(&e));
    assert_eq!(
        h.find_by_fingerprint(&fingerprint("rewritten\n"))
            .map(|e| e.generation),
        Some(2)
    );
    assert_eq!(h.find_by_fingerprint(&fingerprint("nope")), None);

    // A file whose content names another generation is an error, not silently used.
    fs::copy(st.history_path(&root(), 3), st.history_path(&root(), 4)).unwrap();
    assert!(matches!(
        st.get_history(&root(), 4),
        Err(StateError::Json { .. })
    ));
}

#[test]
fn history_pruned_by_count() {
    let (_d, st) = store();
    for g in 1..=10 {
        st.put_history(&root(), &entry(g, 1000 + g as i64, Tree::new()))
            .unwrap();
    }
    let limits = HistoryLimits {
        max_entries: 4,
        max_age_secs: 1_000_000,
    };
    let pruned = st.prune_history(&root(), &limits, 2000).unwrap();
    assert_eq!(pruned, vec![1, 2, 3, 4, 5, 6]);
    assert_eq!(st.history_gens(&root()).unwrap(), vec![7, 8, 9, 10]);
    assert_eq!(st.get_history(&root(), 3).unwrap(), None);
    // Idempotent.
    assert!(st.prune_history(&root(), &limits, 2000).unwrap().is_empty());

    // max_entries 0 still keeps the newest.
    let zero = HistoryLimits {
        max_entries: 0,
        max_age_secs: 1_000_000,
    };
    assert_eq!(
        st.prune_history(&root(), &zero, 2000).unwrap(),
        vec![7, 8, 9]
    );
    assert_eq!(st.history_gens(&root()).unwrap(), vec![10]);
}

#[test]
fn history_pruned_by_age_with_injected_clock() {
    let (_d, st) = store();
    let day = 24 * 3600;
    let t0 = 1_700_000_000;
    for g in 1..=5u64 {
        st.put_history(&root(), &entry(g, t0 + g as i64 * day, Tree::new()))
            .unwrap();
    }
    let limits = HistoryLimits {
        max_entries: 200,
        max_age_secs: 2 * day,
    };
    // At day 5: gens 3, 4, 5 are at most 2 days old (age exactly the limit is kept).
    assert_eq!(
        st.prune_history(&root(), &limits, t0 + 5 * day).unwrap(),
        vec![1, 2]
    );
    assert_eq!(st.history_gens(&root()).unwrap(), vec![3, 4, 5]);
    // Much later: everything is old, but the newest survives.
    assert_eq!(
        st.prune_history(&root(), &limits, t0 + 100 * day).unwrap(),
        vec![3, 4]
    );
    assert_eq!(st.history_gens(&root()).unwrap(), vec![5]);
    assert_eq!(
        st.prune_history(&root(), &limits, t0 + 1000 * day).unwrap(),
        Vec::<u64>::new()
    );
    assert_eq!(HistoryLimits::default().max_entries, 200);
    assert_eq!(HistoryLimits::default().max_age_secs, 30 * day);
}

#[test]
fn history_pruning_keeps_the_checkpoint_generation() {
    // A crash after the history entry of gen 6 but before the checkpoint: the
    // checkpoint still names gen 5, which must stay available (§9.1 guard 1).
    let (_d, st) = store();
    st.save_checkpoint(&sample_checkpoint(5)).unwrap();
    for g in 1..=6 {
        st.put_history(&root(), &entry(g, 0, Tree::new())).unwrap();
    }
    let limits = HistoryLimits {
        max_entries: 1,
        max_age_secs: 10,
    };
    assert_eq!(
        st.prune_history(&root(), &limits, 1000).unwrap(),
        vec![1, 2, 3, 4]
    );
    assert_eq!(st.history_gens(&root()).unwrap(), vec![5, 6]);
    // The explicit form keeps exactly what it is told to.
    assert_eq!(
        st.prune_history_keeping(&root(), &limits, 1000, &BTreeSet::new())
            .unwrap(),
        vec![5]
    );
}

#[test]
fn in_memory_history_prune_and_known_values() {
    let mut h = History::new();
    let t1 = sample_tree();
    let mut t2 = t1.clone();
    t2.set_name(&id(1), "Plan v2").unwrap();
    let mut t3 = t2.clone(); // unchanged node 1
    t3.set_note(&id(4), None).unwrap();
    let mut t4 = t3.clone();
    t4.set_name(&id(1), "<b>Plan</b> &amp; ship").unwrap(); // back to the first value
    let mut t5 = t4.clone();
    t5.remove_subtree(&id(1)).unwrap();
    for (g, t) in [(1, t1), (2, t2), (3, t3), (4, t4), (5, t5)] {
        h.insert(entry(g, g as i64, t));
    }

    let vs = h.known_values(&id(1));
    let summary: Vec<(u64, &str)> = vs.iter().map(|v| (v.generation, v.name.as_str())).collect();
    assert_eq!(
        summary,
        vec![
            (1, "<b>Plan</b> &amp; ship"),
            (2, "Plan v2"),
            (4, "<b>Plan</b> &amp; ship")
        ]
    );
    assert_eq!(vs[0].note.as_deref(), Some("line 1\nline 2 &lt;x&gt;"));
    assert_eq!(vs[0].parent_id, Some(root()));
    assert_eq!(
        h.known_names(&id(1)),
        vec!["<b>Plan</b> &amp; ship", "Plan v2"]
    );
    assert_eq!(h.known_notes(&id(4)), vec![Some(String::new()), None]);
    assert!(h.known_values(&id(77)).is_empty());
    // The completed node and its hidden child are known too (the trees keep them).
    assert!(h.known_values(&id(2))[0].completed);
    assert_eq!(h.known_values(&id(3)).len(), 1);

    let limits = HistoryLimits {
        max_entries: 2,
        max_age_secs: 1000,
    };
    assert_eq!(h.prune(&limits, 10, &BTreeSet::from([1])), vec![2, 3]);
    assert_eq!(h.gens(), vec![1, 4, 5]);
    assert_eq!(h.len(), 3);
}

// ---------------------------------------------------------------------------------------
// Pending deletes (§9.1 guard 2)
// ---------------------------------------------------------------------------------------

#[test]
fn pending_delete_bookkeeping() {
    let grace = 600;
    let fp1 = fingerprint("file v1");
    let fp2 = fingerprint("file v2");
    let mut p = PendingDeletes::default();
    p.observe(&[(id(1), 3, "Plan".into())], &fp1, 1000);
    assert!(p.contains(&id(1)));
    let e = p.get(&id(1)).unwrap();
    assert_eq!(
        (e.first_seen, e.unchanged_cycles, e.hidden_descendants),
        (1000, 0, 3)
    );
    assert_eq!(e.deadline(grace), 1600);
    // Not due: neither old enough nor survived an unchanged cycle.
    assert!(p.due(1000, grace).is_empty());
    // Old enough but no unchanged cycle yet: still not due.
    assert!(p.due(5000, grace).is_empty());

    // Next cycle, file unchanged: one cycle survived; due once the grace has passed.
    p.observe(&[(id(1), 4, "Plan!".into())], &fp1, 1100);
    let e = p.get(&id(1)).unwrap();
    assert_eq!(e.unchanged_cycles, 1);
    assert_eq!(
        (e.hidden_descendants, e.label.as_str(), e.last_seen),
        (4, "Plan!", 1100)
    );
    assert!(p.due(1599, grace).is_empty());
    assert_eq!(p.due(1600, grace), vec![id(1)]);
    assert!(p.due_with(1600, grace, 2).is_empty());

    // The file changed: the cycle count restarts; first_seen does not.
    p.observe(
        &[(id(1), 4, "Plan".into()), (id(2), 0, "Other".into())],
        &fp2,
        1700,
    );
    let e = p.get(&id(1)).unwrap();
    assert_eq!((e.first_seen, e.unchanged_cycles), (1000, 0));
    assert_eq!(e.fingerprint, fp2);
    assert!(p.due(1700, grace).is_empty());
    p.observe(
        &[(id(1), 4, "Plan".into()), (id(2), 0, "Other".into())],
        &fp2,
        1800,
    );
    assert_eq!(p.due(1800, grace), vec![id(1)]);
    assert_eq!(p.due(2400, grace), vec![id(1), id(2)]);

    // The line came back: the caller cancels.
    let removed = p.retain_only(&BTreeSet::from([id(2)]));
    assert_eq!(removed.len(), 1);
    assert_eq!(removed[0].node, id(1));
    assert_eq!(p.remove(&id(2)).map(|e| e.label), Some("Other".into()));
    assert!(p.is_empty());
}

#[test]
fn pending_deletes_persist_between_checkpoints() {
    let (_d, st) = store();
    assert!(st.load_pending(&root()).unwrap().is_empty());

    let mut cp = sample_checkpoint(1);
    cp.pending_deletes
        .observe(&[(id(1), 0, "Plan".into())], &fingerprint("a"), 10);
    st.save_checkpoint(&cp).unwrap();
    assert!(st.pending_path(&root()).is_file());
    assert_eq!(st.load_pending(&root()).unwrap(), cp.pending_deletes);

    // A cycle that does not write the file updates only pending.json; loading the
    // checkpoint sees the newer set.
    let mut p = st.load_pending(&root()).unwrap();
    p.observe(&[(id(1), 0, "Plan".into())], &fingerprint("a"), 20);
    st.save_pending(&root(), &p).unwrap();
    let loaded = st.load_checkpoint(&root()).unwrap().unwrap();
    assert_eq!(loaded.pending_deletes, p);
    assert_eq!(
        loaded.pending_deletes.get(&id(1)).unwrap().unchanged_cycles,
        1
    );

    // Without pending.json, the checkpoint's copy is used.
    fs::remove_file(st.pending_path(&root())).unwrap();
    assert_eq!(st.load_pending(&root()).unwrap(), cp.pending_deletes);

    // JSON shape is plain and forward-compatible.
    let v: Value =
        serde_json::from_str(&fs::read_to_string(st.checkpoint_path(&root())).unwrap()).unwrap();
    let e = &v["pending_deletes"]["entries"][id(1).as_str()];
    assert_eq!(e["first_seen"], json!(10));
    let minimal = json!({"entries": {id(2).as_str(): {
        "node": id(2).as_str(), "first_seen": 5, "fingerprint": "ab"}}});
    let p: PendingDeletes = serde_json::from_value(minimal).unwrap();
    assert_eq!(p.get(&id(2)).unwrap().unchanged_cycles, 0);
}

// ---------------------------------------------------------------------------------------
// Lock
// ---------------------------------------------------------------------------------------

#[test]
fn one_lock_per_root() {
    let (_d, st) = store();
    let l1 = st.lock(&root()).unwrap();
    assert!(matches!(st.lock(&root()), Err(StateError::Locked(r)) if r == root()));
    // Another root is independent.
    let l2 = st.lock(&id(5)).unwrap();
    drop(l1);
    let _l3 = st.lock(&root()).unwrap();
    drop(l2);
    // A lock alone does not make a root (no checkpoint).
    assert!(st.roots().unwrap().is_empty());
}

#[test]
fn odd_root_ids_stay_inside_the_state_dir() {
    let (_d, st) = store();
    let weird = NodeId::from_api("../../escape");
    let rd = st.root_dir(&weird);
    assert_eq!(rd.parent().unwrap(), st.dir().join("roots"));
    let mut cp = sample_checkpoint(1);
    cp.root = weird.clone();
    st.save_checkpoint(&cp).unwrap();
    assert_eq!(st.load_checkpoint(&weird).unwrap().unwrap().root, weird);
    assert_eq!(st.roots().unwrap(), vec![weird]);
}

// ---------------------------------------------------------------------------------------
// Fingerprints
// ---------------------------------------------------------------------------------------

#[test]
fn fingerprint_is_sha256_hex_and_stable() {
    assert_eq!(
        fingerprint(""),
        Fingerprint("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855".into())
    );
    assert_eq!(
        fingerprint("abc").0,
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    // Pinned value for a Markdown text: a change here would make every stored
    // checkpoint look edited (§5.1 fast path).
    assert_eq!(
        fingerprint("- item %%wf:000000000001%%\n  note ✓\n").0,
        "4ebc73711d64ece8b02e1c303caf251be00bc8df88a5d25ed199cba11e815268"
    );
    // Byte-exact: no newline or whitespace normalisation.
    assert_ne!(fingerprint("a\n"), fingerprint("a\r\n"));
    assert_ne!(fingerprint("a"), fingerprint("a\n"));
    // Serialises as a bare string.
    assert_eq!(
        serde_json::to_value(fingerprint("")).unwrap(),
        json!(fingerprint("").0)
    );
}

proptest! {
    #[test]
    fn fingerprint_is_deterministic_and_injective_enough(a in ".*", b in ".*") {
        let fa = fingerprint(&a);
        prop_assert_eq!(&fa, &fingerprint(&a.clone()));
        prop_assert_eq!(fa.0.len(), 64);
        prop_assert!(fa.0.bytes().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c)));
        if a != b {
            prop_assert_ne!(fa, fingerprint(&b));
        }
    }
}
