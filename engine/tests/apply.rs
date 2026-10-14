//! `apply` against the mock server (the oracle): every op kind compared with
//! `plan::simulate`, the two-step create's exact names, pre-write checks against
//! concurrent web-UI edits, crash recovery through the journal, and the live cycle guard.

use std::collections::BTreeSet;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::json;
use syncflowy::apply::{
    Applier, ApplyError, ApplyOptions, Journal, JournalEntry, OpOutcome, SkipReason, is_nonce,
    new_nonce, recover,
};
use syncflowy::plan::{self, CreateName, GuardReason, Op, Plan, PlanSummary};
use syncflowy::testsupport::MockServer;
use syncflowy::tree::{DiffScope, Node, NodeId, Position, ShortId, Tree, outline_diff};
use syncflowy::wfclient::{Api, ApiError, Client, CreateRequest, UpdateRequest};

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

fn child(c: &Client, parent: &NodeId, name: &str) -> NodeId {
    c.create(&CreateRequest {
        parent_id: parent.clone(),
        name: name.into(),
        note: None,
        layout: None,
        position: Position::Bottom,
    })
    .expect("create")
}

fn export(c: &Client) -> Tree {
    Tree::from_nodes(c.export().expect("export")).expect("tree")
}

fn journal_path() -> PathBuf {
    let n: u64 = rand::random();
    std::env::temp_dir()
        .join(format!("syncflowy-apply-{}-{n:016x}", std::process::id()))
        .join("journal.jsonl")
}

fn plan_of(root: &NodeId, ops: Vec<Op>) -> Plan {
    Plan {
        root: root.clone(),
        ops,
        summary: PlanSummary::default(),
    }
}

fn begin(plan: &Plan) -> (PathBuf, Journal) {
    let path = journal_path();
    let j = Journal::begin(&path, &JournalEntry::new_begin(plan, Vec::new())).expect("begin");
    (path, j)
}

fn roots(root: &NodeId) -> BTreeSet<NodeId> {
    BTreeSet::from([root.clone()])
}

fn named<'t>(t: &'t Tree, name: &str) -> Vec<&'t Node> {
    t.iter().filter(|n| n.name == name).collect()
}

/// How the test API misbehaves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Fault {
    None,
    /// Panic (a simulated crash) instead of sending mutating request number `n` (0-based).
    CrashBefore(usize),
    /// Send mutating request `n`, then panic before the caller sees the response.
    CrashAfter(usize),
    /// Send every create, then report a transport error that may have been applied.
    LoseCreateResponses,
}

/// A wrapper over the real client that records every call and injects faults.
struct TestApi<'a> {
    inner: &'a Client,
    fault: Fault,
    mutations: AtomicUsize,
    calls: Mutex<Vec<String>>,
}

impl<'a> TestApi<'a> {
    fn new(inner: &'a Client, fault: Fault) -> TestApi<'a> {
        TestApi {
            inner,
            fault,
            mutations: AtomicUsize::new(0),
            calls: Mutex::new(Vec::new()),
        }
    }

    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }

    fn record(&self, s: String) {
        self.calls.lock().unwrap().push(s);
    }

    fn mutate<T>(
        &self,
        what: String,
        f: impl FnOnce() -> Result<T, ApiError>,
    ) -> Result<T, ApiError> {
        let n = self.mutations.fetch_add(1, Ordering::SeqCst);
        if self.fault == Fault::CrashBefore(n) {
            panic!("injected crash before {what}");
        }
        self.record(what.clone());
        let r = f();
        if self.fault == Fault::CrashAfter(n) {
            panic!("injected crash after {what}");
        }
        r
    }
}

impl Api for TestApi<'_> {
    fn export(&self) -> Result<Vec<Node>, ApiError> {
        self.record("export".into());
        self.inner.export()
    }
    fn get(&self, id: &NodeId) -> Result<Node, ApiError> {
        self.record(format!("get {id}"));
        self.inner.get(id)
    }
    fn get_short(&self, id: &ShortId) -> Result<Node, ApiError> {
        self.record(format!("get {id}"));
        self.inner.get_short(id)
    }
    fn list(&self, parent: Option<&NodeId>) -> Result<Vec<Node>, ApiError> {
        self.record("list".into());
        self.inner.list(parent)
    }
    fn create(&self, req: &CreateRequest) -> Result<NodeId, ApiError> {
        self.mutate(format!("create {} {:?}", req.parent_id, req.name), || {
            let r = self.inner.create(req);
            if self.fault == Fault::LoseCreateResponses && r.is_ok() {
                return Err(ApiError::Transport {
                    message: "connection reset (injected)".into(),
                    maybe_applied: true,
                });
            }
            r
        })
    }
    fn update(&self, id: &NodeId, req: &UpdateRequest) -> Result<(), ApiError> {
        self.mutate(format!("update {id} {req:?}"), || {
            self.inner.update(id, req)
        })
    }
    fn move_node(&self, id: &NodeId, parent: &NodeId, position: Position) -> Result<(), ApiError> {
        self.mutate(format!("move {id} {parent} {}", position.as_str()), || {
            self.inner.move_node(id, parent, position)
        })
    }
    fn complete(&self, id: &NodeId) -> Result<(), ApiError> {
        self.mutate(format!("complete {id}"), || self.inner.complete(id))
    }
    fn uncomplete(&self, id: &NodeId) -> Result<(), ApiError> {
        self.mutate(format!("uncomplete {id}"), || self.inner.uncomplete(id))
    }
    fn delete(&self, id: &NodeId) -> Result<(), ApiError> {
        self.mutate(format!("delete {id}"), || self.inner.delete(id))
    }
    fn request_count(&self) -> u64 {
        self.inner.request_count()
    }
}

/// Run `f` and expect it to panic (an injected crash); silence the panic message.
fn crash<T>(f: impl FnOnce() -> T) {
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let r = catch_unwind(AssertUnwindSafe(f));
    std::panic::set_hook(hook);
    assert!(r.is_err(), "expected the injected crash");
}

fn entries(path: &std::path::Path) -> Vec<JournalEntry> {
    Journal::read(path).expect("read").expect("journal exists")
}

// ---------------------------------------------------------------------------------------
// One mock server for all sections (startup is slow); each section resets it.
// ---------------------------------------------------------------------------------------

#[test]
fn against_the_mock() {
    let mock = MockServer::start();
    type Section = (&'static str, fn(&MockServer));
    let sections: &[Section] = &[
        ("every op kind", every_op_kind_matches_the_simulation),
        ("exact names", two_step_create_writes_exact_names),
        ("pre-write checks", pre_write_checks_skip_concurrent_edits),
        ("dependencies", ops_on_skipped_creates_are_skipped),
        (
            "crash between steps",
            crash_between_create_steps_is_recovered,
        ),
        (
            "torn journal tail",
            recovery_after_a_torn_journal_tail_is_repeatable,
        ),
        ("crash after direct", crash_after_direct_create_is_recovered),
        (
            "crash before send",
            crash_before_any_request_sends_nothing_twice,
        ),
        ("lost response", lost_create_response_is_resolved_by_nonce),
        ("live cycle", move_into_a_live_cycle_is_refused),
        ("failure", api_failure_stops_and_reports_progress),
    ];
    for (name, f) in sections {
        eprintln!("--- {name}");
        mock.reset_all();
        f(&mock);
    }
}

fn every_op_kind_matches_the_simulation(mock: &MockServer) {
    let c = Client::new(mock.client_config());
    let root = top_level(mock, "root");
    let a = child(&c, &root, "A");
    let b = child(&c, &root, "B");
    let cc = child(&c, &root, "C");
    let d = child(&c, &root, "D");
    let e = child(&c, &root, "E");
    let _e1 = child(&c, &e, "E1");
    let done = child(&c, &root, "done");
    c.complete(&done).unwrap();
    let remote = export(&c);

    let t = NodeId::temp;
    let ops = vec![
        Op::Create {
            temp: t(1),
            parent: root.clone(),
            name: CreateName::Direct("plain new".into()),
            note: Some("n1".into()),
            layout: "bullets".into(),
        },
        Op::Create {
            temp: t(2),
            parent: root.clone(),
            name: CreateName::Deferred("1. a &amp; b".into()),
            note: None,
            layout: "todo".into(),
        },
        Op::Create {
            temp: t(3),
            parent: t(2),
            name: CreateName::Deferred("<b>bold</b> - [x] y".into()),
            note: Some("line1\nline2".into()),
            layout: String::new(),
        },
        Op::Move {
            node: cc.clone(),
            parent: a.clone(),
            position: Position::Bottom,
            expected_parent: Some(root.clone()),
        },
        Op::UpdateName {
            node: a.clone(),
            expected: "A".into(),
            name: "A &lt;2&gt;".into(),
        },
        Op::UpdateNote {
            node: b.clone(),
            expected: None,
            note: "note b".into(),
        },
        Op::UpdateLayout {
            node: b.clone(),
            expected: "bullets".into(),
            layout: "h1".into(),
        },
        Op::Move {
            node: b.clone(),
            parent: root.clone(),
            position: Position::Top,
            expected_parent: Some(root.clone()),
        },
        Op::Move {
            node: t(1),
            parent: root.clone(),
            position: Position::Top,
            expected_parent: None,
        },
        Op::Complete { node: d.clone() },
        Op::Complete { node: t(3) },
        Op::Delete {
            node: e.clone(),
            hidden_descendants: 0,
        },
    ];
    let plan = plan_of(&root, ops);
    let expected = plan::simulate(&plan, &remote).expect("simulate");

    let api = TestApi::new(&c, Fault::None);
    let (path, mut journal) = begin(&plan);
    let applier = Applier::new(&api, ApplyOptions::default(), roots(&root));
    let outcome = applier
        .apply(&plan, &remote, &expected, &mut journal)
        .expect("apply");
    assert!(
        outcome.results.iter().all(|r| *r == OpOutcome::Done),
        "{:#?}",
        outcome.log_lines(&plan)
    );
    assert_eq!(outcome.created.len(), 3);
    assert!(
        outcome.discrepancies.is_empty(),
        "{:#?}",
        outcome.discrepancies
    );
    assert!(outcome.export.is_some());
    assert!(outcome.requests > plan.ops.len() as u64);

    // The mock's export equals the simulation (all nodes, completed ones included;
    // priorities are compared only as sibling order).
    let mut want = expected.clone();
    want.remap_ids(&outcome.created).unwrap();
    let actual = export(&c);
    assert_eq!(outline_diff(&want, &actual, &root, DiffScope::All), vec![]);
    let n2 = actual.get(&outcome.created[&t(2)]).unwrap();
    assert_eq!(n2.name, "1. a &amp; b");
    assert_eq!(n2.note, None, "a note-less create keeps note null");
    assert_eq!(n2.layout, "todo");
    let n3 = actual.get(&outcome.created[&t(3)]).unwrap();
    assert_eq!(n3.name, "<b>bold</b> - [x] y");
    assert_eq!(n3.note.as_deref(), Some("line1\nline2"));
    assert!(n3.completed);
    assert!(actual.iter().all(|n| !is_nonce(&n.name)));

    // Requests: one create for the direct name, create + update for the deferred ones.
    let calls = api.calls();
    let creates: Vec<&String> = calls.iter().filter(|s| s.starts_with("create")).collect();
    assert_eq!(creates.len(), 3);
    assert!(creates[0].contains("\"plain new\""));
    assert!(creates[1].contains("⟳syncflowy:") && creates[2].contains("⟳syncflowy:"));

    // The journal: Begin, Intent before every op, Created for creates, Done, Applied.
    let es = entries(&path);
    assert!(matches!(es[0], JournalEntry::Begin { .. }));
    assert!(matches!(es.last().unwrap(), JournalEntry::Applied { .. }));
    for i in 0..plan.ops.len() {
        let pos = |f: &dyn Fn(&JournalEntry) -> bool| es.iter().position(f);
        let intent = pos(&|e| matches!(e, JournalEntry::Intent { index, .. } if *index == i));
        let done_at = pos(&|e| matches!(e, JournalEntry::Done { index } if *index == i));
        assert!(intent.unwrap() < done_at.unwrap(), "op {i}");
        if let Op::Create { name, .. } = &plan.ops[i] {
            let created = pos(&|e| matches!(e, JournalEntry::Created { index, .. } if *index == i));
            assert!(intent.unwrap() < created.unwrap() && created < done_at);
            let nonce = es.iter().find_map(|e| match e {
                JournalEntry::Intent { index, nonce, .. } if *index == i => Some(nonce.clone()),
                _ => None,
            });
            assert_eq!(
                nonce.unwrap().is_some(),
                matches!(name, CreateName::Deferred(_))
            );
        }
    }
    assert_eq!(outcome.log_lines(&plan).len(), plan.ops.len());
    // Recovery of a fully applied journal adopts the created nodes and changes nothing.
    let before = c.request_count();
    let rec = recover(&path, &c).unwrap().unwrap();
    assert_eq!(c.request_count() - before, 1, "only the export");
    assert_eq!(rec.adoptions, outcome.adoptions(&plan));
    assert!(rec.finished.is_empty());
    journal.commit().unwrap();
    assert!(!path.exists());
    assert!(recover(&path, &c).unwrap().is_none());

    // A nonce is a fixed point of the create pipeline (DM§6.2).
    let nonce = new_nonce();
    let x = child(&c, &root, &nonce);
    assert_eq!(c.get(&x).unwrap().name, nonce);
}

fn two_step_create_writes_exact_names(mock: &MockServer) {
    let c = Client::new(mock.client_config());
    let root = top_level(mock, "root");
    let remote = export(&c);
    let names = [
        "1. a &amp; b",
        "# not a heading",
        "- dash",
        "*star* and `code`",
        "  padded  ",
        "[link](http://x.y) &lt;tag&gt;",
        "<i>it</i> <s>s</s> <code>c</code>",
        "",
    ];
    // Each of the first six names sent through create's name directly would be mangled
    // (DM§6.2) …
    for name in &names[..6] {
        let id = child(&c, &root, name);
        let got = c.get(&id).unwrap().name;
        assert_ne!(
            &got, name,
            "the create pipeline keeps {name:?}; pick another"
        );
        c.delete(&id).unwrap();
    }
    // … while the two-step create stores it exactly.
    let ops: Vec<Op> = names
        .iter()
        .enumerate()
        .map(|(i, n)| Op::Create {
            temp: NodeId::temp(i as u32 + 1),
            parent: root.clone(),
            name: CreateName::Deferred((*n).into()),
            note: None,
            layout: String::new(),
        })
        .collect();
    let plan = plan_of(&root, ops);
    let expected = plan::simulate(&plan, &remote).unwrap();
    let (_path, mut journal) = begin(&plan);
    let outcome = Applier::new(&c, ApplyOptions::default(), roots(&root))
        .apply(&plan, &remote, &expected, &mut journal)
        .unwrap();
    assert!(
        outcome.discrepancies.is_empty(),
        "{:#?}",
        outcome.discrepancies
    );
    let actual = export(&c);
    let kids: Vec<String> = actual
        .children(&root)
        .iter()
        .map(|id| actual.get(id).unwrap().name.clone())
        .collect();
    assert_eq!(kids, names);
    journal.commit().unwrap();
}

fn pre_write_checks_skip_concurrent_edits(mock: &MockServer) {
    let c = Client::new(mock.client_config());
    let root = top_level(mock, "root");
    let x = child(&c, &root, "old");
    let y = child(&c, &root, "y");
    let z = child(&c, &root, "z");
    let w = child(&c, &root, "w");
    let gone = child(&c, &root, "gone");
    let remote = export(&c);
    let ops = vec![
        Op::UpdateName {
            node: x.clone(),
            expected: "old".into(),
            name: "ours".into(),
        },
        Op::UpdateNote {
            node: x.clone(),
            expected: None,
            note: "our note".into(),
        },
        Op::Complete { node: y.clone() },
        Op::Delete {
            node: z.clone(),
            hidden_descendants: 0,
        },
        Op::UpdateLayout {
            node: gone.clone(),
            expected: "bullets".into(),
            layout: "h2".into(),
        },
        Op::Move {
            node: w.clone(),
            parent: x.clone(),
            position: Position::Bottom,
            expected_parent: Some(root.clone()),
        },
    ];
    let plan = plan_of(&root, ops);
    let expected = plan::simulate(&plan, &remote).unwrap();

    // Concurrent web-UI edits after the export.
    mock.ui("edit", json!({"id": x.as_str(), "name": "theirs"}));
    mock.ui("complete", json!({"id": y.as_str()}));
    mock.ui("edit", json!({"id": z.as_str(), "name": "z edited"}));
    mock.ui("delete", json!({"id": gone.as_str()}));
    mock.ui("move", json!({"id": w.as_str(), "parent_id": y.as_str()}));

    let api = TestApi::new(&c, Fault::None);
    let (path, mut journal) = begin(&plan);
    let outcome = Applier::new(&api, ApplyOptions::default(), roots(&root))
        .apply(&plan, &remote, &expected, &mut journal)
        .unwrap();
    assert_eq!(
        outcome.results,
        vec![
            OpOutcome::Skipped(SkipReason::ChangedRemotely),
            OpOutcome::Done,
            OpOutcome::Skipped(SkipReason::AlreadyCompleted),
            OpOutcome::Skipped(SkipReason::ChangedRemotely),
            OpOutcome::Skipped(SkipReason::Gone),
            OpOutcome::Skipped(SkipReason::ChangedRemotely),
        ]
    );
    // Only the unchanged note was written.
    let writes: Vec<String> = api
        .calls()
        .into_iter()
        .filter(|s| !s.starts_with("get") && s != "export")
        .collect();
    assert_eq!(writes.len(), 1, "{writes:?}");
    assert!(writes[0].starts_with(&format!("update {x}")) && writes[0].contains("our note"));
    let actual = export(&c);
    assert_eq!(actual.get(&x).unwrap().name, "theirs");
    assert_eq!(actual.get(&x).unwrap().note.as_deref(), Some("our note"));
    assert_eq!(actual.get(&z).unwrap().name, "z edited");
    assert_eq!(actual.parent(&w), Some(&y));
    // The verification reports what differs from M; it is not an error.
    assert!(outcome.discrepancies.iter().any(|d| matches!(
        d,
        syncflowy::tree::OutlineDiff::Name { id, actual, .. } if *id == x && actual == "theirs"
    )));
    let es = entries(&path);
    assert!(es.iter().any(|e| matches!(
        e,
        JournalEntry::Skipped {
            index: 0,
            reason: SkipReason::ChangedRemotely
        }
    )));
    // Skipped ops were never announced as intents.
    assert!(
        !es.iter()
            .any(|e| matches!(e, JournalEntry::Intent { index: 0, .. }))
    );
    journal.commit().unwrap();

    // Without the pre-write check the update is sent (last writer wins).
    let remote = export(&c);
    let plan = plan_of(
        &root,
        vec![Op::UpdateName {
            node: x.clone(),
            expected: "theirs".into(),
            name: "ours".into(),
        }],
    );
    mock.ui("edit", json!({"id": x.as_str(), "name": "theirs again"}));
    let (_p, mut journal) = begin(&plan);
    let opts = ApplyOptions {
        pre_write_check: false,
        verify: false,
    };
    let outcome = Applier::new(&c, opts, roots(&root))
        .apply(&plan, &remote, &remote, &mut journal)
        .unwrap();
    assert_eq!(outcome.results, vec![OpOutcome::Done]);
    assert!(outcome.export.is_none());
    assert_eq!(c.get(&x).unwrap().name, "ours");
    journal.commit().unwrap();
}

fn ops_on_skipped_creates_are_skipped(mock: &MockServer) {
    let c = Client::new(mock.client_config());
    let root = top_level(mock, "root");
    let p = child(&c, &root, "parent");
    let remote = export(&c);
    let plan = plan_of(
        &root,
        vec![
            Op::Create {
                temp: NodeId::temp(1),
                parent: p.clone(),
                name: CreateName::Deferred("new".into()),
                note: None,
                layout: String::new(),
            },
            Op::Create {
                temp: NodeId::temp(2),
                parent: NodeId::temp(1),
                name: CreateName::Direct("grandchild".into()),
                note: None,
                layout: String::new(),
            },
            Op::Complete {
                node: NodeId::temp(2),
            },
            Op::Create {
                temp: NodeId::temp(3),
                parent: root.clone(),
                name: CreateName::Direct("sibling".into()),
                note: None,
                layout: String::new(),
            },
        ],
    );
    let expected = plan::simulate(&plan, &remote).unwrap();
    mock.ui("delete", json!({"id": p.as_str()}));
    let (_path, mut journal) = begin(&plan);
    let outcome = Applier::new(&c, ApplyOptions::default(), roots(&root))
        .apply(&plan, &remote, &expected, &mut journal)
        .unwrap();
    assert_eq!(
        outcome.results,
        vec![
            OpOutcome::Skipped(SkipReason::Gone),
            OpOutcome::Skipped(SkipReason::MissingDependency(NodeId::temp(1))),
            OpOutcome::Skipped(SkipReason::MissingDependency(NodeId::temp(2))),
            OpOutcome::Done,
        ]
    );
    assert_eq!(outcome.created.len(), 1);
    assert!(
        outcome
            .discrepancies
            .contains(&syncflowy::tree::OutlineDiff::Missing(p.clone()))
    );
    journal.commit().unwrap();
}

fn crash_between_create_steps_is_recovered(mock: &MockServer) {
    let c = Client::new(mock.client_config());
    let root = top_level(mock, "root");
    let x = child(&c, &root, "x");
    let remote = export(&c);
    let name = "1. a &amp; b";
    let plan = plan_of(
        &root,
        vec![
            Op::Create {
                temp: NodeId::temp(1),
                parent: root.clone(),
                name: CreateName::Deferred(name.into()),
                note: Some("n".into()),
                layout: "todo".into(),
            },
            Op::UpdateName {
                node: x.clone(),
                expected: "x".into(),
                name: "x2".into(),
            },
        ],
    );
    let expected = plan::simulate(&plan, &remote).unwrap();
    let api = TestApi::new(&c, Fault::CrashAfter(0));
    let (path, mut journal) = begin(&plan);
    crash(|| {
        Applier::new(&api, ApplyOptions::default(), roots(&root))
            .apply(&plan, &remote, &expected, &mut journal)
            .ok();
    });
    drop(journal);
    // The create reached the server, but its id never reached the journal.
    let es = entries(&path);
    assert_eq!(es.len(), 2, "{es:#?}");
    let JournalEntry::Intent {
        index: 0,
        nonce: Some(nonce),
        ..
    } = &es[1]
    else {
        panic!("{es:#?}")
    };
    let after_crash = export(&c);
    let half = named(&after_crash, nonce);
    assert_eq!(half.len(), 1);
    let half_id = half[0].id.clone();

    // Recovery finds the nonce node, finishes step 2 and adopts it.
    let rec = recover(&path, &c).unwrap().unwrap();
    assert_eq!(rec.root.as_ref(), Some(&root));
    assert_eq!(rec.finished, vec![half_id.clone()]);
    assert_eq!(rec.adoptions.len(), 1);
    let a = &rec.adoptions[0];
    assert_eq!((&a.node, &a.parent), (&half_id, &root));
    assert_eq!(a.name, name);
    assert_eq!(a.note.as_deref(), Some("n"));
    assert_eq!(a.layout, "todo");
    let t = export(&c);
    assert_eq!(named(&t, name).len(), 1, "no duplicate");
    assert!(named(&t, nonce).is_empty());
    let n = t.get(&half_id).unwrap();
    assert_eq!((n.note.as_deref(), n.layout.as_str()), (Some("n"), "todo"));
    assert_eq!(t.get(&x).unwrap().name, "x", "other ops are not replayed");
    // The node's id is now in the journal, so a second recovery (a crash during the
    // first one) finds it by id and sends nothing but the export.
    assert!(entries(&path).iter().any(|e| matches!(
        e,
        JournalEntry::Created { index: 0, id, .. } if *id == half_id
    )));
    let before = c.request_count();
    let again = recover(&path, &c).unwrap().unwrap();
    assert_eq!(c.request_count() - before, 1);
    assert!(again.finished.is_empty());
    assert_eq!(again.adoptions, rec.adoptions);

    // The next cycle carries the adoptions; recovering *its* journal still returns them.
    let next = plan_of(&root, Vec::new());
    let path2 = path.clone();
    let j = Journal::begin(
        &path2,
        &JournalEntry::new_begin(&next, rec.adoptions.clone()),
    )
    .unwrap();
    drop(j);
    let carried = recover(&path2, &c).unwrap().unwrap();
    assert_eq!(carried.adoptions, rec.adoptions);
    // Once the node is gone, the carried adoption is dropped.
    c.delete(&half_id).unwrap();
    let carried = recover(&path2, &c).unwrap().unwrap();
    assert!(carried.adoptions.is_empty());
    let _ = std::fs::remove_file(&path2);
}

/// A crash mid-append leaves the journal without a final newline; the `Created` entry
/// recovery appends must still be readable, so that a second recovery (after the nonce
/// is gone) adopts the node instead of letting the next merge create it again.
fn recovery_after_a_torn_journal_tail_is_repeatable(mock: &MockServer) {
    let c = Client::new(mock.client_config());
    let root = top_level(mock, "root");
    let remote = export(&c);
    let plan = plan_of(
        &root,
        vec![Op::Create {
            temp: NodeId::temp(1),
            parent: root.clone(),
            name: CreateName::Deferred("a".into()),
            note: None,
            layout: String::new(),
        }],
    );
    let expected = plan::simulate(&plan, &remote).unwrap();
    let api = TestApi::new(&c, Fault::CrashAfter(0));
    let (path, mut journal) = begin(&plan);
    crash(|| {
        Applier::new(&api, ApplyOptions::default(), roots(&root))
            .apply(&plan, &remote, &expected, &mut journal)
            .ok();
    });
    drop(journal);
    // The crash also cut a journal line mid-write.
    {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        f.write_all(br#"{"type":"done","ind"#).unwrap();
    }
    let rec = recover(&path, &c).unwrap().unwrap();
    assert_eq!(rec.adoptions.len(), 1, "{rec:#?}");
    assert_eq!(rec.finished.len(), 1);
    let id = rec.adoptions[0].node.clone();
    assert!(
        entries(&path)
            .iter()
            .any(|e| matches!(e, JournalEntry::Created { index: 0, id: i, .. } if *i == id)),
        "the Created entry is readable"
    );
    // The nonce is gone now; a second recovery must find the node by its id.
    let again = recover(&path, &c).unwrap().unwrap();
    assert_eq!(again.adoptions, rec.adoptions);
    assert!(again.finished.is_empty());
    assert_eq!(named(&export(&c), "a").len(), 1);
    let _ = std::fs::remove_file(&path);
}

fn crash_after_direct_create_is_recovered(mock: &MockServer) {
    let c = Client::new(mock.client_config());
    let root = top_level(mock, "root");
    // A collaborator's older sibling with the same name is not taken.
    mock.clock(json!({"offset": -600}));
    let older = child(&c, &root, "plain");
    mock.clock(json!({"offset": 0}));
    let remote = export(&c);
    let plan = plan_of(
        &root,
        vec![Op::Create {
            temp: NodeId::temp(1),
            parent: root.clone(),
            name: CreateName::Direct("plain".into()),
            note: None,
            layout: String::new(),
        }],
    );
    let expected = plan::simulate(&plan, &remote).unwrap();
    let api = TestApi::new(&c, Fault::CrashAfter(0));
    let (path, mut journal) = begin(&plan);
    crash(|| {
        Applier::new(&api, ApplyOptions::default(), roots(&root))
            .apply(&plan, &remote, &expected, &mut journal)
            .ok();
    });
    drop(journal);
    let rec = recover(&path, &c).unwrap().unwrap();
    assert!(rec.finished.is_empty());
    assert_eq!(rec.adoptions.len(), 1);
    let made = &rec.adoptions[0].node;
    assert_ne!(made, &older);
    let t = export(&c);
    assert_eq!(named(&t, "plain").len(), 2);
    assert_eq!(rec.adoptions[0].layout, "bullets");
    let _ = std::fs::remove_file(&path);
}

fn crash_before_any_request_sends_nothing_twice(mock: &MockServer) {
    let c = Client::new(mock.client_config());
    let root = top_level(mock, "root");
    let remote = export(&c);
    let plan = plan_of(
        &root,
        vec![
            Op::Create {
                temp: NodeId::temp(1),
                parent: root.clone(),
                name: CreateName::Deferred("first & only".replace('&', "&amp;")),
                note: None,
                layout: String::new(),
            },
            Op::Create {
                temp: NodeId::temp(2),
                parent: NodeId::temp(1),
                name: CreateName::Direct("kid".into()),
                note: None,
                layout: String::new(),
            },
        ],
    );
    let expected = plan::simulate(&plan, &remote).unwrap();
    let api = TestApi::new(&c, Fault::CrashBefore(0));
    let (path, mut journal) = begin(&plan);
    crash(|| {
        Applier::new(&api, ApplyOptions::default(), roots(&root))
            .apply(&plan, &remote, &expected, &mut journal)
            .ok();
    });
    drop(journal);
    assert!(api.calls().iter().all(|s| !s.starts_with("create")));
    // Recovery sends only the export and adopts nothing.
    let before = c.request_count();
    let rec = recover(&path, &c).unwrap().unwrap();
    assert_eq!(c.request_count() - before, 1);
    assert!(rec.adoptions.is_empty() && rec.finished.is_empty());
    assert_eq!(export(&c).len(), remote.len());

    // The next cycle (a fresh journal replacing the old one) creates each node once.
    let mut journal =
        Journal::begin(&path, &JournalEntry::new_begin(&plan, rec.adoptions)).unwrap();
    let outcome = Applier::new(&c, ApplyOptions::default(), roots(&root))
        .apply(&plan, &remote, &expected, &mut journal)
        .unwrap();
    assert!(
        outcome.discrepancies.is_empty(),
        "{:#?}",
        outcome.discrepancies
    );
    let t = export(&c);
    assert_eq!(t.len(), remote.len() + 2);
    assert_eq!(named(&t, "first &amp; only").len(), 1);
    assert_eq!(named(&t, "kid").len(), 1);
    journal.commit().unwrap();
}

fn lost_create_response_is_resolved_by_nonce(mock: &MockServer) {
    let c = Client::new(mock.client_config());
    let root = top_level(mock, "root");
    let remote = export(&c);
    let plan = plan_of(
        &root,
        vec![
            Op::Create {
                temp: NodeId::temp(1),
                parent: root.clone(),
                name: CreateName::Deferred("two &amp; step".into()),
                note: None,
                layout: String::new(),
            },
            Op::Create {
                temp: NodeId::temp(2),
                parent: NodeId::temp(1),
                name: CreateName::Direct("direct".into()),
                note: None,
                layout: String::new(),
            },
        ],
    );
    let expected = plan::simulate(&plan, &remote).unwrap();
    let api = TestApi::new(&c, Fault::LoseCreateResponses);
    let (_path, mut journal) = begin(&plan);
    let outcome = Applier::new(&api, ApplyOptions::default(), roots(&root))
        .apply(&plan, &remote, &expected, &mut journal)
        .unwrap();
    assert_eq!(outcome.results, vec![OpOutcome::Done, OpOutcome::Done]);
    assert!(
        outcome.discrepancies.is_empty(),
        "{:#?}",
        outcome.discrepancies
    );
    let t = export(&c);
    assert_eq!(t.len(), remote.len() + 2, "no duplicates");
    let creates = api
        .calls()
        .iter()
        .filter(|s| s.starts_with("create"))
        .count();
    assert_eq!(
        creates, 2,
        "a create that may have been applied is never resent"
    );
    journal.commit().unwrap();
}

fn move_into_a_live_cycle_is_refused(mock: &MockServer) {
    let c = Client::new(mock.client_config());
    // Production accepts a move into the node's own descendant and detaches the
    // subtree (DM§4.7.5); make the mock do the same, so a sent move would do damage.
    mock.config(json!({"faithful_cycles": true}));
    let root = top_level(mock, "root");
    let a = child(&c, &root, "A");
    let b = child(&c, &root, "B");
    let other = top_level(mock, "elsewhere");
    let remote = export(&c);
    let plan = plan_of(
        &root,
        vec![
            Op::Move {
                node: b.clone(),
                parent: a.clone(),
                position: Position::Bottom,
                expected_parent: Some(root.clone()),
            },
            // Refused against the latest known tree without any request.
            Op::Move {
                node: root.clone(),
                parent: a.clone(),
                position: Position::Bottom,
                expected_parent: None,
            },
            Op::Move {
                node: a.clone(),
                parent: other.clone(),
                position: Position::Bottom,
                expected_parent: Some(root.clone()),
            },
        ],
    );
    let expected = plan::simulate(&plan_of(&root, vec![plan.ops[0].clone()]), &remote).unwrap();
    // A collaborator moves A under B in the web UI: B -> A is now a cycle.
    mock.ui("move", json!({"id": a.as_str(), "parent_id": b.as_str()}));

    let api = TestApi::new(&c, Fault::None);
    let (path, mut journal) = begin(&plan);
    let outcome = Applier::new(&api, ApplyOptions::default(), roots(&root))
        .apply(&plan, &remote, &expected, &mut journal)
        .unwrap();
    assert_eq!(
        outcome.results,
        vec![
            OpOutcome::Skipped(SkipReason::Guard(GuardReason::Cycle)),
            OpOutcome::Skipped(SkipReason::Guard(GuardReason::Cycle)),
            OpOutcome::Skipped(SkipReason::Guard(GuardReason::OutsideSyncRoots)),
        ]
    );
    assert!(
        api.calls().iter().all(|s| !s.starts_with("move")),
        "{:?}",
        api.calls()
    );
    let t = export(&c);
    assert_eq!(t.parent(&a), Some(&b));
    assert_eq!(t.parent(&b), Some(&root));
    assert!(!outcome.discrepancies.is_empty());
    assert!(entries(&path).iter().any(|e| matches!(
        e,
        JournalEntry::Skipped {
            index: 0,
            reason: SkipReason::Guard(GuardReason::Cycle)
        }
    )));
    journal.commit().unwrap();

    // The same live check runs with the pre-write check off.
    let api = TestApi::new(&c, Fault::None);
    let plan = plan_of(&root, vec![plan.ops[0].clone()]);
    let (_p, mut journal) = begin(&plan);
    let opts = ApplyOptions {
        pre_write_check: false,
        verify: false,
    };
    let outcome = Applier::new(&api, opts, roots(&root))
        .apply(&plan, &remote, &remote, &mut journal)
        .unwrap();
    assert_eq!(
        outcome.results,
        vec![OpOutcome::Skipped(SkipReason::Guard(GuardReason::Cycle))]
    );
    assert!(api.calls().iter().all(|s| !s.starts_with("move")));
    journal.commit().unwrap();

    // A legitimate re-parenting move still goes through (and walks up to the root).
    let cc = child(&c, &root, "C");
    let remote2 = export(&c);
    let plan = plan_of(
        &root,
        vec![Op::Move {
            node: cc.clone(),
            parent: a.clone(),
            position: Position::Top,
            expected_parent: Some(root.clone()),
        }],
    );
    let api = TestApi::new(&c, Fault::None);
    let expected = plan::simulate(&plan, &remote2).unwrap();
    let (_p, mut journal) = begin(&plan);
    let outcome = Applier::new(&api, ApplyOptions::default(), roots(&root))
        .apply(&plan, &remote2, &expected, &mut journal)
        .unwrap();
    assert_eq!(outcome.results, vec![OpOutcome::Done]);
    assert!(
        outcome.discrepancies.is_empty(),
        "{:#?}",
        outcome.discrepancies
    );
    let gets = api.calls().iter().filter(|s| s.starts_with("get")).count();
    assert_eq!(gets, 3, "the node, A and B: {:?}", api.calls());
    journal.commit().unwrap();
}

fn api_failure_stops_and_reports_progress(mock: &MockServer) {
    let c = Client::new(mock.client_config());
    let root = top_level(mock, "root");
    let x = child(&c, &root, "x");
    let remote = export(&c);
    // NUL in a create note gives a 500 (DM§6.3), retried and then reported.
    let plan = plan_of(
        &root,
        vec![
            Op::UpdateName {
                node: x.clone(),
                expected: "x".into(),
                name: "x2".into(),
            },
            Op::Create {
                temp: NodeId::temp(1),
                parent: root.clone(),
                name: CreateName::Deferred("bad".into()),
                note: Some("nul \u{0} here".into()),
                layout: String::new(),
            },
            Op::Complete { node: x.clone() },
        ],
    );
    let expected = plan::simulate(&plan, &remote).unwrap();
    let (path, mut journal) = begin(&plan);
    let err = Applier::new(&c, ApplyOptions::default(), roots(&root))
        .apply(&plan, &remote, &expected, &mut journal)
        .unwrap_err();
    let ApplyError::Api {
        index,
        error,
        outcome,
    } = err
    else {
        panic!("{err:?}")
    };
    assert_eq!(index, 1);
    assert!(
        matches!(error, ApiError::Server { status: 500, .. }),
        "{error:?}"
    );
    assert_eq!(
        outcome.results,
        vec![OpOutcome::Done, OpOutcome::NotRun, OpOutcome::NotRun]
    );
    assert!(outcome.created.is_empty());
    let es = entries(&path);
    assert!(matches!(
        es.last().unwrap(),
        JournalEntry::Failed { index: 1, .. }
    ));
    // Recovery: nothing was created, nothing to adopt.
    let rec = recover(&path, &c).unwrap().unwrap();
    assert!(rec.adoptions.is_empty());
    assert_eq!(export(&c).len(), remote.len());
    let _ = std::fs::remove_file(&path);
}
