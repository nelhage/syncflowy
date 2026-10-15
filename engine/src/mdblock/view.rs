//! [`View::build`]: the visible projection of a tree (§4.5) with kinds (§4.3), checkboxes
//! (§4.3, §12.4), tiers (§5.3), placeholders (§10), mirror stubs (§15.1) and conflict
//! markers (§6.1).

use std::collections::{BTreeMap, BTreeSet};

use super::{
    Checkbox, CheckboxPolicy, ConflictMarker, DocConfig, ItemKind, View, ViewError, ViewExtras,
    ViewItem,
};
use crate::mdinline::{ConflictField, FieldKind, InlineConfig, MdText, Tier, md_to_wf, wf_to_md};
use crate::tree::{Node, NodeId, ShortId, Tree};
use crate::wfhtml::to_plain_text;

/// Layout values with a meaning for the file (§4.3, UI:G2).
const LAYOUT_TODO: &str = "todo";
const LAYOUT_CODE: &str = "code-block";
const LAYOUT_QUOTE: &str = "quote-block";
const LAYOUT_NUMBERED: &str = "numbered";
const LAYOUT_DIVIDER: &str = "divider";

/// `wf_to_md`, with text that is rendered without an `ro` flag but does not round-trip
/// (tier `Lossy`, or a `ReadOnly` root note, which cannot carry a flag) replaced by its
/// own normal form `wf_to_md(md_to_wf(text))`. That form means the same as the text
/// (convergence, mdinline EC 3) and is a fixed point, so a re-render of the parsed file
/// gives the same bytes (§5.4). Edits to such a value still go through
/// `mdinline::write_value` (§5.2), which keeps what the rendering could not show.
pub(super) fn md_text(wf: &str, kind: FieldKind, cfg: &InlineConfig, keep_ro: bool) -> MdText {
    let md = wf_to_md(wf, kind, cfg);
    match md.tier {
        Tier::Exact | Tier::Passthrough => md,
        Tier::ReadOnly if keep_ro => md,
        Tier::Lossy | Tier::ReadOnly => MdText {
            text: wf_to_md(&md_to_wf(&md.text, kind, cfg), kind, cfg).text,
            tier: md.tier,
        },
    }
}

struct Builder<'a> {
    tree: &'a Tree,
    cfg: &'a DocConfig,
    extras: &'a ViewExtras,
    conflicts: BTreeMap<NodeId, Vec<ConflictMarker>>,
    seen: BTreeSet<NodeId>,
    /// §15.1: content node -> the placement rendered in full (the link target of stubs).
    full: BTreeMap<NodeId, NodeId>,
}

/// The kind of a node: a divider, a §10 placeholder (an orphan shell), a §15.1 stub, or
/// by layout.
pub(crate) fn kind_of(node: &Node) -> ItemKind {
    // A divider has an empty name by design (UI:G2 sets its note to "").
    if node.layout == LAYOUT_DIVIDER && node.name.is_empty() {
        return ItemKind::Divider;
    }
    if node.stub {
        return ItemKind::MirrorStub;
    }
    // An orphan shell has no content to show (DM§10.4): a read-only placeholder (§10),
    // children never rendered.
    if node.is_orphan_shell() {
        return ItemKind::Placeholder;
    }
    match node.layout.as_str() {
        LAYOUT_CODE => ItemKind::Code,
        LAYOUT_QUOTE => ItemKind::Quote,
        _ => ItemKind::Bullet,
    }
}

impl Builder<'_> {
    fn hidden(&self, node: &Node, depth: u32) -> bool {
        node.completed
            || self.extras.hidden.contains(&node.id)
            || self.cfg.max_depth.is_some_and(|m| depth > m)
    }

    fn children(&mut self, parent: &Node, depth: u32) -> Vec<ViewItem> {
        let numbered = parent.layout == LAYOUT_NUMBERED;
        let mut out = Vec::new();
        for id in self.tree.children(&parent.id) {
            let Some(node) = self.tree.get(id) else {
                continue;
            };
            if self.hidden(node, depth) || !self.seen.insert(id.clone()) {
                continue;
            }
            out.push(self.item(node, depth, numbered));
        }
        out
    }

    /// §15.1: the text of a stub placement: a link to the full placement of the same
    /// content, labelled with the content's plain-text name. Without a full placement in
    /// the tree (a stub outside any view, which should not be rendered) the link names
    /// the content itself.
    fn stub_text(&self, node: &Node) -> String {
        let content = node.content_id();
        let target = self.full.get(content).unwrap_or(content);
        let short = target
            .short()
            .unwrap_or_else(|| ShortId::parse("000000000000").expect("hex"));
        super::stub_text(&short, &to_plain_text(&node.name))
    }

    fn item(&mut self, node: &Node, depth: u32, parent_numbered: bool) -> ViewItem {
        let inline = &self.cfg.inline;
        let kind = kind_of(node);
        let name_kind = if kind == ItemKind::Code {
            FieldKind::Code
        } else {
            FieldKind::Name
        };
        let (name, note) = if kind == ItemKind::MirrorStub {
            (
                MdText {
                    text: self.stub_text(node),
                    tier: Tier::Exact,
                },
                None,
            )
        } else {
            (
                md_text(&node.name, name_kind, inline, true),
                node.note
                    .as_deref()
                    .filter(|n| !n.is_empty())
                    .map(|n| md_text(n, FieldKind::Note, inline, true)),
            )
        };
        let read_only = matches!(kind, ItemKind::Placeholder | ItemKind::MirrorStub)
            || name.tier == Tier::ReadOnly
            || note.as_ref().is_some_and(|n| n.tier == Tier::ReadOnly);
        let heading = (depth <= u32::from(self.cfg.headings)).then_some(depth as u8);
        let children = if matches!(kind, ItemKind::Placeholder | ItemKind::MirrorStub) {
            Vec::new()
        } else {
            self.children(node, depth + 1)
        };
        // §4.5: a node whose children are cut off by `depth:` is not a leaf, although it
        // shows none (deleting its line deletes them).
        let cut_off = self.cfg.max_depth.is_some_and(|m| depth >= m)
            && self.tree.children(&node.id).iter().any(|c| {
                self.tree
                    .get(c)
                    .is_some_and(|n| !n.completed && !self.extras.hidden.contains(c))
            });
        let leaf = children.is_empty() && !cut_off;
        let checkbox = self.checkbox(node, kind, heading.is_some(), leaf);
        let conflicts = self.conflicts.remove(&node.id).unwrap_or_default();
        ViewItem {
            id: node.id.clone(),
            tag: node.id.short(),
            kind,
            heading,
            checkbox,
            ordered: parent_numbered && heading.is_none(),
            name,
            note,
            read_only,
            conflicts,
            children,
        }
    }

    /// §4.3: which incomplete items show `[ ]` (cosmetic).
    fn checkbox(&self, node: &Node, kind: ItemKind, heading: bool, leaf: bool) -> Checkbox {
        // UNVERIFIED (our choice): quotes get no box either (`- [ ] > x` reads badly).
        if heading || kind != ItemKind::Bullet {
            return Checkbox::None;
        }
        // `checkboxes: none` means no boxes at all; a to-do keeps its layout (checkbox
        // forms never change layouts) and is still ticked by typing `[x]`.
        if node.layout == LAYOUT_TODO && self.cfg.checkboxes != CheckboxPolicy::None {
            return Checkbox::Open;
        }
        if let Some(&c) = self.extras.sticky.checkbox.get(&node.id) {
            return match c {
                Checkbox::None => Checkbox::None,
                Checkbox::Open | Checkbox::Ticked => Checkbox::Open,
            };
        }
        let open = match self.cfg.checkboxes {
            CheckboxPolicy::TodosAndLeaves => leaf,
            CheckboxPolicy::TodosOnly | CheckboxPolicy::None => false,
            CheckboxPolicy::All => true,
        };
        if open { Checkbox::Open } else { Checkbox::None }
    }
}

fn sort_markers(v: &mut [ConflictMarker]) {
    v.sort_by_key(|m| match m.field {
        ConflictField::Name => 0,
        ConflictField::Note => 1,
    });
}

/// See [`View::build`].
pub(super) fn build(
    tree: &Tree,
    root: &NodeId,
    cfg: &DocConfig,
    extras: &ViewExtras,
) -> Result<View, ViewError> {
    let root_node = tree
        .get(root)
        .ok_or_else(|| ViewError::RootMissing(root.clone()))?;
    let mut conflicts: BTreeMap<NodeId, Vec<ConflictMarker>> = BTreeMap::new();
    for c in &extras.conflicts {
        conflicts.entry(c.node.clone()).or_default().push(c.clone());
    }
    for v in conflicts.values_mut() {
        sort_markers(v);
    }
    let mut root_conflicts = conflicts.remove(root).unwrap_or_default();
    sort_markers(&mut root_conflicts);
    let full = tree
        .iter()
        .filter(|n| !n.stub && !n.is_orphan_shell())
        .map(|n| (n.content_id().clone(), n.id.clone()))
        .collect();
    let mut b = Builder {
        tree,
        cfg,
        extras,
        conflicts,
        seen: BTreeSet::from([root.clone()]),
        full,
    };
    let items = b.children(root_node, 1);
    // Markers of nodes that are not in the view are dropped from the file (merge reports
    // them), in input order.
    let dropped_conflicts = extras
        .conflicts
        .iter()
        .filter(|c| b.conflicts.contains_key(&c.node))
        .cloned()
        .collect();
    let root_note = root_node
        .note
        .as_deref()
        .filter(|n| !n.is_empty())
        .map(|n| md_text(n, FieldKind::Note, &cfg.inline, false));
    Ok(View {
        root: root.clone(),
        root_completed: root_node.completed,
        root_note,
        root_conflicts,
        items,
        dropped_conflicts,
    })
}
