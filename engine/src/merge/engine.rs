//! The merge algorithm itself. The rules are documented (with design citations) in the
//! module docs of [`super`]; comments here cite the design where a rule is applied.
//!
//! Phases, each over the local items in file pre-order:
//! 1. identity (§4.4): tags, duplicates, adoptions, deleted-tag fallback;
//! 2. pending-delete cancellations and remote visibility (§4.5, §9.1 guards 2–3);
//! 3. local field changes per base item (§5.1, §6.1, §9.1 guard 3, §10);
//! 4. fates: existing, create (new or resurrected, §6.2, unless adopted, §7.3), gone,
//!    dropped;
//! 5. building `M`: cross-file subtrees, field merges, creates, moves, ticks, deletes;
//! 6. visibility of `M`, sibling orders (§6.3), reports, conflict markers, stats.

use std::collections::{BTreeMap, BTreeSet};

use similar::TextDiff;

use super::codec::TextCodec;
use super::diff3::diff3_lines;
use super::order::{lcs_members, merge_sibling_order};
use super::{
    Change, CrossFile, DeleteIntent, FieldPush, MergeError, MergeInput, MergeResult, MergeStats,
    MoveIntent, NewNode, REMATCH_MAX_DISTANCE, REMATCH_MIN_RATIO, ReportEvent, is_placeholder_node,
    visible_nodes,
};
use crate::mdblock::{Checkbox, ConflictMarker, ItemKind, LocalDoc, LocalItem};
use crate::mdinline::{ConflictField, FieldKind, IdTag, InlineConfig, TagFlag, Tier};
use crate::tree::{DEFAULT_LAYOUT, Node, NodeId, Position, ShortId, Tree};

/// How a local item was identified (§4.4).
#[derive(Debug, Clone, PartialEq, Eq)]
enum Ident {
    /// No usable tag: a create (unless resurrected later).
    New,
    /// A tagged item that lost a duplicated tag to another item: a create.
    Duplicate,
    /// Tagged, and the node is in `base_doc` (visible in the base).
    Base(NodeId),
    /// Untagged, matched to a base item by the deleted-tag fallback.
    Rematched(NodeId),
    /// Tagged with a node of this root that was hidden in the base.
    Known(NodeId),
    /// A pending-delete node that reappeared (by tag or by the fallback).
    Pending(NodeId),
    /// A node of another managed root: a cross-file move (node, other root).
    Cross(NodeId, NodeId),
    /// A node created by an interrupted apply (node, index into `adoptions`).
    Adopted(NodeId, usize),
}

impl Ident {
    fn node(&self) -> Option<&NodeId> {
        match self {
            Ident::New | Ident::Duplicate => None,
            Ident::Base(n)
            | Ident::Rematched(n)
            | Ident::Known(n)
            | Ident::Pending(n)
            | Ident::Cross(n, _)
            | Ident::Adopted(n, _) => Some(n),
        }
    }

    fn base_node(&self) -> Option<&NodeId> {
        match self {
            Ident::Base(n) | Ident::Rematched(n) => Some(n),
            _ => None,
        }
    }
}

/// What becomes of a local item in `M`.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Fate {
    Undecided,
    /// An existing node, in `M`.
    Existing(NodeId),
    /// A create (new or resurrected) with this temp id.
    Create(NodeId),
    /// Its node is gone remotely (deleted or moved out) and nothing needs it.
    Gone,
    /// Dropped from the file (guard 3 artefact, or an impossible cross-file move).
    Dropped,
}

impl Fate {
    fn id(&self) -> Option<&NodeId> {
        match self {
            Fate::Existing(n) | Fate::Create(n) => Some(n),
            _ => None,
        }
    }
}

/// A non-marker item of `local_doc`.
struct LItem<'a> {
    item: &'a LocalItem,
    /// Parent item (index into `items`), `None` for top-level items.
    parent: Option<usize>,
    /// Index among the non-marker siblings.
    index: usize,
    /// Non-marker children (indices into `items`).
    children: Vec<usize>,
    ident: Ident,
    fate: Fate,
    /// For a resurrection: the remotely deleted node it replaces (kept when the
    /// resurrection is adopted, so the adopted node gets the resurrected values).
    resurrects: Option<NodeId>,
    /// For a cross-file move: the destination parent in `M`.
    cross_dest: Option<NodeId>,
    /// The parent the item ends up under in `M` (creates: after lifting).
    placed_parent: Option<NodeId>,
}

/// An item of `base_doc` that names a node.
struct BItem<'a> {
    item: &'a LocalItem,
    /// The node of its parent item (the root for top-level items).
    parent: NodeId,
    /// Index among the non-marker siblings.
    index: usize,
}

/// Local field changes of one base item (§5.1), after guard 3 and the read-only rules.
#[derive(Debug, Clone)]
struct Detected {
    /// The local name Markdown, if the name changed.
    name: Option<String>,
    name_kind: FieldKind,
    /// The user saw all of the base name, so a write that cannot merge onto it loses
    /// nothing worth a [`ReportEvent::LossyWrite`]: the item changed kind (a code block
    /// unfenced, say) and the base round-trips in its old kind, or a read-only name was
    /// edited with `force` (the `🔒` was in view).
    name_seen: bool,
    /// The local note Markdown (lines joined by `\n`), if the note changed.
    note: Option<String>,
    /// As `name_seen`, for a read-only note edited with `force`.
    note_seen: bool,
    /// The target layout, if the Markdown changed kind (§4.3).
    layout: Option<String>,
}

impl Detected {
    fn any(&self) -> bool {
        self.name.is_some() || self.note.is_some() || self.layout.is_some()
    }
}

/// What a create writes (§7.2): canonical WF-HTML name and note, and the layout; `lost`
/// names the fields whose §5.2 write dropped what Markdown could not show.
struct CreateValues {
    name: String,
    note: Option<String>,
    layout: String,
    lost: Vec<ConflictField>,
}

/// A candidate identity from a tag.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Cand {
    Local(NodeId),
    Cross(NodeId, NodeId),
}

impl Cand {
    fn node(&self) -> &NodeId {
        match self {
            Cand::Local(n) | Cand::Cross(n, _) => n,
        }
    }
}

struct Engine<'a, 'c> {
    inp: MergeInput<'a>,
    codec: &'c dyn TextCodec,
    inline: &'a InlineConfig,
    root: NodeId,
    force: bool,
    safe: bool,
    /// Origin -> live shells in the remote tree (§15.1: a delete of an origin that still
    /// has mirrors is refused).
    shell_index: BTreeMap<NodeId, Vec<NodeId>>,

    base_sub: BTreeSet<NodeId>,
    remote_sub: BTreeSet<NodeId>,
    remote_vis: BTreeSet<NodeId>,
    base_items: BTreeMap<NodeId, BItem<'a>>,
    /// Base items in file order.
    base_order: Vec<NodeId>,
    base_children: BTreeMap<NodeId, Vec<NodeId>>,
    priors: Vec<BTreeMap<ShortId, &'a LocalItem>>,

    items: Vec<LItem<'a>>,
    markers: Vec<(Option<usize>, &'a LocalItem)>,
    /// `adoptions` already matched to an item.
    adopted: Vec<bool>,
    detected: Vec<Option<Detected>>,

    m: Tree,
    pushes: BTreeMap<NodeId, FieldPush>,
    creates: Vec<NewNode>,
    moves: Vec<MoveIntent>,
    ticks: Vec<NodeId>,
    deletes: Vec<DeleteIntent>,
    deleted: BTreeSet<NodeId>,
    conflicts: BTreeMap<(NodeId, ConflictField), ConflictMarker>,
    new_conflicts: usize,
    report: Vec<ReportEvent>,
    refused: BTreeSet<NodeId>,
    cancelled: BTreeSet<NodeId>,
    /// Base nodes moved out of the subtree remotely whose local edits or tick are pushed
    /// where they are now (§6.2); they are not in `M`.
    outside: BTreeSet<NodeId>,
    suppressed: usize,
    stats: MergeStats,
}

pub(super) fn run(inp: MergeInput<'_>, codec: &dyn TextCodec) -> Result<MergeResult, MergeError> {
    check_root(&inp)?;
    let mut e = Engine::new(inp, codec);
    e.identify();
    e.cancel_and_visibility();
    e.detect_all();
    e.decide_fates();
    e.build_merged();
    Ok(e.finish())
}

fn check_root(inp: &MergeInput<'_>) -> Result<(), MergeError> {
    let root = inp.root;
    if !inp.remote.contains(root) {
        return Err(MergeError::RootMissing(root.clone()));
    }
    let short = root.short();
    for doc in [inp.base_doc, inp.local_doc] {
        let r = &doc.frontmatter.root;
        let full_ok = r.full.as_ref().is_none_or(|f| f == root);
        if Some(&r.short) != short.as_ref() || !full_ok {
            return Err(MergeError::RootMismatch {
                expected: root.clone(),
                found: r.raw.clone(),
            });
        }
    }
    if !inp.base.is_empty() && !inp.base.contains(root) {
        return Err(MergeError::RootMismatch {
            expected: root.clone(),
            found: "a base tree without the root".to_owned(),
        });
    }
    Ok(())
}

fn field_kind(item: &LocalItem) -> FieldKind {
    if item.kind == ItemKind::Code {
        FieldKind::Code
    } else {
        FieldKind::Name
    }
}

fn has_ro_flag(item: &LocalItem) -> bool {
    item.tag
        .as_ref()
        .is_some_and(|t| t.flags.contains(&TagFlag::ReadOnly))
}

fn is_untagged(item: &LocalItem) -> bool {
    item.tag.is_none() && item.extra_tags.is_empty()
}

fn similarity(a: &str, b: &str) -> f32 {
    TextDiff::from_chars(a, b).ratio()
}

fn ratio_key(r: f32) -> i64 {
    (f64::from(r) * 1_000_000.0).round() as i64
}

fn non_markers(items: &[LocalItem]) -> Vec<&LocalItem> {
    items.iter().filter(|i| !i.is_marker()).collect()
}

/// §4.3: a parent's children all switched to (or from) ordered markers. Only a parent
/// whose layout is `bullets` (to `numbered`) or `numbered` (back to `bullets`) can
/// change this way: every other layout (`todo`, `h1`, `board`, `p`, …) is "kept as is"
/// (§4.3 table), so ordered children typed under it do not touch it.
fn numbered_target(
    base_layout: &str,
    base_children: &[LocalItem],
    local_children: &[LocalItem],
) -> Option<String> {
    let b = non_markers(base_children);
    let l = non_markers(local_children);
    let b_all = !b.is_empty() && b.iter().all(|x| x.ordered);
    let l_all = !l.is_empty() && l.iter().all(|x| x.ordered);
    let l_any = l.iter().any(|x| x.ordered);
    if l_all && !b_all && base_layout == DEFAULT_LAYOUT {
        Some("numbered".to_owned())
    } else if b_all && !l.is_empty() && !l_any && base_layout == "numbered" {
        Some(DEFAULT_LAYOUT.to_owned())
    } else {
        None
    }
}

/// The layout an item kind written in Markdown stands for, for the kinds that name
/// one explicitly (§4.3): a fence, a quote, a `---` divider.
fn kind_layout(kind: ItemKind) -> Option<&'static str> {
    match kind {
        ItemKind::Code => Some("code-block"),
        ItemKind::Quote => Some("quote-block"),
        ItemKind::Divider => Some("divider"),
        _ => None,
    }
}

/// §4.3: the layout the Markdown explicitly asks for, if its kind changed. `base_layout`
/// is the node's layout in the base.
///
/// * The item's own kind changed to a fence, a quote or `---`: that kind's layout; from
///   one of them back to a plain item: `bullets`.
/// * Otherwise only a plain item can change, and only through the ordered markers of its
///   children ([`numbered_target`]). Code, quote and divider items, and every layout
///   §4.3 keeps as is, never derive a layout change from their children.
fn layout_target(base: &LocalItem, local: &LocalItem, base_layout: &str) -> Option<String> {
    use ItemKind::{Bullet, ConflictMarker, MirrorStub, Placeholder};
    match (base.kind, local.kind) {
        (Placeholder | MirrorStub, _)
        | (_, Placeholder | MirrorStub)
        | (ConflictMarker(_), _)
        | (_, ConflictMarker(_)) => None,
        (b, l) if b != l => match kind_layout(l) {
            Some(t) => Some(t.to_owned()),
            // Back to a plain item from a fence, quote or divider.
            None => Some(DEFAULT_LAYOUT.to_owned()),
        },
        (Bullet, Bullet) => numbered_target(base_layout, &base.children, &local.children),
        _ => None,
    }
}

fn walk_doc<'a>(items: &'a [LocalItem], out: &mut Vec<&'a LocalItem>) {
    for it in items {
        out.push(it);
        walk_doc(&it.children, out);
    }
}

impl<'a, 'c> Engine<'a, 'c> {
    fn new(inp: MergeInput<'a>, codec: &'c dyn TextCodec) -> Engine<'a, 'c> {
        let root = inp.root.clone();
        let mut base_sub = BTreeSet::new();
        if inp.base.contains(&root) {
            base_sub.insert(root.clone());
            base_sub.extend(inp.base.descendants(&root));
        }
        let mut remote_sub = BTreeSet::new();
        remote_sub.insert(root.clone());
        remote_sub.extend(inp.remote.descendants(&root));

        let mut priors = Vec::new();
        for doc in inp.prior_docs {
            let mut all = Vec::new();
            walk_doc(&doc.items, &mut all);
            let mut map = BTreeMap::new();
            for it in all {
                if it.is_marker() {
                    continue;
                }
                if let Some(t) = &it.tag {
                    map.entry(t.short.clone()).or_insert(it);
                }
            }
            priors.push(map);
        }

        let mut e = Engine {
            inp,
            codec,
            inline: &inp.cfg.doc.inline,
            root: root.clone(),
            force: inp.cfg.force,
            safe: inp.cfg.safe_mode,
            shell_index: inp.remote.shells_by_origin(),
            base_sub,
            remote_sub,
            remote_vis: BTreeSet::new(),
            base_items: BTreeMap::new(),
            base_order: Vec::new(),
            base_children: BTreeMap::new(),
            priors,
            items: Vec::new(),
            markers: Vec::new(),
            adopted: vec![false; inp.adoptions.len()],
            detected: Vec::new(),
            m: Tree::new(),
            pushes: BTreeMap::new(),
            creates: Vec::new(),
            moves: Vec::new(),
            ticks: Vec::new(),
            deletes: Vec::new(),
            deleted: BTreeSet::new(),
            conflicts: BTreeMap::new(),
            new_conflicts: 0,
            report: Vec::new(),
            refused: BTreeSet::new(),
            cancelled: BTreeSet::new(),
            outside: BTreeSet::new(),
            suppressed: 0,
            stats: MergeStats::default(),
        };
        e.base_children.insert(root.clone(), Vec::new());
        e.index_base(&inp.base_doc.items, &root);
        e.flatten(&inp.local_doc.items, None);
        e.detected = vec![None; e.items.len()];
        e
    }

    // ----- helpers ---------------------------------------------------------------------

    fn md_to_wf(&self, md: &str, kind: FieldKind) -> String {
        self.codec.md_to_wf(md, kind, self.inline)
    }

    fn plain_of_md(&self, item: &LocalItem) -> String {
        self.codec
            .plain_text(&self.md_to_wf(&item.name_md, field_kind(item)))
    }

    fn note_md(item: &LocalItem) -> String {
        item.note_md.join("\n")
    }

    fn refuse(&mut self, node: &NodeId, line: usize) {
        if self.refused.insert(node.clone()) {
            self.report.push(ReportEvent::ReadOnlyEditRefused {
                node: node.clone(),
                line,
            });
        }
    }

    /// The node of an item's parent as identified (root for top-level items); `None`
    /// if the parent is new.
    fn parent_node(&self, i: usize) -> Option<NodeId> {
        match self.items[i].parent {
            None => Some(self.root.clone()),
            Some(p) => self.items[p].ident.node().cloned(),
        }
    }

    /// Where the item's local parent is in `M` (root, an existing node or a create);
    /// `None` if the parent item is gone or dropped.
    fn dest_of(&self, i: usize) -> Option<NodeId> {
        match self.items[i].parent {
            None => Some(self.root.clone()),
            Some(p) => self.items[p].fate.id().cloned(),
        }
    }

    /// A base item moved to another parent in the file.
    fn moved_locally(&self, i: usize) -> bool {
        let Some(n) = self.items[i].ident.base_node() else {
            return false;
        };
        let bp = &self.base_items[n].parent;
        self.parent_node(i).as_ref() != Some(bp)
    }

    /// §6.2: the base item's node was moved out of the subtree remotely (it still
    /// exists), and the item carries a local edit or a tick that must follow it.
    fn moved_out(&self, i: usize, nd: &NodeId) -> bool {
        !self.remote_sub.contains(nd)
            && self.inp.remote.contains(nd)
            && (self.items[i].item.checkbox == Checkbox::Ticked
                || self.detected[i].as_ref().is_some_and(Detected::any))
    }

    fn is_existing_placeholder(&self, tree: &Tree, id: &NodeId) -> bool {
        !self.force && id != &self.root && tree.get(id).is_some_and(is_placeholder_node)
    }

    /// §15.1: the origin that blocks a delete of `n` in `M`, if any: `n` itself or a
    /// node below it, not looking through shells (a shell placement's descendants in the
    /// view are its origin's children, which its delete leaves alone), that still has
    /// live shells or registry entries (DM§10.5).
    fn mirror_delete_blocker(&self, n: &NodeId) -> Option<NodeId> {
        let node = self.m.get(n)?;
        if node.is_shell() {
            return None;
        }
        let mut candidates = vec![n.clone()];
        candidates.extend(self.m.descendants_where(n, |x| !x.is_shell()));
        candidates.into_iter().find(|x| {
            self.m.get(x).is_some_and(|node| {
                !node.is_shell()
                    && (node.has_registered_mirrors()
                        || self.shell_index.get(x).is_some_and(|s| !s.is_empty()))
            })
        })
    }

    /// The descendants of `n` in `M` that a delete of `n` removes: none for a shell
    /// placement (its view children are the content's), and below a shell placement
    /// nothing but the shell itself (§15.1).
    fn deleted_descendants(&self, n: &NodeId) -> Vec<NodeId> {
        if self.m.get(n).is_some_and(Node::is_shell) {
            return Vec::new();
        }
        self.m.descendants_where(n, |x| !x.is_shell())
    }

    /// The node changed remotely since the base (fields, completion or parent; §6.2
    /// "edited"). Nodes absent from the base are remote creates: changed.
    fn changed_self(&self, n: &NodeId) -> bool {
        let (Some(b), Some(r)) = (self.inp.base.get(n), self.inp.remote.get(n)) else {
            return true;
        };
        if !self.base_sub.contains(n) {
            return true;
        }
        b.name != r.name
            || b.note.as_deref().unwrap_or("") != r.note.as_deref().unwrap_or("")
            || b.layout != r.layout
            || b.completed != r.completed
            || b.parent_id != r.parent_id
    }

    fn changed_deep(&self, n: &NodeId) -> bool {
        self.changed_self(n) || self.m.descendants(n).iter().any(|d| self.changed_self(d))
    }

    // ----- setup -----------------------------------------------------------------------

    fn index_base(&mut self, items: &'a [LocalItem], parent: &NodeId) {
        let mut index = 0;
        for it in items {
            if it.is_marker() {
                continue;
            }
            let node = it
                .tag
                .as_ref()
                .and_then(|t| self.inp.ids.get(&t.short))
                .filter(|n| {
                    self.base_sub.contains(*n)
                        && *n != &self.root
                        && !self.base_items.contains_key(*n)
                })
                .cloned();
            match node {
                Some(n) => {
                    self.base_items.insert(
                        n.clone(),
                        BItem {
                            item: it,
                            parent: parent.clone(),
                            index,
                        },
                    );
                    self.base_order.push(n.clone());
                    self.base_children
                        .entry(parent.clone())
                        .or_default()
                        .push(n.clone());
                    self.base_children.entry(n.clone()).or_default();
                    self.index_base(&it.children, &n);
                }
                // An untagged base item cannot be identified (renders always tag).
                None => self.index_base(&it.children, parent),
            }
            index += 1;
        }
    }

    fn flatten(&mut self, items: &'a [LocalItem], parent: Option<usize>) -> Vec<usize> {
        let mut kids = Vec::new();
        let mut index = 0;
        for it in items {
            if it.is_marker() {
                self.markers.push((parent, it));
                continue;
            }
            let id = self.items.len();
            self.items.push(LItem {
                item: it,
                parent,
                index,
                children: Vec::new(),
                ident: Ident::New,
                fate: Fate::Undecided,
                resurrects: None,
                cross_dest: None,
                placed_parent: None,
            });
            index += 1;
            kids.push(id);
            let ch = self.flatten(&it.children, Some(id));
            self.items[id].children = ch;
        }
        kids
    }

    // ----- phase 1: identity (§4.4) ------------------------------------------------------

    fn other_root_of(&self, n: &NodeId) -> Option<NodeId> {
        let roots = &self.inp.cross.other_roots;
        if roots.contains(n) {
            return None;
        }
        self.inp
            .remote
            .ancestors(n)
            .into_iter()
            .find(|a| roots.contains(a))
    }

    fn classify_tag(&self, t: &IdTag) -> Option<Cand> {
        let n = self.inp.ids.get(&t.short)?;
        if n == &self.root {
            return None;
        }
        if self.base_sub.contains(n) || self.remote_sub.contains(n) {
            return Some(Cand::Local(n.clone()));
        }
        if self.inp.remote.contains(n) {
            return self.other_root_of(n).map(|r| Cand::Cross(n.clone(), r));
        }
        None
    }

    fn identify(&mut self) {
        let n = self.items.len();
        // Step 1: tags; "take the last valid one" (§4.4).
        let mut cand: Vec<Option<Cand>> = vec![None; n];
        for (i, slot) in cand.iter_mut().enumerate() {
            let it = self.items[i].item;
            let tags: Vec<&IdTag> = it.tag.iter().chain(it.extra_tags.iter().rev()).collect();
            let Some(first) = tags.first() else { continue };
            *slot = tags.iter().find_map(|t| self.classify_tag(t));
            if slot.is_none() {
                self.report.push(ReportEvent::ForeignTag {
                    short: first.short.clone(),
                    line: it.line,
                });
            }
        }
        // Duplicated tags: the best match of the base item keeps the identity. Parents
        // first (file pre-order), so that "same parent" compares with the parent's
        // resolved identity (a pasted copy of a subtree is a new parent, §4.4).
        let mut by_node: BTreeMap<NodeId, Vec<usize>> = BTreeMap::new();
        for (i, c) in cand.iter().enumerate() {
            if let Some(c) = c {
                by_node.entry(c.node().clone()).or_default().push(i);
            }
        }
        for list in by_node.values() {
            if let [only] = list.as_slice() {
                self.items[*only].ident = self.ident_of(cand[*only].clone().expect("candidate"));
            }
        }
        let mut state: BTreeMap<NodeId, bool> = BTreeMap::new();
        for c in cand.iter().flatten() {
            if by_node[c.node()].len() > 1 {
                self.resolve_duplicate(c.node(), &by_node, &cand, &mut state);
            }
        }
        // Items whose tags were all invalid stay `New`.
        self.adopt();
        self.rematch();
    }

    /// The identity a winning candidate gives its item.
    fn ident_of(&self, c: Cand) -> Ident {
        match c {
            Cand::Cross(n, r) => Ident::Cross(n, r),
            Cand::Local(n) => {
                if self.base_items.contains_key(&n) {
                    Ident::Base(n)
                } else if self.inp.pending_deletes.contains(&n) && self.remote_sub.contains(&n) {
                    Ident::Pending(n)
                } else {
                    Ident::Known(n)
                }
            }
        }
    }

    /// Resolve the items sharing the tag of `node`, after the duplicated tags of their
    /// parents (`state`: `false` while in progress, `true` once done; a cycle between
    /// groups falls back to the parent's raw tag).
    fn resolve_duplicate(
        &mut self,
        node: &NodeId,
        by_node: &BTreeMap<NodeId, Vec<usize>>,
        cand: &[Option<Cand>],
        state: &mut BTreeMap<NodeId, bool>,
    ) {
        if state.contains_key(node) {
            return;
        }
        state.insert(node.clone(), false);
        let list = &by_node[node];
        for &i in list {
            if let Some(p) = self.items[i].parent
                && let Some(pc) = &cand[p]
                && by_node[pc.node()].len() > 1
            {
                self.resolve_duplicate(pc.node(), by_node, cand, state);
            }
        }
        let w = self.best_duplicate(node, list, cand, state);
        self.report.push(ReportEvent::DuplicateTag {
            short: node.short().expect("resolved through the id map"),
            lines: list.iter().map(|&i| self.items[i].item.line).collect(),
            kept_line: self.items[w].item.line,
        });
        for &i in list {
            if i != w {
                self.items[i].ident = Ident::Duplicate;
            }
        }
        self.items[w].ident = self.ident_of(cand[w].clone().expect("candidate"));
        state.insert(node.clone(), true);
    }

    /// §4.4: the occurrence that best matches the base item (text similarity, then same
    /// parent, then position). "Same parent" uses the parent item's resolved identity:
    /// under a pasted copy of the parent (a `Duplicate`, so a new node) it is false.
    fn best_duplicate(
        &self,
        node: &NodeId,
        list: &[usize],
        cand: &[Option<Cand>],
        state: &BTreeMap<NodeId, bool>,
    ) -> usize {
        let Some(bi) = self.base_items.get(node) else {
            return list[0];
        };
        let base_text = self.plain_of_md(bi.item);
        let mut best = list[0];
        let mut best_key = None;
        for &i in list {
            let li = &self.items[i];
            let text = self.plain_of_md(li.item);
            let parent = match li.parent {
                None => Some(&self.root),
                Some(p) => match &cand[p] {
                    // A cycle between duplicated groups: the parent's raw tag.
                    Some(c) if state.get(c.node()) == Some(&false) => Some(c.node()),
                    _ => self.items[p].ident.node(),
                },
            };
            let key = (
                ratio_key(similarity(&text, &base_text)),
                parent == Some(&bi.parent),
                -(li.index.abs_diff(bi.index) as i64),
            );
            if best_key.is_none_or(|b| key > b) {
                best_key = Some(key);
                best = i;
            }
        }
        best
    }

    fn claimed(&self) -> BTreeSet<NodeId> {
        self.items
            .iter()
            .filter_map(|li| li.ident.node().cloned())
            .collect()
    }

    /// Step 3: adoptions from an interrupted apply.
    fn adopt(&mut self) {
        let adoptions = self.inp.adoptions;
        if adoptions.is_empty() {
            return;
        }
        let mut claimed = self.claimed();
        for i in 0..self.items.len() {
            let it = self.items[i].item;
            if self.items[i].ident != Ident::New || !is_untagged(it) {
                continue;
            }
            let Some(pn) = self.parent_node(i) else {
                continue;
            };
            let wf = self.md_to_wf(&it.name_md, field_kind(it));
            let found = adoptions.iter().enumerate().find(|(k, a)| {
                !self.adopted[*k]
                    && a.parent == pn
                    && a.name == wf
                    && !claimed.contains(&a.node)
                    && self.remote_sub.contains(&a.node)
            });
            if let Some((k, a)) = found {
                self.adopted[k] = true;
                claimed.insert(a.node.clone());
                self.items[i].ident = Ident::Adopted(a.node.clone(), k);
                self.report.push(ReportEvent::Adopted {
                    node: a.node.clone(),
                    line: it.line,
                });
            }
        }
    }

    /// Step 4: the deleted-tag fallback, also for reappearing pending deletes (§9.1
    /// guard 2). Greedy by (ratio, distance), parents before children.
    fn rematch(&mut self) {
        let remote = self.inp.remote;
        let pending = self.inp.pending_deletes;
        loop {
            let claimed = self.claimed();
            // (-ratio, distance, item, candidate, is_pending)
            let mut pairs: Vec<(i64, usize, usize, NodeId, bool)> = Vec::new();
            for i in 0..self.items.len() {
                let li = &self.items[i];
                if li.ident != Ident::New || !is_untagged(li.item) {
                    continue;
                }
                let Some(pn) = self.parent_node(i) else {
                    continue;
                };
                let text = self.plain_of_md(li.item);
                // Only the base items under the same parent are candidates.
                let siblings = self
                    .base_children
                    .get(&pn)
                    .map(Vec::as_slice)
                    .unwrap_or(&[]);
                for node in siblings {
                    let bi = &self.base_items[node];
                    if claimed.contains(node) {
                        continue;
                    }
                    let dist = li.index.abs_diff(bi.index);
                    if dist > REMATCH_MAX_DISTANCE {
                        continue;
                    }
                    let r = similarity(&text, &self.plain_of_md(bi.item));
                    if r >= REMATCH_MIN_RATIO {
                        pairs.push((-ratio_key(r), dist, i, node.clone(), false));
                    }
                }
                for p in pending {
                    if claimed.contains(p)
                        || self.base_items.contains_key(p)
                        || !self.remote_sub.contains(p)
                        || remote.parent(p) != Some(&pn)
                    {
                        continue;
                    }
                    // Its position among the siblings the file showed, plus itself.
                    let Some(pos) = remote
                        .children(&pn)
                        .iter()
                        .filter(|c| {
                            *c == p
                                || (!pending.contains(*c)
                                    && remote.get(c).is_some_and(|x| !x.completed))
                        })
                        .position(|c| c == p)
                    else {
                        continue;
                    };
                    let dist = li.index.abs_diff(pos);
                    if dist > REMATCH_MAX_DISTANCE {
                        continue;
                    }
                    let name = &remote.get(p).expect("in remote").name;
                    let r = similarity(&text, &self.codec.plain_text(name));
                    if r >= REMATCH_MIN_RATIO {
                        pairs.push((-ratio_key(r), dist, i, p.clone(), true));
                    }
                }
            }
            if pairs.is_empty() {
                return;
            }
            pairs.sort();
            let mut taken_items = BTreeSet::new();
            let mut taken_nodes = BTreeSet::new();
            for (_, _, i, node, is_pending) in pairs {
                if taken_items.contains(&i) || taken_nodes.contains(&node) {
                    continue;
                }
                taken_items.insert(i);
                taken_nodes.insert(node.clone());
                self.report.push(ReportEvent::Rematched {
                    node: node.clone(),
                    line: self.items[i].item.line,
                });
                self.items[i].ident = if is_pending {
                    Ident::Pending(node)
                } else {
                    Ident::Rematched(node)
                };
            }
        }
    }

    // ----- phase 2: cancellations and visibility --------------------------------------

    fn cancel_and_visibility(&mut self) {
        for i in 0..self.items.len() {
            if let Ident::Pending(n) = &self.items[i].ident
                && self.cancelled.insert(n.clone())
            {
                self.report
                    .push(ReportEvent::PendingDeleteCancelled { node: n.clone() });
            }
        }
        let hidden: BTreeSet<NodeId> = self
            .inp
            .pending_deletes
            .difference(&self.cancelled)
            .cloned()
            .collect();
        self.remote_vis = visible_nodes(self.inp.remote, &self.root, &self.inp.cfg.doc, &hidden);
        // §9.1 guard 3 (structure): a tagged item not in the base and hidden remotely is
        // a merge artefact.
        for li in &mut self.items {
            if let Ident::Known(n) | Ident::Pending(n) = &li.ident
                && !self.remote_vis.contains(n)
            {
                li.fate = Fate::Dropped;
                self.report.push(ReportEvent::ResurrectionDropped {
                    node: n.clone(),
                    line: li.item.line,
                });
            }
        }
    }

    // ----- phase 3: local field changes (§5.1, §6.1) -----------------------------------

    fn detect_all(&mut self) {
        for i in 0..self.items.len() {
            let n = match &self.items[i].ident {
                Ident::Base(n) | Ident::Rematched(n) => n.clone(),
                // §6.4: a line cut from another file of the run is compared with that
                // file's render of it (`CrossFile::bases`). UNVERIFIED design choice:
                // without one, nothing is pushed (the pasted text may be any older
                // render, guard 3), as for nodes hidden in the base.
                Ident::Cross(n, _) if self.inp.cross.bases.contains_key(n) => n.clone(),
                Ident::Known(n) if self.arrived(n) => n.clone(),
                _ => continue,
            };
            let d = self.detect(i, &n);
            self.detected[i] = Some(d);
        }
    }

    /// §6.4 + §9: a node of another file's root that is already in this root remotely
    /// but has no line in this file's base: the cross-file move was applied by an
    /// earlier pass of this cycle, which merges again because a write was skipped
    /// (`sync::rerun`). Its tag now resolves inside the subtree ([`Ident::Known`]), yet
    /// the pasted line is still an old render from the source file, so its fields are
    /// merged against that file's base ([`CrossFile::bases`]) exactly as in the pass
    /// that moved it: the skipped edit becomes a §6.1 conflict instead of being dropped
    /// silently. `bases` only holds nodes that were under another root at the run's
    /// export, so outside such a rerun no node of this subtree has an entry.
    fn arrived(&self, n: &NodeId) -> bool {
        self.remote_sub.contains(n) && self.inp.cross.bases.contains_key(n)
    }

    /// The base of item `i`'s field merge (§5.1, §6.1): its line in the base render, the
    /// node's base value, and its lines in earlier renders (guard 3). For a cross-file
    /// move (§6.4), also one already applied ([`Engine::arrived`]), all three come from
    /// the source file (`CrossFile::bases`).
    fn base_of(&self, i: usize, n: &NodeId) -> (&'a LocalItem, &'a Node, Vec<&'a LocalItem>) {
        let cross = match &self.items[i].ident {
            Ident::Cross(..) => true,
            Ident::Known(k) => self.arrived(k),
            _ => false,
        };
        if cross {
            let cross: &'a CrossFile = self.inp.cross;
            let cb = &cross.bases[n];
            return (&cb.item, &cb.wf, cb.prior.iter().collect());
        }
        let base: &'a Tree = self.inp.base;
        let priors = match n.short() {
            Some(short) => self
                .priors
                .iter()
                .filter_map(|p| p.get(&short))
                .copied()
                .collect(),
            None => Vec::new(),
        };
        (
            self.base_items[n].item,
            base.get(n).expect("base item in base"),
            priors,
        )
    }

    fn detect(&mut self, i: usize, n: &NodeId) -> Detected {
        let li = self.items[i].item;
        let (bi, base_node, priors) = self.base_of(i, n);
        // Guard 3: the local value equals the node's value in an earlier render.
        let stale = |local_wf: &str, value: &dyn Fn(&LocalItem) -> String| {
            priors.iter().any(|p| value(p) == local_wf)
        };
        let kb = field_kind(bi);
        let kl = field_kind(li);
        // §10 placeholders, §15.1 stubs and §5.3 read-only items.
        let ro_flagged =
            has_ro_flag(bi) || matches!(bi.kind, ItemKind::Placeholder | ItemKind::MirrorStub);
        let ro_item = !self.force && ro_flagged;
        let mut refused = false;
        let mut d = Detected {
            name: None,
            name_kind: kl,
            name_seen: false,
            note: None,
            note_seen: false,
            layout: None,
        };

        let wl = self.md_to_wf(&li.name_md, kl);
        if self.md_to_wf(&bi.name_md, kb) != wl {
            let stale = stale(&wl, &|p| {
                self.codec.md_to_wf(&p.name_md, field_kind(p), self.inline)
            });
            if stale {
                self.report.push(ReportEvent::StaleEcho {
                    node: n.clone(),
                    field: ConflictField::Name,
                });
            } else {
                let tier = self.codec.classify(&base_node.name, kb, self.inline);
                if ro_item || (!self.force && tier == Tier::ReadOnly) {
                    refused = true;
                } else {
                    d.name = Some(li.name_md.clone());
                    d.name_seen = ro_flagged
                        || tier == Tier::ReadOnly
                        || (kb != kl && tier <= Tier::Passthrough);
                }
            }
        }

        let local_note = Self::note_md(li);
        let wl = self.md_to_wf(&local_note, FieldKind::Note);
        if self.md_to_wf(&Self::note_md(bi), FieldKind::Note) != wl {
            let stale = stale(&wl, &|p| {
                self.codec
                    .md_to_wf(&Self::note_md(p), FieldKind::Note, self.inline)
            });
            if stale {
                self.report.push(ReportEvent::StaleEcho {
                    node: n.clone(),
                    field: ConflictField::Note,
                });
            } else {
                let base_note = base_node.note.as_deref().unwrap_or("");
                let tier = self.codec.classify(base_note, FieldKind::Note, self.inline);
                if ro_item || (!self.force && tier == Tier::ReadOnly) {
                    refused = true;
                } else {
                    d.note = Some(local_note);
                    d.note_seen = ro_flagged || tier == Tier::ReadOnly;
                }
            }
        }

        if let Some(t) = layout_target(bi, li, &base_node.layout)
            && t != base_node.layout
        {
            if ro_item {
                refused = true;
            } else {
                d.layout = Some(t);
            }
        }
        if refused {
            self.refuse(n, li.line);
        }
        d
    }

    // ----- phase 4: fates ----------------------------------------------------------------

    fn decide_fates(&mut self) {
        let n = self.items.len();
        let remote = self.inp.remote;
        // §6.2: a node deleted remotely but edited (or moved) locally, or with local
        // children added, is resurrected with its whole local subtree.
        let mut res = vec![false; n];
        for i in (0..n).rev() {
            let Some(node) = self.items[i].ident.base_node() else {
                continue;
            };
            if remote.contains(node) {
                continue;
            }
            let self_edit =
                self.detected[i].as_ref().is_some_and(Detected::any) || self.moved_locally(i);
            let child_needs = self.items[i]
                .children
                .iter()
                .any(|&c| match &self.items[c].ident {
                    Ident::New | Ident::Duplicate | Ident::Cross(..) => true,
                    Ident::Base(cn) | Ident::Rematched(cn) => {
                        res[c] || (self.remote_sub.contains(cn) && self.moved_locally(c))
                    }
                    _ => false,
                });
            res[i] = self_edit || child_needs;
        }
        for (i, li) in self.items.iter().enumerate() {
            if let (Some(p), Some(node)) = (li.parent, li.ident.base_node())
                && res[p]
                && !remote.contains(node)
            {
                res[i] = true;
            }
        }

        let mut next = 1u32;
        let mut temp = || {
            let t = NodeId::temp(next);
            next += 1;
            t
        };
        let mut claimed = self.claimed();
        for (i, &resurrect) in res.iter().enumerate() {
            if self.items[i].fate == Fate::Dropped {
                continue;
            }
            let fate = match self.items[i].ident.clone() {
                Ident::New | Ident::Duplicate => match self.adopt_create(i, &mut claimed) {
                    Some(nd) => Fate::Existing(nd),
                    None => Fate::Create(temp()),
                },
                Ident::Base(nd) | Ident::Rematched(nd) => {
                    if self.remote_sub.contains(&nd) {
                        Fate::Existing(nd)
                    } else if !remote.contains(&nd) && resurrect {
                        self.items[i].resurrects = Some(nd);
                        match self.adopt_create(i, &mut claimed) {
                            Some(nd) => Fate::Existing(nd),
                            None => Fate::Create(temp()),
                        }
                    } else {
                        Fate::Gone
                    }
                }
                Ident::Known(nd) | Ident::Pending(nd) | Ident::Adopted(nd, _) => Fate::Existing(nd),
                Ident::Cross(nd, _) => self.cross_fate(i, nd),
            };
            self.items[i].fate = fate;
        }
    }

    fn cross_fate(&mut self, i: usize, nd: NodeId) -> Fate {
        if self.safe {
            self.suppressed += 1;
            return Fate::Dropped;
        }
        let remote_parent = self.inp.remote.parent(&nd).cloned();
        // Carried along with a parent that moves in from the same place.
        if let Some(p) = self.items[i].parent
            && let Ident::Cross(pn, _) = &self.items[p].ident
            && self.items[p].fate == Fate::Existing(pn.clone())
            && remote_parent.as_ref() == Some(pn)
        {
            return Fate::Existing(nd);
        }
        match self.dest_of(i) {
            None => {
                self.report.push(ReportEvent::DroppedMove {
                    node: nd,
                    cycle: false,
                });
                Fate::Dropped
            }
            Some(d) => {
                if self.is_existing_placeholder(self.inp.remote, &d) {
                    self.refuse(&d, self.items[i].item.line);
                    return Fate::Dropped;
                }
                self.items[i].cross_dest = Some(d);
                Fate::Existing(nd)
            }
        }
    }

    /// §7.3: an item about to become a create that an interrupted apply, or an earlier
    /// pass of this cycle (a CAS rerun, §9), already created: an untagged item whose
    /// parent was not known at step 3 of identity (it is adopted itself, or resurrected),
    /// an item that lost a duplicated tag, one whose tags are all invalid, or a
    /// resurrection (its tag still names the deleted node). It is matched like step 3
    /// (same parent in `M`, the name it would be created with) and becomes
    /// [`Ident::Adopted`], so the journal never re-creates a node.
    fn adopt_create(&mut self, i: usize, claimed: &mut BTreeSet<NodeId>) -> Option<NodeId> {
        let adoptions = self.inp.adoptions;
        if adoptions.is_empty() {
            return None;
        }
        // The create's parent (`create_parent`, without refusing anything yet): a parent
        // that is itself a create has no adoption.
        let parent = match self.create_parent_walk(i, self.inp.remote).0 {
            None => self.root.clone(),
            Some(p) if !p.is_temp() => p,
            Some(_) => return None,
        };
        let name = self.create_values(i).name;
        let (k, a) = adoptions.iter().enumerate().find(|(k, a)| {
            !self.adopted[*k]
                && a.parent == parent
                && a.name == name
                && !claimed.contains(&a.node)
                && self.remote_sub.contains(&a.node)
        })?;
        self.adopted[k] = true;
        claimed.insert(a.node.clone());
        self.items[i].ident = Ident::Adopted(a.node.clone(), k);
        self.report.push(ReportEvent::Adopted {
            node: a.node.clone(),
            line: self.items[i].item.line,
        });
        Some(a.node.clone())
    }

    // ----- phase 5: build M ----------------------------------------------------------------

    fn build_merged(&mut self) {
        self.m = self.inp.remote.subtree(&self.root).expect("root checked");
        // Cross-file subtrees, detached until their move (§6.4).
        for i in 0..self.items.len() {
            if self.items[i].cross_dest.is_none() {
                continue;
            }
            let Fate::Existing(nd) = self.items[i].fate.clone() else {
                continue;
            };
            for x in self.inp.remote.preorder(&nd) {
                if !self.m.contains(&x) {
                    let node = self.inp.remote.get(&x).expect("in remote").clone();
                    self.m.insert(node).expect("not in M");
                }
            }
        }
        self.merge_root_fields();
        for i in 0..self.items.len() {
            match (self.items[i].ident.clone(), self.items[i].fate.clone()) {
                (Ident::Base(nd) | Ident::Rematched(nd), Fate::Existing(_)) => {
                    self.merge_base_fields(i, &nd)
                }
                // §6.4: the same three-way merge, against the source file's base.
                (Ident::Cross(nd, _), Fate::Existing(_)) => self.merge_base_fields(i, &nd),
                (Ident::Known(nd), Fate::Existing(_)) if self.arrived(&nd) => {
                    self.merge_base_fields(i, &nd)
                }
                (Ident::Adopted(nd, k), Fate::Existing(_)) => self.merge_adopted_fields(i, &nd, k),
                (Ident::Base(nd) | Ident::Rematched(nd), Fate::Gone) if self.moved_out(i, &nd) => {
                    // §6.2: moved out of the subtree remotely, but the node exists: the
                    // local edits are pushed where it is now (module docs).
                    self.outside.insert(nd.clone());
                    self.merge_base_fields(i, &nd);
                }
                _ => {}
            }
        }
        for i in 0..self.items.len() {
            if let Fate::Create(t) = self.items[i].fate.clone() {
                self.create(i, t);
            }
        }
        for i in 0..self.items.len() {
            let Fate::Existing(nd) = self.items[i].fate.clone() else {
                continue;
            };
            match self.items[i].ident.clone() {
                Ident::Base(_) | Ident::Rematched(_) => self.local_move(i, &nd),
                Ident::Cross(_, r) => self.cross_move(i, &nd, &r),
                _ => {}
            }
        }
        for i in 0..self.items.len() {
            let nd = match (&self.items[i].fate, &self.items[i].ident) {
                (Fate::Existing(nd), _) => nd.clone(),
                (Fate::Gone, Ident::Base(nd) | Ident::Rematched(nd)) if self.moved_out(i, nd) => {
                    self.outside.insert(nd.clone());
                    nd.clone()
                }
                _ => continue,
            };
            if self.items[i].item.checkbox == Checkbox::Ticked {
                if self.tick_refused(i, &nd) {
                    self.refuse(&nd, self.items[i].item.line);
                } else {
                    self.tick(&nd);
                }
            }
        }
        self.apply_deletes();
    }

    fn add_conflict(&mut self, n: &NodeId, field: ConflictField, local_md: String) {
        if self.outside.contains(n) {
            // The line leaves the file, so no marker can hold the local text (§6.2).
            self.report.push(ReportEvent::MovedOutConflict {
                node: n.clone(),
                field,
                local_md,
            });
            return;
        }
        self.conflicts.insert(
            (n.clone(), field),
            ConflictMarker {
                node: n.clone(),
                field,
                local_md,
            },
        );
        self.new_conflicts += 1;
        self.report.push(ReportEvent::Conflict {
            node: n.clone(),
            field,
        });
    }

    fn commit_push(&mut self, n: &NodeId, push: FieldPush) {
        if !push.is_empty() {
            self.pushes.insert(n.clone(), push);
        }
    }

    /// §6.1 for a name (character-level text, no line merge). `seen`: see
    /// [`Detected::name_seen`].
    #[allow(clippy::too_many_arguments)]
    fn merge_name(
        &mut self,
        n: &NodeId,
        kind: FieldKind,
        seen: bool,
        base_val: &str,
        remote_val: &str,
        md: &str,
        ticked: bool,
        push: &mut FieldPush,
    ) {
        let w = self.codec.write_value(base_val, md, kind, self.inline);
        if base_val == remote_val {
            if w.wf != remote_val {
                self.set_name(n, remote_val, w.wf, w.lost && !seen, push);
            }
        } else if w.wf != remote_val {
            if ticked {
                // §6.2 table row 1: the completion is pushed and the remote edit kept;
                // the line leaves the file, so the report keeps the typed text.
                self.report.push(ReportEvent::TickedConflict {
                    node: n.clone(),
                    field: ConflictField::Name,
                    local_md: md.to_owned(),
                });
            } else {
                // Both changed differently: remote wins, local kept as a marker.
                self.add_conflict(n, ConflictField::Name, md.to_owned());
            }
        }
    }

    fn set_name(
        &mut self,
        n: &NodeId,
        expected: &str,
        new: String,
        lost: bool,
        push: &mut FieldPush,
    ) {
        if lost {
            self.report.push(ReportEvent::LossyWrite {
                node: n.clone(),
                field: ConflictField::Name,
            });
        }
        if !self.outside.contains(n) {
            self.m.set_name(n, new.clone()).expect("node in M");
        }
        push.name = Some(Change {
            expected: expected.to_owned(),
            new,
        });
    }

    fn set_note(
        &mut self,
        n: &NodeId,
        expected: Option<&str>,
        new: String,
        lost: bool,
        push: &mut FieldPush,
    ) {
        if lost {
            self.report.push(ReportEvent::LossyWrite {
                node: n.clone(),
                field: ConflictField::Note,
            });
        }
        if !self.outside.contains(n) {
            self.m.set_note(n, Some(new.clone())).expect("node in M");
        }
        push.note = Some(Change {
            expected: expected.map(str::to_owned),
            new: Some(new),
        });
    }

    fn set_layout(&mut self, n: &NodeId, expected: &str, new: &str, push: &mut FieldPush) {
        if !self.outside.contains(n) {
            self.m.set_layout(n, new).expect("node in M");
        }
        push.layout = Some(Change {
            expected: expected.to_owned(),
            new: new.to_owned(),
        });
    }

    /// §6.1 for a note: a line-level diff3 first, then a conflict. `seen`: see
    /// [`Detected::note_seen`].
    #[allow(clippy::too_many_arguments)]
    fn merge_note(
        &mut self,
        n: &NodeId,
        seen: bool,
        base_val: &str,
        remote: Option<&str>,
        md: &str,
        ticked: bool,
        push: &mut FieldPush,
    ) {
        let rv = remote.unwrap_or("");
        let w = self
            .codec
            .write_value(base_val, md, FieldKind::Note, self.inline);
        let lost = w.lost && !seen;
        if base_val == rv {
            if w.wf != rv {
                self.set_note(n, remote, w.wf, lost, push);
            }
            return;
        }
        if w.wf == rv {
            return;
        }
        if ticked {
            // §6.2 table row 1: ticked vs edited remotely: push nothing but the
            // completion, keep the remote edit, and report the typed text.
            self.report.push(ReportEvent::TickedConflict {
                node: n.clone(),
                field: ConflictField::Note,
                local_md: md.to_owned(),
            });
            return;
        }
        match diff3_lines(base_val, &w.wf, rv) {
            Some(merged) if self.codec.is_canonical(&merged) => {
                if merged != rv {
                    self.set_note(n, remote, merged, lost, push);
                }
            }
            _ => self.add_conflict(n, ConflictField::Note, md.to_owned()),
        }
    }

    /// §6.1 for a layout: remote wins a conflict (there is no marker form for it).
    fn merge_layout(
        &mut self,
        n: &NodeId,
        base_val: &str,
        remote_val: &str,
        target: &str,
        push: &mut FieldPush,
    ) {
        // UNVERIFIED design choice: a layout changed on both sides keeps the remote
        // value without a conflict marker.
        if base_val == remote_val && target != remote_val {
            self.set_layout(n, remote_val, target, push);
        }
    }

    fn merge_root_fields(&mut self) {
        let root = self.root.clone();
        let remote_root = self.inp.remote.get(&root).expect("root checked").clone();
        let base_root = self
            .inp
            .base
            .get(&root)
            .cloned()
            .unwrap_or_else(|| remote_root.clone());
        let base_doc: &LocalDoc = self.inp.base_doc;
        let local_doc: &LocalDoc = self.inp.local_doc;
        let mut push = FieldPush::default();

        let local_md = local_doc.root_note.join("\n");
        let wl = self.md_to_wf(&local_md, FieldKind::Note);
        if self.md_to_wf(&base_doc.root_note.join("\n"), FieldKind::Note) != wl {
            let stale = self.inp.prior_docs.iter().any(|p| {
                self.codec
                    .md_to_wf(&p.root_note.join("\n"), FieldKind::Note, self.inline)
                    == wl
            });
            let base_note = base_root.note.as_deref().unwrap_or("");
            if stale {
                self.report.push(ReportEvent::StaleEcho {
                    node: root.clone(),
                    field: ConflictField::Note,
                });
            } else if !self.force
                && self.codec.classify(base_note, FieldKind::Note, self.inline) == Tier::ReadOnly
            {
                // The preamble has no line number of its own: 0 stands for it.
                self.refuse(&root, 0);
            } else {
                // With `force`, a read-only preamble was edited in view of its `🔒`.
                let seen = self.codec.classify(base_note, FieldKind::Note, self.inline)
                    == Tier::ReadOnly;
                self.merge_note(
                    &root,
                    seen,
                    base_note,
                    remote_root.note.as_deref(),
                    &local_md,
                    false,
                    &mut push,
                );
            }
        }
        if let Some(t) = numbered_target(&base_root.layout, &base_doc.items, &local_doc.items)
            && t != base_root.layout
        {
            self.merge_layout(&root, &base_root.layout, &remote_root.layout, &t, &mut push);
        }
        self.commit_push(&root, push);
    }

    fn merge_base_fields(&mut self, i: usize, nd: &NodeId) {
        let Some(d) = self.detected[i].clone() else {
            return;
        };
        if !d.any() {
            return;
        }
        let ticked = self.items[i].item.checkbox == Checkbox::Ticked;
        let b = self.base_of(i, nd).1.clone();
        let r = self.inp.remote.get(nd).expect("existing").clone();
        let mut push = FieldPush::default();
        if let Some(t) = &d.layout {
            self.merge_layout(nd, &b.layout, &r.layout, t, &mut push);
        }
        // A `- ---` line has no name of its own: its empty name only stands for the
        // divider. If the node does not end up a divider (the remote changed the layout
        // too and won), pushing the empty name would blank the node and turn it into a
        // suspected mirror shell (§10), so the name is left alone.
        let layout_now = match &push.layout {
            Some(c) => c.new.as_str(),
            None => r.layout.as_str(),
        };
        let divider_lost = self.items[i].item.kind == ItemKind::Divider && layout_now != "divider";
        if let Some(md) = &d.name
            && !divider_lost
        {
            self.merge_name(
                nd,
                d.name_kind,
                d.name_seen,
                &b.name,
                &r.name,
                md,
                ticked,
                &mut push,
            );
        }
        if let Some(md) = &d.note {
            self.merge_note(
                nd,
                d.note_seen,
                b.note.as_deref().unwrap_or(""),
                r.note.as_deref(),
                md,
                ticked,
                &mut push,
            );
        }
        self.commit_push(nd, push);
    }

    /// Adopted nodes (an interrupted create): bring them to the values the create would
    /// have written (for a resurrection, the §5.2 merge with its base values).
    fn merge_adopted_fields(&mut self, i: usize, nd: &NodeId, k: usize) {
        let a = &self.inp.adoptions[k];
        let r = self.inp.remote.get(nd).expect("existing").clone();
        let mut push = FieldPush::default();
        let v = self.create_values(i);
        if v.name != r.name {
            self.set_name(nd, &r.name, v.name, false, &mut push);
        }
        let wl = v.note.unwrap_or_default();
        if wl != r.note.as_deref().unwrap_or("") {
            self.set_note(nd, r.note.as_deref(), wl, false, &mut push);
        }
        if a.layout != r.layout {
            let layout = a.layout.clone();
            self.set_layout(nd, &r.layout, &layout, &mut push);
        }
        self.commit_push(nd, push);
    }

    /// The parent a create goes under: the nearest local ancestor that is in `M`;
    /// existing placeholders are skipped (refused) unless `force` (§10).
    fn create_parent(&mut self, i: usize) -> NodeId {
        let line = self.items[i].item.line;
        let (parent, placeholders) = self.create_parent_walk(i, &self.m);
        for x in placeholders {
            self.refuse(&x, line);
        }
        parent.unwrap_or_else(|| self.root.clone())
    }

    /// The walk of [`Engine::create_parent`] against `tree`: the parent (`None` for the
    /// root) and the existing placeholders skipped on the way.
    fn create_parent_walk(&self, i: usize, tree: &Tree) -> (Option<NodeId>, Vec<NodeId>) {
        let mut skipped = Vec::new();
        let mut p = self.items[i].parent;
        while let Some(j) = p {
            match &self.items[j].fate {
                Fate::Create(t) => return (Some(t.clone()), skipped),
                Fate::Existing(x) => {
                    if self.is_existing_placeholder(tree, x) {
                        skipped.push(x.clone());
                    } else {
                        return (Some(x.clone()), skipped);
                    }
                }
                Fate::Gone | Fate::Dropped | Fate::Undecided => {}
            }
            p = self.items[j].parent;
        }
        (None, skipped)
    }

    fn new_layout(&self, li: &LocalItem) -> String {
        let l = match li.kind {
            ItemKind::Code => "code-block",
            ItemKind::Quote => "quote-block",
            ItemKind::Divider => "divider",
            _ => {
                let kids = non_markers(&li.children);
                if !kids.is_empty() && kids.iter().all(|c| c.ordered) {
                    "numbered"
                } else if li.checkbox == Checkbox::Open && self.inp.cfg.todo_from_checkbox {
                    "todo"
                } else {
                    DEFAULT_LAYOUT
                }
            }
        };
        l.to_owned()
    }

    fn create(&mut self, i: usize, t: NodeId) {
        let li = self.items[i].item;
        let parent = self.create_parent(i);
        let resurrects = self.items[i].resurrects.clone();
        let CreateValues {
            name,
            note,
            layout,
            lost,
        } = self.create_values(i);
        for field in lost {
            self.report.push(ReportEvent::LossyWrite {
                node: t.clone(),
                field,
            });
        }
        let mut ticked = li.checkbox == Checkbox::Ticked;
        if ticked && self.safe {
            self.suppressed += 1;
            ticked = false;
        }
        let mut node = Node::new(t.clone(), Some(parent.clone()), name.clone());
        node.note = note.clone();
        node.layout = layout.clone();
        self.m
            .place(node, Some(&parent), Position::Bottom)
            .expect("parent in M");
        if ticked {
            self.m.set_completed(&t, true, None).expect("created");
        }
        self.items[i].placed_parent = Some(parent.clone());
        self.creates.push(NewNode {
            temp: t.clone(),
            parent,
            name,
            note,
            layout,
            ticked,
            line: li.line,
            resurrects: resurrects.clone(),
        });
        if let Some(old) = resurrects {
            self.report.push(ReportEvent::Resurrected { old, new: t });
        }
    }

    /// The values a create of item `i` writes: the local text, or for a resurrection
    /// ([`Engine::resurrected_values`]) the base values with the local edits applied.
    fn create_values(&self, i: usize) -> CreateValues {
        let li = self.items[i].item;
        match &self.items[i].resurrects {
            Some(old) => self.resurrected_values(i, old),
            None => CreateValues {
                name: self.md_to_wf(&li.name_md, field_kind(li)),
                note: (!li.note_md.is_empty())
                    .then(|| self.md_to_wf(&Self::note_md(li), FieldKind::Note)),
                layout: self.new_layout(li),
                lost: Vec::new(),
            },
        }
    }

    /// §6.2 resurrection: the base values, with the local edits applied (§5.2), so the
    /// locally typed text is never lost.
    fn resurrected_values(&self, i: usize, old: &NodeId) -> CreateValues {
        let li = self.items[i].item;
        let b = self.inp.base.get(old).expect("base item in base").clone();
        let d = self.detected[i].clone().expect("base item detected");
        let kind = field_kind(li);
        let mut lost = Vec::new();
        let name = match &d.name {
            Some(md) => {
                let w = self
                    .codec
                    .write_value(&b.name, md, d.name_kind, self.inline);
                if w.lost && !d.name_seen {
                    lost.push(ConflictField::Name);
                }
                w.wf
            }
            None if self.codec.is_canonical(&b.name) => b.name.clone(),
            None => self.md_to_wf(&li.name_md, kind),
        };
        let base_note = b.note.clone().unwrap_or_default();
        let note = match &d.note {
            Some(md) => {
                let w = self
                    .codec
                    .write_value(&base_note, md, FieldKind::Note, self.inline);
                if w.lost && !d.note_seen {
                    lost.push(ConflictField::Note);
                }
                w.wf
            }
            None if self.codec.is_canonical(&base_note) => base_note,
            None => self.md_to_wf(&Self::note_md(li), FieldKind::Note),
        };
        let note = (!note.is_empty()).then_some(note);
        let layout = d.layout.clone().unwrap_or(b.layout);
        CreateValues {
            name,
            note,
            layout,
            lost,
        }
    }

    /// §6.2 local move of a base item.
    fn local_move(&mut self, i: usize, nd: &NodeId) {
        if !self.moved_locally(i) {
            return;
        }
        if self.safe {
            self.suppressed += 1;
            return;
        }
        let line = self.items[i].item.line;
        let Some(dest) = self.dest_of(i) else {
            // The destination is gone (moved out, or a dropped artefact).
            self.report.push(ReportEvent::DroppedMove {
                node: nd.clone(),
                cycle: false,
            });
            return;
        };
        let base_parent = self.base_items[nd].parent.clone();
        let remote_parent = self.inp.remote.parent(nd).cloned().expect("in the subtree");
        if remote_parent != base_parent {
            // Move vs move: remote wins.
            if remote_parent != dest {
                self.report.push(ReportEvent::DroppedMove {
                    node: nd.clone(),
                    cycle: false,
                });
            }
            return;
        }
        if self.is_existing_placeholder(&self.m, &dest) {
            self.refuse(&dest, line);
            return;
        }
        if &dest == nd || self.m.is_descendant(&dest, nd) {
            // A cycle with a remote move: remote wins (§6.2).
            self.report.push(ReportEvent::DroppedMove {
                node: nd.clone(),
                cycle: true,
            });
            return;
        }
        self.m
            .move_node(nd, Some(&dest), Position::Bottom)
            .expect("checked");
        self.moves.push(MoveIntent {
            node: nd.clone(),
            from: remote_parent,
            to: dest,
        });
    }

    /// §6.4 cross-file move into this file.
    fn cross_move(&mut self, i: usize, nd: &NodeId, other_root: &NodeId) {
        let Some(mut dest) = self.items[i].cross_dest.clone() else {
            return;
        };
        if &dest == nd || self.m.is_descendant(&dest, nd) {
            self.report.push(ReportEvent::DroppedMove {
                node: nd.clone(),
                cycle: true,
            });
            // The subtree is already detached in M: keep it reachable under the root.
            dest = self.root.clone();
        }
        self.m
            .move_node(nd, Some(&dest), Position::Bottom)
            .expect("checked");
        self.moves.push(MoveIntent {
            node: nd.clone(),
            from: self
                .inp
                .remote
                .parent(nd)
                .cloned()
                .expect("inside another root"),
            to: dest,
        });
        self.report.push(ReportEvent::MovedIn {
            node: nd.clone(),
            from_file_root: Some(other_root.clone()),
        });
    }

    /// §10: a tick of a suspected mirror shell is a write to it like any other, refused
    /// without `force` (whether completing a shell completes its origin is unknown,
    /// §13 Q8). The line is a placeholder (now or in the base), or the node looks like a
    /// shell remotely. Read-only text (§5.3 tier 3) does not stop a tick: completing
    /// does not touch the text that could not be represented.
    fn tick_refused(&self, i: usize, nd: &NodeId) -> bool {
        if self.force {
            return false;
        }
        let li = &self.items[i];
        let placeholder = |k: ItemKind| matches!(k, ItemKind::Placeholder | ItemKind::MirrorStub);
        placeholder(li.item.kind)
            || li
                .ident
                .base_node()
                .is_some_and(|b| placeholder(self.base_items[b].item.kind))
            || self.is_existing_placeholder(self.inp.remote, nd)
    }

    /// §4.3 a tick; never re-complete (DM§4.5).
    fn tick(&mut self, nd: &NodeId) {
        if self.safe {
            self.suppressed += 1;
            return;
        }
        if self.inp.remote.get(nd).is_some_and(|r| r.completed) {
            self.report
                .push(ReportEvent::AlreadyCompleted { node: nd.clone() });
            return;
        }
        self.ticks.push(nd.clone());
        if !self.outside.contains(nd) {
            self.m.set_completed(nd, true, None).expect("node in M");
        }
    }

    /// Whether a locally deleted (or pending) node must stay: deletes lose to remote
    /// edits of the node or its subtree, to added descendants, and to descendants that
    /// survive for another reason (§6.2).
    fn kept(
        &self,
        n: &NodeId,
        all: &BTreeSet<NodeId>,
        refused: &BTreeSet<NodeId>,
        memo: &mut BTreeMap<NodeId, bool>,
    ) -> bool {
        if let Some(k) = memo.get(n) {
            return *k;
        }
        let mut keep = refused.contains(n) || self.changed_self(n);
        for c in self.m.children(n).to_vec() {
            let ck = if all.contains(&c) {
                self.kept(&c, all, refused, memo)
            } else if self.base_items.contains_key(&c) || !self.base_sub.contains(&c) {
                // Visible in the file and not deleted, or a remote addition.
                true
            } else {
                // A hidden descendant: deleted with it unless it changed.
                self.changed_deep(&c)
            };
            keep |= ck;
        }
        memo.insert(n.clone(), keep);
        keep
    }

    fn apply_deletes(&mut self) {
        let claimed = self.claimed();
        let pending = self.inp.pending_deletes;
        let dset: BTreeSet<NodeId> = self
            .base_items
            .keys()
            .filter(|n| {
                !claimed.contains(*n)
                    && self.m.contains(n)
                    && !self.inp.cross.claimed_elsewhere.contains(*n)
            })
            .cloned()
            .collect();
        let mut pset = BTreeSet::new();
        for p in pending {
            if self.cancelled.contains(p) || claimed.contains(p) || dset.contains(p) {
                continue;
            }
            if self.inp.cross.claimed_elsewhere.contains(p) {
                // §9.1 guard 2: the tag showed up in another file of the run (a split
                // cut and paste); that file's merge moves the node, so the delete is off.
                self.cancelled.insert(p.clone());
                self.report
                    .push(ReportEvent::PendingDeleteCancelled { node: p.clone() });
                continue;
            }
            if self.m.contains(p) {
                pset.insert(p.clone());
            } else {
                // Left the subtree or deleted remotely: no longer pending.
                self.cancelled.insert(p.clone());
            }
        }
        let mut refused: BTreeSet<NodeId> = dset
            .iter()
            .filter(|n| !self.force && self.base_items[*n].item.kind == ItemKind::Placeholder)
            .cloned()
            .collect();
        // §15.1: an origin that still has mirrors cannot be deleted (DM§10.5), with or
        // without `force`; neither can a node holding one.
        let all: BTreeSet<NodeId> = dset.union(&pset).cloned().collect();
        let mut mirror_refused: BTreeMap<NodeId, NodeId> = BTreeMap::new();
        for n in &all {
            if let Some(origin) = self.mirror_delete_blocker(n) {
                mirror_refused.insert(n.clone(), origin);
                refused.insert(n.clone());
                // Nothing of the refused subtree goes: the user asked for one delete,
                // which cannot be done as asked, so it is left whole and reported.
                refused.extend(
                    self.m
                        .descendants(n)
                        .into_iter()
                        .filter(|d| all.contains(d)),
                );
            }
        }
        let mut memo = BTreeMap::new();
        let mut top = Vec::new();
        for n in self.m.preorder(&self.root) {
            if !all.contains(&n) {
                continue;
            }
            let k = self.kept(&n, &all, &refused, &mut memo);
            let parent = self.m.parent(&n).cloned();
            let parent_in = parent.as_ref().is_some_and(|p| all.contains(p));
            let parent_kept = parent
                .as_ref()
                .is_some_and(|p| all.contains(p) && memo.get(p).copied().unwrap_or(false));
            if k {
                if let Some(origin) = mirror_refused.get(&n) {
                    if pset.contains(&n) {
                        self.cancelled.insert(n.clone());
                    }
                    if !parent_kept || pset.contains(&n) {
                        self.report.push(ReportEvent::MirrorDeleteRefused {
                            node: n.clone(),
                            origin: origin.clone(),
                        });
                    }
                } else if pset.contains(&n) {
                    self.cancelled.insert(n.clone());
                    self.report
                        .push(ReportEvent::DeleteLost { node: n.clone() });
                } else if !parent_kept {
                    self.report.push(if refused.contains(&n) {
                        ReportEvent::PlaceholderDeleteRefused { node: n.clone() }
                    } else {
                        ReportEvent::DeleteLost { node: n.clone() }
                    });
                }
            } else if !parent_in || parent_kept {
                top.push(n);
            }
        }
        if self.safe {
            self.suppressed += top.iter().filter(|n| dset.contains(*n)).count();
            return;
        }
        for n in top {
            // What the delete removes (§15.1: not a content's children shown at a shell);
            // the view subtree leaves `M` whole, since that is what the file stops showing.
            let desc = self.deleted_descendants(&n);
            let vis = desc
                .iter()
                .filter(|d| self.base_items.contains_key(*d))
                .count();
            let hidden = desc.len() - vis;
            let label = self.codec.plain_text(&self.m.get(&n).expect("in M").name);
            if dset.contains(&n) {
                self.stats.deleted_visible += 1 + vis;
                self.stats.deleted_hidden += hidden;
            }
            self.deleted.insert(n.clone());
            self.deleted.extend(desc);
            self.m.remove_subtree(&n).expect("in M");
            self.deletes.push(DeleteIntent {
                node: n,
                visible_descendants: vis,
                hidden_descendants: hidden,
                label,
            });
        }
    }

    // ----- phase 6: orders, reports, result ---------------------------------------------

    fn finish(mut self) -> MergeResult {
        let root = self.root.clone();
        let remote = self.inp.remote;
        let hidden: BTreeSet<NodeId> = self
            .inp
            .pending_deletes
            .iter()
            .filter(|p| !self.cancelled.contains(*p) && !self.deleted.contains(*p))
            .cloned()
            .collect();
        let m_vis = visible_nodes(&self.m, &root, &self.inp.cfg.doc, &hidden);

        // Local children lists, in file order, by where items end up.
        let mut local_children: BTreeMap<NodeId, Vec<NodeId>> = BTreeMap::new();
        local_children.insert(root.clone(), Vec::new());
        for i in 0..self.items.len() {
            let Some(id) = self.items[i].fate.id().cloned() else {
                continue;
            };
            local_children.entry(id.clone()).or_default();
            let parent = match &self.items[i].placed_parent {
                Some(p) => p.clone(),
                None => {
                    let mut p = self.items[i].parent;
                    loop {
                        match p {
                            None => break root.clone(),
                            Some(j) => match self.items[j].fate.id() {
                                Some(x) => break x.clone(),
                                None => p = self.items[j].parent,
                            },
                        }
                    }
                }
            };
            local_children.entry(parent).or_default().push(id);
        }

        // §6.3 sibling orders over visible nodes. An adopted node is a local create that
        // an interrupted apply placed at the bottom (§7.1 step 1) and may not have
        // reordered yet: it takes its local position, not its remote one.
        let adopted: BTreeSet<NodeId> = self
            .items
            .iter()
            .filter_map(|li| match &li.ident {
                Ident::Adopted(n, _) => Some(n.clone()),
                _ => None,
            })
            .collect();
        let mut orders = BTreeMap::new();
        for p in &m_vis {
            let kids: Vec<NodeId> = self.m.children(p).to_vec();
            let keep: BTreeSet<NodeId> = kids
                .iter()
                .filter(|c| m_vis.contains(*c))
                .cloned()
                .collect();
            if keep.is_empty() {
                continue;
            }
            let empty = Vec::new();
            let base_order = if p == &root || self.base_items.contains_key(p) {
                self.base_children.get(p).unwrap_or(&empty)
            } else {
                &empty
            };
            let remote_order: Vec<NodeId> = remote
                .children(p)
                .iter()
                .filter(|c| self.remote_vis.contains(*c) && !adopted.contains(*c))
                .cloned()
                .collect();
            let local_order = local_children.get(p).map(Vec::as_slice);
            if self.safe
                && let Some(local) = local_order
            {
                // Reorders suppressed by safe mode (§9.1 guard 1), for the report.
                let lcs = lcs_members(base_order, local);
                let in_base: BTreeSet<&NodeId> = base_order.iter().collect();
                self.suppressed += local
                    .iter()
                    .filter(|x| in_base.contains(x) && !lcs.contains(*x))
                    .count();
            }
            let target = merge_sibling_order(
                base_order,
                local_order,
                &remote_order,
                &keep,
                &kids,
                !self.safe,
            );
            // Hidden siblings are never moved (§6.3, §8.1), so their mutual order is
            // kept exactly; each follows the visible sibling that preceded it as soon as
            // that one is placed, which is where top/bottom moves of the visible nodes
            // leave it.
            let mut hidden_queue: std::collections::VecDeque<(Option<NodeId>, NodeId)> =
                std::collections::VecDeque::new();
            let mut anchor: Option<NodeId> = None;
            for c in &kids {
                if keep.contains(c) {
                    anchor = Some(c.clone());
                } else {
                    hidden_queue.push_back((anchor.clone(), c.clone()));
                }
            }
            let mut emitted: BTreeSet<NodeId> = BTreeSet::new();
            let mut full: Vec<NodeId> = Vec::with_capacity(kids.len());
            let mut flush = |full: &mut Vec<NodeId>, emitted: &BTreeSet<NodeId>| {
                while let Some((a, _)) = hidden_queue.front() {
                    if a.as_ref().is_some_and(|a| !emitted.contains(a)) {
                        break;
                    }
                    full.push(hidden_queue.pop_front().expect("front").1);
                }
            };
            flush(&mut full, &emitted);
            for v in &target {
                full.push(v.clone());
                emitted.insert(v.clone());
                flush(&mut full, &emitted);
            }
            self.m.reorder_children(p, &full).expect("a permutation");
            orders.insert(p.clone(), target);
        }

        self.report_remote_changes();
        self.report_vanished(&m_vis);
        self.carry_markers(&m_vis);
        if self.safe {
            self.report.push(ReportEvent::SafeMode {
                suppressed: self.suppressed,
            });
        }

        let mut base_all = Vec::new();
        walk_doc(&self.inp.base_doc.items, &mut base_all);
        let mut stats = self.stats;
        stats.base_visible = base_all.iter().filter(|i| !i.is_marker()).count();
        stats.local_items = self.items.len();
        stats.ticked = self.ticks.len() + self.creates.iter().filter(|c| c.ticked).count();
        stats.created = self.creates.len();
        stats.pushed_fields = self.pushes.values().map(FieldPush::count).sum();
        stats.conflicts = self.new_conflicts;

        MergeResult {
            root,
            merged: self.m,
            pushes: self.pushes,
            creates: self.creates,
            moves: self.moves,
            ticks: self.ticks,
            deletes: self.deletes,
            orders,
            conflicts: self.conflicts.into_values().collect(),
            report: self.report,
            stats,
            cancelled_pending: self.cancelled,
            outside: self.outside,
        }
    }

    /// MovedOut, Disappeared and Appeared (§4.5, §6.2), top-most nodes only.
    fn report_remote_changes(&mut self) {
        let remote = self.inp.remote;
        let moved_out: BTreeSet<&NodeId> = self
            .base_order
            .iter()
            .filter(|n| remote.contains(n) && !self.remote_sub.contains(*n))
            .collect();
        let mut events = Vec::new();
        for n in &self.base_order {
            let parent = remote.parent(n);
            if moved_out.contains(n) {
                if !parent.is_some_and(|p| moved_out.contains(p)) {
                    events.push(ReportEvent::MovedOut {
                        node: n.clone(),
                        new_parent: parent.cloned(),
                    });
                }
            } else if self.remote_sub.contains(n)
                && !self.remote_vis.contains(n)
                && !self.inp.pending_deletes.contains(n)
                && parent.is_some_and(|p| self.remote_vis.contains(p))
            {
                events.push(ReportEvent::Disappeared { node: n.clone() });
            }
        }
        let appeared: BTreeSet<&NodeId> = self
            .remote_vis
            .iter()
            .filter(|n| {
                *n != &self.root
                    && self.base_sub.contains(*n)
                    && !self.base_items.contains_key(*n)
                    && !self.cancelled.contains(*n)
            })
            .collect();
        for n in remote.preorder(&self.root) {
            if appeared.contains(&n) && !remote.parent(&n).is_some_and(|p| appeared.contains(p)) {
                events.push(ReportEvent::Appeared { node: n });
            }
        }
        self.report.extend(events);
    }

    /// §6.2 table: local work pushed under a node completed remotely.
    fn report_vanished(&mut self, m_vis: &BTreeSet<NodeId>) {
        let mut worked: Vec<NodeId> = Vec::new();
        worked.extend(self.pushes.keys().cloned());
        worked.extend(self.creates.iter().map(|c| c.temp.clone()));
        worked.extend(self.moves.iter().map(|mv| mv.node.clone()));
        let mut seen = BTreeSet::new();
        for n in worked {
            if !seen.insert(n.clone()) || !self.m.contains(&n) || m_vis.contains(&n) {
                continue;
            }
            let mut path = vec![n.clone()];
            path.extend(self.m.ancestors(&n));
            let cause = path
                .iter()
                .take_while(|x| *x != &self.root)
                .filter(|x| {
                    self.m.get(x).is_some_and(|y| y.completed)
                        && self.inp.remote.get(x).is_some_and(|y| y.completed)
                })
                .last()
                .cloned();
            if let Some(c) = cause {
                self.report.push(ReportEvent::VanishedWork {
                    node: n,
                    completed_ancestor: c,
                });
            }
        }
    }

    /// Conflict markers in the file are carried forward while their node is visible.
    fn carry_markers(&mut self, m_vis: &BTreeSet<NodeId>) {
        for (parent, marker) in self.markers.clone() {
            let ItemKind::ConflictMarker(field) = marker.kind else {
                continue;
            };
            let local_md = match field {
                ConflictField::Name => marker.name_md.clone(),
                ConflictField::Note => Self::note_md(marker),
            };
            let (target, fallback) = match parent {
                None => (Some(self.root.clone()), Some(self.root.clone())),
                Some(j) => (
                    self.items[j].fate.id().cloned(),
                    self.items[j].ident.node().cloned(),
                ),
            };
            match target {
                Some(id) if m_vis.contains(&id) => {
                    self.conflicts
                        .entry((id.clone(), field))
                        .or_insert(ConflictMarker {
                            node: id,
                            field,
                            local_md,
                        });
                }
                other => {
                    if let Some(node) = other.or(fallback) {
                        self.report.push(ReportEvent::ConflictLost { node, field });
                    }
                }
            }
        }
    }
}
