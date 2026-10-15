//! Property tests of the merge (design §5.1, §6, and the postconditions in the `merge`
//! module docs), with the plain-text codec.
//!
//! Random outlines are built from op lists: a base tree (some nodes completed, so
//! hidden), local edits of its rendered doc (rename, note, delete, insert, move, tick),
//! and remote edits of the tree (rename, note, complete/uncomplete, delete, create,
//! move).

#[path = "merge_support.rs"]
mod merge_support;

use std::collections::{BTreeMap, BTreeSet};

use merge_support::{Case, DocSpec, ItemSpec, build_doc, id, lbl, render_spec};
use proptest::prelude::*;
use syncflowy::mdblock::DocConfig;
use syncflowy::merge::{MergeResult, ReportEvent, merge_sibling_order, visible_nodes};
use syncflowy::tree::{DiffScope, Node, NodeId, OutlineDiff, Position, Tree, outline_diff};

// ---------------------------------------------------------------------------------------
// Generators
// ---------------------------------------------------------------------------------------

/// A pseudo-random 6-letter word for `seed` (distinct seeds give dissimilar words, so
/// the §4.4 fallback rarely pairs unrelated items).
fn word(seed: u64) -> String {
    let mut x = seed
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    (0..6)
        .map(|_| {
            x = x
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (b'a' + ((x >> 33) % 26) as u8) as char
        })
        .collect()
}

fn root() -> NodeId {
    id("root")
}

/// Base tree: each op adds a node under an existing one.
fn build_base(ops: &[(u8, u8, bool)]) -> Tree {
    let r = root();
    let mut t = Tree::from_nodes([Node::new(r.clone(), None, "Root")]).expect("tree");
    let mut all = vec![r];
    for (k, (parent, flags, note)) in ops.iter().enumerate() {
        let p = all[*parent as usize % all.len()].clone();
        let nid = id(&format!("n{k}"));
        let mut n = Node::new(nid.clone(), Some(p.clone()), word(k as u64));
        if *note {
            n.note = Some(format!(
                "{}\n{}",
                word(1000 + k as u64),
                word(2000 + k as u64)
            ));
        }
        t.place(n, Some(&p), Position::Bottom).expect("place");
        if flags % 6 == 0 {
            t.set_completed(&nid, true, Some(1)).expect("complete");
        }
        all.push(nid);
    }
    t
}

fn preorder_paths(items: &[ItemSpec], prefix: &mut Vec<usize>, out: &mut Vec<Vec<usize>>) {
    for (i, it) in items.iter().enumerate() {
        prefix.push(i);
        out.push(prefix.clone());
        preorder_paths(&it.children, prefix, out);
        prefix.pop();
    }
}

fn paths(items: &[ItemSpec]) -> Vec<Vec<usize>> {
    let mut out = Vec::new();
    preorder_paths(items, &mut Vec::new(), &mut out);
    out
}

fn list_at<'a>(items: &'a mut Vec<ItemSpec>, path: &[usize]) -> &'a mut Vec<ItemSpec> {
    match path.split_first() {
        None => items,
        Some((first, rest)) => list_at(&mut items[*first].children, rest),
    }
}

fn item_at<'a>(items: &'a mut [ItemSpec], path: &[usize]) -> &'a mut ItemSpec {
    let (first, rest) = path.split_first().expect("non-empty path");
    if rest.is_empty() {
        &mut items[*first]
    } else {
        item_at(&mut items[*first].children, rest)
    }
}

fn remove_at(items: &mut Vec<ItemSpec>, path: &[usize]) -> ItemSpec {
    let (last, parent) = path.split_last().expect("non-empty path");
    list_at(items, parent).remove(*last)
}

/// Local edit kinds: 0 rename, 1 note, 2 delete, 3 insert, 4 move, 5 tick.
type LocalOp = (u8, u16, u16, u16);

fn apply_local(spec: &mut DocSpec, ops: &[LocalOp], allowed: &[u8], typed: &mut Vec<String>) {
    for (k, (kind, a, b, c)) in ops.iter().enumerate() {
        let kind = allowed[*kind as usize % allowed.len()];
        let ps = paths(&spec.items);
        let seed = 50_000 + k as u64;
        match kind {
            0 | 1 | 2 | 4 | 5 if ps.is_empty() => {}
            0 => {
                let w = word(seed);
                typed.push(w.clone());
                item_at(&mut spec.items, &ps[*a as usize % ps.len()]).name = w;
            }
            1 => {
                item_at(&mut spec.items, &ps[*a as usize % ps.len()]).note = vec![word(seed)];
            }
            2 => {
                remove_at(&mut spec.items, &ps[*a as usize % ps.len()]);
            }
            3 => {
                let w = word(seed);
                typed.push(w.clone());
                let parent: Vec<usize> = match *b as usize % (ps.len() + 1) {
                    0 => Vec::new(),
                    i => ps[i - 1].clone(),
                };
                let list = list_at(&mut spec.items, &parent);
                let at = *c as usize % (list.len() + 1);
                list.insert(
                    at,
                    ItemSpec {
                        name: w,
                        ..ItemSpec::default()
                    },
                );
            }
            4 => {
                let moved = remove_at(&mut spec.items, &ps[*a as usize % ps.len()]);
                let ps = paths(&spec.items);
                let parent: Vec<usize> = match *b as usize % (ps.len() + 1) {
                    0 => Vec::new(),
                    i => ps[i - 1].clone(),
                };
                let list = list_at(&mut spec.items, &parent);
                let at = *c as usize % (list.len() + 1);
                list.insert(at, moved);
            }
            _ => {
                item_at(&mut spec.items, &ps[*a as usize % ps.len()]).check = Some("x".into());
            }
        }
    }
}

/// Remote edit kinds: 0 rename, 1 note, 2 toggle completion, 3 delete, 4 create,
/// 5 move.
type RemoteOp = (u8, u16, u16, bool);

fn apply_remote(t: &mut Tree, ops: &[RemoteOp], allowed: &[u8]) {
    let r = root();
    for (k, (kind, a, b, top)) in ops.iter().enumerate() {
        let kind = allowed[*kind as usize % allowed.len()];
        let nodes: Vec<NodeId> = t.descendants(&r);
        let seed = 90_000 + k as u64;
        let pos = if *top {
            Position::Top
        } else {
            Position::Bottom
        };
        if nodes.is_empty() && kind != 4 {
            continue;
        }
        let pick = |i: u16| nodes[i as usize % nodes.len()].clone();
        match kind {
            0 => t.set_name(&pick(*a), word(seed)).expect("rename"),
            1 => t.set_note(&pick(*a), Some(word(seed))).expect("note"),
            2 => {
                let n = pick(*a);
                let c = t.get(&n).expect("node").completed;
                t.set_completed(&n, !c, (!c).then_some(2))
                    .expect("complete");
            }
            3 => {
                t.remove_subtree(&pick(*a)).expect("delete");
            }
            4 => {
                let mut all = vec![r.clone()];
                all.extend(nodes.iter().cloned());
                let p = all[*b as usize % all.len()].clone();
                let n = Node::new(id(&format!("r{k}")), Some(p.clone()), word(seed));
                t.place(n, Some(&p), pos).expect("create");
            }
            _ => {
                let n = pick(*a);
                let mut all = vec![r.clone()];
                all.extend(nodes.iter().cloned());
                let p = all[*b as usize % all.len()].clone();
                // Cycles are refused by the tree, as by a sane API client.
                let _ = t.move_node(&n, Some(&p), pos);
            }
        }
    }
}

fn base_ops() -> impl Strategy<Value = Vec<(u8, u8, bool)>> {
    prop::collection::vec((any::<u8>(), any::<u8>(), any::<bool>()), 0..12)
}

fn local_ops() -> impl Strategy<Value = Vec<LocalOp>> {
    prop::collection::vec(
        (any::<u8>(), any::<u16>(), any::<u16>(), any::<u16>()),
        0..8,
    )
}

fn remote_ops() -> impl Strategy<Value = Vec<RemoteOp>> {
    prop::collection::vec(
        (any::<u8>(), any::<u16>(), any::<u16>(), any::<bool>()),
        0..8,
    )
}

const ALL_LOCAL: &[u8] = &[0, 1, 2, 3, 4, 5];
const ALL_REMOTE: &[u8] = &[0, 1, 2, 3, 4, 5];

struct Scenario {
    case: Case,
    local_spec: DocSpec,
    typed: Vec<String>,
}

fn scenario(
    base_ops: &[(u8, u8, bool)],
    local: &[LocalOp],
    local_kinds: &[u8],
    remote: &[RemoteOp],
    remote_kinds: &[u8],
) -> Scenario {
    let r = root();
    let base = build_base(base_ops);
    let mut remote_tree = base.clone();
    apply_remote(&mut remote_tree, remote, remote_kinds);
    let base_spec = render_spec(&base, &r, &DocConfig::default(), &BTreeSet::new());
    let mut local_spec = base_spec.clone();
    let mut typed = Vec::new();
    apply_local(&mut local_spec, local, local_kinds, &mut typed);
    let case = Case::new(
        r.clone(),
        base,
        remote_tree,
        build_doc(&r, &base_spec),
        build_doc(&r, &local_spec),
    );
    Scenario {
        case,
        local_spec,
        typed,
    }
}

// ---------------------------------------------------------------------------------------
// Helpers for the checks
// ---------------------------------------------------------------------------------------

/// A tag-free outline: (name, note, children).
#[derive(Debug, Clone, PartialEq, Eq)]
struct Shape(String, Vec<String>, Vec<Shape>);

fn shape(items: &[ItemSpec]) -> Vec<Shape> {
    items
        .iter()
        .filter(|i| i.check.as_deref() != Some("x"))
        .map(|i| Shape(i.name.clone(), i.note.clone(), shape(&i.children)))
        .collect()
}

fn base_visible(case: &Case) -> BTreeSet<NodeId> {
    let mut out = BTreeSet::new();
    fn walk(items: &[syncflowy::mdblock::LocalItem], out: &mut BTreeSet<NodeId>, case: &Case) {
        for it in items {
            if let Some(t) = &it.tag
                && let Some(n) = case.ids.get(&t.short)
            {
                out.insert(n.clone());
            }
            walk(&it.children, out, case);
        }
    }
    walk(&case.base_doc.items, &mut out, case);
    out
}

fn deleted_in_remote(r: &MergeResult, remote: &Tree) -> BTreeSet<NodeId> {
    let mut out = BTreeSet::new();
    for d in &r.deletes {
        out.insert(d.node.clone());
        out.extend(remote.descendants(&d.node));
    }
    out
}

/// Postcondition: everything that differs between M and the remote subtree is listed
/// in pushes, creates, moves, ticks or deletes (order aside).
fn check_explained(r: &MergeResult, remote: &Tree) -> Result<(), TestCaseError> {
    let rsub = remote.subtree(&r.root).expect("root");
    let deleted = deleted_in_remote(r, remote);
    let created: BTreeSet<&NodeId> = r.creates.iter().map(|c| &c.temp).collect();
    let moved: BTreeMap<&NodeId, &NodeId> = r.moves.iter().map(|m| (&m.node, &m.to)).collect();
    let ticks: BTreeSet<&NodeId> = r.ticks.iter().collect();
    for d in outline_diff(&r.merged, &rsub, &r.root, DiffScope::All) {
        match &d {
            OutlineDiff::Missing(n) => prop_assert!(created.contains(n), "{d:?}"),
            OutlineDiff::Extra(n) => prop_assert!(deleted.contains(n), "{d:?}"),
            OutlineDiff::Parent { id, expected, .. } => {
                prop_assert!(moved.get(id).copied() == expected.as_ref(), "{d:?}")
            }
            OutlineDiff::Name { id, expected, .. } => prop_assert!(
                r.pushes
                    .get(id)
                    .and_then(|p| p.name.as_ref())
                    .map(|c| &c.new)
                    == Some(expected),
                "{d:?}"
            ),
            OutlineDiff::Note { id, expected, .. } => prop_assert!(
                r.pushes
                    .get(id)
                    .and_then(|p| p.note.as_ref())
                    .map(|c| &c.new)
                    == Some(expected),
                "{d:?}"
            ),
            OutlineDiff::Layout { id, expected, .. } => prop_assert!(
                r.pushes
                    .get(id)
                    .and_then(|p| p.layout.as_ref())
                    .map(|c| &c.new)
                    == Some(expected),
                "{d:?}"
            ),
            OutlineDiff::Completed { id, .. } => prop_assert!(ticks.contains(id), "{d:?}"),
            OutlineDiff::Order { .. } => {}
        }
    }
    Ok(())
}

/// Postcondition: `orders[p]` lists exactly the visible children of every visible
/// parent of M, in M's sibling order.
fn check_orders(r: &MergeResult) -> Result<(), TestCaseError> {
    let vis = visible_nodes(&r.merged, &r.root, &DocConfig::default(), &BTreeSet::new());
    for p in &vis {
        let kids: Vec<NodeId> = r
            .merged
            .children(p)
            .iter()
            .filter(|c| vis.contains(*c))
            .cloned()
            .collect();
        match r.orders.get(p) {
            None => prop_assert!(kids.is_empty(), "no order for {}", lbl(p)),
            Some(o) => prop_assert_eq!(o, &kids, "order of {}", lbl(p)),
        }
    }
    for p in r.orders.keys() {
        prop_assert!(vis.contains(p), "order for hidden {}", lbl(p));
    }
    Ok(())
}

fn lcs_len(a: &[u32], b: &[u32]) -> Vec<Vec<usize>> {
    let mut dp = vec![vec![0usize; b.len() + 1]; a.len() + 1];
    for i in (0..a.len()).rev() {
        for j in (0..b.len()).rev() {
            dp[i][j] = if a[i] == b[j] {
                dp[i + 1][j + 1] + 1
            } else {
                dp[i + 1][j].max(dp[i][j + 1])
            };
        }
    }
    dp
}

// ---------------------------------------------------------------------------------------
// Properties
// ---------------------------------------------------------------------------------------

/// 192 cases by default; `PROPTEST_CASES` overrides (for deeper runs).
fn config() -> ProptestConfig {
    let cases = std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(192);
    ProptestConfig::with_cases(cases)
}

proptest! {
    #![proptest_config(config())]

    /// merge(B, B, L) pushes exactly L's changes: M's visible outline is L (ticked items
    /// leave it), nothing conflicts, and every op is backed by a local change.
    #[test]
    fn unchanged_remote_takes_exactly_the_local_changes(b in base_ops(), l in local_ops()) {
        let s = scenario(&b, &l, ALL_LOCAL, &[], ALL_REMOTE);
        let r = s.case.run().expect("merge");
        for e in &r.report {
            prop_assert!(matches!(e, ReportEvent::Rematched { .. }), "unexpected event {e:?}");
        }
        prop_assert!(r.conflicts.is_empty());
        let m_spec = render_spec(&r.merged, &s.case.root, &DocConfig::default(), &BTreeSet::new());
        prop_assert_eq!(shape(&m_spec.items), shape(&s.local_spec.items));
        // Every op is backed by a local change of that tagged item.
        let mut local_by_tag: BTreeMap<String, (&ItemSpec, Option<String>)> = BTreeMap::new();
        fn index<'a>(items: &'a [ItemSpec], parent: Option<String>, out: &mut BTreeMap<String, (&'a ItemSpec, Option<String>)>) {
            for it in items {
                if let Some(t) = &it.tag {
                    out.insert(t.clone(), (it, parent.clone()));
                }
                index(&it.children, Some(it.tag.clone().unwrap_or_else(|| "<new>".into())), out);
            }
        }
        index(&s.local_spec.items, None, &mut local_by_tag);
        let base_spec = render_spec(&s.case.base, &s.case.root, &DocConfig::default(), &BTreeSet::new());
        let mut base_by_tag = BTreeMap::new();
        index(&base_spec.items, None, &mut base_by_tag);
        for (n, p) in &r.pushes {
            let t = lbl(n);
            if let (Some((li, _)), Some((bi, _))) = (local_by_tag.get(&t), base_by_tag.get(&t)) {
                prop_assert!(li.name != bi.name || li.note != bi.note, "push without a change: {}", t);
                prop_assert!(p.name.is_none() || li.name != bi.name);
                prop_assert!(p.note.is_none() || li.note != bi.note);
            }
        }
        for n in &r.ticks {
            // A line typed `[x]` in place of a deleted one may be matched to its node
            // (§4.4 deleted-tag fallback): a tick of that node.
            let rematched = r.report.iter().any(|e| matches!(e, ReportEvent::Rematched { node, .. } if node == n));
            match local_by_tag.get(&lbl(n)) {
                Some((li, _)) => prop_assert_eq!(li.check.as_deref(), Some("x")),
                None => prop_assert!(rematched, "ticked item is tagged or rematched: {}", lbl(n)),
            }
        }
        for mv in &r.moves {
            let t = lbl(&mv.node);
            let (_, lp) = local_by_tag.get(&t).expect("moved item is tagged");
            let (_, bp) = base_by_tag.get(&t).expect("moved item was in the base");
            prop_assert_ne!(lp, bp);
        }
        for d in &r.deletes {
            prop_assert!(!local_by_tag.contains_key(&lbl(&d.node)), "delete of a kept item {}", lbl(&d.node));
        }
        check_explained(&r, &s.case.remote)?;
        check_orders(&r)?;
    }

    /// merge(B, R, L unchanged) takes R: no pushes, and M equals the remote subtree.
    #[test]
    fn unchanged_local_takes_the_remote(b in base_ops(), rops in remote_ops()) {
        let s = scenario(&b, &[], ALL_LOCAL, &rops, ALL_REMOTE);
        let r = s.case.run().expect("merge");
        prop_assert!(!r.has_local_changes(), "{:?}", r);
        prop_assert!(r.conflicts.is_empty());
        prop_assert!(r.is_noop(&s.case.remote));
        let rsub = s.case.remote.subtree(&s.case.root).expect("root");
        let diff = outline_diff(&r.merged, &rsub, &s.case.root, DiffScope::All);
        prop_assert!(diff.is_empty(), "{:?}", diff);
        check_orders(&r)?;
    }

    /// Same inputs, same output (temp ids included).
    #[test]
    fn merge_is_deterministic(b in base_ops(), l in local_ops(), rops in remote_ops()) {
        let s = scenario(&b, &l, ALL_LOCAL, &rops, ALL_REMOTE);
        let first = s.case.run().expect("merge");
        let second = s.case.clone().run().expect("merge");
        prop_assert_eq!(first, second);
    }

    /// No op targets a node the Markdown user could not see (only ticks and deletes of
    /// visible ancestors reach hidden nodes); hidden nodes keep their remote parent and
    /// relative order; and M is explained by the ops (module postconditions).
    #[test]
    fn hidden_nodes_are_never_targeted(b in base_ops(), l in local_ops(), rops in remote_ops()) {
        let s = scenario(&b, &l, ALL_LOCAL, &rops, ALL_REMOTE);
        let r = s.case.run().expect("merge");
        let vis = base_visible(&s.case);
        let root = &s.case.root;
        for n in r.pushes.keys() {
            prop_assert!(n == root || vis.contains(n), "push to hidden {}", lbl(n));
        }
        for m in &r.moves {
            prop_assert!(vis.contains(&m.node), "move of hidden {}", lbl(&m.node));
        }
        for n in &r.ticks {
            prop_assert!(vis.contains(n), "tick of hidden {}", lbl(n));
        }
        for d in &r.deletes {
            prop_assert!(vis.contains(&d.node), "delete of hidden {}", lbl(&d.node));
        }
        let remote = &s.case.remote;
        let hidden: Vec<NodeId> = s.case.base.descendants(root).into_iter()
            .filter(|n| !vis.contains(n) && r.merged.contains(n) && remote.contains(n))
            .collect();
        for h in &hidden {
            prop_assert_eq!(r.merged.parent(h), remote.parent(h), "hidden {} re-parented", lbl(h));
        }
        let hidden_set: BTreeSet<&NodeId> = hidden.iter().collect();
        for p in r.merged.ids() {
            let in_m: Vec<&NodeId> = r.merged.children(p).iter().filter(|c| hidden_set.contains(c)).collect();
            let in_r: Vec<&NodeId> = remote.children(p).iter().filter(|c| in_m.contains(c)).collect();
            prop_assert_eq!(in_m, in_r, "hidden order under {}", lbl(p));
        }
        check_explained(&r, remote)?;
        check_orders(&r)?;
    }

    /// The resurrection path never loses locally typed text: with remote deletes, every
    /// name typed locally ends up in M (possibly in a resurrected node) or in a
    /// conflict marker.
    #[test]
    fn resurrection_never_loses_typed_text(b in base_ops(), l in local_ops(), rops in remote_ops()) {
        // Local: renames, inserts, moves, ticks (no deletes). Remote: deletes only.
        let s = scenario(&b, &l, &[0, 3, 4, 5], &rops, &[3]);
        let r = s.case.run().expect("merge");
        let names: BTreeSet<&str> = r.merged.iter().map(|n| n.name.as_str()).collect();
        for t in &s.typed {
            let in_local = {
                fn has(items: &[ItemSpec], t: &str) -> bool {
                    items.iter().any(|i| i.name == t || has(&i.children, t))
                }
                has(&s.local_spec.items, t)
            };
            if !in_local {
                continue; // overwritten by a later local rename
            }
            prop_assert!(
                names.contains(t.as_str()) || r.conflicts.iter().any(|c| c.local_md == *t),
                "typed text {} lost; report {:?}", t, r.report
            );
        }
    }

    /// §6.3 sibling-order merge: exactly the kept nodes; an unchanged local order yields
    /// the remote order; an unchanged remote order yields the local order; nodes
    /// unmoved on both sides keep their relative order; locally placed nodes come after
    /// their nearest local predecessor in the result.
    #[test]
    fn sibling_order_merge(
        n in 0u32..8,
        lperm in prop::collection::vec(any::<u16>(), 0..8),
        rperm in prop::collection::vec(any::<u16>(), 0..8),
        ldrop in prop::collection::vec(any::<bool>(), 8),
        rdrop in prop::collection::vec(any::<bool>(), 8),
        lnew in 0u32..3,
        rnew in 0u32..3,
        same_local in any::<bool>(),
        same_remote in any::<bool>(),
    ) {
        let base: Vec<u32> = (0..n).collect();
        let shuffle = |perm: &[u16], drop: &[bool], new_from: u32, new_count: u32| {
            let mut v: Vec<u32> = base.iter().copied().filter(|x| !drop[*x as usize]).collect();
            for (i, p) in perm.iter().enumerate() {
                if v.len() > 1 {
                    let a = i % v.len();
                    let b = *p as usize % v.len();
                    v.swap(a, b);
                }
            }
            for k in 0..new_count {
                let at = (k as usize * 3) % (v.len() + 1);
                v.insert(at, new_from + k);
            }
            v
        };
        let local: Vec<u32> = if same_local { base.clone() } else { shuffle(&lperm, &ldrop, 100, lnew) };
        let remote: Vec<u32> = if same_remote { base.clone() } else { shuffle(&rperm, &rdrop, 200, rnew) };
        // Kept: what is in the remote (minus local deletes of base nodes) plus local
        // creates.
        let keep_v: Vec<u32> = remote.iter().copied()
            .filter(|x| *x >= 100 || local.contains(x))
            .chain(local.iter().copied().filter(|x| (100..200).contains(x)))
            .collect();
        let to_id = |x: &u32| NodeId::from_api(format!("{x:03}"));
        let ids = |v: &[u32]| v.iter().map(to_id).collect::<Vec<_>>();
        let keep: BTreeSet<NodeId> = keep_v.iter().map(to_id).collect();
        let out = merge_sibling_order(&ids(&base), Some(&ids(&local)), &ids(&remote), &keep, &ids(&keep_v), true);
        let pos: BTreeMap<NodeId, usize> = out.iter().cloned().enumerate().map(|(i, x)| (x, i)).collect();
        prop_assert_eq!(out.len(), keep.len());
        prop_assert_eq!(pos.keys().cloned().collect::<BTreeSet<_>>(), keep.clone());
        let restrict = |order: &[u32]| -> Vec<NodeId> {
            let ids: Vec<NodeId> = order.iter().map(to_id).collect();
            out.iter().filter(|x| ids.contains(x)).cloned().collect()
        };
        let filtered = |order: &[u32]| -> Vec<NodeId> {
            order.iter().map(to_id).filter(|x| keep.contains(x)).collect()
        };
        if local == base {
            prop_assert_eq!(restrict(&remote), filtered(&remote));
        }
        if remote == base {
            prop_assert_eq!(restrict(&local), filtered(&local));
        }
        // Unmoved on both sides: in an LCS of base with each side. Check one LCS each by
        // the DP: an element is "certainly unmoved" if every LCS contains it, which we
        // approximate by: removing it shortens the LCS.
        let essential = |side: &[u32], x: u32| {
            let full = lcs_len(&base, side)[0][0];
            let b2: Vec<u32> = base.iter().copied().filter(|y| *y != x).collect();
            let s2: Vec<u32> = side.iter().copied().filter(|y| *y != x).collect();
            lcs_len(&b2, &s2)[0][0] < full
        };
        let stable: Vec<u32> = remote.iter().copied()
            .filter(|x| *x < 100 && keep.contains(&to_id(x)) && essential(&local, *x) && essential(&remote, *x))
            .collect();
        prop_assert_eq!(restrict(&stable), filtered(&stable));
        // Local creates come right after something that preceded them locally.
        for (i, x) in local.iter().enumerate() {
            if !(100..200).contains(x) {
                continue;
            }
            let pred = local[..i].iter().rev().find(|y| keep.contains(&to_id(y)));
            if let Some(y) = pred {
                prop_assert!(pos[&to_id(y)] < pos[&to_id(x)], "{} not after {}", x, y);
            }
        }
    }
}

/// §9.1 guard 4 "tag lost in a merge" at scale: a 1500-node file whose tags were all
/// stripped rematches every line (no creates, no deletes) in reasonable time.
#[test]
fn large_file_with_all_tags_lost_rematches_everything() {
    let r = root();
    let base = build_tree_wide(1500);
    let spec = render_spec(&base, &r, &DocConfig::default(), &BTreeSet::new());
    fn untag(items: &mut [ItemSpec]) {
        for it in items {
            it.tag = None;
            untag(&mut it.children);
        }
    }
    let mut local = spec.clone();
    untag(&mut local.items);
    let case = Case::new(
        r.clone(),
        base.clone(),
        base,
        build_doc(&r, &spec),
        build_doc(&r, &local),
    );
    let start = std::time::Instant::now();
    let res = case.run().expect("merge");
    let elapsed = start.elapsed();
    assert!(res.creates.is_empty(), "{} creates", res.creates.len());
    assert!(res.deletes.is_empty(), "{} deletes", res.deletes.len());
    assert!(res.pushes.is_empty());
    assert!(elapsed.as_secs() < 60, "took {elapsed:?}");
}

/// A tree of `n` nodes that is both wide and deep (parents drawn from recent nodes).
fn build_tree_wide(n: u32) -> Tree {
    let r = root();
    let mut t = Tree::from_nodes([Node::new(r.clone(), None, "Root")]).expect("tree");
    let mut all = vec![r];
    for k in 0..n {
        let pick = (k.wrapping_mul(2654435761) >> 24) as usize;
        let window = all.len().min(40);
        let p = all[all.len() - 1 - (pick % window)].clone();
        let nid = id(&format!("w{k}"));
        let mut node = Node::new(nid.clone(), Some(p.clone()), word(u64::from(k)));
        if k % 3 == 0 {
            node.note = Some(word(7000 + u64::from(k)));
        }
        t.place(node, Some(&p), Position::Bottom).expect("place");
        all.push(nid);
    }
    t
}
