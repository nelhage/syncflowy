//! Shared helpers for the merge tests (included with `#[path]` by `merge_fixtures.rs`
//! and `merge_props.rs`; also compiled on its own, where it only checks itself).
//!
//! * Node ids are written as short labels (`a`, `b1`, `root`): [`id`] maps a label of at
//!   most 6 ASCII alphanumerics to a full UUID whose last 12 hex digits are the label's
//!   bytes, and [`label`] maps back, so results can be shown with labels.
//! * Trees are nested JSON [`NodeSpec`]s; documents are [`DocSpec`]s whose items are
//!   built into `LocalDoc` values directly (no Markdown parsing).
//! * [`render_doc`] is a minimal stand-in for `mdblock`'s renderer, for the plain-text
//!   codec: it projects a tree onto the visible items exactly as the base doc would be.
//! * [`summarize`] turns a `MergeResult` into labelled JSON for fixture comparison.

#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use syncflowy::apply::Adoption;
use syncflowy::mdblock::{
    Checkbox, DocConfig, Frontmatter, ItemKind, LocalDoc, LocalItem, PLACEHOLDER_TEXT, RootRef,
};
use syncflowy::mdinline::{ConflictField, FieldKind, IdTag, InlineConfig, TagFlag, TagForm, Tier};
use syncflowy::merge::{
    CrossFile, MergeConfig, MergeError, MergeInput, MergeResult, PlainTextCodec, TextCodec,
    merge_with, visible_nodes,
};
use syncflowy::tree::{IdMap, Node, NodeId, ShortId, Tree};

const UUID_PREFIX: &str = "00000000-0000-4000-8000-";

/// The full id of a label (`temp-<n>` gives a temp id).
pub fn id(label: &str) -> NodeId {
    if let Some(n) = label.strip_prefix("temp-") {
        return NodeId::temp(n.parse().expect("temp index"));
    }
    assert!(
        !label.is_empty() && label.len() <= 6 && label.bytes().all(|b| b.is_ascii_alphanumeric()),
        "bad label {label:?}"
    );
    let hex: String = label.bytes().map(|b| format!("{b:02x}")).collect();
    NodeId::parse(&format!("{UUID_PREFIX}{hex:0>12}")).expect("uuid")
}

/// The short id of a label.
pub fn short(label: &str) -> ShortId {
    id(label).short().expect("short")
}

/// The label of a full id or a short id made by [`id`].
pub fn label(s: &str) -> Option<String> {
    let hex = match s.strip_prefix(UUID_PREFIX) {
        Some(h) => h,
        None if s.len() == 12 && s.bytes().all(|b| b.is_ascii_hexdigit()) => s,
        None => return None,
    };
    if hex.len() != 12 {
        return None;
    }
    let bytes: Option<Vec<u8>> = (0..6)
        .map(|i| u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).ok())
        .collect();
    let bytes: Vec<u8> = bytes?.into_iter().skip_while(|b| *b == 0).collect();
    if bytes.is_empty() || !bytes.iter().all(u8::is_ascii_alphanumeric) {
        return None;
    }
    String::from_utf8(bytes).ok()
}

/// The label of a node id (or the id itself).
pub fn lbl(id: &NodeId) -> String {
    label(id.as_str()).unwrap_or_else(|| id.to_string())
}

// ---------------------------------------------------------------------------------------
// Trees
// ---------------------------------------------------------------------------------------

/// A node and its subtree. Priorities default to 100, 200, … in list order.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NodeSpec {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub note: Option<String>,
    #[serde(default)]
    pub layout: Option<String>,
    #[serde(default)]
    pub completed: bool,
    #[serde(default)]
    pub priority: Option<i64>,
    #[serde(default)]
    pub children: Vec<NodeSpec>,
}

fn collect_nodes(specs: &[NodeSpec], parent: Option<&NodeId>, out: &mut Vec<Node>) {
    for (i, s) in specs.iter().enumerate() {
        let nid = id(&s.id);
        let mut n = Node::new(nid.clone(), parent.cloned(), s.name.clone());
        n.note = s.note.clone();
        if let Some(l) = &s.layout {
            n.layout = l.clone();
        }
        n.completed = s.completed;
        n.completed_at = s.completed.then_some(1);
        n.priority = s.priority.unwrap_or(100 * (i as i64 + 1));
        out.push(n);
        collect_nodes(&s.children, Some(&nid), out);
    }
}

/// Build a tree from top-level specs.
pub fn build_tree(specs: &[NodeSpec]) -> Tree {
    let mut nodes = Vec::new();
    collect_nodes(specs, None, &mut nodes);
    Tree::from_nodes(nodes).expect("valid tree spec")
}

// ---------------------------------------------------------------------------------------
// Documents
// ---------------------------------------------------------------------------------------

/// A parsed Markdown document, written directly.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DocSpec {
    /// The root note lines.
    #[serde(default)]
    pub note: Vec<String>,
    #[serde(default)]
    pub items: Vec<ItemSpec>,
}

/// One item. `tag` is a label; `kind` is `code`, `quote`, `divider`, `placeholder`,
/// `marker-name` or `marker-note`; `check` is `" "` or `"x"`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ItemSpec {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extra: Vec<String>,
    #[serde(default)]
    pub name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub note: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub check: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub ordered: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub ro: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<ItemSpec>,
}

fn tag_of(label: &str, ro: bool) -> IdTag {
    let mut flags = BTreeSet::new();
    if ro {
        flags.insert(TagFlag::ReadOnly);
    }
    IdTag {
        short: short(label),
        flags,
        form: TagForm::BlockId,
    }
}

fn build_items(specs: &[ItemSpec], line: &mut usize) -> Vec<LocalItem> {
    specs
        .iter()
        .map(|s| {
            let kind = match s.kind.as_deref() {
                None | Some("bullet") => ItemKind::Bullet,
                Some("code") => ItemKind::Code,
                Some("quote") => ItemKind::Quote,
                Some("divider") => ItemKind::Divider,
                Some("placeholder") => ItemKind::Placeholder,
                Some("marker-name") => ItemKind::ConflictMarker(ConflictField::Name),
                Some("marker-note") => ItemKind::ConflictMarker(ConflictField::Note),
                Some(k) => panic!("unknown kind {k}"),
            };
            let checkbox = match s.check.as_deref() {
                None => Checkbox::None,
                Some(" ") => Checkbox::Open,
                Some("x") => Checkbox::Ticked,
                Some(c) => panic!("unknown checkbox {c:?}"),
            };
            let first = *line;
            *line += 1 + s.note.len();
            let end_line = *line - 1;
            let children = build_items(&s.children, line);
            LocalItem {
                tag: s.tag.as_deref().map(|t| tag_of(t, s.ro)),
                extra_tags: s.extra.iter().map(|t| tag_of(t, false)).collect(),
                kind,
                heading: None,
                checkbox,
                ordered: s.ordered,
                name_md: s.name.clone(),
                note_md: s.note.clone(),
                children,
                line: first,
                end_line,
            }
        })
        .collect()
}

/// Build a `LocalDoc` for `root` (line numbers count items and note lines from 1).
pub fn build_doc(root: &NodeId, spec: &DocSpec) -> LocalDoc {
    let short = root.short().expect("root short id");
    let mut fm = Frontmatter::new(RootRef {
        raw: short.to_string(),
        short,
        full: None,
    });
    fm.generation = Some(1);
    let mut line = 1 + spec.note.len();
    LocalDoc {
        frontmatter: fm,
        root_note: spec.note.clone(),
        items: build_items(&spec.items, &mut line),
        warnings: Vec::new(),
    }
}

/// The doc spec of a tree as the renderer would show it with the plain-text codec
/// (visible nodes only, §4.5; placeholders and read-only values flagged `ro`).
pub fn render_spec(
    tree: &Tree,
    root: &NodeId,
    cfg: &DocConfig,
    hidden: &BTreeSet<NodeId>,
) -> DocSpec {
    let vis = visible_nodes(tree, root, cfg, hidden);
    let codec = PlainTextCodec;
    let inline = InlineConfig::default();
    fn lines(s: &Option<String>) -> Vec<String> {
        match s.as_deref() {
            None | Some("") => Vec::new(),
            Some(s) => PlainTextCodec
                .plain_text(s)
                .split('\n')
                .map(str::to_owned)
                .collect(),
        }
    }
    fn items(
        tree: &Tree,
        parent: &NodeId,
        vis: &BTreeSet<NodeId>,
        codec: &PlainTextCodec,
        inline: &InlineConfig,
    ) -> Vec<ItemSpec> {
        let ordered = tree.get(parent).is_some_and(|p| p.layout == "numbered");
        tree.children(parent)
            .iter()
            .filter(|c| vis.contains(*c))
            .map(|c| {
                let n = tree.get(c).expect("node");
                let placeholder = n.looks_like_mirror_shell() && n.layout != "divider";
                let (kind, fk) = match n.layout.as_str() {
                    _ if placeholder => (Some("placeholder".to_owned()), FieldKind::Name),
                    "code-block" => (Some("code".to_owned()), FieldKind::Code),
                    "quote-block" => (Some("quote".to_owned()), FieldKind::Name),
                    "divider" if n.name.is_empty() => (Some("divider".to_owned()), FieldKind::Name),
                    _ => (None, FieldKind::Name),
                };
                let ro = placeholder
                    || codec.classify(&n.name, fk, inline) == Tier::ReadOnly
                    || codec.classify(n.note.as_deref().unwrap_or(""), FieldKind::Note, inline)
                        == Tier::ReadOnly;
                let name = if placeholder {
                    PLACEHOLDER_TEXT.to_owned()
                } else {
                    codec.plain_text(&n.name).replace('\n', " ⏎ ")
                };
                ItemSpec {
                    tag: Some(lbl(c)),
                    name,
                    note: lines(&n.note),
                    kind,
                    ordered,
                    ro,
                    children: items(tree, c, vis, codec, inline),
                    ..ItemSpec::default()
                }
            })
            .collect()
    }
    DocSpec {
        note: tree.get(root).map(|r| lines(&r.note)).unwrap_or_default(),
        items: items(tree, root, &vis, &codec, &inline),
    }
}

// ---------------------------------------------------------------------------------------
// Running a case
// ---------------------------------------------------------------------------------------

/// Everything one merge reads, owned.
#[derive(Debug, Clone)]
pub struct Case {
    pub root: NodeId,
    pub base: Tree,
    pub remote: Tree,
    pub base_doc: LocalDoc,
    pub local_doc: LocalDoc,
    pub ids: IdMap,
    pub cfg: MergeConfig,
    pub cross: CrossFile,
    pub pending: BTreeSet<NodeId>,
    pub adoptions: Vec<Adoption>,
    pub prior_docs: Vec<LocalDoc>,
}

impl Case {
    /// A case with defaults: ids from both trees, empty cross/pending/adoptions/priors.
    pub fn new(
        root: NodeId,
        base: Tree,
        remote: Tree,
        base_doc: LocalDoc,
        local_doc: LocalDoc,
    ) -> Case {
        let mut ids = IdMap::from_tree(&remote);
        ids.extend_from_tree(&base);
        Case {
            root,
            base,
            remote,
            base_doc,
            local_doc,
            ids,
            cfg: MergeConfig::default(),
            cross: CrossFile::default(),
            pending: BTreeSet::new(),
            adoptions: Vec::new(),
            prior_docs: Vec::new(),
        }
    }

    pub fn input(&self) -> MergeInput<'_> {
        MergeInput {
            root: &self.root,
            base: &self.base,
            remote: &self.remote,
            base_doc: &self.base_doc,
            local_doc: &self.local_doc,
            ids: &self.ids,
            cfg: &self.cfg,
            cross: &self.cross,
            pending_deletes: &self.pending,
            adoptions: &self.adoptions,
            prior_docs: &self.prior_docs,
        }
    }

    pub fn run(&self) -> Result<MergeResult, MergeError> {
        merge_with(&self.input(), &PlainTextCodec)
    }
}

// ---------------------------------------------------------------------------------------
// Summaries
// ---------------------------------------------------------------------------------------

/// Replace every id string made by [`id`] (full or short) by its label.
pub fn relabel(v: Value) -> Value {
    match v {
        Value::String(s) => Value::String(label(&s).unwrap_or(s)),
        Value::Array(a) => Value::Array(a.into_iter().map(relabel).collect()),
        Value::Object(o) => Value::Object(
            o.into_iter()
                .map(|(k, v)| (label(&k).unwrap_or(k), relabel(v)))
                .collect(),
        ),
        v => v,
    }
}

/// The merged tree as indented lines: `label "name" note=… layout=… [done]`.
pub fn outline(tree: &Tree, root: &NodeId) -> Vec<String> {
    let mut out = Vec::new();
    fn walk(tree: &Tree, id: &NodeId, depth: usize, out: &mut Vec<String>) {
        let n = tree.get(id).expect("node");
        let mut s = format!("{}{} {:?}", "  ".repeat(depth), lbl(id), n.name);
        if let Some(note) = &n.note {
            s.push_str(&format!(" note={note:?}"));
        }
        if n.layout != "bullets" {
            s.push_str(&format!(" layout={}", n.layout));
        }
        if n.completed {
            s.push_str(" [done]");
        }
        out.push(s);
        for c in tree.children(id) {
            walk(tree, c, depth + 1, out);
        }
    }
    if tree.contains(root) {
        walk(tree, root, 0, &mut out);
    }
    out
}

/// A labelled JSON summary of a merge result, for fixtures.
pub fn summarize(r: &MergeResult) -> Value {
    let pushes: BTreeMap<String, Value> = r
        .pushes
        .iter()
        .map(|(k, p)| {
            let mut o = serde_json::Map::new();
            if let Some(c) = &p.name {
                o.insert("name".into(), json!([c.expected, c.new]));
            }
            if let Some(c) = &p.note {
                o.insert("note".into(), json!([c.expected, c.new]));
            }
            if let Some(c) = &p.layout {
                o.insert("layout".into(), json!([c.expected, c.new]));
            }
            (lbl(k), Value::Object(o))
        })
        .collect();
    let v = json!({
        "pushes": pushes,
        "creates": r.creates.iter().map(|c| json!({
            "temp": c.temp, "parent": c.parent, "name": c.name, "note": c.note,
            "layout": c.layout, "ticked": c.ticked, "line": c.line, "resurrects": c.resurrects,
        })).collect::<Vec<_>>(),
        "moves": r.moves.iter().map(|m| json!([m.node, m.from, m.to])).collect::<Vec<_>>(),
        "ticks": r.ticks,
        "deletes": r.deletes.iter().map(|d| json!({
            "node": d.node, "visible": d.visible_descendants, "hidden": d.hidden_descendants,
            "label": d.label,
        })).collect::<Vec<_>>(),
        "orders": r.orders,
        "conflicts": r.conflicts.iter().map(|c| json!([c.node, c.field, c.local_md])).collect::<Vec<_>>(),
        "report": r.report,
        "cancelled_pending": r.cancelled_pending,
        "stats": r.stats,
        "merged": outline(&r.merged, &r.root),
    });
    relabel(v)
}

#[test]
fn labels_round_trip() {
    for l in ["a", "root", "b12", "zzzzzz"] {
        assert_eq!(label(id(l).as_str()).as_deref(), Some(l));
        assert_eq!(label(short(l).as_str()).as_deref(), Some(l));
    }
    assert_eq!(label("temp-1"), None);
}
