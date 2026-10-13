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

#![allow(
    unused_variables,
    unused_imports,
    unused_mut,
    dead_code,
    clippy::diverging_sub_expression,
    clippy::ptr_arg
)] // stubs; remove when implemented

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
        todo!()
    }

    /// Wrap an id received from the API (export, GET, list, create). The server is
    /// trusted: no validation is done, because legacy ids need not be RFC UUIDs.
    pub fn from_api(s: impl Into<String>) -> NodeId {
        todo!()
    }

    /// The `n`-th temporary id (`temp-<n>`), used by `merge` for nodes created in
    /// Markdown. Temporary ids sort after every UUID (`t` > hex digits).
    pub fn temp(n: u32) -> NodeId {
        todo!()
    }

    /// True for ids made by [`NodeId::temp`].
    pub fn is_temp(&self) -> bool {
        todo!()
    }

    /// `Some(n)` for `temp-<n>`.
    pub fn temp_index(&self) -> Option<u32> {
        todo!()
    }

    /// True if this is a lowercase hyphenated UUID, i.e. acceptable to the API's
    /// mutating endpoints (DM§3.1).
    pub fn is_uuid(&self) -> bool {
        todo!()
    }

    /// The id as a string.
    pub fn as_str(&self) -> &str {
        todo!()
    }

    /// The short id (last 12 hex digits of the dash-less id, DM§1.1). `None` for
    /// temporary ids and for ids with fewer than 12 hex digits.
    pub fn short(&self) -> Option<ShortId> {
        todo!()
    }
}

impl fmt::Display for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        todo!()
    }
}

impl FromStr for NodeId {
    type Err = IdError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        todo!()
    }
}

fn is_uuid_str(s: &str) -> bool {
    todo!()
}

/// A 12-hex-digit short id (DM§1.1), always stored lowercase.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ShortId(String);

impl ShortId {
    /// Parse 12 hex digits, case-insensitively (GET accepts any case, DM§3.1).
    pub fn parse(s: &str) -> Result<ShortId, IdError> {
        todo!()
    }

    /// The short id of `id` (see [`NodeId::short`]).
    pub fn of(id: &NodeId) -> Option<ShortId> {
        todo!()
    }

    /// The 12 lowercase hex digits.
    pub fn as_str(&self) -> &str {
        todo!()
    }
}

impl fmt::Display for ShortId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        todo!()
    }
}

impl FromStr for ShortId {
    type Err = IdError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        todo!()
    }
}

impl TryFrom<String> for ShortId {
    type Error = IdError;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        todo!()
    }
}

impl From<ShortId> for String {
    fn from(s: ShortId) -> String {
        todo!()
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
        todo!()
    }

    /// The map of every node in `tree`.
    pub fn from_tree(tree: &Tree) -> IdMap {
        todo!()
    }

    /// Add every node of `tree`.
    pub fn extend_from_tree(&mut self, tree: &Tree) {
        todo!()
    }

    /// Add one id. Temporary ids and ids without a short form are ignored. Adding the
    /// same full id twice is a no-op; a second, different full id makes the short id
    /// ambiguous.
    pub fn insert(&mut self, id: &NodeId) {
        todo!()
    }

    /// The full id for `short`, unless unknown or ambiguous.
    pub fn get(&self, short: &ShortId) -> Option<&NodeId> {
        todo!()
    }

    /// True if two different full ids share `short`.
    pub fn is_ambiguous(&self, short: &ShortId) -> bool {
        todo!()
    }

    /// Number of resolvable short ids.
    pub fn len(&self) -> usize {
        todo!()
    }

    /// True if nothing is resolvable.
    pub fn is_empty(&self) -> bool {
        todo!()
    }

    /// All resolvable pairs, in short-id order.
    pub fn iter(&self) -> impl Iterator<Item = (&ShortId, &NodeId)> {
        let it: std::iter::Empty<_> = todo!();
        #[allow(unreachable_code)]
        it
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
        todo!()
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
}

impl Node {
    /// A fresh incomplete node with layout [`DEFAULT_LAYOUT`], `note: None`, priority 0
    /// and zero timestamps; callers set what they need.
    pub fn new(id: NodeId, parent_id: Option<NodeId>, name: impl Into<String>) -> Node {
        todo!()
    }

    /// The mirror-shell heuristic of §10 (UI:H2): `name == ""` and `note` is `None` or
    /// `""`. Genuinely empty bullets match too; callers treat both as read-only
    /// placeholders and never render their children.
    pub fn looks_like_mirror_shell(&self) -> bool {
        todo!()
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

/// The API's `data` object; it only ever holds `layoutMode` (DM§1.2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiNodeData {
    /// The layout mode string.
    #[serde(rename = "layoutMode", default = "default_layout")]
    pub layout_mode: String,
}

fn default_layout() -> String {
    todo!()
}

impl Default for ApiNodeData {
    fn default() -> Self {
        todo!()
    }
}

impl From<ApiNode> for Node {
    fn from(a: ApiNode) -> Node {
        todo!()
    }
}

impl From<&Node> for ApiNode {
    fn from(n: &Node) -> ApiNode {
        todo!()
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
        todo!()
    }
}

impl TryFrom<TreeRepr> for Tree {
    type Error = TreeError;
    fn try_from(r: TreeRepr) -> Result<Tree, TreeError> {
        todo!()
    }
}

const NO_CHILDREN: &[NodeId] = &[];

impl Tree {
    /// An empty tree.
    pub fn new() -> Tree {
        todo!()
    }

    /// Build from nodes (for example an export's node list, in any order). Parents may
    /// be absent (projections); duplicates are an error.
    pub fn from_nodes(nodes: impl IntoIterator<Item = Node>) -> Result<Tree, TreeError> {
        todo!()
    }

    /// Build from the API's records (`GET /nodes-export`'s `nodes`).
    pub fn from_api(nodes: impl IntoIterator<Item = ApiNode>) -> Result<Tree, TreeError> {
        todo!()
    }

    /// The API records of every node, in id order.
    pub fn to_api(&self) -> Vec<ApiNode> {
        todo!()
    }

    /// The projection on `root` and all its descendants (the root keeps its
    /// `parent_id`, so it is the projection's only top node). `None` if `root` is absent.
    pub fn subtree(&self, root: &NodeId) -> Option<Tree> {
        todo!()
    }

    // ----- queries -----------------------------------------------------------------

    /// Number of nodes.
    pub fn len(&self) -> usize {
        todo!()
    }

    /// True if there are no nodes.
    pub fn is_empty(&self) -> bool {
        todo!()
    }

    /// The node with id `id`.
    pub fn get(&self, id: &NodeId) -> Option<&Node> {
        todo!()
    }

    /// True if `id` is in the tree.
    pub fn contains(&self, id: &NodeId) -> bool {
        todo!()
    }

    /// All nodes, in id order.
    pub fn iter(&self) -> impl Iterator<Item = &Node> {
        let it: std::iter::Empty<_> = todo!();
        #[allow(unreachable_code)]
        it
    }

    /// All ids, in id order.
    pub fn ids(&self) -> impl Iterator<Item = &NodeId> {
        let it: std::iter::Empty<_> = todo!();
        #[allow(unreachable_code)]
        it
    }

    /// The stored `parent_id` of `id` (which may name a node outside the tree), or
    /// `None` for top-level or unknown nodes.
    pub fn parent(&self, id: &NodeId) -> Option<&NodeId> {
        todo!()
    }

    /// The children of `id`, sorted by `(priority, id)`. Empty for leaves and unknown ids.
    pub fn children(&self, id: &NodeId) -> &[NodeId] {
        todo!()
    }

    /// The children of `parent` where `None` means the account's top level.
    pub fn children_of(&self, parent: Option<&NodeId>) -> &[NodeId] {
        todo!()
    }

    /// Top nodes of this tree: nodes whose parent is `None` or absent from the tree,
    /// sorted by `(priority, id)` within each parent and then by parent.
    pub fn tops(&self) -> Vec<NodeId> {
        todo!()
    }

    /// Ancestors of `id` that are in the tree, nearest first (parent, grandparent, …).
    /// Stops at the first parent not in the tree and at a repeated node (cycles).
    pub fn ancestors(&self, id: &NodeId) -> Vec<NodeId> {
        todo!()
    }

    /// Descendants of `id` in pre-order (children in sibling order), excluding `id`.
    pub fn descendants(&self, id: &NodeId) -> Vec<NodeId> {
        todo!()
    }

    fn preorder_into(
        &self,
        id: &NodeId,
        seen: &mut BTreeSet<NodeId>,
        out: &mut Vec<NodeId>,
        descend: &dyn Fn(&Node) -> bool,
    ) {
        todo!()
    }

    /// `id` followed by its descendants, in pre-order.
    pub fn preorder(&self, id: &NodeId) -> Vec<NodeId> {
        todo!()
    }

    /// True if `node` is strictly below `ancestor`.
    pub fn is_descendant(&self, node: &NodeId, ancestor: &NodeId) -> bool {
        todo!()
    }

    /// Depth of `node` below `root`: 0 for the root, 1 for its children, … `None` if
    /// `node` is not `root` or a descendant of it.
    pub fn depth_below(&self, node: &NodeId, root: &NodeId) -> Option<usize> {
        todo!()
    }

    /// Groups (of two or more) of children of `parent` (`None` = top level) that share a
    /// priority, in sibling order. Their relative display order in the UI is unknown
    /// (§6.3, DM§7).
    pub fn priority_ties(&self, parent: Option<&NodeId>) -> Vec<Vec<NodeId>> {
        todo!()
    }

    /// True if two children of `parent` share a priority.
    pub fn has_priority_ties(&self, parent: Option<&NodeId>) -> bool {
        todo!()
    }

    // ----- visibility (§4.5) ---------------------------------------------------------

    /// True if `id` is visible from the sync root `root` by the completion rule: `root`
    /// itself always is; a descendant is visible if neither it nor any ancestor strictly
    /// below `root` is completed. Nodes outside `root`'s subtree are not visible.
    pub fn is_visible(&self, id: &NodeId, root: &NodeId) -> bool {
        todo!()
    }

    /// The visible descendants of `root` (excluding `root`), in pre-order.
    pub fn visible_descendants(&self, root: &NodeId) -> Vec<NodeId> {
        todo!()
    }

    fn visible_into(&self, id: &NodeId, seen: &mut BTreeSet<NodeId>, out: &mut Vec<NodeId>) {
        todo!()
    }

    /// The incomplete children of `id`, in sibling order. If `id` is visible from some
    /// root, these are exactly its visible children.
    pub fn visible_children(&self, id: &NodeId) -> Vec<NodeId> {
        todo!()
    }

    /// Number of descendants of `id` that are hidden *relative to `id`*: completed, or
    /// below a completed node (excluding `id` itself). This is what a delete of `id`
    /// takes with it that the Markdown user cannot see (§4.5, §8.3).
    pub fn count_hidden_descendants(&self, id: &NodeId) -> usize {
        todo!()
    }

    // ----- structural edits ------------------------------------------------------------

    /// Insert `node` with its own `parent_id` and `priority`. The parent need not be in
    /// the tree (projections). Errors if the id exists.
    pub fn insert(&mut self, node: Node) -> Result<(), TreeError> {
        todo!()
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
        todo!()
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
        todo!()
    }

    /// Remove `id` and all its descendants; returns the removed nodes in pre-order.
    pub fn remove_subtree(&mut self, id: &NodeId) -> Result<Vec<Node>, TreeError> {
        todo!()
    }

    /// Set the WF-HTML name.
    pub fn set_name(&mut self, id: &NodeId, name: impl Into<String>) -> Result<(), TreeError> {
        todo!()
    }

    /// Set the WF-HTML note.
    pub fn set_note(&mut self, id: &NodeId, note: Option<String>) -> Result<(), TreeError> {
        todo!()
    }

    /// Set the layout mode.
    pub fn set_layout(&mut self, id: &NodeId, layout: impl Into<String>) -> Result<(), TreeError> {
        todo!()
    }

    /// Set completion and `completedAt`.
    pub fn set_completed(
        &mut self,
        id: &NodeId,
        completed: bool,
        completed_at: Option<i64>,
    ) -> Result<(), TreeError> {
        todo!()
    }

    /// Set `modifiedAt`.
    pub fn set_modified_at(
        &mut self,
        id: &NodeId,
        modified_at: Option<i64>,
    ) -> Result<(), TreeError> {
        todo!()
    }

    /// Set the raw priority and re-sort the siblings.
    pub fn set_priority(&mut self, id: &NodeId, priority: i64) -> Result<(), TreeError> {
        todo!()
    }

    /// Replace node ids everywhere (node ids and parent ids) according to `map`, for
    /// example temporary ids -> the real ids returned by create. Ids absent from `map`
    /// are kept. Errors if the result has duplicate ids.
    pub fn remap_ids(&mut self, map: &BTreeMap<NodeId, NodeId>) -> Result<(), TreeError> {
        todo!()
    }

    /// Make the children of `parent` appear in exactly `order` (a permutation of the
    /// current children) by rewriting their priorities to 100, 200, … This is for
    /// merged trees, whose priorities carry no remote meaning; it is not an API
    /// operation (the planner turns order differences into top/bottom moves, §8.1).
    pub fn reorder_children(&mut self, parent: &NodeId, order: &[NodeId]) -> Result<(), TreeError> {
        todo!()
    }

    // ----- internals -------------------------------------------------------------------

    fn node_mut(&mut self, id: &NodeId) -> Result<&mut Node, TreeError> {
        todo!()
    }

    fn detach(&mut self, id: &NodeId) {
        todo!()
    }

    fn sort_siblings(&mut self, key: &Option<NodeId>) {
        todo!()
    }

    /// DM§7 `assign_priority`, renumbering siblings when a top insert finds min <= 0.
    fn assign_priority(
        &mut self,
        parent: Option<&NodeId>,
        pos: Position,
        exclude: Option<&NodeId>,
    ) -> i64 {
        todo!()
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
    todo!()
}

/// Breadth-first list of `root` and its descendants with their depth below `root`.
/// Convenience for renderers and tests.
pub fn levels(tree: &Tree, root: &NodeId) -> Vec<(NodeId, usize)> {
    todo!()
}
