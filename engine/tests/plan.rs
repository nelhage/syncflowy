//! Planner tests (design §7.1, §7.2, §8.1–§8.3).
//!
//! * A corpus of `(merge result, remote)` fixtures (`tests/fixtures/plan/*.plan`): the
//!   plan, replayed by `simulate` on the remote tree, gives the merged tree (structure,
//!   fields, completion, visible sibling order); its ops come in exactly the §7.1 order;
//!   and the rendered ops match the fixture's `== ops` section (written when missing or
//!   when `PLAN_BLESS=1`).
//! * Random scenarios: random remote trees with hidden nodes, random local edits
//!   (renames, notes, layouts, creates, deletes, moves, cross-file moves, reorders,
//!   ticks), with the same checks.
//! * Guards (§8.2), brakes (§8.3) and the create fast path (§7.2).
//!
//! The merge module is developed separately, so the `MergeResult`s here come from
//! [`derive_merge`], which computes exactly what the merge promises to produce (its
//! postconditions) from the remote tree and the expected merged tree `M`.
//!
//! # Fixture format
//!
//! ```text
//! // comment
//! == config            (optional: force | no-fast-path | roots <key>… | deferred <key>…)
//! == remote            (the export: an outline, two spaces per level)
//! == merged            (M: the sync root's subtree; its first line is the root)
//! == ops               (expected plan, one rendered op per line)
//! ```
//!
//! An outline line is `<key> <name> {<layout>} [x] | <note>`; everything after the key
//! is optional (`{bullets}`, not completed and no note by default; ` |` alone is the
//! empty note; `\n` in a note is a newline). Keys name nodes (stable UUIDs are made up);
//! `+<n>` is the temp id `temp-<n>` of a node to create.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use proptest::prelude::*;
use syncflowy::merge::{
    Change, DeleteIntent, FieldPush, MergeResult, MergeStats, MoveIntent, NewNode,
};
use syncflowy::plan::{
    self, Brake, BrakeConfig, CreateName, GuardReason, Op, Plan, PlanConfig, PlanError,
};
use syncflowy::tree::{DiffScope, Node, NodeId, OutlineDiff, Position, Tree, outline_diff};

// =======================================================================================
// Outline DSL
// =======================================================================================

/// Fixture keys <-> node ids.
#[derive(Debug, Default, Clone)]
struct Keys {
    by_key: BTreeMap<String, NodeId>,
    by_id: BTreeMap<NodeId, String>,
}

impl Keys {
    fn id(&mut self, key: &str) -> NodeId {
        if let Some(id) = self.by_key.get(key) {
            return id.clone();
        }
        let id = if let Some(n) = key.strip_prefix('+') {
            NodeId::temp(n.parse().expect("temp key is +<number>"))
        } else {
            let i = self.by_key.len() as u64;
            NodeId::from_api(format!("{:08x}-0000-4000-8000-{i:012x}", 0x1000_0000 + i))
        };
        self.by_key.insert(key.to_owned(), id.clone());
        self.by_id.insert(id.clone(), key.to_owned());
        id
    }

    fn get(&self, key: &str) -> NodeId {
        self.by_key
            .get(key)
            .unwrap_or_else(|| panic!("unknown key {key}"))
            .clone()
    }

    fn key(&self, id: &NodeId) -> String {
        self.by_id
            .get(id)
            .cloned()
            .unwrap_or_else(|| id.to_string())
    }
}

fn unescape(s: &str) -> String {
    s.replace("\\n", "\n")
}

/// Parse an outline. The top-level lines get parent `top_parent`.
fn parse_outline(text: &str, keys: &mut Keys, top_parent: Option<NodeId>) -> Tree {
    let mut nodes = Vec::new();
    let mut stack: Vec<(usize, NodeId)> = Vec::new();
    let mut counts: BTreeMap<Option<NodeId>, i64> = BTreeMap::new();
    for raw in text.lines() {
        if raw.trim().is_empty() || raw.trim_start().starts_with("//") {
            continue;
        }
        let indent = raw.len() - raw.trim_start_matches(' ').len();
        assert!(indent % 2 == 0, "odd indent: {raw:?}");
        let depth = indent / 2;
        while stack.last().is_some_and(|(d, _)| *d >= depth) {
            stack.pop();
        }
        let parent = stack
            .last()
            .map(|(_, id)| id.clone())
            .or(top_parent.clone());
        let line = &raw[indent..];
        let (main, note) = match line.find(" | ") {
            Some(i) => (&line[..i], Some(unescape(&line[i + 3..]))),
            None => match line.strip_suffix(" |") {
                Some(m) => (m, Some(String::new())),
                None => (line, None),
            },
        };
        let mut main = main.trim_end();
        let completed = match main.strip_suffix(" [x]") {
            Some(m) => {
                main = m;
                true
            }
            None => false,
        };
        let mut layout = "bullets".to_owned();
        if main.ends_with('}')
            && let Some(i) = main.rfind(" {")
        {
            layout = main[i + 2..main.len() - 1].to_owned();
            main = &main[..i];
        }
        let (key, name) = main.split_once(' ').unwrap_or((main, ""));
        let id = keys.id(key);
        let count = counts.entry(parent.clone()).or_default();
        *count += 1;
        let mut node = Node::new(id.clone(), parent, name);
        node.note = note;
        node.layout = layout;
        node.completed = completed;
        node.priority = 100 * *count;
        nodes.push(node);
        stack.push((depth, id));
    }
    Tree::from_nodes(nodes).expect("valid outline")
}

#[derive(Debug, Default)]
struct Fixture {
    name: String,
    path: PathBuf,
    config: Vec<String>,
    remote: String,
    merged: String,
    ops: Option<String>,
}

fn parse_fixture(path: PathBuf) -> Fixture {
    let text = std::fs::read_to_string(&path).expect("read fixture");
    let mut fx = Fixture {
        name: path.file_stem().unwrap().to_string_lossy().into_owned(),
        path,
        ..Fixture::default()
    };
    let mut section = String::new();
    let mut ops = None::<String>;
    for line in text.lines() {
        if let Some(s) = line.strip_prefix("== ") {
            section = s.trim().to_owned();
            if section == "ops" {
                ops = Some(String::new());
            }
            continue;
        }
        let buf = match section.as_str() {
            "" => continue,
            "config" => {
                if !line.trim().is_empty() {
                    fx.config.push(line.trim().to_owned());
                }
                continue;
            }
            "remote" => &mut fx.remote,
            "merged" => &mut fx.merged,
            "ops" => ops.as_mut().unwrap(),
            other => panic!("unknown section {other}"),
        };
        buf.push_str(line);
        buf.push('\n');
    }
    fx.ops = ops;
    fx
}

/// A parsed scenario: remote export, merge result, planner config, key names.
struct Scenario {
    keys: Keys,
    remote: Tree,
    merge: MergeResult,
    cfg: PlanConfig,
}

fn scenario(remote: &str, merged: &str, config: &[String]) -> Scenario {
    let mut keys = Keys::default();
    let remote_t = parse_outline(remote, &mut keys, None);
    let root_key = merged
        .lines()
        .find(|l| !l.trim().is_empty() && !l.trim_start().starts_with("//"))
        .expect("merged has a root")
        .split_whitespace()
        .next()
        .unwrap()
        .to_owned();
    let root = keys.get(&root_key);
    let top_parent = remote_t.parent(&root).cloned();
    let mut merged_t = parse_outline(merged, &mut keys, top_parent);
    // The root keeps its remote priority among its own siblings.
    let root_prio = remote_t.get(&root).expect("root in remote").priority;
    merged_t.set_priority(&root, root_prio).unwrap();
    let mut cfg = PlanConfig::default();
    for c in config {
        let mut words = c.split_whitespace();
        match words.next().unwrap() {
            "force" => cfg.force = true,
            "no-fast-path" => cfg.fast_path_creates = false,
            "roots" => cfg.sync_roots.extend(words.map(|k| keys.get(k))),
            "deferred" => cfg.deferred_deletes.extend(words.map(|k| keys.get(k))),
            other => panic!("unknown config {other}"),
        }
    }
    cfg.sync_roots.insert(root.clone());
    let merge = derive_merge(&remote_t, &merged_t, &root);
    Scenario {
        keys,
        remote: remote_t,
        merge,
        cfg,
    }
}

// =======================================================================================
// What the merge produces, derived from R and M (merge's postconditions)
// =======================================================================================

/// Compute the `MergeResult` whose `merged` is `m`: creates (temp ids, pre-order),
/// moves, pushes, ticks, top-most deletes and visible orders are exactly the
/// differences between `m` and `remote`.
fn derive_merge(remote: &Tree, m: &Tree, root: &NodeId) -> MergeResult {
    let order = m.preorder(root);
    let mut creates = Vec::new();
    let mut moves = Vec::new();
    let mut pushes = BTreeMap::new();
    let mut ticks = Vec::new();
    for id in &order {
        let n = m.get(id).unwrap();
        if id.is_temp() {
            creates.push(NewNode {
                temp: id.clone(),
                parent: n.parent_id.clone().unwrap(),
                name: n.name.clone(),
                note: n.note.clone(),
                layout: n.layout.clone(),
                ticked: n.completed,
                line: 0,
                resurrects: None,
            });
            continue;
        }
        let r = remote
            .get(id)
            .expect("real node of M is in the remote tree");
        if id != root && r.parent_id != n.parent_id {
            moves.push(MoveIntent {
                node: id.clone(),
                from: r.parent_id.clone().unwrap(),
                to: n.parent_id.clone().unwrap(),
            });
        }
        let mut push = FieldPush::default();
        if r.name != n.name {
            push.name = Some(Change {
                expected: r.name.clone(),
                new: n.name.clone(),
            });
        }
        if r.note != n.note {
            assert!(n.note.is_some(), "cannot push a null note");
            push.note = Some(Change {
                expected: r.note.clone(),
                new: n.note.clone(),
            });
        }
        if r.layout != n.layout {
            push.layout = Some(Change {
                expected: r.layout.clone(),
                new: n.layout.clone(),
            });
        }
        if push != FieldPush::default() {
            pushes.insert(id.clone(), push);
        }
        if n.completed && !r.completed {
            ticks.push(id.clone());
        }
        assert!(!(r.completed && !n.completed), "the API never uncompletes");
    }
    let mut deletes = Vec::new();
    let mut deleted_visible = 0;
    let mut deleted_hidden = 0;
    // What the file showed: the root's subtree, plus the subtrees of nodes moved in from
    // other roots (they arrive with their descendants).
    let mut scopes = vec![root.clone()];
    scopes.extend(
        order
            .iter()
            .filter(|id| !id.is_temp() && *id != root && !remote.is_descendant(id, root))
            .cloned(),
    );
    for id in scopes.iter().flat_map(|s| remote.descendants(s)) {
        if m.contains(&id) {
            continue;
        }
        if !m.contains(remote.parent(&id).unwrap()) {
            continue; // not top-most
        }
        let visible = remote.visible_descendants(&id).len();
        let hidden = remote.count_hidden_descendants(&id);
        deleted_visible += 1 + visible;
        deleted_hidden += hidden;
        deletes.push(DeleteIntent {
            node: id.clone(),
            visible_descendants: visible,
            hidden_descendants: hidden,
            label: remote.get(&id).unwrap().name.clone(),
        });
    }
    let mut orders = BTreeMap::new();
    for p in &order {
        if m.is_visible(p, root) {
            let kids = m.visible_children(p);
            if !kids.is_empty() {
                orders.insert(p.clone(), kids);
            }
        }
    }
    let pushed_fields = pushes
        .values()
        .map(|p: &FieldPush| {
            p.name.is_some() as usize + p.note.is_some() as usize + p.layout.is_some() as usize
        })
        .sum();
    let stats = MergeStats {
        base_visible: remote.visible_descendants(root).len(),
        local_items: m.visible_descendants(root).len(),
        deleted_visible,
        deleted_hidden,
        ticked: ticks.len() + creates.iter().filter(|c| c.ticked).count(),
        created: creates.len(),
        pushed_fields,
        conflicts: 0,
    };
    MergeResult {
        root: root.clone(),
        merged: m.clone(),
        pushes,
        creates,
        moves,
        ticks,
        deletes,
        orders,
        conflicts: Vec::new(),
        report: Vec::new(),
        stats,
        cancelled_pending: BTreeSet::new(),
        outside: BTreeSet::new(),
    }
}

// =======================================================================================
// Checks every successful plan must pass
// =======================================================================================

/// The §7.1 step of each op: 1 create, 2 re-parent, 3 update, 4 reorder, 5 complete,
/// 6 delete. Replays the ops on a copy of `remote` to tell moves apart.
fn phases(plan: &Plan, remote: &Tree) -> Vec<u8> {
    let mut s = remote.clone();
    let mut out = Vec::new();
    for op in &plan.ops {
        out.push(match op {
            Op::Create { .. } => 1,
            Op::Move { node, parent, .. } => {
                if s.parent(node) == Some(parent) {
                    4
                } else {
                    2
                }
            }
            Op::UpdateName { .. } | Op::UpdateNote { .. } | Op::UpdateLayout { .. } => 3,
            Op::Complete { .. } => 5,
            Op::Delete { .. } => 6,
        });
        plan::simulate_op(op, &mut s).unwrap();
    }
    out
}

/// Brute force: the longest run of `target` whose positions in `current` increase.
fn longest_valid_block(target: &[NodeId], current: &[NodeId]) -> usize {
    let pos = |id: &NodeId| current.iter().position(|c| c == id).unwrap();
    let mut best = 0;
    for i in 0..target.len() {
        for j in i..target.len() {
            if (i..j).all(|k| pos(&target[k]) < pos(&target[k + 1])) {
                best = best.max(j - i + 1);
            }
        }
    }
    best
}

/// Plan, then check the postconditions; returns the plan.
fn check_plan(merge: &MergeResult, remote: &Tree, cfg: &PlanConfig) -> Plan {
    let plan = plan::plan(merge, remote, cfg).unwrap_or_else(|e| panic!("plan failed: {e}"));
    let sim = plan::simulate(&plan, remote).unwrap_or_else(|e| panic!("simulate: {e}"));
    let root = &merge.root;

    // Deferred deletes stay in Workflowy; M does not have them.
    let mut sim_cmp = sim.clone();
    for d in &cfg.deferred_deletes {
        if sim_cmp.contains(d) {
            sim_cmp.remove_subtree(d).unwrap();
        }
    }
    // Structure, fields, completion and visible sibling order.
    let diffs = outline_diff(&merge.merged, &sim_cmp, root, DiffScope::Visible);
    assert!(diffs.is_empty(), "visible outline differs: {diffs:#?}");
    // Hidden nodes too (fields, parents, completion), except their order.
    let diffs: Vec<OutlineDiff> = outline_diff(&merge.merged, &sim_cmp, root, DiffScope::All)
        .into_iter()
        .filter(|d| !matches!(d, OutlineDiff::Order { .. }))
        .collect();
    assert!(diffs.is_empty(), "outline differs: {diffs:#?}");

    // §7.1 order.
    let ph = phases(&plan, remote);
    assert!(
        ph.windows(2).all(|w| w[0] <= w[1]),
        "ops out of §7.1 order: {ph:?}\n{:#?}",
        plan.ops
    );
    // Creates in the merge's order, parents first.
    let created: Vec<&NodeId> = plan
        .ops
        .iter()
        .filter_map(|op| match op {
            Op::Create { temp, .. } => Some(temp),
            _ => None,
        })
        .collect();
    let expected: Vec<&NodeId> = merge.creates.iter().map(|c| &c.temp).collect();
    assert_eq!(created, expected, "creates in merge order");
    // Every create is at the bottom of an explicit parent; every move carries one.
    // Hidden nodes are never moved or updated unless the merge asked for it.
    let asked: BTreeSet<&NodeId> = merge
        .pushes
        .keys()
        .chain(merge.moves.iter().map(|m| &m.node))
        .collect();
    for op in &plan.ops {
        if let Op::Move { node, .. }
        | Op::UpdateName { node, .. }
        | Op::UpdateNote { node, .. }
        | Op::UpdateLayout { node, .. } = op
        {
            let visible =
                node.is_temp() || cfg.sync_roots.iter().any(|r| remote.is_visible(node, r));
            assert!(
                visible || asked.contains(node),
                "hidden node {node} touched by {op:?}"
            );
        }
    }
    // §8.1: reorder moves per parent = n − |longest block| over S after steps 1–3.
    let mut s3 = remote.clone();
    for (op, p) in plan.ops.iter().zip(&ph) {
        if *p < 4 {
            plan::simulate_op(op, &mut s3).unwrap();
        }
    }
    for (p, target) in &merge.orders {
        let members: BTreeSet<&NodeId> = target.iter().collect();
        let current: Vec<NodeId> = s3
            .visible_children(p)
            .into_iter()
            .filter(|c| members.contains(c))
            .collect();
        let count = plan
            .ops
            .iter()
            .zip(&ph)
            .filter(|(op, ph)| **ph == 4 && matches!(op, Op::Move { parent, .. } if parent == p))
            .count();
        assert_eq!(
            count,
            target.len() - longest_valid_block(target, &current),
            "reorder cost for {p}"
        );
    }
    // Summary.
    let s = plan.summary;
    let count = |k: u8| ph.iter().filter(|p| **p == k).count();
    assert_eq!(s.creates, count(1));
    assert_eq!(s.moves, count(2));
    assert_eq!(s.updates, count(3));
    assert_eq!(s.reorders, count(4));
    assert_eq!(s.completes, count(5));
    assert_eq!(s.deletes, count(6));
    assert_eq!(s.requests, plan.ops.iter().map(Op::requests).sum::<usize>());
    plan
}

fn render_ops(plan: &Plan, keys: &Keys) -> String {
    let k = |id: &NodeId| keys.key(id);
    let mut out = String::new();
    for op in &plan.ops {
        let line = match op {
            Op::Create {
                temp,
                parent,
                name,
                note,
                layout,
            } => {
                let (how, value) = match name {
                    CreateName::Direct(v) => ("direct", v),
                    CreateName::Deferred(v) => ("deferred", v),
                };
                format!(
                    "create {} in {} {how} {value:?} note={note:?} layout={layout}",
                    k(temp),
                    k(parent)
                )
            }
            Op::UpdateName {
                node,
                expected,
                name,
            } => format!("update-name {} {expected:?} -> {name:?}", k(node)),
            Op::UpdateNote {
                node,
                expected,
                note,
            } => format!("update-note {} {expected:?} -> {note:?}", k(node)),
            Op::UpdateLayout {
                node,
                expected,
                layout,
            } => format!("update-layout {} {expected} -> {layout}", k(node)),
            Op::Move {
                node,
                parent,
                position,
                expected_parent,
            } => format!(
                "move {} to {} {} (from {})",
                k(node),
                k(parent),
                position.as_str(),
                expected_parent.as_ref().map_or("-".to_owned(), k)
            ),
            Op::Complete { node } => format!("complete {}", k(node)),
            Op::Delete {
                node,
                hidden_descendants,
                ..
            } => format!("delete {} (hidden {hidden_descendants})", k(node)),
        };
        out.push_str(&line);
        out.push('\n');
    }
    out
}

// =======================================================================================
// The corpus
// =======================================================================================

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/plan")
}

#[test]
fn corpus_plans_simulate_to_the_merged_tree() {
    let mut paths: Vec<PathBuf> = std::fs::read_dir(fixture_dir())
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "plan"))
        .collect();
    paths.sort();
    assert!(paths.len() >= 10, "corpus too small: {paths:?}");
    let bless = std::env::var_os("PLAN_BLESS").is_some();
    for path in paths {
        let fx = parse_fixture(path);
        let sc = scenario(&fx.remote, &fx.merged, &fx.config);
        let plan = check_plan(&sc.merge, &sc.remote, &sc.cfg);
        let rendered = render_ops(&plan, &sc.keys);
        match &fx.ops {
            Some(expected) if !bless => assert_eq!(
                expected.trim_end(),
                rendered.trim_end(),
                "fixture {} (rerun with PLAN_BLESS=1 to accept)",
                fx.name
            ),
            _ => {
                let text = std::fs::read_to_string(&fx.path).unwrap();
                let head = match text.find("\n== ops") {
                    Some(i) => &text[..i + 1],
                    None => &text,
                };
                let new = format!("{}== ops\n{rendered}", head.trim_end().to_owned() + "\n");
                std::fs::write(&fx.path, new).unwrap();
                eprintln!("wrote ops of {}", fx.name);
            }
        }
    }
}

// =======================================================================================
// Random scenarios
// =======================================================================================

const NAMES: &[&str] = &[
    "Alpha",
    "two words",
    "a &amp; b",
    "- dash",
    "<b>bold</b> text",
    "",
    "Ünïcode ✓",
    " padded ",
    "[2026-01-15]",
];
const NOTES: &[&str] = &["", "note", "multi\nline", "&lt;x&gt;"];
const LAYOUTS: &[&str] = &["bullets", "todo", "h1", "code-block"];

#[derive(Debug, Clone)]
enum Edit {
    Rename(usize, usize),
    Renote(usize, usize),
    Relayout(usize, usize),
    Create(usize, usize, usize, bool),
    Delete(usize),
    Move(usize, usize, usize),
    MoveIn(usize, usize, usize),
    Shuffle(usize, Vec<usize>),
    Tick(usize),
}

fn edit_strategy() -> impl Strategy<Value = Edit> {
    let i = || 0usize..1000;
    prop_oneof![
        1 => (i(), i()).prop_map(|(a, b)| Edit::Rename(a, b)),
        1 => (i(), i()).prop_map(|(a, b)| Edit::Renote(a, b)),
        1 => (i(), i()).prop_map(|(a, b)| Edit::Relayout(a, b)),
        3 => (i(), i(), i(), prop::bool::weighted(0.2))
            .prop_map(|(a, b, c, d)| Edit::Create(a, b, c, d)),
        1 => i().prop_map(Edit::Delete),
        3 => (i(), i(), i()).prop_map(|(a, b, c)| Edit::Move(a, b, c)),
        1 => (i(), i(), i()).prop_map(|(a, b, c)| Edit::MoveIn(a, b, c)),
        2 => (i(), prop::collection::vec(i(), 0..8)).prop_map(|(a, b)| Edit::Shuffle(a, b)),
        1 => i().prop_map(Edit::Tick),
    ]
}

/// One remote node of the random root subtree: (parent choice, completed, name, note).
type NodeSpec = (usize, bool, usize, usize);

fn uid(prefix: u32, i: usize) -> NodeId {
    NodeId::from_api(format!("{prefix:08x}-0000-4000-8000-{i:012x}"))
}

/// The remote export: `home` holds the sync root `root` (random subtree), another sync
/// root `other` (three nodes, for cross-file moves) and an unmanaged node.
fn random_remote(specs: &[NodeSpec]) -> (Tree, NodeId, NodeId) {
    let home = uid(0xa, 0);
    let root = uid(0xa, 1);
    let other = uid(0xa, 2);
    let mut nodes = vec![
        Node::new(home.clone(), None, "home"),
        Node::new(root.clone(), Some(home.clone()), "root"),
        Node::new(other.clone(), Some(home.clone()), "other"),
        Node::new(uid(0xa, 3), Some(home.clone()), "unmanaged"),
    ];
    for (i, n) in nodes.iter_mut().enumerate().skip(1) {
        n.priority = 100 * i as i64;
    }
    for i in 0..3 {
        let parent = if i == 1 { uid(0xc, 0) } else { other.clone() };
        let mut n = Node::new(uid(0xc, i), Some(parent), format!("other {i}"));
        n.priority = 100 * (i as i64 + 1);
        nodes.push(n);
    }
    let mut ids = vec![root.clone()];
    for (i, (pc, completed, name, note)) in specs.iter().enumerate() {
        let id = uid(0xb, i);
        let parent = ids[pc % ids.len()].clone();
        let mut n = Node::new(id.clone(), Some(parent), NAMES[name % NAMES.len()]);
        n.completed = *completed;
        n.note = match note % (NOTES.len() + 1) {
            0 => None,
            k => Some(NOTES[k - 1].to_owned()),
        };
        n.priority = 100 * (i as i64 + 1);
        nodes.push(n);
        ids.push(id);
    }
    (Tree::from_nodes(nodes).unwrap(), root, other)
}

/// Put the visible children of `parent` in `order`, keeping hidden ones in their slots.
fn set_visible_order(m: &mut Tree, parent: &NodeId, order: &[NodeId]) {
    let full = m.children(parent).to_vec();
    let mut it = order.iter();
    let new: Vec<NodeId> = full
        .iter()
        .map(|c| {
            if m.get(c).unwrap().completed {
                c.clone()
            } else {
                it.next().unwrap().clone()
            }
        })
        .collect();
    m.reorder_children(parent, &new).unwrap();
}

/// Move the (visible) child `node` of `parent` to visible index `k`.
fn insert_visible(m: &mut Tree, parent: &NodeId, node: &NodeId, k: usize) {
    let mut vis = m.visible_children(parent);
    vis.retain(|x| x != node);
    vis.insert(k % (vis.len() + 1), node.clone());
    set_visible_order(m, parent, &vis);
}

/// Apply local edits to `M` (starting as the remote subtree). Only visible nodes are
/// edited; hidden nodes keep their parent and slot (the merge's postconditions).
fn apply_edits(remote: &Tree, root: &NodeId, other: &NodeId, edits: &[Edit]) -> Tree {
    let mut m = remote.subtree(root).unwrap();
    let mut next_temp = 1;
    for e in edits {
        let visible = m.visible_descendants(root);
        let with_root: Vec<NodeId> = std::iter::once(root.clone())
            .chain(visible.iter().cloned())
            .collect();
        let pick = |i: usize, v: &[NodeId]| -> Option<NodeId> {
            (!v.is_empty()).then(|| v[i % v.len()].clone())
        };
        match e {
            Edit::Rename(i, n) => {
                let id = pick(*i, &with_root).unwrap();
                m.set_name(&id, NAMES[n % NAMES.len()]).unwrap();
            }
            Edit::Renote(i, n) => {
                let id = pick(*i, &with_root).unwrap();
                m.set_note(&id, Some(NOTES[n % NOTES.len()].to_owned()))
                    .unwrap();
            }
            Edit::Relayout(i, n) => {
                let id = pick(*i, &with_root).unwrap();
                m.set_layout(&id, LAYOUTS[n % LAYOUTS.len()]).unwrap();
            }
            Edit::Create(p, k, n, ticked) => {
                let parent = pick(*p, &with_root).unwrap();
                let id = NodeId::temp(next_temp);
                next_temp += 1;
                let mut node = Node::new(id.clone(), None, NAMES[n % NAMES.len()]);
                node.layout = LAYOUTS[n % LAYOUTS.len()].to_owned();
                if n % 3 == 0 {
                    node.note = Some(NOTES[n % NOTES.len()].to_owned());
                }
                m.place(node, Some(&parent), Position::Bottom).unwrap();
                if *ticked {
                    m.set_completed(&id, true, None).unwrap();
                } else {
                    insert_visible(&mut m, &parent, &id, *k);
                }
            }
            Edit::Delete(i) => {
                if let Some(id) = pick(*i, &visible) {
                    m.remove_subtree(&id).unwrap();
                }
            }
            Edit::Move(i, d, k) => {
                let (Some(id), Some(dest)) = (pick(*i, &visible), pick(*d, &with_root)) else {
                    continue;
                };
                if dest == id || m.is_descendant(&dest, &id) {
                    continue;
                }
                m.move_node(&id, Some(&dest), Position::Bottom).unwrap();
                insert_visible(&mut m, &dest, &id, *k);
            }
            Edit::MoveIn(i, d, k) => {
                let candidates: Vec<NodeId> = remote
                    .children(other)
                    .iter()
                    .filter(|c| !m.contains(c))
                    .cloned()
                    .collect();
                let Some(id) = pick(*i, &candidates) else {
                    continue;
                };
                let dest = pick(*d, &with_root).unwrap();
                let sub = remote.subtree(&id).unwrap();
                let fresh: Vec<NodeId> = sub
                    .preorder(&id)
                    .into_iter()
                    .filter(|n| !m.contains(n))
                    .collect();
                for nid in fresh {
                    let mut n = sub.get(&nid).unwrap().clone();
                    if nid == id {
                        n.parent_id = Some(dest.clone());
                        n.priority = i64::MAX / 2;
                    }
                    m.insert(n).unwrap();
                }
                insert_visible(&mut m, &dest, &id, *k);
            }
            Edit::Shuffle(p, swaps) => {
                let parent = pick(*p, &with_root).unwrap();
                let mut vis = m.visible_children(&parent);
                if vis.len() < 2 {
                    continue;
                }
                for (j, s) in swaps.iter().enumerate() {
                    let n = vis.len();
                    vis.swap(j % n, s % n);
                }
                set_visible_order(&mut m, &parent, &vis);
            }
            Edit::Tick(i) => {
                if let Some(id) = pick(*i, &visible) {
                    m.set_completed(&id, true, None).unwrap();
                }
            }
        }
    }
    m
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(400))]

    /// For random remote trees and random local edits, the plan simulates to `M`.
    #[test]
    fn random_scenarios_simulate_to_the_merged_tree(
        specs in prop::collection::vec(
            (0usize..100, prop::bool::weighted(0.2), 0usize..100, 0usize..100),
            0..14,
        ),
        edits in prop::collection::vec(edit_strategy(), 0..10),
        fast_path in any::<bool>(),
    ) {
        let (remote, root, other) = random_remote(&specs);
        let m = apply_edits(&remote, &root, &other, &edits);
        let merge = derive_merge(&remote, &m, &root);
        let cfg = PlanConfig {
            sync_roots: [root.clone(), other.clone()].into(),
            force: true,
            fast_path_creates: fast_path,
            ..PlanConfig::default()
        };
        let plan = check_plan(&merge, &remote, &cfg);
        for op in &plan.ops {
            if let Op::Create { name, .. } = op {
                let direct = matches!(name, CreateName::Direct(_));
                prop_assert_eq!(direct, fast_path && plan::create_name_is_safe(name.value()));
            }
        }
        // No edits, no ops.
        if edits.is_empty() {
            prop_assert!(plan.is_empty());
        }
    }
}

// =======================================================================================
// Guards (§8.2)
// =======================================================================================

fn guard_reason(r: Result<Plan, PlanError>) -> (GuardReason, Op) {
    match r {
        Err(PlanError::Guard { op, reason }) => (reason, *op),
        other => panic!("expected a guard error, got {other:?}"),
    }
}

const NESTED: &str = "\
home
  root
    a A
      b B
        c C
    d D
  other
    o O
  outside
    x X
";

const NESTED_M: &str = "\
root
  a A
    b B
      c C
  d D
";

#[test]
fn cycle_guard_refuses_a_move_under_a_descendant() {
    let mut sc = scenario(NESTED, NESTED_M, &[]);
    let (a, c) = (sc.keys.get("a"), sc.keys.get("c"));
    sc.merge.moves.push(MoveIntent {
        node: a.clone(),
        from: sc.keys.get("root"),
        to: c.clone(),
    });
    let (reason, op) = guard_reason(plan::plan(&sc.merge, &sc.remote, &sc.cfg));
    assert_eq!(reason, GuardReason::Cycle);
    assert!(matches!(op, Op::Move { node, parent, .. } if node == a && parent == c));
    // Under itself.
    sc.merge.moves = vec![MoveIntent {
        node: a.clone(),
        from: sc.keys.get("root"),
        to: a.clone(),
    }];
    let (reason, _) = guard_reason(plan::plan(&sc.merge, &sc.remote, &sc.cfg));
    assert_eq!(reason, GuardReason::Cycle);
}

#[test]
fn cycle_guard_sees_cycles_created_by_earlier_moves() {
    // a under d is fine; then d under b (a's descendant) closes a cycle d -> a -> b -> d
    // that exists only after the first move.
    let mut sc = scenario(NESTED, NESTED_M, &[]);
    let (a, b, d, root) = (
        sc.keys.get("a"),
        sc.keys.get("b"),
        sc.keys.get("d"),
        sc.keys.get("root"),
    );
    sc.merge.moves = vec![
        MoveIntent {
            node: a.clone(),
            from: root.clone(),
            to: d.clone(),
        },
        MoveIntent {
            node: d.clone(),
            from: root.clone(),
            to: b.clone(),
        },
    ];
    // Each move alone is valid against the remote tree.
    for m in &sc.merge.moves {
        assert!(!sc.remote.is_descendant(&m.to, &m.node));
    }
    let (reason, op) = guard_reason(plan::plan(&sc.merge, &sc.remote, &sc.cfg));
    assert_eq!(reason, GuardReason::Cycle);
    let Op::Move { node, .. } = op else {
        panic!("{op:?}")
    };
    assert!(node == a || node == d);
}

#[test]
fn moves_are_ordered_to_avoid_transient_cycles() {
    // Swap a parent and its child: root{a{b}} -> root{b{a}}. Listed child-first, the
    // first move (a under b) is a cycle at that point; the planner moves b out first.
    let remote = "home\n  root\n    a A\n      b B\n";
    let merged = "root\n  b B\n    a A\n";
    let mut sc = scenario(remote, merged, &[]);
    sc.merge.moves.reverse();
    assert_eq!(sc.merge.moves[0].node, sc.keys.get("a"));
    let plan = check_plan(&sc.merge, &sc.remote, &sc.cfg);
    assert_eq!(
        render_ops(&plan, &sc.keys),
        "move b to root bottom (from a)\nmove a to b bottom (from root)\n"
    );
}

#[test]
fn boundary_guard_keeps_moves_inside_sync_roots() {
    let mut sc = scenario(NESTED, NESTED_M, &[]);
    let (a, x, o, root) = (
        sc.keys.get("a"),
        sc.keys.get("x"),
        sc.keys.get("o"),
        sc.keys.get("root"),
    );
    // Destination outside every sync root.
    sc.merge.moves = vec![MoveIntent {
        node: a.clone(),
        from: root.clone(),
        to: x.clone(),
    }];
    let (reason, _) = guard_reason(plan::plan(&sc.merge, &sc.remote, &sc.cfg));
    assert_eq!(reason, GuardReason::OutsideSyncRoots);
    // A node from an unmanaged part of the account.
    sc.merge.moves = vec![MoveIntent {
        node: x.clone(),
        from: sc.keys.get("outside"),
        to: root.clone(),
    }];
    let (reason, _) = guard_reason(plan::plan(&sc.merge, &sc.remote, &sc.cfg));
    assert_eq!(reason, GuardReason::OutsideSyncRoots);
    // A node from another root is refused unless that root is managed in this run.
    sc.merge.moves = vec![MoveIntent {
        node: o.clone(),
        from: sc.keys.get("other"),
        to: root.clone(),
    }];
    let (reason, _) = guard_reason(plan::plan(&sc.merge, &sc.remote, &sc.cfg));
    assert_eq!(reason, GuardReason::OutsideSyncRoots);
    // A sync root itself is never moved, even into another managed root.
    sc.cfg.sync_roots.insert(sc.keys.get("other"));
    sc.merge.moves = vec![MoveIntent {
        node: root.clone(),
        from: sc.keys.get("home"),
        to: o.clone(),
    }];
    let (reason, _) = guard_reason(plan::plan(&sc.merge, &sc.remote, &sc.cfg));
    assert_eq!(reason, GuardReason::OutsideSyncRoots);
}

#[test]
fn boundary_guard_allows_moves_between_managed_roots() {
    let merged = "\
root
  a A
    b B
      c C
  d D
  o O
";
    let sc = scenario(NESTED, merged, &["roots other".to_owned()]);
    let plan = check_plan(&sc.merge, &sc.remote, &sc.cfg);
    assert_eq!(
        render_ops(&plan, &sc.keys),
        "move o to root bottom (from other)\n"
    );
    // Without `other` in the run's roots, the same move is refused.
    let mut cfg = sc.cfg.clone();
    cfg.sync_roots.remove(&sc.keys.get("other"));
    let (reason, _) = guard_reason(plan::plan(&sc.merge, &sc.remote, &cfg));
    assert_eq!(reason, GuardReason::OutsideSyncRoots);
}

#[test]
fn guards_keep_writes_inside_the_root() {
    let base = scenario(NESTED, NESTED_M, &[]);
    let (x, root) = (base.keys.get("x"), base.keys.get("root"));

    let mut sc = scenario(NESTED, NESTED_M, &[]);
    sc.merge.pushes.insert(
        x.clone(),
        FieldPush {
            name: Some(Change {
                expected: "X".into(),
                new: "Y".into(),
            }),
            ..FieldPush::default()
        },
    );
    let (reason, _) = guard_reason(plan::plan(&sc.merge, &sc.remote, &sc.cfg));
    assert_eq!(reason, GuardReason::OutsideRoot);

    let mut sc = scenario(NESTED, NESTED_M, &[]);
    sc.merge.ticks.push(x.clone());
    let (reason, _) = guard_reason(plan::plan(&sc.merge, &sc.remote, &sc.cfg));
    assert_eq!(reason, GuardReason::OutsideRoot);

    // A node moved out remotely whose local edits follow it (§6.2, `merge.outside`):
    // its update and completion are planned and handed to apply; a delete is not.
    let mut sc = scenario(NESTED, NESTED_M, &[]);
    sc.merge.pushes.insert(
        x.clone(),
        FieldPush {
            name: Some(Change {
                expected: "X".into(),
                new: "Y".into(),
            }),
            ..FieldPush::default()
        },
    );
    sc.merge.ticks.push(x.clone());
    sc.merge.outside.insert(x.clone());
    let p = plan::plan(&sc.merge, &sc.remote, &sc.cfg).expect("plan");
    assert!(
        matches!(&p.ops[..], [Op::UpdateName { node: a, .. }, Op::Complete { node: b }] if a == &x && b == &x)
    );
    assert!(p.outside.contains(&x));
    sc.merge.deletes.push(DeleteIntent {
        node: x.clone(),
        visible_descendants: 0,
        hidden_descendants: 0,
        label: String::new(),
    });
    let (reason, _) = guard_reason(plan::plan(&sc.merge, &sc.remote, &sc.cfg));
    assert_eq!(reason, GuardReason::OutsideRoot);

    for target in [x.clone(), root.clone()] {
        let mut sc = scenario(NESTED, NESTED_M, &["force".to_owned()]);
        sc.merge.deletes.push(DeleteIntent {
            node: target,
            visible_descendants: 0,
            hidden_descendants: 0,
            label: String::new(),
        });
        let (reason, _) = guard_reason(plan::plan(&sc.merge, &sc.remote, &sc.cfg));
        assert_eq!(reason, GuardReason::OutsideRoot);
    }

    let mut sc = scenario(NESTED, NESTED_M, &[]);
    sc.merge.creates.push(NewNode {
        temp: NodeId::temp(1),
        parent: x.clone(),
        name: "n".into(),
        note: None,
        layout: "bullets".into(),
        ticked: false,
        line: 0,
        resurrects: None,
    });
    let (reason, _) = guard_reason(plan::plan(&sc.merge, &sc.remote, &sc.cfg));
    assert_eq!(reason, GuardReason::OutsideRoot);

    let mut sc = scenario(NESTED, NESTED_M, &[]);
    sc.merge.creates.push(NewNode {
        temp: NodeId::temp(1),
        parent: NodeId::temp(7),
        name: "n".into(),
        note: None,
        layout: "bullets".into(),
        ticked: false,
        line: 0,
        resurrects: None,
    });
    let (reason, _) = guard_reason(plan::plan(&sc.merge, &sc.remote, &sc.cfg));
    assert_eq!(reason, GuardReason::UnknownNode);
}

#[test]
fn hidden_siblings_in_a_target_order_are_refused() {
    let remote = "home\n  root\n    a A\n    h H [x]\n    b B\n";
    let merged = "root\n  a A\n  h H [x]\n  b B\n";
    let mut sc = scenario(remote, merged, &[]);
    let root = sc.keys.get("root");
    sc.merge.orders.insert(
        root,
        vec![sc.keys.get("b"), sc.keys.get("h"), sc.keys.get("a")],
    );
    let (reason, _) = guard_reason(plan::plan(&sc.merge, &sc.remote, &sc.cfg));
    assert_eq!(reason, GuardReason::HiddenNode);
}

#[test]
fn inconsistent_merge_results_are_refused() {
    let mut sc = scenario(NESTED, NESTED_M, &[]);
    let a = sc.keys.get("a");
    sc.merge.pushes.insert(
        a,
        FieldPush {
            name: Some(Change {
                expected: "not the remote value".into(),
                new: "Z".into(),
            }),
            ..FieldPush::default()
        },
    );
    assert!(matches!(
        plan::plan(&sc.merge, &sc.remote, &sc.cfg),
        Err(PlanError::Inconsistent(_))
    ));
    let mut sc = scenario(NESTED, NESTED_M, &[]);
    sc.merge.root = NodeId::from_api("ffffffff-ffff-4fff-8fff-ffffffffffff");
    assert!(matches!(
        plan::plan(&sc.merge, &sc.remote, &sc.cfg),
        Err(PlanError::Inconsistent(_))
    ));
}

// =======================================================================================
// Brakes (§8.3)
// =======================================================================================

/// A root with `n` visible children `c0…`; the first `hidden_parents` of them have two
/// completed children each.
fn wide(n: usize, hidden_parents: usize) -> (String, Vec<String>) {
    let mut remote = String::from("home\n  root\n");
    let mut lines = Vec::new();
    for i in 0..n {
        let mut block = format!("    c{i} item {i}\n");
        if i < hidden_parents {
            block.push_str(&format!("      h{i}a done [x]\n      h{i}b done [x]\n"));
        }
        remote.push_str(&block);
        lines.push(block);
    }
    (remote, lines)
}

/// M = the wide tree without the first `deleted` children, with the next `ticked`
/// children ticked.
fn wide_scenario(
    n: usize,
    hidden_parents: usize,
    deleted: usize,
    ticked: usize,
    config: &[&str],
) -> Scenario {
    let (remote, lines) = wide(n, hidden_parents);
    let mut merged = String::from("root\n");
    for (i, block) in lines.iter().enumerate().skip(deleted) {
        if i < deleted + ticked {
            let mut it = block.lines();
            merged.push_str(&format!("{} [x]\n", &it.next().unwrap()[2..]));
            for l in it {
                merged.push_str(&format!("{}\n", &l[2..]));
            }
        } else {
            for l in block.lines() {
                merged.push_str(&format!("{}\n", &l[2..]));
            }
        }
    }
    let config: Vec<String> = config.iter().map(|s| s.to_string()).collect();
    scenario(&remote, &merged, &config)
}

fn brake(sc: &Scenario) -> Option<Brake> {
    match plan::plan(&sc.merge, &sc.remote, &sc.cfg) {
        Ok(_) => None,
        Err(PlanError::Brake(b)) => Some(b),
        Err(e) => panic!("{e}"),
    }
}

#[test]
fn delete_brake_fraction_threshold() {
    // Default: fraction 0.2 from 5 deletes on. 20 visible: 4 deletes pass, 5 brake.
    assert_eq!(brake(&wide_scenario(20, 0, 4, 0, &[])), None);
    assert_eq!(
        brake(&wide_scenario(20, 0, 5, 0, &[])),
        Some(Brake::TooManyDeletes {
            visible: 5,
            hidden: 0,
            base_visible: 20
        })
    );
    // 30 visible: 6 = 20% passes (not more than), 7 brakes.
    assert_eq!(brake(&wide_scenario(30, 0, 6, 0, &[])), None);
    assert!(matches!(
        brake(&wide_scenario(30, 0, 7, 0, &[])),
        Some(Brake::TooManyDeletes { .. })
    ));
}

#[test]
fn delete_brake_counts_hidden_descendants() {
    // 2 visible deletes are under every threshold, but they take 4 hidden (completed)
    // children with them: 6 >= 5 and 6 > 20% of 20.
    assert_eq!(brake(&wide_scenario(20, 0, 2, 0, &[])), None);
    assert_eq!(
        brake(&wide_scenario(20, 2, 2, 0, &[])),
        Some(Brake::TooManyDeletes {
            visible: 2,
            hidden: 4,
            base_visible: 20
        })
    );
}

#[test]
fn delete_brake_absolute_threshold() {
    // 300 visible: 50 deletes (17%) pass, 51 exceed the absolute limit of 50.
    assert_eq!(brake(&wide_scenario(300, 0, 50, 0, &[])), None);
    assert_eq!(
        brake(&wide_scenario(300, 0, 51, 0, &[])),
        Some(Brake::TooManyDeletes {
            visible: 51,
            hidden: 0,
            base_visible: 300
        })
    );
}

#[test]
fn empty_file_brake() {
    assert_eq!(
        brake(&wide_scenario(3, 0, 3, 0, &[])),
        Some(Brake::EmptyFile { base_visible: 3 })
    );
    // An empty base is not a brake.
    assert_eq!(brake(&wide_scenario(0, 0, 0, 0, &[])), None);
}

#[test]
fn empty_file_brake_holds_when_the_merge_keeps_a_deleted_node() {
    // The file was truncated to its frontmatter, but a collaborator had edited c2 (or c2
    // is a mirror placeholder): the merge keeps c2 (delete vs edit, §6.2; §10) and
    // deletes only c0 and c1. The file is still empty, so the brake holds (§8.3).
    let mut sc = wide_scenario(3, 0, 2, 0, &[]);
    assert_eq!(sc.merge.stats.deleted_visible, 2);
    sc.merge.stats.local_items = 0;
    assert_eq!(brake(&sc), Some(Brake::EmptyFile { base_visible: 3 }));
    // The same merge with the item still in the file is an ordinary small delete.
    sc.merge.stats.local_items = 1;
    assert_eq!(brake(&sc), None);
}

#[test]
fn empty_file_that_deletes_nothing_is_not_braked() {
    // Every item of the file was cut into another file of the run (§6.4): the other
    // file moves the nodes, this merge deletes nothing, so nothing is lost (§8.3).
    let mut sc = wide_scenario(3, 0, 0, 0, &[]);
    sc.merge.stats.local_items = 0;
    assert_eq!(brake(&sc), None);
    // One delete is enough to make the empty file a brake again.
    let mut sc = wide_scenario(3, 0, 1, 0, &[]);
    sc.merge.stats.local_items = 0;
    assert_eq!(brake(&sc), Some(Brake::EmptyFile { base_visible: 3 }));
}

#[test]
fn completion_brake_has_its_own_higher_thresholds() {
    // Ticks are not deletes: 10 of 20 (50%) pass, where 10 deletes would brake.
    assert_eq!(brake(&wide_scenario(20, 0, 0, 10, &[])), None);
    assert_eq!(
        brake(&wide_scenario(20, 0, 0, 11, &[])),
        Some(Brake::TooManyCompletes {
            completes: 11,
            base_visible: 20
        })
    );
    // Absolute: 200 of 1000 pass, 201 brake.
    assert_eq!(brake(&wide_scenario(1000, 0, 0, 200, &[])), None);
    assert!(matches!(
        brake(&wide_scenario(1000, 0, 0, 201, &[])),
        Some(Brake::TooManyCompletes { completes: 201, .. })
    ));
}

#[test]
fn force_bypasses_every_brake() {
    for sc in [
        wide_scenario(20, 2, 5, 0, &["force"]),
        wide_scenario(300, 0, 51, 0, &["force"]),
        wide_scenario(3, 0, 3, 0, &["force"]),
        wide_scenario(20, 0, 0, 11, &["force"]),
    ] {
        let plan = check_plan(&sc.merge, &sc.remote, &sc.cfg);
        assert!(!plan.is_empty());
    }
}

#[test]
fn brake_thresholds_are_configurable() {
    let mut sc = wide_scenario(20, 0, 2, 0, &[]);
    assert_eq!(brake(&sc), None);
    sc.cfg.brakes = BrakeConfig {
        max_deletes: 1,
        ..BrakeConfig::default()
    };
    assert!(matches!(brake(&sc), Some(Brake::TooManyDeletes { .. })));
}

#[test]
fn deferred_deletes_count_toward_brakes_but_are_not_sent() {
    let sc = wide_scenario(20, 0, 5, 0, &["deferred c0 c1 c2 c3 c4"]);
    assert!(matches!(brake(&sc), Some(Brake::TooManyDeletes { .. })));
    let sc = wide_scenario(20, 0, 2, 0, &["deferred c0"]);
    let plan = check_plan(&sc.merge, &sc.remote, &sc.cfg);
    assert_eq!(render_ops(&plan, &sc.keys), "delete c1 (hidden 0)\n");
}

#[test]
fn relisted_pending_deletes_do_not_brake_again() {
    // The merge re-lists still-pending deletes of earlier cycles in `deletes` but leaves
    // them out of `stats` (they were counted when first deleted, §9.1 guard 2). When one
    // falls due its delete is sent; it must not count again (here 5 due deletes would
    // exceed the fraction brake). The base is the render after the first delete, so it
    // no longer shows them: 15 visible, all still in the file.
    let mut sc = wide_scenario(20, 0, 5, 0, &[]);
    assert!(matches!(brake(&sc), Some(Brake::TooManyDeletes { .. })));
    sc.merge.stats.base_visible = 15;
    sc.merge.stats.deleted_visible = 0;
    sc.merge.stats.deleted_hidden = 0;
    assert_eq!(brake(&sc), None);
    let plan = check_plan(&sc.merge, &sc.remote, &sc.cfg);
    assert_eq!(plan.summary.deletes, 5);
    // The same when the due deletes were the whole file: the render that followed them
    // was empty, so the base is too, and an empty base is not an emptied file.
    let mut sc = wide_scenario(3, 0, 3, 0, &[]);
    assert_eq!(brake(&sc), Some(Brake::EmptyFile { base_visible: 3 }));
    sc.merge.stats.base_visible = 0;
    sc.merge.stats.deleted_visible = 0;
    sc.merge.stats.deleted_hidden = 0;
    assert_eq!(brake(&sc), None);
    let plan = check_plan(&sc.merge, &sc.remote, &sc.cfg);
    assert_eq!(plan.summary.deletes, 3);
}

// =======================================================================================
// Create fast path (§7.2) and the plan summary
// =======================================================================================

#[test]
fn create_fast_path_uses_the_whitelist() {
    let remote = "home\n  root\n";
    let merged = "root\n  +1 Plain name\n  +2 a &amp; b\n  +3\n  +4 - dash\n";
    let sc = scenario(remote, merged, &[]);
    let plan = check_plan(&sc.merge, &sc.remote, &sc.cfg);
    assert_eq!(
        render_ops(&plan, &sc.keys),
        "create +1 in root direct \"Plain name\" note=None layout=bullets\n\
         create +2 in root deferred \"a &amp; b\" note=None layout=bullets\n\
         create +3 in root deferred \"\" note=None layout=bullets\n\
         create +4 in root deferred \"- dash\" note=None layout=bullets\n"
    );
    assert_eq!(plan.summary.creates, 4);
    assert_eq!(plan.summary.requests, 1 + 2 + 2 + 2);
    // Off: every create is deferred.
    let sc = scenario(remote, merged, &["no-fast-path".to_owned()]);
    let plan = check_plan(&sc.merge, &sc.remote, &sc.cfg);
    assert!(plan.ops.iter().all(|op| matches!(
        op,
        Op::Create {
            name: CreateName::Deferred(_),
            ..
        }
    )));
    assert_eq!(plan.summary.requests, 8);
}

#[test]
fn summary_counts_every_kind() {
    let remote = "\
home
  root
    a A
      a1 A1
    b B
      b1 B1 [x]
    c C
    d D
";
    let merged = "\
root
  c C
  a A2 | note
    +1 New
  d D [x]
";
    let sc = scenario(remote, merged, &["force".to_owned()]);
    let plan = check_plan(&sc.merge, &sc.remote, &sc.cfg);
    let s = plan.summary;
    assert_eq!(
        (
            s.creates,
            s.moves,
            s.updates,
            s.reorders,
            s.completes,
            s.deletes
        ),
        (1, 0, 2, 1, 1, 2)
    );
    assert_eq!(s.hidden_deleted, 1);
}

#[test]
fn priority_ties_are_never_a_reason_to_write() {
    // a and b share a priority (a UI race, §6.3). An unchanged order writes nothing; a
    // change elsewhere moves only what changed and leaves the tie as it was.
    let remote = "home\n  root\n    a A\n    b B\n    c C\n    d D\n";
    let tie = |sc: &mut Scenario| {
        let (a, b) = (sc.keys.get("a"), sc.keys.get("b"));
        let p = sc.remote.get(&a).unwrap().priority;
        sc.remote.set_priority(&b, p).unwrap();
        assert!(sc.remote.has_priority_ties(Some(&sc.keys.get("root"))));
        sc.merge.merged.set_priority(&b, p).unwrap();
    };
    let mut sc = scenario(remote, "root\n  a A\n  b B\n  c C\n  d D\n", &[]);
    tie(&mut sc);
    let plan = check_plan(&sc.merge, &sc.remote, &sc.cfg);
    assert!(plan.is_empty(), "{:?}", plan.ops);

    let mut sc = scenario(remote, "root\n  a A\n  b B\n  d D\n  c C\n", &[]);
    tie(&mut sc);
    let plan = check_plan(&sc.merge, &sc.remote, &sc.cfg);
    assert_eq!(
        render_ops(&plan, &sc.keys),
        "move c to root bottom (from root)\n"
    );
}
