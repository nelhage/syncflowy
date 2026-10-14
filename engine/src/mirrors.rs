//! Mirrors as their origins (design §15.1), on the beta API's mirror-aware records
//! (DM§10.5).
//!
//! Responsibility: turn an export in which mirror roots ("shells") carry their origin's
//! content into the **view tree** the Markdown shows, and translate what the merge
//! decides on that view back to the real nodes the API takes. Pure: no I/O.
//!
//! Consumes: [`crate::tree`] (`Tree`, `Node`, `Mirror`), [`crate::merge::MergeResult`].
//! Consumed by: `sync` (builds the view per root, translates before planning), `plan`
//! and `apply` (the boundary guards through [`MirrorIndex::in_scope`]), `mdblock` and
//! `merge` (read the `stub` flag the view sets).
//!
//! # Two trees (§15.1 "The view tree")
//!
//! An export from the beta API is the **real tree** `R`: every node once, with its real
//! parent. A shell reports its origin's `name`, `note`, `layoutMode` and completion, and
//! `data.mirror.origin_id`; an origin's children appear once, under the origin; a
//! shell's own persisted children (a hidden trap: invisible in lists, deleted with the
//! shell, DM§10.5) appear under the shell. [`real_tree`] is `R` without those persisted
//! children, which no file ever shows.
//!
//! The **view tree** `V` of one sync root ([`view_tree`]) is what that root's file
//! renders. Every node of `V` is a **placement** showing a **content node**: a plain
//! node shows itself, a shell shows its origin ([`crate::tree::Node::content_id`]).
//! The synced scope is the closure of the root under "follow mirrors": the children of
//! a shell placement are its origin's children. **Each content node is rendered once**:
//! one placement, the **full** one, carries the content's children (they are
//! re-parented under it in `V`); every other placement of the same content, in the
//! scope or outside it, is a **stub** ([`crate::tree::Node::stub`]), a leaf that
//! `mdblock` renders as a link to the full placement and that the merge treats as
//! read-only. A content node whose own node lies outside the scope but which is
//! mirrored into it is therefore shown in full at its shell, and its own node becomes a
//! stub. Cycles (a node mirrored into its own subtree, DM§10.2) end in a stub.
//!
//! **Which placement is full** (§15.1): the first visible one in the pre-order of the
//! view (the root itself always counts as visible), unless the previous render's choice
//! (`previous_full`, from the checkpoint's base) still exists in the scope and is
//! visible: then that one, so that a reorder never swaps a full rendering and a stub
//! and rewrites lines nobody edited. A content whose placements are all hidden keeps
//! the previous choice if it is still in the scope, else gets its first placement. Since the full choice decides which children are reachable, the choice is
//! iterated to a fixed point (a few rounds at most).
//!
//! Orphan shells (origin deleted, DM§10.4) have no content to show: they stay leaves
//! and `mdblock` renders them as read-only placeholders. Outside the root's scope `V`
//! equals [`real_tree`], except that every shell is a stub.
//!
//! Without mirror metadata (a production export, or an account without a mirror) the
//! view tree is the export itself and §10's placeholder heuristic applies instead.
//!
//! # Translation (§15.1 "Mapping Markdown edits")
//!
//! The merge runs on `V` (base and remote alike), so its result names placements.
//! [`to_real`] rewrites it for the planner, which runs on [`real_tree`]:
//!
//! | on the view | on the real tree |
//! |---|---|
//! | edit, tick or create under placement `p` | the same on `content(p)` (UI:H3, H4) |
//! | move placement `p` under `q` | move the real node `p` (a shell moves itself, its origin stays) under `content(q)` |
//! | sibling order under `p` | the order of `content(p)`'s children |
//! | delete a shell placement | `DELETE /nodes/S/mirror`: the content survives (`plan` picks the endpoint) |
//! | delete a plain placement | a plain delete; refused by the merge when the subtree holds an origin that still has mirrors (DM§10.5) |
//!
//! The merged tree `M` becomes `M_real`: parents are mapped to content ids, a content
//! node rendered at a shell takes that shell's merged fields (that is where the user
//! edited it) and is added to `M_real` when its own node lies outside the merge's
//! subtree, and every shell carries its content's fields, as the API reports it.
//!
//! # Scope through mirrors
//!
//! A content node mirrored into a root lies outside the root's real subtree. The
//! boundary guards of `plan` and `apply` (§8.2) therefore ask [`MirrorIndex::in_scope`]:
//! a node is in scope if walking up real parents, and from any origin to each of its
//! live shells, reaches a sync root.
//!
//! # Not implemented from §15.1 (recorded here deliberately)
//!
//! * A stub is read-only: edits to its text are refused and rewritten (the design drops
//!   them silently), a tick of it is refused instead of completing the content, and
//!   items typed under it are lifted to its parent and reported like items typed under
//!   a §10 placeholder (the design keeps them as a conflict marker). Edit the full
//!   placement instead.
//! * Deleting the origin's own placement while shells exist is refused and reported
//!   (`merge::ReportEvent::MirrorDeleteRefused`), like any delete whose subtree holds an
//!   origin with mirrors; the UI's promotion of a shell (UI:H6) is not emulated.
//! * Creating a mirror from Markdown is not supported.

use std::collections::{BTreeMap, BTreeSet};

use crate::merge::{DeleteIntent, MergeResult, MoveIntent, NewNode};
use crate::tree::{Node, NodeId, Tree};

/// How many rounds the sticky full-placement choice is iterated before it is taken as
/// it stands (each round can only move a choice to a sticky or visible placement, so
/// the fixed point is reached in practice within two or three).
const MAX_ROUNDS: usize = 8;

/// The live shells of every origin in a tree, for [`MirrorIndex::in_scope`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MirrorIndex {
    shells: BTreeMap<NodeId, Vec<NodeId>>,
}

impl MirrorIndex {
    /// Index `tree`'s shells by origin (from the shells' `origin`, DM§10.5).
    pub fn new(tree: &Tree) -> MirrorIndex {
        MirrorIndex {
            shells: tree.shells_by_origin(),
        }
    }

    /// True if there are no mirrors at all.
    pub fn is_empty(&self) -> bool {
        self.shells.is_empty()
    }

    /// The live shells of `origin` that are still in `tree` (shells deleted since the
    /// index was built are skipped).
    pub fn shells_of<'t>(&self, tree: &'t Tree, origin: &NodeId) -> Vec<&'t Node> {
        self.shells
            .get(origin)
            .map(|v| v.iter().filter_map(|s| tree.get(s)).collect())
            .unwrap_or_default()
    }

    /// True if `id` is one of `roots` or lies below one in `tree`, where "below" follows
    /// real parents and, from an origin, each of its live shells (module docs). Cycles
    /// are safe. A node absent from `tree` is not in scope.
    pub fn in_scope(&self, tree: &Tree, id: &NodeId, roots: &BTreeSet<NodeId>) -> bool {
        if !tree.contains(id) {
            return false;
        }
        let mut seen = BTreeSet::new();
        let mut stack = vec![id.clone()];
        while let Some(cur) = stack.pop() {
            if !seen.insert(cur.clone()) {
                continue;
            }
            if roots.contains(&cur) {
                return true;
            }
            if let Some(p) = tree.parent(&cur)
                && tree.contains(p)
            {
                stack.push(p.clone());
            }
            if let Some(shells) = self.shells.get(&cur) {
                stack.extend(shells.iter().filter(|s| tree.contains(s)).cloned());
            }
        }
        false
    }
}

/// `tree` without the persisted children of shells (DM§10.5: hidden in lists, deleted
/// with the shell; never shown). The real tree the planner and the applier work on.
pub fn real_tree(tree: &Tree) -> Tree {
    if !tree.has_mirror_metadata() {
        return tree.clone();
    }
    let mut out = tree.clone();
    drop_persisted_children(&mut out);
    out
}

fn drop_persisted_children(tree: &mut Tree) {
    let shells: Vec<NodeId> = tree
        .iter()
        .filter(|n| {
            n.mirror
                .as_ref()
                .is_some_and(|m| matches!(m, crate::tree::Mirror::Shell { .. }))
        })
        .map(|n| n.id.clone())
        .collect();
    for s in shells {
        for c in tree.children(&s).to_vec() {
            if tree.contains(&c) {
                let _ = tree.remove_subtree(&c);
            }
        }
    }
}

/// The content node -> full placement map of a view tree (or of a checkpoint's base,
/// which is a view subtree): every non-stub node names itself as its content's
/// placement. The input to [`view_tree`]'s stickiness for the next render.
pub fn full_placements(tree: &Tree) -> BTreeMap<NodeId, NodeId> {
    tree.iter()
        .filter(|n| !n.stub && !n.is_orphan_shell())
        .map(|n| (n.content_id().clone(), n.id.clone()))
        .collect()
}

/// The placements of `content` in `tree` (its own node if present, then its live
/// shells), in id order.
pub fn placements_of(tree: &Tree, content: &NodeId) -> Vec<NodeId> {
    let mut out = Vec::new();
    if tree.contains(content) {
        out.push(content.clone());
    }
    out.extend(tree.shells_of(content));
    out
}

/// The full (non-stub) placement of `content` in a view tree, if any.
pub fn full_placement_of(tree: &Tree, content: &NodeId) -> Option<NodeId> {
    placements_of(tree, content)
        .into_iter()
        .find(|p| tree.get(p).is_some_and(|n| !n.stub))
}

/// One placement met by the closure walk.
#[derive(Debug, Clone)]
struct Seen {
    placement: NodeId,
    visible: bool,
}

/// The pre-order walk of the view from `root` for a given full-placement choice:
/// content -> its placements in visit order. A content's children are expanded at
/// its chosen placement, or at its first placement when none is chosen yet.
fn walk(tree: &Tree, root: &NodeId, full: &BTreeMap<NodeId, NodeId>) -> Vec<(NodeId, Vec<Seen>)> {
    let mut order: Vec<NodeId> = Vec::new();
    let mut seen: BTreeMap<NodeId, Vec<Seen>> = BTreeMap::new();
    // (placement, hidden by an ancestor)
    let mut stack: Vec<(NodeId, bool)> = vec![(root.clone(), false)];
    while let Some((p, hidden)) = stack.pop() {
        let Some(node) = tree.get(&p) else { continue };
        let content = if node.is_orphan_shell() {
            p.clone()
        } else {
            node.content_id().clone()
        };
        let visible = &p == root || (!hidden && !node.completed);
        let entry = seen.entry(content.clone()).or_insert_with(|| {
            order.push(content.clone());
            Vec::new()
        });
        entry.push(Seen {
            placement: p.clone(),
            visible,
        });
        let expand = match full.get(&content) {
            Some(f) => *f == p,
            None => entry.len() == 1,
        };
        if !expand || node.is_orphan_shell() || !tree.contains(&content) {
            continue;
        }
        let child_hidden = hidden || (&p != root && node.completed);
        // Children in reverse so that the stack pops them in sibling order.
        for c in tree.children(&content).iter().rev() {
            stack.push((c.clone(), child_hidden));
        }
    }
    order
        .into_iter()
        .map(|c| {
            let v = seen.remove(&c).expect("recorded");
            (c, v)
        })
        .collect()
}

/// The view tree of `root` over `export` (module docs). `previous_full` is the
/// stickiness input ([`full_placements`] of the previous base); pass an empty map for
/// a first render. Without mirror metadata the export is returned as it is.
pub fn view_tree(export: &Tree, root: &NodeId, previous_full: &BTreeMap<NodeId, NodeId>) -> Tree {
    if !export.has_mirror_metadata() || !export.contains(root) {
        return export.clone();
    }
    let mut v = export.clone();
    drop_persisted_children(&mut v);

    let mut full: BTreeMap<NodeId, NodeId> = BTreeMap::new();
    let mut visited = walk(&v, root, &full);
    for _ in 0..MAX_ROUNDS {
        let mut changed = false;
        for (content, places) in &visited {
            let current = full
                .get(content)
                .cloned()
                .unwrap_or_else(|| places[0].placement.clone());
            let any_visible = places.iter().any(|s| s.visible);
            let sticky = previous_full
                .get(content)
                .filter(|q| {
                    places
                        .iter()
                        .any(|s| &s.placement == *q && (s.visible || !any_visible))
                })
                .cloned();
            let want = sticky
                .or_else(|| {
                    places
                        .iter()
                        .find(|s| s.visible)
                        .map(|s| s.placement.clone())
                })
                .unwrap_or_else(|| places[0].placement.clone());
            if want != current || !full.contains_key(content) {
                changed |= want != current;
                full.insert(content.clone(), want);
            }
        }
        if !changed {
            break;
        }
        visited = walk(&v, root, &full);
    }
    // Contents the final walk did not reach are outside the scope: drop their entries.
    let reached: BTreeSet<&NodeId> = visited.iter().map(|(c, _)| c).collect();
    full.retain(|c, _| reached.contains(c));

    // Hang each content's children under its full placement; mark the stubs.
    for (content, f) in &full {
        if f != content && v.contains(content) {
            let _ = v.adopt_children(content, f);
        }
    }
    let ids: Vec<NodeId> = v.ids().cloned().collect();
    for id in ids {
        let node = v.get(&id).expect("in tree");
        if node.is_orphan_shell() {
            continue;
        }
        let content = node.content_id().clone();
        let stub = match full.get(&content) {
            Some(f) => *f != id,
            // Outside the scope: the content itself is full, every shell a stub.
            None => node.is_shell(),
        };
        if stub != node.stub {
            let _ = v.set_stub(&id, stub);
        }
    }
    v
}

/// The content id of placement `p` (a temp id, or an id absent from both trees, is
/// its own content).
fn content_of(view: &Tree, merged: &Tree, p: &NodeId) -> NodeId {
    view.get(p)
        .or_else(|| merged.get(p))
        .map(|n| n.content_id().clone())
        .unwrap_or_else(|| p.clone())
}

fn copy_content_fields(from: &Node, to: &mut Node) {
    to.name = from.name.clone();
    to.note = from.note.clone();
    to.layout = from.layout.clone();
    to.completed = from.completed;
    to.completed_at = from.completed_at;
    to.modified_at = from.modified_at;
}

/// Translate a merge result made on the view tree `view` into one on the real tree
/// `real` (module docs, "Translation"). With no mirror metadata the result is returned
/// unchanged.
pub fn to_real(merge: &MergeResult, view: &Tree, real: &Tree) -> MergeResult {
    if !view.has_mirror_metadata() {
        return merge.clone();
    }
    let m = &merge.merged;
    let content = |p: &NodeId| content_of(view, m, p);

    // M_real, pass 1: every placement with its parent mapped to a content id.
    let mut nodes: BTreeMap<NodeId, Node> = BTreeMap::new();
    for n in m.iter() {
        let mut r = n.clone();
        r.stub = false;
        r.parent_id = n.parent_id.as_ref().map(&content);
        nodes.insert(r.id.clone(), r);
    }
    // Pass 2: a content rendered in full at a shell takes the shell's merged fields;
    // its own node is added when the merge's subtree did not hold it.
    for n in m.iter() {
        if !(n.is_shell() && !n.stub) {
            continue;
        }
        let Some(o) = n.origin().cloned() else {
            continue;
        };
        match nodes.get_mut(&o) {
            Some(ro) => copy_content_fields(n, ro),
            None => {
                let Some(mut ro) = real.get(&o).or_else(|| view.get(&o)).cloned() else {
                    continue;
                };
                ro.stub = false;
                copy_content_fields(n, &mut ro);
                nodes.insert(o, ro);
            }
        }
    }
    // Pass 3: every shell shows its content's fields, as the API reports it.
    for n in m.iter() {
        if !n.is_shell() {
            continue;
        }
        let Some(o) = n.origin().cloned() else {
            continue;
        };
        if let Some(c) = nodes.get(&o).cloned()
            && let Some(s) = nodes.get_mut(&n.id)
        {
            copy_content_fields(&c, s);
        }
    }
    let merged = match Tree::from_nodes(nodes.into_values()) {
        Ok(t) => t,
        Err(e) => {
            // Ids are unique by construction; keep the view's M rather than fail.
            log::warn!("mirrors: cannot build the real merged tree: {e}");
            m.clone()
        }
    };

    let mut pushes = BTreeMap::new();
    for (p, push) in &merge.pushes {
        pushes.entry(content(p)).or_insert_with(|| push.clone());
    }
    let creates: Vec<NewNode> = merge
        .creates
        .iter()
        .map(|c| NewNode {
            parent: content(&c.parent),
            ..c.clone()
        })
        .collect();
    let moves: Vec<MoveIntent> = merge
        .moves
        .iter()
        .map(|mv| MoveIntent {
            node: mv.node.clone(),
            from: content(&mv.from),
            to: content(&mv.to),
        })
        .collect();
    let ticks: Vec<NodeId> = merge.ticks.iter().map(&content).collect();
    let deletes: Vec<DeleteIntent> = merge.deletes.clone();
    let mut orders = BTreeMap::new();
    for (p, order) in &merge.orders {
        orders.entry(content(p)).or_insert_with(|| order.clone());
    }
    let outside: BTreeSet<NodeId> = merge.outside.iter().map(&content).collect();

    MergeResult {
        root: merge.root.clone(),
        merged,
        pushes,
        creates,
        moves,
        ticks,
        deletes,
        orders,
        conflicts: merge.conflicts.clone(),
        report: merge.report.clone(),
        stats: merge.stats,
        cancelled_pending: merge.cancelled_pending.clone(),
        outside,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree::Mirror;

    fn uid(n: u32) -> NodeId {
        NodeId::parse(&format!("00000000-0000-4000-8000-{n:012x}")).unwrap()
    }

    fn node(n: u32, parent: Option<u32>, prio: i64, name: &str) -> Node {
        let mut x = Node::new(uid(n), parent.map(uid), name);
        x.priority = prio;
        x
    }

    fn shell(n: u32, parent: Option<u32>, prio: i64, origin: &Node) -> Node {
        let mut s = node(n, parent, prio, &origin.name);
        s.mirror = Some(Mirror::Shell {
            origin: Some(origin.id.clone()),
        });
        s
    }

    /// Root 1 / {A 2 / {a1 3}, S 4 = shell of O 5}; O 5 (top level) / {o1 6, o2 7};
    /// S's persisted child 8.
    fn account() -> Tree {
        let mut o = node(5, None, 200, "O");
        o.mirror = Some(Mirror::Origin {
            mirror_ids: vec![uid(4)],
        });
        Tree::from_nodes([
            node(1, None, 100, "Root"),
            node(2, Some(1), 100, "A"),
            node(3, Some(2), 100, "a1"),
            shell(4, Some(1), 200, &o),
            o,
            node(6, Some(5), 100, "o1"),
            node(7, Some(5), 200, "o2"),
            node(8, Some(4), 100, "persisted"),
        ])
        .unwrap()
    }

    #[test]
    fn real_tree_drops_persisted_children_of_shells() {
        let r = real_tree(&account());
        assert!(!r.contains(&uid(8)));
        assert_eq!(r.children(&uid(5)), &[uid(6), uid(7)]);
        assert_eq!(r.len(), 7);
    }

    #[test]
    fn view_hangs_origin_children_under_the_shell_in_scope() {
        let v = view_tree(&account(), &uid(1), &BTreeMap::new());
        assert_eq!(v.children(&uid(4)), &[uid(6), uid(7)]);
        assert!(v.children(&uid(5)).is_empty());
        assert!(!v.get(&uid(4)).unwrap().stub);
        assert!(
            v.get(&uid(5)).unwrap().stub,
            "the origin outside the scope is a stub"
        );
        assert!(!v.contains(&uid(8)));
        assert_eq!(
            full_placements(&v.subtree(&uid(1)).unwrap()),
            BTreeMap::from([
                (uid(1), uid(1)),
                (uid(2), uid(2)),
                (uid(3), uid(3)),
                (uid(5), uid(4)),
                (uid(6), uid(6)),
                (uid(7), uid(7)),
            ])
        );
        assert_eq!(full_placement_of(&v, &uid(5)), Some(uid(4)));
        assert_eq!(placements_of(&v, &uid(5)), vec![uid(5), uid(4)]);
    }

    #[test]
    fn first_placement_in_preorder_is_full_and_stickiness_keeps_the_previous() {
        // Root 1 / {S 4 = shell of O 5, O 5 / {o1 6}}: the shell comes first.
        let mut o = node(5, Some(1), 300, "O");
        o.mirror = Some(Mirror::Origin {
            mirror_ids: vec![uid(4)],
        });
        let t = Tree::from_nodes([
            node(1, None, 100, "Root"),
            shell(4, Some(1), 200, &o),
            o,
            node(6, Some(5), 100, "o1"),
        ])
        .unwrap();
        let v = view_tree(&t, &uid(1), &BTreeMap::new());
        assert!(!v.get(&uid(4)).unwrap().stub);
        assert!(v.get(&uid(5)).unwrap().stub);
        assert_eq!(v.children(&uid(4)), &[uid(6)]);
        // Sticky: the previous render had the origin in full.
        let prev = BTreeMap::from([(uid(5), uid(5))]);
        let v = view_tree(&t, &uid(1), &prev);
        assert!(v.get(&uid(4)).unwrap().stub);
        assert!(!v.get(&uid(5)).unwrap().stub);
        assert_eq!(v.children(&uid(5)), &[uid(6)]);
        // A hidden sticky choice loses to a visible placement.
        let mut t2 = t.clone();
        t2.set_completed(&uid(5), true, Some(1)).unwrap();
        // Completion is the content's: the API reports it on both placements.
        t2.set_completed(&uid(4), true, Some(1)).unwrap();
        let v = view_tree(&t2, &uid(1), &prev);
        assert!(
            !v.get(&uid(5)).unwrap().stub,
            "all hidden: the sticky one stays"
        );
        let mut t3 = t.clone();
        // O under a completed parent 9, S visible under the root.
        t3.insert(node(9, Some(1), 50, "done")).unwrap();
        t3.set_completed(&uid(9), true, Some(1)).unwrap();
        t3.move_node(&uid(5), Some(&uid(9)), crate::tree::Position::Bottom)
            .unwrap();
        let v = view_tree(&t3, &uid(1), &prev);
        assert!(!v.get(&uid(4)).unwrap().stub, "the visible placement wins");
        assert!(v.get(&uid(5)).unwrap().stub);
    }

    #[test]
    fn cycles_end_in_a_stub() {
        // O 5 / {o1 6 / {S 4 = shell of O}}: a node mirrored into its own subtree.
        let mut o = node(5, None, 100, "O");
        o.mirror = Some(Mirror::Origin {
            mirror_ids: vec![uid(4)],
        });
        let t = Tree::from_nodes([
            o.clone(),
            node(6, Some(5), 100, "o1"),
            shell(4, Some(6), 100, &o),
        ])
        .unwrap();
        let v = view_tree(&t, &uid(5), &BTreeMap::new());
        assert!(!v.get(&uid(5)).unwrap().stub);
        assert!(v.get(&uid(4)).unwrap().stub);
        assert_eq!(v.children(&uid(5)), &[uid(6)]);
        assert_eq!(v.children(&uid(6)), &[uid(4)]);
        assert!(v.children(&uid(4)).is_empty());
    }

    #[test]
    fn scope_follows_mirrors() {
        let r = real_tree(&account());
        let idx = MirrorIndex::new(&r);
        let roots = BTreeSet::from([uid(1)]);
        assert!(idx.in_scope(&r, &uid(3), &roots));
        assert!(idx.in_scope(&r, &uid(4), &roots));
        assert!(
            idx.in_scope(&r, &uid(5), &roots),
            "the origin, through its shell"
        );
        assert!(idx.in_scope(&r, &uid(6), &roots), "the origin's child");
        assert!(!idx.in_scope(&r, &uid(9), &roots));
        let mut gone = r.clone();
        gone.remove_subtree(&uid(4)).unwrap();
        assert!(!idx.in_scope(&gone, &uid(6), &roots), "the shell is gone");
    }

    #[test]
    fn production_export_is_its_own_view() {
        let t = Tree::from_nodes([node(1, None, 100, "Root"), node(2, Some(1), 100, "x")]).unwrap();
        assert_eq!(view_tree(&t, &uid(1), &BTreeMap::new()), t);
        assert_eq!(real_tree(&t), t);
        assert!(MirrorIndex::new(&t).is_empty());
    }
}
