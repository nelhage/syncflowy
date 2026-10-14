//! Differential test of `apply` against the mock: random valid op sequences (every op
//! kind, both create paths, top and bottom placements, re-parenting and reorders) are
//! applied to the mock server, whose export must equal `plan::simulate`'s prediction
//! (structure, fields, completion and sibling order; priorities only as order).

use std::collections::BTreeSet;

use serde_json::json;
use syncflowy::apply::{Applier, ApplyOptions, Journal, JournalEntry, OpOutcome};
use syncflowy::plan::{self, CreateName, Op, Plan, PlanSummary, create_name_is_safe};
use syncflowy::testsupport::MockServer;
use syncflowy::tree::{DiffScope, NodeId, Position, Tree, outline_diff};
use syncflowy::wfclient::{Api, Client, CreateRequest};

/// A small deterministic PRNG (xorshift64*), so failures are reproducible by seed.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
    fn pick<'a, T>(&mut self, v: &'a [T]) -> &'a T {
        &v[self.below(v.len())]
    }
    fn chance(&mut self, percent: u64) -> bool {
        self.next() % 100 < percent
    }
}

const NAMES: &[&str] = &[
    "plain",
    "two words",
    "1. numbered",
    "a &amp; b",
    "<b>bold</b> text",
    "- dash",
    "# hash",
    "*star*",
    "x",
    "",
    "  spaced ",
    "ünïcödé",
    "[l](http://a.b)",
    "&lt;tag&gt;",
];
const NOTES: &[Option<&str>] = &[None, Some(""), Some("note"), Some("multi\nline &amp; x")];
const LAYOUTS: &[&str] = &["", "bullets", "todo", "h1", "p"];

fn position(rng: &mut Rng) -> Position {
    if rng.chance(50) {
        Position::Top
    } else {
        Position::Bottom
    }
}

/// One random op that is valid on `s` (the simulated tree), or `None` if the chosen
/// kind has no candidate.
fn random_op(rng: &mut Rng, s: &Tree, root: &NodeId, next_temp: &mut u32) -> Option<Op> {
    let below: Vec<NodeId> = s.descendants(root);
    let mut all = vec![root.clone()];
    all.extend(below.iter().cloned());
    let kind = rng.below(10);
    if below.is_empty() && kind > 1 {
        return None;
    }
    Some(match kind {
        0 | 1 => {
            *next_temp += 1;
            let name = rng.pick(NAMES).to_string();
            let name = if create_name_is_safe(&name) && rng.chance(70) {
                CreateName::Direct(name)
            } else {
                CreateName::Deferred(name)
            };
            Op::Create {
                temp: NodeId::temp(*next_temp),
                parent: rng.pick(&all).clone(),
                name,
                note: rng.pick(NOTES).map(str::to_owned),
                layout: rng.pick(LAYOUTS).to_string(),
            }
        }
        2 => {
            // Re-parent.
            let node = rng.pick(&below).clone();
            let blocked: BTreeSet<NodeId> = s.preorder(&node).into_iter().collect();
            let dests: Vec<NodeId> = all
                .iter()
                .filter(|d| !blocked.contains(*d))
                .cloned()
                .collect();
            let parent = rng.pick(&dests).clone();
            Op::Move {
                expected_parent: (!node.is_temp())
                    .then(|| s.parent(&node).cloned())
                    .flatten(),
                node,
                parent,
                position: position(rng),
            }
        }
        3 | 4 => {
            // Reorder within the parent.
            let node = rng.pick(&below).clone();
            let parent = s.parent(&node).cloned().expect("below the root");
            Op::Move {
                expected_parent: (!node.is_temp()).then(|| parent.clone()),
                node,
                parent,
                position: position(rng),
            }
        }
        5 => {
            let node = rng.pick(&below).clone();
            Op::UpdateName {
                expected: s.get(&node).unwrap().name.clone(),
                node,
                name: rng.pick(NAMES).to_string(),
            }
        }
        6 => {
            let node = rng.pick(&below).clone();
            Op::UpdateNote {
                expected: s.get(&node).unwrap().note.clone(),
                node,
                note: rng.pick(NOTES).unwrap_or("n").to_owned(),
            }
        }
        7 => {
            let node = rng.pick(&below).clone();
            let layout = rng.pick(&LAYOUTS[1..]).to_string();
            Op::UpdateLayout {
                expected: s.get(&node).unwrap().layout.clone(),
                node,
                layout,
            }
        }
        8 => {
            let open: Vec<NodeId> = below
                .iter()
                .filter(|n| !s.get(n).unwrap().completed)
                .cloned()
                .collect();
            if open.is_empty() {
                return None;
            }
            Op::Complete {
                node: rng.pick(&open).clone(),
            }
        }
        _ => {
            if !rng.chance(40) {
                return None;
            }
            Op::Delete {
                node: rng.pick(&below).clone(),
                hidden_descendants: 0,
            }
        }
    })
}

#[test]
fn random_plans_apply_as_simulated() {
    let mock = MockServer::start();
    let rounds: u64 = std::env::var("APPLY_RANDOM_ROUNDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(8);
    for seed in 1..=rounds {
        mock.reset_all();
        let c = Client::new(mock.client_config());
        let mut rng = Rng(0x9e37_79b9_7f4a_7c15 ^ seed.wrapping_mul(0x1234_5678_9abc_def1));
        let (status, body) = mock.post(
            "/api/v1/nodes",
            &json!({"parent_id": "None", "name": "root", "position": "bottom"}),
        );
        assert_eq!(status, 200);
        let root = NodeId::parse(body["item_id"].as_str().unwrap()).unwrap();
        // A random starting tree of up to 12 nodes, some completed.
        let mut ids = vec![root.clone()];
        for i in 0..rng.below(12) {
            let parent = rng.pick(&ids).clone();
            let id = c
                .create(&CreateRequest {
                    parent_id: parent,
                    name: format!("n{i}"),
                    note: None,
                    layout: None,
                    position: position(&mut rng),
                })
                .unwrap();
            if rng.chance(15) {
                c.complete(&id).unwrap();
            }
            ids.push(id);
        }
        let remote = Tree::from_nodes(c.export().unwrap()).unwrap();

        let mut s = remote.clone();
        let mut ops = Vec::new();
        let mut next_temp = 0;
        let n_ops = 10 + rng.below(30);
        while ops.len() < n_ops {
            let Some(op) = random_op(&mut rng, &s, &root, &mut next_temp) else {
                continue;
            };
            plan::simulate_op(&op, &mut s).unwrap_or_else(|e| panic!("seed {seed}: {op:?}: {e}"));
            ops.push(op);
        }
        let plan = Plan {
            root: root.clone(),
            ops,
            summary: PlanSummary::default(),
        };
        let expected = plan::simulate(&plan, &remote).unwrap();
        assert_eq!(expected, s);

        let n: u64 = rand::random();
        let path = std::env::temp_dir()
            .join(format!(
                "syncflowy-apply-random-{}-{n:016x}",
                std::process::id()
            ))
            .join("journal.jsonl");
        let mut journal =
            Journal::begin(&path, &JournalEntry::new_begin(&plan, Vec::new())).unwrap();
        let outcome = Applier::new(&c, ApplyOptions::default(), BTreeSet::from([root.clone()]))
            .apply(&plan, &remote, &expected, &mut journal)
            .unwrap_or_else(|e| panic!("seed {seed}: {e}"));
        let log = outcome.log_lines(&plan);
        assert!(
            outcome.results.iter().all(|r| *r == OpOutcome::Done),
            "seed {seed}: {log:#?}"
        );
        assert!(
            outcome.discrepancies.is_empty(),
            "seed {seed}: {:#?}\n{log:#?}",
            outcome.discrepancies
        );
        let mut want = expected;
        want.remap_ids(&outcome.created).unwrap();
        let actual = Tree::from_nodes(c.export().unwrap()).unwrap();
        assert_eq!(
            outline_diff(&want, &actual, &root, DiffScope::All),
            vec![],
            "seed {seed}: {log:#?}"
        );
        journal.commit().unwrap();
        let _ = std::fs::remove_dir(path.parent().unwrap());
    }
}
