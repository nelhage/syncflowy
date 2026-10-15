//! The outline model: node ids, nodes, and an indexed tree of them.
//!
//! Responsibility (design §3 "tree", §4.5 visibility, §6.3 sibling order, §10 mirror
//! shells): hold a snapshot of (part of) a Workflowy account as plain data, answer
//! structural questions about it, and offer the pure structural edits that the merge,
//! the planner's simulation and the applier's "latest known remote tree" need. Nothing
//! here performs I/O.
//!
//! # Ids
//!
//! * [`NodeId`] is the full id string as the API returns it (a lowercase hyphenated
//!   UUID for every API-created node; legacy accounts also hold non-RFC-variant ids,
//!   DM§1.1). A [`NodeId`] can also be a **temporary id** (`temp-<n>`, see
//!   [`NodeId::temp`]) that the merge allocates for a node typed in Markdown that does
//!   not exist in Workflowy yet. Temporary ids never reach the API: the client refuses
//!   every id that is not a full UUID for mutations (DM§3.1), and `apply` maps temporary
//!   ids to the real ids returned by create.
//! * [`ShortId`] is the last 12 hex characters of the dash-less UUID (DM§1.1). It is
//!   what the Markdown file carries in its id tags (§4.4). [`IdMap`] maps short ids
//!   back to full ids.
//!
//! # Order (§6.3)
//!
//! Siblings are ordered by `(priority, id)`. Priorities are only meaningful within one
//! snapshot: they are rewritten by API moves and are not comparable across snapshots, so
//! everything above this module compares **sequences of ids**, never priorities. Equal
//! priorities can only come from UI races; their display order is unknown (DM§7), so
//! [`Tree::priority_ties`] lets callers detect them and refuse to write because of one.
//!
//! # Visibility (§4.5)
//!
//! A node is *visible from a sync root* if it is the root itself (always rendered, even
//! when completed), or if it is a descendant of the root and neither it nor any ancestor
//! strictly below the root is completed. This module implements only the completion
//! rule; `mdblock::View` adds the other hiding rules (mirror placeholders hide their
//! children, §10; an optional depth limit; pending deletes, §9.1 guard 2).
//!
//! # Structural edits
//!
//! [`Tree::place`] and [`Tree::move_node`] reproduce the API's priority algorithm
//! (DM§7) exactly (including the renumber when the minimum priority reaches 0), so a
//! simulated plan produces the same sibling order as the server.
//! [`Tree::reorder_children`] rewrites priorities to realise an explicit order and is
//! meant for merged (not remote) trees.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------------------
// Ids
// ---------------------------------------------------------------------------------------

const TEMP_PREFIX: &str = "temp-";

/// Errors from parsing ids.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum IdError {
    /// Not a lowercase hyphenated UUID (`8-4-4-4-12` lowercase hex digits).
    #[error("not a full lowercase UUID: {0:?}")]
    NotUuid(String),
    /// Not 12 hex digits.
    #[error("not a 12-hex-digit short id: {0:?}")]
    NotShortId(String),
}

/// A full node id, or a temporary id for a node that does not exist remotely yet.
///
/// Ordering is plain string ordering; it is the tie-breaker of sibling order (§6.3).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct NodeId(String);

impl NodeId {
    /// Parse a full id typed by a person or read from configuration: it must be a
    /// lowercase hyphenated UUID (DM§3.1: mutations accept nothing else). Uppercase input
    /// is rejected rather than lowercased, because the API treats an uppercase UUID as a
    /// different (missing) id.
    pub fn parse(s: &str) -> Result<NodeId, IdError> {
        if is_uuid_str(s) {
            Ok(NodeId(s.to_owned()))
        } else {
            Err(IdError::NotUuid(s.to_owned()))
        }
    }

    /// Wrap an id received from the API (export, GET, list, create). The server is
    /// trusted: no validation is done, because legacy ids need not be RFC UUIDs.
    pub fn from_api(s: impl Into<String>) -> NodeId {
        NodeId(s.into())
    }

    /// The `n`-th temporary id (`temp-<n>`), used by `merge` for nodes created in
    /// Markdown. Temporary ids sort after every UUID (`t` > hex digits).
    pub fn temp(n: u32) -> NodeId {
        NodeId(format!("{TEMP_PREFIX}{n}"))
    }

    /// True for ids made by [`NodeId::temp`].
    pub fn is_temp(&self) -> bool {
        self.temp_index().is_some()
    }

    /// `Some(n)` for `temp-<n>`.
    pub fn temp_index(&self) -> Option<u32> {
        self.0.strip_prefix(TEMP_PREFIX)?.parse().ok()
    }

    /// True if this is a lowercase hyphenated UUID, i.e. acceptable to the API's
    /// mutating endpoints (DM§3.1).
    pub fn is_uuid(&self) -> bool {
        is_uuid_str(&self.0)
    }

    /// The id as a string.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The short id (last 12 hex digits of the dash-less id, DM§1.1). `None` for
    /// temporary ids and for ids with fewer than 12 hex digits.
    pub fn short(&self) -> Option<ShortId> {
        if self.is_temp() {
            return None;
        }
        let hex: String = self.0.chars().filter(|c| *c != '-').collect();
        if hex.len() < 12 {
            return None;
        }
        ShortId::parse(&hex[hex.len() - 12..]).ok()
    }
}

impl fmt::Display for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for NodeId {
    type Err = IdError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        NodeId::parse(s)
    }
}

fn is_uuid_str(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 36
        && b.iter().enumerate().all(|(i, &c)| match i {
            8 | 13 | 18 | 23 => c == b'-',
            _ => c.is_ascii_digit() || (b'a'..=b'f').contains(&c),
        })
}

/// A 12-hex-digit short id (DM§1.1), always stored lowercase.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ShortId(String);

impl ShortId {
    /// Parse 12 hex digits, case-insensitively (GET accepts any case, DM§3.1).
    pub fn parse(s: &str) -> Result<ShortId, IdError> {
        if s.len() == 12 && s.bytes().all(|c| c.is_ascii_hexdigit()) {
            Ok(ShortId(s.to_ascii_lowercase()))
        } else {
            Err(IdError::NotShortId(s.to_owned()))
        }
    }

    /// The short id of `id` (see [`NodeId::short`]).
    pub fn of(id: &NodeId) -> Option<ShortId> {
        id.short()
    }

    /// The 12 lowercase hex digits.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ShortId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for ShortId {
    type Err = IdError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        ShortId::parse(s)
    }
}

impl TryFrom<String> for ShortId {
    type Error = IdError;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        ShortId::parse(&s)
    }
}

impl From<ShortId> for String {
    fn from(s: ShortId) -> String {
        s.0
    }
}

/// Short id -> full id, built from every node the engine knows (the base tree and the
/// latest export). A short id shared by two different full ids is **ambiguous**: it is
/// remembered as such and never resolved (a 48-bit collision is unlikely but legacy ids
/// are not random).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdMap {
    map: BTreeMap<ShortId, NodeId>,
    ambiguous: BTreeSet<ShortId>,
}

impl IdMap {
    /// An empty map.
    pub fn new() -> IdMap {
        IdMap::default()
    }

    /// The map of every node in `tree`.
    pub fn from_tree(tree: &Tree) -> IdMap {
        let mut m = IdMap::new();
        m.extend_from_tree(tree);
        m
    }

    /// Add every node of `tree`.
    pub fn extend_from_tree(&mut self, tree: &Tree) {
        for id in tree.ids() {
            self.insert(id);
        }
    }

    /// Add one id. Temporary ids and ids without a short form are ignored. Adding the
    /// same full id twice is a no-op; a second, different full id makes the short id
    /// ambiguous.
    pub fn insert(&mut self, id: &NodeId) {
        let Some(short) = id.short() else { return };
        if self.ambiguous.contains(&short) {
            return;
        }
        match self.map.get(&short) {
            Some(existing) if existing != id => {
                self.map.remove(&short);
                self.ambiguous.insert(short);
            }
            Some(_) => {}
            None => {
                self.map.insert(short, id.clone());
            }
        }
    }

    /// The full id for `short`, unless unknown or ambiguous.
    pub fn get(&self, short: &ShortId) -> Option<&NodeId> {
        self.map.get(short)
    }

    /// True if two different full ids share `short`.
    pub fn is_ambiguous(&self, short: &ShortId) -> bool {
        self.ambiguous.contains(short)
    }

    /// Number of resolvable short ids.
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// True if nothing is resolvable.
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// All resolvable pairs, in short-id order.
    pub fn iter(&self) -> impl Iterator<Item = (&ShortId, &NodeId)> {
        self.map.iter()
    }
}

// ---------------------------------------------------------------------------------------
// Nodes
// ---------------------------------------------------------------------------------------

/// Placement for create and move: the API only offers the two ends (DM§7, §8.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Position {
    /// First among the siblings.
    Top,
    /// Last among the siblings.
    Bottom,
}

impl Position {
    /// `"top"` or `"bottom"`, as the API spells it.
    pub fn as_str(self) -> &'static str {
        match self {
            Position::Top => "top",
            Position::Bottom => "bottom",
        }
    }
}

/// The layout every API-created node gets by default (DM§4.3).
pub const DEFAULT_LAYOUT: &str = "bullets";

/// One node, with the API's fields under Rust names. `name` and `note` are raw WF-HTML
/// strings exactly as the API returned them (never unescaped here).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Node {
    /// Full id (or a temporary id in merged trees).
    pub id: NodeId,
    /// Parent id; `None` for top-level nodes (DM§11 root sentinel).
    pub parent_id: Option<NodeId>,
    /// WF-HTML name; may be `""`.
    pub name: String,
    /// WF-HTML note with raw `\n`; `None` and `Some("")` are distinct (DM§6.4).
    pub note: Option<String>,
    /// `data.layoutMode`: any string (`bullets`, `todo`, `code-block`, …).
    pub layout: String,
    /// Completion flag. Completion never cascades (DM§4.5).
    pub completed: bool,
    /// `completedAt`, unix seconds.
    pub completed_at: Option<i64>,
    /// `createdAt`, unix seconds.
    pub created_at: i64,
    /// `modifiedAt`, unix seconds; `None` on some old nodes (DM§1.1).
    pub modified_at: Option<i64>,
    /// Sparse sort key among siblings (DM§7). Only meaningful within one snapshot.
    pub priority: i64,
    /// The beta API's mirror metadata (`data.mirror`, DM§10.5): `None` on plain nodes.
    /// Absent from checkpoints written before mirrors were modelled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mirror: Option<Mirror>,
    /// Only in a **view tree** (`crate::mirrors::view_tree`, §15.1): this placement of
    /// its content node is rendered as a stub, because the content's full rendering
    /// (text and children) sits at another placement. Always `false` in an export.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub stub: bool,
}

/// A node's part in a mirror (DM§10.5, beta API only).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mirror {
    /// A mirror root ("shell"): the API reports its origin's content in its place.
    /// `origin` is `None` for an orphan (the origin was deleted): the API then reports
    /// the shell's own empty row and refuses writes through it.
    Shell {
        /// The origin, a non-mirror node.
        origin: Option<NodeId>,
    },
    /// A mirror origin whose registry (`data.mirror.mirror_ids`) is non-empty. The
    /// registry is what blocks a delete of the node (DM§10.5): it can name deleted
    /// shells ("ghosts") and miss live ones, so the live shells are found from the
    /// shells' `origin` instead ([`Tree::shells_of`]).
    Origin {
        /// `mirror_ids`, sorted by id as the API returns them.
        mirror_ids: Vec<NodeId>,
    },
}

impl Node {
    /// A fresh incomplete node with layout [`DEFAULT_LAYOUT`], `note: None`, priority 0
    /// and zero timestamps; callers set what they need.
    pub fn new(id: NodeId, parent_id: Option<NodeId>, name: impl Into<String>) -> Node {
        Node {
            id,
            parent_id,
            name: name.into(),
            note: None,
            layout: DEFAULT_LAYOUT.to_owned(),
            completed: false,
            completed_at: None,
            created_at: 0,
            modified_at: None,
            priority: 0,
            mirror: None,
            stub: false,
        }
    }

    /// True for a mirror root with an accessible origin (DM§10.5).
    pub fn is_shell(&self) -> bool {
        matches!(self.mirror, Some(Mirror::Shell { origin: Some(_) }))
    }

    /// True for a mirror root whose origin is gone (DM§10.4, §10.5 "orphans").
    pub fn is_orphan_shell(&self) -> bool {
        matches!(self.mirror, Some(Mirror::Shell { origin: None }))
    }

    /// The origin of a shell, `None` otherwise (orphans included).
    pub fn origin(&self) -> Option<&NodeId> {
        match &self.mirror {
            Some(Mirror::Shell { origin }) => origin.as_ref(),
            _ => None,
        }
    }

    /// The **content node** this placement shows (§15.1): a shell's origin, else the
    /// node itself.
    pub fn content_id(&self) -> &NodeId {
        self.origin().unwrap_or(&self.id)
    }

    /// True if the node's mirror registry is non-empty: the beta API refuses to delete
    /// it, or any ancestor of it (DM§10.5).
    pub fn has_registered_mirrors(&self) -> bool {
        matches!(&self.mirror, Some(Mirror::Origin { mirror_ids }) if !mirror_ids.is_empty())
    }
}

/// The API's node record with its exact JSON field names and key order (DM§1.2):
/// `id, name, note, parent_id, priority, completed, data.layoutMode, createdAt,
/// modifiedAt, completedAt`. Deserialisation is lenient about missing optional fields
/// and ignores unknown ones.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiNode {
    /// Node id.
    pub id: String,
    /// WF-HTML name.
    pub name: String,
    /// WF-HTML note or null.
    #[serde(default)]
    pub note: Option<String>,
    /// Parent id or null (top level).
    #[serde(default)]
    pub parent_id: Option<String>,
    /// Sort key.
    #[serde(default)]
    pub priority: i64,
    /// Completion flag.
    #[serde(default)]
    pub completed: bool,
    /// `{"layoutMode": …}`.
    #[serde(default)]
    pub data: ApiNodeData,
    /// Creation time, unix seconds.
    #[serde(rename = "createdAt", default)]
    pub created_at: i64,
    /// Modification time, unix seconds, or null.
    #[serde(rename = "modifiedAt", default)]
    pub modified_at: Option<i64>,
    /// Completion time, unix seconds, or null.
    #[serde(rename = "completedAt", default)]
    pub completed_at: Option<i64>,
}

/// The API's `data` object: `layoutMode` (DM§1.2) and, on the beta API, `mirror`
/// (DM§10.5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiNodeData {
    /// The layout mode string.
    #[serde(rename = "layoutMode", default = "default_layout")]
    pub layout_mode: String,
    /// `{"origin_id": …}` on a shell (null for an orphan), `{"mirror_ids": […]}` on an
    /// origin with a non-empty registry; absent otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mirror: Option<ApiMirror>,
}

/// `data.mirror` as the beta API serialises it (DM§10.5). Exactly one key is present.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiMirror {
    /// A shell's origin; serialised as `null` for an orphan (`Some(None)`). Not present
    /// on origins (`None`).
    #[serde(
        default,
        deserialize_with = "nullable_string",
        skip_serializing_if = "Option::is_none"
    )]
    pub origin_id: Option<Option<String>>,
    /// An origin's registry. Not present on shells.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mirror_ids: Option<Vec<String>>,
}

/// `null` reads as `Some(None)` (serde would otherwise fold it into an absent key).
fn nullable_string<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<Option<Option<String>>, D::Error> {
    Option::<String>::deserialize(d).map(Some)
}

impl ApiMirror {
    fn to_mirror(&self) -> Option<Mirror> {
        if let Some(origin) = &self.origin_id {
            return Some(Mirror::Shell {
                origin: origin.clone().map(NodeId::from_api),
            });
        }
        let ids = self.mirror_ids.as_ref()?;
        Some(Mirror::Origin {
            mirror_ids: ids.iter().cloned().map(NodeId::from_api).collect(),
        })
    }

    fn from_mirror(m: &Mirror) -> ApiMirror {
        match m {
            Mirror::Shell { origin } => ApiMirror {
                origin_id: Some(origin.as_ref().map(|o| o.as_str().to_owned())),
                mirror_ids: None,
            },
            Mirror::Origin { mirror_ids } => ApiMirror {
                origin_id: None,
                mirror_ids: Some(mirror_ids.iter().map(|i| i.as_str().to_owned()).collect()),
            },
        }
    }
}

fn default_layout() -> String {
    DEFAULT_LAYOUT.to_owned()
}

impl Default for ApiNodeData {
    fn default() -> Self {
        ApiNodeData {
            layout_mode: default_layout(),
            mirror: None,
        }
    }
}

impl From<ApiNode> for Node {
    fn from(a: ApiNode) -> Node {
        Node {
            id: NodeId::from_api(a.id),
            parent_id: a.parent_id.map(NodeId::from_api),
            name: a.name,
            note: a.note,
            layout: a.data.layout_mode,
            completed: a.completed,
            completed_at: a.completed_at,
            created_at: a.created_at,
            modified_at: a.modified_at,
            priority: a.priority,
            mirror: a.data.mirror.as_ref().and_then(ApiMirror::to_mirror),
            stub: false,
        }
    }
}

impl From<&Node> for ApiNode {
    fn from(n: &Node) -> ApiNode {
        ApiNode {
            id: n.id.as_str().to_owned(),
            name: n.name.clone(),
            note: n.note.clone(),
            parent_id: n.parent_id.as_ref().map(|p| p.as_str().to_owned()),
            priority: n.priority,
            completed: n.completed,
            data: ApiNodeData {
                layout_mode: n.layout.clone(),
                mirror: n.mirror.as_ref().map(ApiMirror::from_mirror),
            },
            created_at: n.created_at,
            modified_at: n.modified_at,
            completed_at: n.completed_at,
        }
    }
}

// ---------------------------------------------------------------------------------------
// Tree
// ---------------------------------------------------------------------------------------

/// Errors from tree construction and structural edits.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TreeError {
    /// Two nodes share an id.
    #[error("duplicate node id {0}")]
    DuplicateId(NodeId),
    /// The node is not in the tree.
    #[error("unknown node {0}")]
    UnknownNode(NodeId),
    /// The requested parent is not in the tree.
    #[error("unknown parent {0}")]
    UnknownParent(NodeId),
    /// A move would put `node` under itself or one of its descendants (§8.2; the API
    /// would return 200 and detach the subtree, DM§4.7.5).
    #[error("moving {node} under {dest} would create a cycle")]
    Cycle {
        /// The node being moved.
        node: NodeId,
        /// The requested destination parent.
        dest: NodeId,
    },
    /// `reorder_children` got something other than a permutation of the children.
    #[error("not a permutation of the children of {0}")]
    NotAPermutation(NodeId),
}

/// An indexed snapshot of nodes: a flat id -> node map plus a children index whose
/// sibling lists are sorted by `(priority, id)` (§6.3).
///
/// A tree may be a whole export or a projection ([`Tree::subtree`]); a node whose
/// `parent_id` is `None` or names a node absent from the tree is a *top* node of the
/// tree ([`Tree::tops`]). Exports can contain parent cycles (DM§4.7.5); every walk
/// upwards is cycle-safe and every walk downwards visits each node at most once.
///
/// Serialises (for checkpoints) as `{"nodes": [Node, …]}` in id order; the index is
/// rebuilt on load.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(into = "TreeRepr", try_from = "TreeRepr")]
pub struct Tree {
    nodes: BTreeMap<NodeId, Node>,
    /// parent_id (as stored, `None` = top level) -> children sorted by (priority, id).
    children: BTreeMap<Option<NodeId>, Vec<NodeId>>,
}

#[derive(Serialize, Deserialize)]
struct TreeRepr {
    nodes: Vec<Node>,
}

impl From<Tree> for TreeRepr {
    fn from(t: Tree) -> TreeRepr {
        TreeRepr {
            nodes: t.nodes.into_values().collect(),
        }
    }
}

impl TryFrom<TreeRepr> for Tree {
    type Error = TreeError;
    fn try_from(r: TreeRepr) -> Result<Tree, TreeError> {
        Tree::from_nodes(r.nodes)
    }
}

const NO_CHILDREN: &[NodeId] = &[];

impl Tree {
    /// An empty tree.
    pub fn new() -> Tree {
        Tree::default()
    }

    /// Build from nodes (for example an export's node list, in any order). Parents may
    /// be absent (projections); duplicates are an error.
    pub fn from_nodes(nodes: impl IntoIterator<Item = Node>) -> Result<Tree, TreeError> {
        let mut t = Tree::new();
        for n in nodes {
            if t.nodes.contains_key(&n.id) {
                return Err(TreeError::DuplicateId(n.id));
            }
            t.children
                .entry(n.parent_id.clone())
                .or_default()
                .push(n.id.clone());
            t.nodes.insert(n.id.clone(), n);
        }
        let keys: Vec<Option<NodeId>> = t.children.keys().cloned().collect();
        for k in keys {
            t.sort_siblings(&k);
        }
        Ok(t)
    }

    /// Build from the API's records (`GET /nodes-export`'s `nodes`).
    pub fn from_api(nodes: impl IntoIterator<Item = ApiNode>) -> Result<Tree, TreeError> {
        Tree::from_nodes(nodes.into_iter().map(Node::from))
    }

    /// The API records of every node, in id order.
    pub fn to_api(&self) -> Vec<ApiNode> {
        self.nodes.values().map(ApiNode::from).collect()
    }

    /// The projection on `root` and all its descendants (the root keeps its
    /// `parent_id`, so it is the projection's only top node). `None` if `root` is absent.
    pub fn subtree(&self, root: &NodeId) -> Option<Tree> {
        let root_node = self.nodes.get(root)?;
        let mut nodes = vec![root_node.clone()];
        nodes.extend(
            self.descendants(root)
                .into_iter()
                .map(|id| self.nodes[&id].clone()),
        );
        // Cannot fail: ids are unique in self.
        Tree::from_nodes(nodes).ok()
    }

    // ----- queries -----------------------------------------------------------------

    /// Number of nodes.
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// True if there are no nodes.
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// The node with id `id`.
    pub fn get(&self, id: &NodeId) -> Option<&Node> {
        self.nodes.get(id)
    }

    /// True if `id` is in the tree.
    pub fn contains(&self, id: &NodeId) -> bool {
        self.nodes.contains_key(id)
    }

    /// All nodes, in id order.
    pub fn iter(&self) -> impl Iterator<Item = &Node> {
        self.nodes.values()
    }

    /// All ids, in id order.
    pub fn ids(&self) -> impl Iterator<Item = &NodeId> {
        self.nodes.keys()
    }

    /// True if any node carries mirror metadata (DM§10.5). False for an account without
    /// mirrors, whose view tree is then the export itself (`mirrors::view_tree`). The
    /// engine targets the beta API only, so a node without metadata is never a shell.
    pub fn has_mirror_metadata(&self) -> bool {
        self.nodes.values().any(|n| n.mirror.is_some())
    }

    /// The live shells of every origin, from the shells' `origin` (never from the
    /// registry, DM§10.5): origin -> shells in id order.
    pub fn shells_by_origin(&self) -> BTreeMap<NodeId, Vec<NodeId>> {
        let mut out: BTreeMap<NodeId, Vec<NodeId>> = BTreeMap::new();
        for n in self.nodes.values() {
            if let Some(o) = n.origin() {
                out.entry(o.clone()).or_default().push(n.id.clone());
            }
        }
        out
    }

    /// The live shells of `origin` (see [`Tree::shells_by_origin`]).
    pub fn shells_of(&self, origin: &NodeId) -> Vec<NodeId> {
        self.nodes
            .values()
            .filter(|n| n.origin() == Some(origin))
            .map(|n| n.id.clone())
            .collect()
    }

    /// Descendants of `id` in pre-order, not descending below nodes for which `descend`
    /// is false (those nodes are still listed).
    pub fn descendants_where(&self, id: &NodeId, descend: impl Fn(&Node) -> bool) -> Vec<NodeId> {
        let mut out = Vec::new();
        let mut seen = BTreeSet::new();
        seen.insert(id.clone());
        self.preorder_into(id, &mut seen, &mut out, &descend);
        out
    }

    /// The stored `parent_id` of `id` (which may name a node outside the tree), or
    /// `None` for top-level or unknown nodes.
    pub fn parent(&self, id: &NodeId) -> Option<&NodeId> {
        self.nodes.get(id)?.parent_id.as_ref()
    }

    /// The children of `id`, sorted by `(priority, id)`. Empty for leaves and unknown ids.
    pub fn children(&self, id: &NodeId) -> &[NodeId] {
        self.children
            .get(&Some(id.clone()))
            .map(Vec::as_slice)
            .unwrap_or(NO_CHILDREN)
    }

    /// The children of `parent` where `None` means the account's top level.
    pub fn children_of(&self, parent: Option<&NodeId>) -> &[NodeId] {
        match parent {
            Some(p) => self.children(p),
            None => self
                .children
                .get(&None)
                .map(Vec::as_slice)
                .unwrap_or(NO_CHILDREN),
        }
    }

    /// Top nodes of this tree: nodes whose parent is `None` or absent from the tree,
    /// sorted by `(priority, id)` within each parent and then by parent.
    pub fn tops(&self) -> Vec<NodeId> {
        let mut out = Vec::new();
        for (parent, kids) in &self.children {
            let is_top = match parent {
                None => true,
                Some(p) => !self.nodes.contains_key(p),
            };
            if is_top {
                out.extend(kids.iter().cloned());
            }
        }
        out
    }

    /// Ancestors of `id` that are in the tree, nearest first (parent, grandparent, …).
    /// Stops at the first parent not in the tree and at a repeated node (cycles).
    pub fn ancestors(&self, id: &NodeId) -> Vec<NodeId> {
        let mut out = Vec::new();
        let mut seen = BTreeSet::new();
        seen.insert(id.clone());
        let mut cur = self.parent(id);
        while let Some(p) = cur {
            if !self.nodes.contains_key(p) || !seen.insert(p.clone()) {
                break;
            }
            out.push(p.clone());
            cur = self.parent(p);
        }
        out
    }

    /// Descendants of `id` in pre-order (children in sibling order), excluding `id`.
    pub fn descendants(&self, id: &NodeId) -> Vec<NodeId> {
        let mut out = Vec::new();
        let mut seen = BTreeSet::new();
        seen.insert(id.clone());
        self.preorder_into(id, &mut seen, &mut out, &|_| true);
        out
    }

    fn preorder_into(
        &self,
        id: &NodeId,
        seen: &mut BTreeSet<NodeId>,
        out: &mut Vec<NodeId>,
        descend: &dyn Fn(&Node) -> bool,
    ) {
        for c in self.children(id) {
            if !seen.insert(c.clone()) {
                continue;
            }
            out.push(c.clone());
            if descend(&self.nodes[c]) {
                self.preorder_into(c, seen, out, descend);
            }
        }
    }

    /// `id` followed by its descendants, in pre-order.
    pub fn preorder(&self, id: &NodeId) -> Vec<NodeId> {
        if !self.contains(id) {
            return Vec::new();
        }
        let mut out = vec![id.clone()];
        out.extend(self.descendants(id));
        out
    }

    /// True if `node` is strictly below `ancestor`.
    pub fn is_descendant(&self, node: &NodeId, ancestor: &NodeId) -> bool {
        node != ancestor && self.ancestors(node).iter().any(|a| a == ancestor)
    }

    /// Depth of `node` below `root`: 0 for the root, 1 for its children, … `None` if
    /// `node` is not `root` or a descendant of it.
    pub fn depth_below(&self, node: &NodeId, root: &NodeId) -> Option<usize> {
        if node == root {
            return self.contains(node).then_some(0);
        }
        self.ancestors(node)
            .iter()
            .position(|a| a == root)
            .map(|i| i + 1)
    }

    /// Groups (of two or more) of children of `parent` (`None` = top level) that share a
    /// priority, in sibling order. Their relative display order in the UI is unknown
    /// (§6.3, DM§7).
    pub fn priority_ties(&self, parent: Option<&NodeId>) -> Vec<Vec<NodeId>> {
        let mut out: Vec<Vec<NodeId>> = Vec::new();
        let mut prev: Option<i64> = None;
        for id in self.children_of(parent) {
            let p = self.nodes[id].priority;
            if prev == Some(p) {
                let last = out.last_mut().expect("group exists");
                last.push(id.clone());
            } else {
                out.push(vec![id.clone()]);
            }
            prev = Some(p);
        }
        out.retain(|g| g.len() > 1);
        out
    }

    /// True if two children of `parent` share a priority.
    pub fn has_priority_ties(&self, parent: Option<&NodeId>) -> bool {
        self.children_of(parent)
            .windows(2)
            .any(|w| self.nodes[&w[0]].priority == self.nodes[&w[1]].priority)
    }

    // ----- visibility (§4.5) ---------------------------------------------------------

    /// True if `id` is visible from the sync root `root` by the completion rule: `root`
    /// itself always is; a descendant is visible if neither it nor any ancestor strictly
    /// below `root` is completed. Nodes outside `root`'s subtree are not visible.
    pub fn is_visible(&self, id: &NodeId, root: &NodeId) -> bool {
        if id == root {
            return self.contains(id);
        }
        let Some(node) = self.nodes.get(id) else {
            return false;
        };
        if node.completed {
            return false;
        }
        for a in self.ancestors(id) {
            if &a == root {
                return true;
            }
            if self.nodes[&a].completed {
                return false;
            }
        }
        false
    }

    /// The visible descendants of `root` (excluding `root`), in pre-order.
    pub fn visible_descendants(&self, root: &NodeId) -> Vec<NodeId> {
        let mut out = Vec::new();
        let mut seen = BTreeSet::new();
        seen.insert(root.clone());
        self.visible_into(root, &mut seen, &mut out);
        out
    }

    fn visible_into(&self, id: &NodeId, seen: &mut BTreeSet<NodeId>, out: &mut Vec<NodeId>) {
        for c in self.children(id) {
            if self.nodes[c].completed || !seen.insert(c.clone()) {
                continue;
            }
            out.push(c.clone());
            self.visible_into(c, seen, out);
        }
    }

    /// The incomplete children of `id`, in sibling order. If `id` is visible from some
    /// root, these are exactly its visible children.
    pub fn visible_children(&self, id: &NodeId) -> Vec<NodeId> {
        self.children(id)
            .iter()
            .filter(|c| !self.nodes[*c].completed)
            .cloned()
            .collect()
    }

    /// Number of descendants of `id` that are hidden *relative to `id`*: completed, or
    /// below a completed node (excluding `id` itself). This is what a delete of `id`
    /// takes with it that the Markdown user cannot see (§4.5, §8.3).
    pub fn count_hidden_descendants(&self, id: &NodeId) -> usize {
        let all = self.descendants(id).len();
        let visible = self.visible_descendants(id).len();
        all - visible
    }

    // ----- structural edits ------------------------------------------------------------

    /// Insert `node` with its own `parent_id` and `priority`. The parent need not be in
    /// the tree (projections). Errors if the id exists.
    pub fn insert(&mut self, node: Node) -> Result<(), TreeError> {
        if self.nodes.contains_key(&node.id) {
            return Err(TreeError::DuplicateId(node.id));
        }
        let key = node.parent_id.clone();
        self.children
            .entry(key.clone())
            .or_default()
            .push(node.id.clone());
        self.nodes.insert(node.id.clone(), node);
        self.sort_siblings(&key);
        Ok(())
    }

    /// Insert `node` under `parent` (`None` = top level) at `pos`, assigning its
    /// priority with the API's algorithm (DM§7), which may renumber the siblings. The
    /// node's `parent_id` and `priority` fields are overwritten. The parent must be in
    /// the tree (or `None`).
    pub fn place(
        &mut self,
        mut node: Node,
        parent: Option<&NodeId>,
        pos: Position,
    ) -> Result<(), TreeError> {
        if self.nodes.contains_key(&node.id) {
            return Err(TreeError::DuplicateId(node.id));
        }
        if let Some(p) = parent
            && !self.nodes.contains_key(p)
        {
            return Err(TreeError::UnknownParent(p.clone()));
        }
        node.priority = self.assign_priority(parent, pos, None);
        node.parent_id = parent.cloned();
        self.insert(node)
    }

    /// Move `id` under `new_parent` (`None` = top level) at `pos`, with the API's
    /// priority algorithm (DM§7: also for same-parent moves, excluding the node itself).
    /// Refuses a move under the node itself or one of its descendants (§8.2) with
    /// [`TreeError::Cycle`].
    pub fn move_node(
        &mut self,
        id: &NodeId,
        new_parent: Option<&NodeId>,
        pos: Position,
    ) -> Result<(), TreeError> {
        if !self.nodes.contains_key(id) {
            return Err(TreeError::UnknownNode(id.clone()));
        }
        if let Some(p) = new_parent {
            if !self.nodes.contains_key(p) {
                return Err(TreeError::UnknownParent(p.clone()));
            }
            if p == id || self.is_descendant(p, id) {
                return Err(TreeError::Cycle {
                    node: id.clone(),
                    dest: p.clone(),
                });
            }
        }
        let priority = self.assign_priority(new_parent, pos, Some(id));
        self.detach(id);
        let node = self.nodes.get_mut(id).expect("checked");
        node.parent_id = new_parent.cloned();
        node.priority = priority;
        let key = new_parent.cloned();
        self.children
            .entry(key.clone())
            .or_default()
            .push(id.clone());
        self.sort_siblings(&key);
        Ok(())
    }

    /// Re-parent every child of `from` under `to`, keeping each child's priority (a
    /// projection, not an API move: `crate::mirrors::view_tree` hangs an origin's
    /// children under the placement that renders them, §15.1). Both must be in the tree.
    pub fn adopt_children(&mut self, from: &NodeId, to: &NodeId) -> Result<(), TreeError> {
        if !self.nodes.contains_key(from) {
            return Err(TreeError::UnknownNode(from.clone()));
        }
        if !self.nodes.contains_key(to) {
            return Err(TreeError::UnknownParent(to.clone()));
        }
        if from == to {
            return Ok(());
        }
        let kids = self
            .children
            .remove(&Some(from.clone()))
            .unwrap_or_default();
        for k in &kids {
            self.nodes.get_mut(k).expect("child").parent_id = Some(to.clone());
        }
        let key = Some(to.clone());
        self.children.entry(key.clone()).or_default().extend(kids);
        self.sort_siblings(&key);
        Ok(())
    }

    /// Remove `id` and all its descendants; returns the removed nodes in pre-order.
    pub fn remove_subtree(&mut self, id: &NodeId) -> Result<Vec<Node>, TreeError> {
        if !self.nodes.contains_key(id) {
            return Err(TreeError::UnknownNode(id.clone()));
        }
        let order = self.preorder(id);
        self.detach(id);
        let mut out = Vec::with_capacity(order.len());
        for n in &order {
            self.children.remove(&Some(n.clone()));
            out.push(self.nodes.remove(n).expect("present"));
        }
        Ok(out)
    }

    /// Set the WF-HTML name.
    pub fn set_name(&mut self, id: &NodeId, name: impl Into<String>) -> Result<(), TreeError> {
        self.node_mut(id)?.name = name.into();
        Ok(())
    }

    /// Set the WF-HTML note.
    pub fn set_note(&mut self, id: &NodeId, note: Option<String>) -> Result<(), TreeError> {
        self.node_mut(id)?.note = note;
        Ok(())
    }

    /// Set the layout mode.
    pub fn set_layout(&mut self, id: &NodeId, layout: impl Into<String>) -> Result<(), TreeError> {
        self.node_mut(id)?.layout = layout.into();
        Ok(())
    }

    /// Set completion and `completedAt`.
    pub fn set_completed(
        &mut self,
        id: &NodeId,
        completed: bool,
        completed_at: Option<i64>,
    ) -> Result<(), TreeError> {
        let n = self.node_mut(id)?;
        n.completed = completed;
        n.completed_at = completed_at;
        Ok(())
    }

    /// Set the view-tree `stub` flag (`crate::mirrors::view_tree`).
    pub fn set_stub(&mut self, id: &NodeId, stub: bool) -> Result<(), TreeError> {
        self.node_mut(id)?.stub = stub;
        Ok(())
    }

    /// Set `modifiedAt`.
    pub fn set_modified_at(
        &mut self,
        id: &NodeId,
        modified_at: Option<i64>,
    ) -> Result<(), TreeError> {
        self.node_mut(id)?.modified_at = modified_at;
        Ok(())
    }

    /// Set the raw priority and re-sort the siblings.
    pub fn set_priority(&mut self, id: &NodeId, priority: i64) -> Result<(), TreeError> {
        let n = self.node_mut(id)?;
        n.priority = priority;
        let key = n.parent_id.clone();
        self.sort_siblings(&key);
        Ok(())
    }

    /// Replace node ids everywhere (node ids and parent ids) according to `map`, for
    /// example temporary ids -> the real ids returned by create. Ids absent from `map`
    /// are kept. Errors if the result has duplicate ids.
    pub fn remap_ids(&mut self, map: &BTreeMap<NodeId, NodeId>) -> Result<(), TreeError> {
        let get = |id: &NodeId| map.get(id).cloned().unwrap_or_else(|| id.clone());
        let nodes: Vec<Node> = std::mem::take(&mut self.nodes)
            .into_values()
            .map(|mut n| {
                n.id = get(&n.id);
                n.parent_id = n.parent_id.as_ref().map(get);
                n
            })
            .collect();
        *self = Tree::from_nodes(nodes)?;
        Ok(())
    }

    /// Make the children of `parent` appear in exactly `order` (a permutation of the
    /// current children) by rewriting their priorities to 100, 200, … This is for
    /// merged trees, whose priorities carry no remote meaning; it is not an API
    /// operation (the planner turns order differences into top/bottom moves, §8.1).
    pub fn reorder_children(&mut self, parent: &NodeId, order: &[NodeId]) -> Result<(), TreeError> {
        let current: BTreeSet<&NodeId> = self.children(parent).iter().collect();
        let wanted: BTreeSet<&NodeId> = order.iter().collect();
        if current != wanted || wanted.len() != order.len() {
            return Err(TreeError::NotAPermutation(parent.clone()));
        }
        for (i, id) in order.iter().enumerate() {
            self.nodes.get_mut(id).expect("child exists").priority = 100 * (i as i64 + 1);
        }
        self.children.insert(Some(parent.clone()), order.to_vec());
        Ok(())
    }

    // ----- internals -------------------------------------------------------------------

    fn node_mut(&mut self, id: &NodeId) -> Result<&mut Node, TreeError> {
        self.nodes
            .get_mut(id)
            .ok_or_else(|| TreeError::UnknownNode(id.clone()))
    }

    fn detach(&mut self, id: &NodeId) {
        let key = self.nodes[id].parent_id.clone();
        if let Some(kids) = self.children.get_mut(&key) {
            kids.retain(|c| c != id);
            if kids.is_empty() {
                self.children.remove(&key);
            }
        }
    }

    fn sort_siblings(&mut self, key: &Option<NodeId>) {
        let nodes = &self.nodes;
        if let Some(kids) = self.children.get_mut(key) {
            kids.sort_by(|a, b| (nodes[a].priority, a).cmp(&(nodes[b].priority, b)));
        }
    }

    /// DM§7 `assign_priority`, renumbering siblings when a top insert finds min <= 0.
    fn assign_priority(
        &mut self,
        parent: Option<&NodeId>,
        pos: Position,
        exclude: Option<&NodeId>,
    ) -> i64 {
        let others: Vec<NodeId> = self
            .children_of(parent)
            .iter()
            .filter(|c| Some(*c) != exclude)
            .cloned()
            .collect();
        if others.is_empty() {
            return 100;
        }
        let prios = others.iter().map(|c| self.nodes[c].priority);
        match pos {
            Position::Bottom => {
                let max = prios.max().expect("non-empty");
                // ceil((max + 100) / 100) * 100
                -((-(max + 100)).div_euclid(100)) * 100
            }
            Position::Top => {
                let min = prios.min().expect("non-empty");
                if min > 0 {
                    return min / 2;
                }
                // Renumber 200, 300, … in (priority, created_at, id) order; the new node
                // gets 100. UNVERIFIED: the server breaks ties by creation sequence,
                // which a snapshot does not expose; createdAt then id approximates it.
                let mut sorted = others.clone();
                sorted.sort_by(|a, b| {
                    let (na, nb) = (&self.nodes[a], &self.nodes[b]);
                    (na.priority, na.created_at, a).cmp(&(nb.priority, nb.created_at, b))
                });
                for (rank, c) in sorted.iter().enumerate() {
                    self.nodes.get_mut(c).expect("child").priority = 100 * (rank as i64 + 2);
                }
                let key = parent.cloned();
                self.sort_siblings(&key);
                100
            }
        }
    }
}

// ---------------------------------------------------------------------------------------
// Outline comparison
// ---------------------------------------------------------------------------------------

/// Which nodes [`outline_diff`] compares.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffScope {
    /// Every node in `root`'s subtree, and full sibling order.
    All,
    /// Only nodes visible from `root` (§4.5) in either tree, and the order of visible
    /// siblings. This is what the Markdown side can observe.
    Visible,
}

/// One difference found by [`outline_diff`]. Priorities and timestamps are never
/// compared; order is compared as id sequences.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum OutlineDiff {
    /// In `expected` but not in `actual`.
    Missing(NodeId),
    /// In `actual` but not in `expected`.
    Extra(NodeId),
    /// Different parent.
    Parent {
        id: NodeId,
        expected: Option<NodeId>,
        actual: Option<NodeId>,
    },
    /// Different name.
    Name {
        id: NodeId,
        expected: String,
        actual: String,
    },
    /// Different note.
    Note {
        id: NodeId,
        expected: Option<String>,
        actual: Option<String>,
    },
    /// Different layout.
    Layout {
        id: NodeId,
        expected: String,
        actual: String,
    },
    /// Different completion flag.
    Completed {
        id: NodeId,
        expected: bool,
        actual: bool,
    },
    /// Same children set (in scope) but a different order.
    Order {
        parent: NodeId,
        expected: Vec<NodeId>,
        actual: Vec<NodeId>,
    },
}

/// Compare the outlines below `root` in two trees: membership, parent, name, note,
/// layout, completion and sibling order (as id sequences). Used to verify an apply
/// (§7.1: "re-export and check that the remote now equals M") and in tests. Results are
/// in a deterministic order (by id, then kind).
pub fn outline_diff(
    expected: &Tree,
    actual: &Tree,
    root: &NodeId,
    scope: DiffScope,
) -> Vec<OutlineDiff> {
    let in_scope = |t: &Tree| -> BTreeSet<NodeId> {
        if !t.contains(root) {
            return BTreeSet::new();
        }
        let mut s: BTreeSet<NodeId> = match scope {
            DiffScope::All => t.descendants(root).into_iter().collect(),
            DiffScope::Visible => t.visible_descendants(root).into_iter().collect(),
        };
        s.insert(root.clone());
        s
    };
    let e_set = in_scope(expected);
    let a_set = in_scope(actual);
    let mut out = Vec::new();
    for id in e_set.difference(&a_set) {
        out.push(OutlineDiff::Missing(id.clone()));
    }
    for id in a_set.difference(&e_set) {
        out.push(OutlineDiff::Extra(id.clone()));
    }
    let children_in = |t: &Tree, set: &BTreeSet<NodeId>, id: &NodeId| -> Vec<NodeId> {
        t.children(id)
            .iter()
            .filter(|c| set.contains(*c))
            .cloned()
            .collect()
    };
    for id in e_set.intersection(&a_set) {
        let (e, a) = (&expected.nodes[id], &actual.nodes[id]);
        if id != root && e.parent_id != a.parent_id {
            out.push(OutlineDiff::Parent {
                id: id.clone(),
                expected: e.parent_id.clone(),
                actual: a.parent_id.clone(),
            });
        }
        if e.name != a.name {
            out.push(OutlineDiff::Name {
                id: id.clone(),
                expected: e.name.clone(),
                actual: a.name.clone(),
            });
        }
        if e.note != a.note {
            out.push(OutlineDiff::Note {
                id: id.clone(),
                expected: e.note.clone(),
                actual: a.note.clone(),
            });
        }
        if e.layout != a.layout {
            out.push(OutlineDiff::Layout {
                id: id.clone(),
                expected: e.layout.clone(),
                actual: a.layout.clone(),
            });
        }
        if e.completed != a.completed {
            out.push(OutlineDiff::Completed {
                id: id.clone(),
                expected: e.completed,
                actual: a.completed,
            });
        }
        let ec = children_in(expected, &e_set, id);
        let ac = children_in(actual, &a_set, id);
        let ec_common: Vec<NodeId> = ec.iter().filter(|c| a_set.contains(*c)).cloned().collect();
        let ac_common: Vec<NodeId> = ac.iter().filter(|c| e_set.contains(*c)).cloned().collect();
        let es: BTreeSet<&NodeId> = ec_common.iter().collect();
        let as_: BTreeSet<&NodeId> = ac_common.iter().collect();
        if es == as_ && ec_common != ac_common {
            out.push(OutlineDiff::Order {
                parent: id.clone(),
                expected: ec,
                actual: ac,
            });
        }
    }
    out
}

/// Breadth-first list of `root` and its descendants with their depth below `root`.
/// Convenience for renderers and tests.
pub fn levels(tree: &Tree, root: &NodeId) -> Vec<(NodeId, usize)> {
    let mut out = Vec::new();
    if !tree.contains(root) {
        return out;
    }
    let mut seen = BTreeSet::new();
    let mut q = VecDeque::from([(root.clone(), 0usize)]);
    seen.insert(root.clone());
    while let Some((id, d)) = q.pop_front() {
        for c in tree.children(&id) {
            if seen.insert(c.clone()) {
                q.push_back((c.clone(), d + 1));
            }
        }
        out.push((id, d));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uid(n: u32) -> NodeId {
        NodeId::parse(&format!("00000000-0000-4000-8000-{n:012x}")).unwrap()
    }

    fn node(n: u32, parent: Option<u32>, prio: i64) -> Node {
        let mut x = Node::new(uid(n), parent.map(uid), format!("n{n}"));
        x.priority = prio;
        x.created_at = n as i64;
        x
    }

    /// root(1) -> a(2) [b(4), c(5)], d(3)
    fn sample() -> Tree {
        Tree::from_nodes(vec![
            node(1, None, 100),
            node(2, Some(1), 100),
            node(3, Some(1), 200),
            node(4, Some(2), 300),
            node(5, Some(2), 400),
        ])
        .unwrap()
    }

    #[test]
    fn node_id_parse_and_short() {
        let id = NodeId::parse("5e2e76e4-fea4-4f1f-9c5b-a846cfce4c6d").unwrap();
        assert!(id.is_uuid());
        assert_eq!(id.short().unwrap().as_str(), "a846cfce4c6d");
        assert!(NodeId::parse("5E2E76E4-FEA4-4F1F-9C5B-A846CFCE4C6D").is_err());
        assert!(NodeId::parse("5e2e76e4fea44f1f9c5ba846cfce4c6d").is_err());
        assert!(NodeId::parse("a846cfce4c6d").is_err());
        let t = NodeId::temp(7);
        assert!(t.is_temp());
        assert_eq!(t.temp_index(), Some(7));
        assert!(!t.is_uuid());
        assert_eq!(t.short(), None);
        assert!(!id.is_temp());
        // Legacy ids from the API are accepted unchecked.
        let legacy = NodeId::from_api("0123456789abcdef0123");
        assert_eq!(legacy.short().unwrap().as_str(), "89abcdef0123");
    }

    #[test]
    fn short_id_parse() {
        assert_eq!(
            ShortId::parse("A846CFCE4C6D").unwrap().as_str(),
            "a846cfce4c6d"
        );
        assert!(ShortId::parse("a846cfce4c6").is_err());
        assert!(ShortId::parse("a846cfce4c6g").is_err());
        let json = serde_json::to_string(&ShortId::parse("a846cfce4c6d").unwrap()).unwrap();
        assert_eq!(json, "\"a846cfce4c6d\"");
        assert!(serde_json::from_str::<ShortId>("\"zz\"").is_err());
    }

    #[test]
    fn id_map_resolves_and_detects_ambiguity() {
        let t = sample();
        let mut m = IdMap::from_tree(&t);
        assert_eq!(m.len(), 5);
        assert_eq!(m.get(&uid(2).short().unwrap()), Some(&uid(2)));
        m.insert(&uid(2)); // same id: no-op
        assert_eq!(m.get(&uid(2).short().unwrap()), Some(&uid(2)));
        let clash = NodeId::parse("11111111-0000-4000-8000-000000000002").unwrap();
        m.insert(&clash);
        let s = uid(2).short().unwrap();
        assert!(m.is_ambiguous(&s));
        assert_eq!(m.get(&s), None);
        m.insert(&NodeId::temp(1));
        assert_eq!(m.len(), 4);
        let back: IdMap = serde_json::from_str(&serde_json::to_string(&m).unwrap()).unwrap();
        assert_eq!(back, m);
    }

    #[test]
    fn api_node_json_roundtrip_and_key_order() {
        let raw = r#"{"id":"5e2e76e4-fea4-4f1f-9c5b-a846cfce4c6d","name":"a &amp; b","note":null,"parent_id":null,"priority":28,"completed":false,"data":{"layoutMode":"bullets"},"createdAt":1791834396,"modifiedAt":null,"completedAt":null}"#;
        let a: ApiNode = serde_json::from_str(raw).unwrap();
        assert_eq!(serde_json::to_string(&a).unwrap(), raw);
        let n = Node::from(a.clone());
        assert_eq!(n.layout, "bullets");
        assert_eq!(n.modified_at, None);
        assert_eq!(ApiNode::from(&n), a);
        // Lenient on missing optional fields and unknown keys.
        let m: ApiNode = serde_json::from_str(r#"{"id":"x","name":"","extra":1}"#).unwrap();
        assert_eq!(m.data.layout_mode, "bullets");
        assert_eq!(m.note, None);
    }

    #[test]
    fn children_sorted_by_priority_then_id() {
        let t = Tree::from_nodes(vec![
            node(1, None, 0),
            node(9, Some(1), 50),
            node(8, Some(1), 50),
            node(7, Some(1), 10),
        ])
        .unwrap();
        assert_eq!(t.children(&uid(1)), &[uid(7), uid(8), uid(9)]);
        assert!(t.has_priority_ties(Some(&uid(1))));
        assert_eq!(t.priority_ties(Some(&uid(1))), vec![vec![uid(8), uid(9)]]);
        assert!(!sample().has_priority_ties(Some(&uid(1))));
        assert_eq!(t.tops(), vec![uid(1)]);
    }

    #[test]
    fn duplicate_ids_rejected() {
        assert_eq!(
            Tree::from_nodes(vec![node(1, None, 0), node(1, None, 1)]),
            Err(TreeError::DuplicateId(uid(1)))
        );
    }

    #[test]
    fn navigation() {
        let t = sample();
        assert_eq!(t.parent(&uid(4)), Some(&uid(2)));
        assert_eq!(t.ancestors(&uid(4)), vec![uid(2), uid(1)]);
        assert_eq!(t.descendants(&uid(1)), vec![uid(2), uid(4), uid(5), uid(3)]);
        assert_eq!(t.preorder(&uid(2)), vec![uid(2), uid(4), uid(5)]);
        assert!(t.is_descendant(&uid(5), &uid(1)));
        assert!(!t.is_descendant(&uid(1), &uid(1)));
        assert!(!t.is_descendant(&uid(3), &uid(2)));
        assert_eq!(t.depth_below(&uid(5), &uid(1)), Some(2));
        assert_eq!(t.depth_below(&uid(1), &uid(1)), Some(0));
        assert_eq!(t.depth_below(&uid(3), &uid(2)), None);
        let levels = levels(&t, &uid(1));
        assert_eq!(levels[0], (uid(1), 0));
        assert_eq!(levels.len(), 5);
    }

    #[test]
    fn subtree_projection() {
        let t = sample();
        let s = t.subtree(&uid(2)).unwrap();
        assert_eq!(s.len(), 3);
        assert_eq!(s.tops(), vec![uid(2)]);
        assert_eq!(s.parent(&uid(2)), Some(&uid(1)));
        assert_eq!(s.ancestors(&uid(4)), vec![uid(2)]);
        assert!(t.subtree(&uid(99)).is_none());
    }

    #[test]
    fn cycles_are_survivable() {
        // a(1) <-> b(2) parent cycle, as production can produce (DM§4.7.5).
        let t = Tree::from_nodes(vec![node(1, Some(2), 100), node(2, Some(1), 100)]).unwrap();
        assert_eq!(t.ancestors(&uid(1)), vec![uid(2)]);
        assert_eq!(t.descendants(&uid(1)), vec![uid(2)]);
        assert!(t.tops().is_empty());
    }

    #[test]
    fn visibility() {
        let mut t = sample();
        t.set_completed(&uid(2), true, Some(5)).unwrap();
        let root = uid(1);
        assert!(t.is_visible(&root, &root));
        assert!(!t.is_visible(&uid(2), &root));
        assert!(!t.is_visible(&uid(4), &root)); // incomplete, under a completed node
        assert!(t.is_visible(&uid(3), &root));
        assert_eq!(t.visible_descendants(&root), vec![uid(3)]);
        assert_eq!(t.visible_children(&root), vec![uid(3)]);
        // The sync root is visible even when completed.
        t.set_completed(&root, true, Some(6)).unwrap();
        assert!(t.is_visible(&root, &root));
        assert!(t.is_visible(&uid(3), &root));
        // Outside the root: not visible.
        assert!(!t.is_visible(&uid(1), &uid(2)));
        // Hidden descendants relative to a node.
        let mut u = sample();
        u.set_completed(&uid(4), true, None).unwrap();
        assert_eq!(u.count_hidden_descendants(&uid(1)), 1);
        u.set_completed(&uid(2), true, None).unwrap();
        assert_eq!(u.count_hidden_descendants(&uid(1)), 3);
        assert_eq!(u.count_hidden_descendants(&uid(2)), 1);
    }

    #[test]
    fn place_follows_api_priorities() {
        let mut t = Tree::from_nodes(vec![node(1, None, 0)]).unwrap();
        let p = uid(1);
        // Empty parent: 100.
        t.place(Node::new(uid(10), None, "a"), Some(&p), Position::Top)
            .unwrap();
        assert_eq!(t.get(&uid(10)).unwrap().priority, 100);
        // Top halves: 100 -> 50 -> 25 -> 12 -> 6 -> 3 -> 1 -> 0.
        let expect = [50, 25, 12, 6, 3, 1, 0];
        for (i, e) in expect.iter().enumerate() {
            let id = uid(11 + i as u32);
            t.place(Node::new(id.clone(), None, "x"), Some(&p), Position::Top)
                .unwrap();
            assert_eq!(t.get(&id).unwrap().priority, *e);
        }
        // Then a renumber: new = 100, others 200, 300, … in display order.
        t.place(Node::new(uid(30), None, "r"), Some(&p), Position::Top)
            .unwrap();
        let prios: Vec<i64> = t
            .children(&p)
            .iter()
            .map(|c| t.get(c).unwrap().priority)
            .collect();
        assert_eq!(prios, (1..=9).map(|k| k * 100).collect::<Vec<_>>());
        assert_eq!(t.children(&p)[0], uid(30));
        assert_eq!(t.children(&p)[1], uid(17));
        assert_eq!(t.children(&p)[8], uid(10));
    }

    #[test]
    fn bottom_formula() {
        for (sibs, want) in [
            (vec![100, 200, 300], 400),
            (vec![0, 1, 2], 200),
            (vec![25, 50], 200),
            (vec![50], 200),
            (vec![150], 300),
            (vec![250], 400),
            (vec![0], 100),
        ] {
            let mut nodes = vec![node(1, None, 0)];
            for (i, p) in sibs.iter().enumerate() {
                nodes.push(node(10 + i as u32, Some(1), *p));
            }
            let mut t = Tree::from_nodes(nodes).unwrap();
            t.place(
                Node::new(uid(99), None, "b"),
                Some(&uid(1)),
                Position::Bottom,
            )
            .unwrap();
            assert_eq!(t.get(&uid(99)).unwrap().priority, want, "siblings {sibs:?}");
        }
    }

    #[test]
    fn move_node_and_cycles() {
        let mut t = sample();
        // Same-parent move to top excludes self: [self=100(a), 200(d)] -> a stays first.
        t.move_node(&uid(3), Some(&uid(1)), Position::Top).unwrap();
        assert_eq!(t.children(&uid(1)), &[uid(3), uid(2)]);
        assert_eq!(t.get(&uid(3)).unwrap().priority, 50);
        // Cross-parent move to bottom.
        t.move_node(&uid(3), Some(&uid(2)), Position::Bottom)
            .unwrap();
        assert_eq!(t.children(&uid(2)), &[uid(4), uid(5), uid(3)]);
        assert_eq!(t.get(&uid(3)).unwrap().priority, 500);
        assert_eq!(t.parent(&uid(3)), Some(&uid(2)));
        // Cycles refused.
        assert_eq!(
            t.move_node(&uid(2), Some(&uid(4)), Position::Top),
            Err(TreeError::Cycle {
                node: uid(2),
                dest: uid(4)
            })
        );
        assert!(matches!(
            t.move_node(&uid(2), Some(&uid(2)), Position::Top),
            Err(TreeError::Cycle { .. })
        ));
        assert_eq!(
            t.move_node(&uid(2), Some(&uid(77)), Position::Top),
            Err(TreeError::UnknownParent(uid(77)))
        );
        assert_eq!(
            t.move_node(&uid(77), None, Position::Top),
            Err(TreeError::UnknownNode(uid(77)))
        );
        // Move to the top level.
        t.move_node(&uid(5), None, Position::Bottom).unwrap();
        assert_eq!(t.parent(&uid(5)), None);
        assert!(t.tops().contains(&uid(5)));
    }

    #[test]
    fn remove_subtree_and_setters() {
        let mut t = sample();
        let removed = t.remove_subtree(&uid(2)).unwrap();
        assert_eq!(
            removed.iter().map(|n| n.id.clone()).collect::<Vec<_>>(),
            vec![uid(2), uid(4), uid(5)]
        );
        assert_eq!(t.len(), 2);
        assert_eq!(t.children(&uid(1)), &[uid(3)]);
        assert!(t.children(&uid(2)).is_empty());
        t.set_name(&uid(3), "x").unwrap();
        t.set_note(&uid(3), Some("n".into())).unwrap();
        t.set_layout(&uid(3), "todo").unwrap();
        t.set_modified_at(&uid(3), Some(9)).unwrap();
        let n = t.get(&uid(3)).unwrap();
        assert_eq!(
            (n.name.as_str(), n.note.as_deref(), n.layout.as_str()),
            ("x", Some("n"), "todo")
        );
        assert_eq!(
            t.set_name(&uid(2), "gone"),
            Err(TreeError::UnknownNode(uid(2)))
        );
        t.insert(node(6, Some(1), 50)).unwrap();
        assert_eq!(t.children(&uid(1)), &[uid(6), uid(3)]);
        t.set_priority(&uid(6), 900).unwrap();
        assert_eq!(t.children(&uid(1)), &[uid(3), uid(6)]);
        assert_eq!(
            t.insert(node(6, Some(1), 1)),
            Err(TreeError::DuplicateId(uid(6)))
        );
    }

    #[test]
    fn reorder_and_remap() {
        let mut t = sample();
        t.reorder_children(&uid(2), &[uid(5), uid(4)]).unwrap();
        assert_eq!(t.children(&uid(2)), &[uid(5), uid(4)]);
        assert!(t.reorder_children(&uid(2), &[uid(5)]).is_err());
        assert!(t.reorder_children(&uid(2), &[uid(5), uid(5)]).is_err());
        t.place(
            Node::new(NodeId::temp(1), None, "new"),
            Some(&uid(2)),
            Position::Bottom,
        )
        .unwrap();
        t.place(
            Node::new(NodeId::temp(2), None, "kid"),
            Some(&NodeId::temp(1)),
            Position::Bottom,
        )
        .unwrap();
        let map = BTreeMap::from([(NodeId::temp(1), uid(50)), (NodeId::temp(2), uid(51))]);
        t.remap_ids(&map).unwrap();
        assert_eq!(t.children(&uid(2)).last(), Some(&uid(50)));
        assert_eq!(t.children(&uid(50)), &[uid(51)]);
        assert!(!t.contains(&NodeId::temp(1)));
    }

    #[test]
    fn tree_serde_roundtrip() {
        let t = sample();
        let s = serde_json::to_string(&t).unwrap();
        assert!(s.starts_with("{\"nodes\":["));
        let back: Tree = serde_json::from_str(&s).unwrap();
        assert_eq!(back, t);
        assert_eq!(back.children(&uid(1)), t.children(&uid(1)));
        let api = t.to_api();
        assert_eq!(Tree::from_api(api).unwrap(), t);
    }

    #[test]
    fn outline_diff_reports_differences() {
        let a = sample();
        assert!(outline_diff(&a, &a, &uid(1), DiffScope::All).is_empty());
        let mut b = sample();
        b.set_name(&uid(4), "changed").unwrap();
        b.reorder_children(&uid(1), &[uid(3), uid(2)]).unwrap();
        b.remove_subtree(&uid(5)).unwrap();
        b.insert(node(6, Some(3), 1)).unwrap();
        let d = outline_diff(&a, &b, &uid(1), DiffScope::All);
        assert!(d.contains(&OutlineDiff::Missing(uid(5))));
        assert!(d.contains(&OutlineDiff::Extra(uid(6))));
        assert!(
            d.iter()
                .any(|x| matches!(x, OutlineDiff::Name { id, .. } if *id == uid(4)))
        );
        assert!(
            d.iter()
                .any(|x| matches!(x, OutlineDiff::Order { parent, .. } if *parent == uid(1)))
        );
        // Priorities alone are not differences.
        let mut c = sample();
        c.set_priority(&uid(4), 301).unwrap();
        assert!(outline_diff(&a, &c, &uid(1), DiffScope::All).is_empty());
        // Visible scope ignores hidden nodes.
        let mut h1 = sample();
        let mut h2 = sample();
        h1.set_completed(&uid(2), true, None).unwrap();
        h2.set_completed(&uid(2), true, None).unwrap();
        h2.set_name(&uid(4), "hidden edit").unwrap();
        assert!(outline_diff(&h1, &h2, &uid(1), DiffScope::Visible).is_empty());
        assert!(!outline_diff(&h1, &h2, &uid(1), DiffScope::All).is_empty());
    }
}
