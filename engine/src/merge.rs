//! The three-way merge of one sync root: base (checkpoint), remote (export) and local
//! (the Markdown file), producing the merged tree `M` with the provenance the planner
//! needs, the conflicts to keep in the file, and a human-readable report.
//!
//! Responsibility (design §5.1 change detection in two domains, §5.2 lossy writes, §6 in
//! full, §4.4 id fragility, §4.5 projections, §9.1 guards 1–3 as far as they concern the
//! merge, §10 placeholders). Pure: no I/O.
//!
//! Consumes: [`crate::tree`] (`Tree`, ids, `IdMap`, visibility), [`crate::mdblock`]
//! (`LocalDoc`/`LocalItem`, `DocConfig`, `ConflictMarker`), [`crate::mdinline`]
//! (`md_to_wf`, `write_value`, `classify`, tiers) through a [`TextCodec`], the `similar`
//! crate (text similarity for §4.4 matching, line-level diff3 for notes, LCS for §6.3).
//! Consumed by: `plan` (the whole [`MergeResult`]), `sync` (conflicts and report;
//! deletes for the deferred-delete policy), `mdblock` (conflict markers via
//! `ViewExtras`).
//!
//! [`merge`] uses the production [`MdInlineCodec`]; [`merge_with`] takes any
//! [`TextCodec`] (tests use [`PlainTextCodec`], whose Markdown is plain text).
//!
//! # Domains (§5.1)
//!
//! * **Local changes** are `diff(base_doc, local_doc)`: both are parsed by the same
//!   parser, and identity is by tag. A field changed locally iff its Markdown differs
//!   (for names and notes compare `md_to_wf` of both sides, so escaping variants are not
//!   edits). If the two docs are equal the local side has no changes at all.
//! * **Remote changes** are `diff(base, remote)` on raw WF strings, restricted to the
//!   root's subtree (plus nodes moved in or out of it). A `null` note and `""` are the
//!   same value for change detection (nobody can see the difference).
//! * **Visibility**: locally visible-in-base = the nodes tagged in `base_doc` (plus the
//!   root); remotely visible = the `View` rules of `mdblock` applied to the remote tree
//!   (completed nodes and their subtrees, pending deletes, nodes deeper than
//!   `DocConfig::max_depth`, children of mirror placeholders; the root is always
//!   visible). The merge computes this itself so that it does not depend on rendering.
//!   A node absent from `base_doc` was hidden, and its absence from `local_doc` means
//!   nothing (§4.5).
//!
//! # Required behaviour (each rule cites the design)
//!
//! **Identity (§4.4).** Resolve each non-marker local item to a node:
//! 1. The winning tag, or failing that the last other tag on the line that is valid
//!    (§4.4 "take the last valid one"), that resolves through `ids` to a node in `base`
//!    or in the remote subtree identifies it. If several items carry the same tag, the
//!    one that best matches its `base_doc` item (text similarity of the plain text of
//!    the names, then same parent, then position) keeps it; the others are new nodes
//!    ([`ReportEvent::DuplicateTag`]).
//! 2. A tag resolving to a node inside another managed root (`cross.other_roots`, the
//!    roots themselves excluded) is a cross-file move into this file (§6.4). Any other
//!    tag (unknown, ambiguous, or a node elsewhere in the account) is ignored with
//!    [`ReportEvent::ForeignTag`] and the item is new. UNVERIFIED design choice: §4.4
//!    would move any existing node; we only move nodes out of managed roots, to avoid
//!    pulling unmanaged content into a file.
//! 3. `adoptions` (nodes created by a crashed earlier apply, see `apply::recover`): an
//!    untagged item whose parent resolves to `adoption.parent` and whose
//!    `md_to_wf(name_md)` equals `adoption.name` takes `adoption.node` (first match in
//!    file order) instead of becoming a create ([`ReportEvent::Adopted`]).
//! 4. Deleted-tag fallback: an untagged item under the same parent as an unmatched base
//!    item (tagged in `base_doc`, tag missing from `local_doc`), at a nearby sibling
//!    position (at most [`REMATCH_MAX_DISTANCE`] apart), with similar text (similarity
//!    ratio >= [`REMATCH_MIN_RATIO`]), is that base item edited
//!    ([`ReportEvent::Rematched`]). Same for an item that reappears where a
//!    pending-delete node was (§9.1 guard 2). Pairs are chosen greedily by best ratio,
//!    then nearest position, parents before children.
//! 5. Everything else is a local create, numbered `NodeId::temp(1)`, `temp(2)`, … in file
//!    pre-order (resurrections are numbered in the same sequence).
//!
//! **Fields (§6.1, §5.2).** For `name`, `note`, `layout`:
//! unchanged/changed -> take remote; changed/unchanged -> push local; both changed to
//! the same WF value -> nothing; both changed differently -> conflict: remote wins in
//! Workflowy and the local Markdown is kept as a [`ConflictMarker`]; for notes try a
//! line-level diff3 first (clean merges push the merged note). Pushed values come from
//! `mdinline::write_value(base_value, local_md, …)` (§5.2) and are canonical WF-HTML;
//! `lost: true` produces [`ReportEvent::LossyWrite`]. Layout changes only when the
//! Markdown explicitly changed kind (§4.3): fence added/removed (`code-block` /
//! `bullets`), quote added/removed (`quote-block` / `bullets`), all visible children of
//! a parent switched to/from ordered markers (`numbered` / `bullets`). Checkbox forms
//! never change layouts; new items typed with `[ ]` get `todo` only if
//! [`MergeConfig::todo_from_checkbox`]. A layout conflict has no marker form: remote
//! wins silently (UNVERIFIED design choice).
//! Read-only items (base item had the `ro` flag or the base value classifies as
//! `Tier::ReadOnly`) and placeholders (§10) are never pushed unless `force`
//! ([`ReportEvent::ReadOnlyEditRefused`]); the remote value is kept and the file is
//! re-rendered. Items typed (or moved) under an existing placeholder are refused the
//! same way: creates are lifted to the placeholder's parent, moves are dropped.
//! Guard 3 (stale echo): if the local value equals the node's value in one of
//! `prior_docs` and differs from `base_doc`'s, it is not an edit
//! ([`ReportEvent::StaleEcho`]).
//! Nodes without a base item: cross-file moves push a field only where the remote value
//! is exactly representable (tier `Exact`/`Passthrough`) and the local text differs;
//! adopted nodes are brought to the local values; reappearing pending deletes and
//! other tagged nodes that were hidden in the base never push fields (their text may
//! be stale, guard 3).
//!
//! **Completion (§4.3, §6.2 table).** `[x]` on a visible item is a tick: `ticks` gets the
//! node (unless it is already completed remotely: [`ReportEvent::AlreadyCompleted`]), and
//! other edits to it and its subtree are still pushed (planned before the completion).
//! A field of a ticked node that was also changed remotely keeps the remote value with
//! no conflict marker (the node leaves the file). A new item typed as `[x]` is created
//! and then completed. Ticks are never brakes.
//! Remote completion of a locally edited node or ancestor: push the edits anyway, report
//! [`ReportEvent::VanishedWork`]. Local delete vs remote completion: the delete loses.
//! Local move vs remote completion of the node or of the destination: apply the move.
//! Tick vs remote delete: nothing to do, unless the item was also edited or got new
//! children: then it is resurrected and created ticked, so typed text is never lost.
//!
//! **Structure (§6.2).** Creates never conflict. Delete vs edit: deletes lose (a remote
//! edit of the node or its subtree, or a remote child added, keeps a locally deleted
//! node: [`ReportEvent::DeleteLost`]; a node deleted remotely but edited locally — or with
//! local children added, or moved locally — is resurrected as a local create with a new
//! temp id together with its whole local subtree: [`ReportEvent::Resurrected`]; its
//! remotely deleted ancestors are resurrected with it). Move vs move: remote wins
//! ([`ReportEvent::DroppedMove`]). Move vs delete: the move wins. A remote move out of
//! the subtree removes the node from the file ([`ReportEvent::MovedOut`]); local edits
//! of it are not pushed (it is outside the root) and local creates under it go to its
//! nearest surviving local ancestor. A remote move in adds it. A local move that would
//! create a cycle with a remote move is dropped (remote wins, [`ReportEvent::DroppedMove`]
//! with `cycle: true`). Hidden nodes are never moved, edited or deleted except as a side
//! effect the user asked for (a tick, or the delete of a visible ancestor, which
//! `deletes` reports with its hidden descendants). Deletes are decided per node: a
//! locally deleted node is kept if it or anything below it changed remotely or must
//! survive; its other deleted children are still deleted.
//! Placeholders are never deleted unless `force` ([`ReportEvent::PlaceholderDeleteRefused`]).
//! A node missing from `local_doc` whose tag is in `cross.claimed_elsewhere` moved to
//! another file: it is neither deleted nor moved here (the other file's merge moves
//! it); it stays in `M` at its remote position.
//!
//! **Pending deletes (§9.1 guard 2).** Nodes in `pending_deletes` that are still in the
//! remote subtree and were neither cancelled nor changed remotely are listed in
//! `deletes` again on every merge (and removed from `M`), so that `sync` can keep
//! deferring them and finally send them; they do not count in [`MergeStats`] (they were
//! counted when first deleted). A pending node that reappears in the file, or that
//! changed remotely (delete vs edit), or that left the subtree, is in
//! `cancelled_pending`.
//!
//! **Appear / disappear (§4.5, §6.2 last row).** Remote uncompletion (of the node or an
//! ancestor) is an appear: the node is in `M` at its remote position and the renderer
//! shows it; never a conflict ([`ReportEvent::Appeared`]). Remote completion of a visible
//! node is a disappear, not a delete ([`ReportEvent::Disappeared`]). Guard 3 (structure):
//! a tagged item that is not in `base_doc` and is hidden remotely (or gone) is a merge
//! artefact: drop it from the file, never treat it as a move
//! ([`ReportEvent::ResurrectionDropped`]); untagged items under it are kept (lifted to
//! its parent).
//!
//! **Sibling order (§6.3).** Per parent, over visible children only
//! ([`merge_sibling_order`]): compute each side's moved set relative to the base order
//! (elements not in the LCS of base and that side); start from the remote order and
//! re-insert each locally moved or created node after its nearest local predecessor
//! that is also in the result. Never write because of a priority tie: if the local order
//! of a parent did not change, the target order equals the remote order.
//!
//! **Conflict markers.** Markers present in `local_doc` are carried forward (attached to
//! their parent item's node) unless their node is gone or hidden
//! ([`ReportEvent::ConflictLost`]); a marker the user deleted is resolved. New conflicts
//! add markers (replacing a carried marker of the same node and field).
//!
//! **Safe mode (§9.1 guard 1).** With [`MergeConfig::safe_mode`] (the file's `gen` is
//! missing or older than the kept history): apply local field edits only where the local
//! value differs from every known version of that node (base and `prior_docs`); no
//! deletes, no moves, no reorders, no ticks; creates are allowed (appended after the
//! existing visible children). Report [`ReportEvent::SafeMode`].
//!
//! # Postconditions an implementer must test
//!
//! * `M` (`merged`) contains the root's whole subtree as it should be after the push:
//!   remote values except where `pushes` override, creates under temp ids, local moves
//!   applied, ticked nodes `completed: true` (with `completed_at: None`), deleted
//!   subtrees removed, nodes moved out removed, nodes moved in present. Hidden nodes keep
//!   their remote parent and relative position.
//! * For every visible parent `p` in `M` (a visible node with at least one visible
//!   child), `orders[p]` lists exactly its visible children in `M` (temp ids included)
//!   in target order, and `M`'s sibling order agrees with it.
//! * `pushes`, `creates`, `moves`, `ticks`, `deletes` are exactly the differences between
//!   `M` and the remote tree that the planner must realise (nothing else differs except
//!   sibling order, given by `orders`). Every pushed name/note is canonical WF-HTML.
//! * With `local_doc == base_doc` there are no pushes, creates, moves, ticks or deletes,
//!   and `M` equals the remote subtree up to priorities.
//! * Determinism: same inputs, same output (including temp id numbering).

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::apply::Adoption;
use crate::mdblock::{ConflictMarker, DocConfig, LocalDoc};
use crate::mdinline::ConflictField;
use crate::tree::{IdMap, NodeId, ShortId, Tree};

mod codec;
mod diff3;
mod engine;
mod order;

pub use codec::{MdInlineCodec, PlainTextCodec, TextCodec};
pub use order::merge_sibling_order;

/// §4.4 deleted-tag fallback: minimum similarity ratio (`similar`'s character ratio of
/// the plain texts) for an untagged item to be rematched with a base item.
pub const REMATCH_MIN_RATIO: f32 = 0.6;
/// §4.4 deleted-tag fallback: maximum distance between the sibling indices of the
/// untagged item and the base item. UNVERIFIED design choice: "nearby" is not
/// quantified by the design.
pub const REMATCH_MAX_DISTANCE: usize = 3;

/// Merge options.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MergeConfig {
    /// The file's rendering config (inline options for `md_to_wf`, headings, depth limit:
    /// it must be the same config the base was rendered with, so visibility agrees).
    pub doc: DocConfig,
    /// `--force` or frontmatter `force: true`: allow edits and deletes of placeholders and
    /// read-only items.
    pub force: bool,
    /// Safe mode (§9.1 guard 1); see the module docs.
    pub safe_mode: bool,
    /// New items typed with `[ ]` get layout `todo` (§4.3 "if configured so"). Default
    /// false: new items get `bullets`.
    pub todo_from_checkbox: bool,
}

/// What other files of the same run claim (§6.4). Empty for single-file syncs.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CrossFile {
    /// The sync roots of the other files in this run. Tags of nodes inside them are
    /// cross-file moves into this file.
    pub other_roots: BTreeSet<NodeId>,
    /// Nodes whose tags appear in another file's current text (a cut from this file
    /// pasted there): missing here means "moved away", not "deleted".
    pub claimed_elsewhere: BTreeSet<NodeId>,
}

/// Everything the merge reads.
#[derive(Debug, Clone, Copy)]
pub struct MergeInput<'a> {
    /// The sync root.
    pub root: &'a NodeId,
    /// `base_wf`: the root's subtree when `base_doc` was rendered (hidden nodes
    /// included). For guard 1 this is the history entry of the file's `gen`.
    pub base: &'a Tree,
    /// The latest export (the whole account, so moves out of and into the subtree and
    /// cross-file tags can be resolved).
    pub remote: &'a Tree,
    /// `parse(base_md)`.
    pub base_doc: &'a LocalDoc,
    /// `parse(current file)`.
    pub local_doc: &'a LocalDoc,
    /// Short id -> full id over the base and the whole export.
    pub ids: &'a IdMap,
    /// Options.
    pub cfg: &'a MergeConfig,
    /// Other files of this run.
    pub cross: &'a CrossFile,
    /// Nodes whose delete is pending (§9.1 guard 2): hidden from the file; if one
    /// reappears in `local_doc` the pending delete is cancelled
    /// ([`ReportEvent::PendingDeleteCancelled`]).
    pub pending_deletes: &'a BTreeSet<NodeId>,
    /// Nodes created by an interrupted apply (see `apply::recover`).
    pub adoptions: &'a [Adoption],
    /// Parsed earlier renders (older generations than the base), for guard 3. May be
    /// empty.
    pub prior_docs: &'a [LocalDoc],
}

/// A field change to push, with the remote value expected just before the write (for
/// the pre-write GET check, §7.1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Change<T> {
    /// The value in the remote tree the merge saw.
    pub expected: T,
    /// The value to write (canonical WF-HTML for names and notes).
    pub new: T,
}

/// Fields of one existing node to push. At least one is `Some`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldPush {
    /// New name.
    pub name: Option<Change<String>>,
    /// New note. `new` is never `None` (the API cannot write null, DM§6.4).
    pub note: Option<Change<Option<String>>>,
    /// New layout.
    pub layout: Option<Change<String>>,
}

impl FieldPush {
    /// True if no field is set.
    pub fn is_empty(&self) -> bool {
        self.name.is_none() && self.note.is_none() && self.layout.is_none()
    }

    /// Number of fields set.
    pub fn count(&self) -> usize {
        usize::from(self.name.is_some())
            + usize::from(self.note.is_some())
            + usize::from(self.layout.is_some())
    }
}

/// A node typed in Markdown, to create (§7.2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewNode {
    /// Its temporary id in `M`.
    pub temp: NodeId,
    /// Parent: a real id or the temp id of an earlier create.
    pub parent: NodeId,
    /// Canonical WF-HTML name.
    pub name: String,
    /// Canonical WF-HTML note, `None` if the item has no note lines.
    pub note: Option<String>,
    /// Layout (`bullets`, `todo`, `code-block`, `quote-block`, …).
    pub layout: String,
    /// Typed as `[x]`: complete after creating (§4.3).
    pub ticked: bool,
    /// Line in the local file (for reports).
    pub line: usize,
    /// For a resurrection (§6.2): the remotely deleted node this replaces.
    pub resurrects: Option<NodeId>,
}

/// A re-parenting to push.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MoveIntent {
    /// The node (real id).
    pub node: NodeId,
    /// Its parent in the remote tree (possibly outside this root for cross-file moves).
    pub from: NodeId,
    /// Its parent in `M` (a real id or a temp id).
    pub to: NodeId,
}

/// A local delete to push (top-most deleted nodes only; delete is recursive).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeleteIntent {
    /// The node.
    pub node: NodeId,
    /// Visible descendants deleted with it (the user saw them go).
    pub visible_descendants: usize,
    /// Hidden (completed) descendants deleted with it (§4.5, §8.3: count toward brakes
    /// and name them in the report).
    pub hidden_descendants: usize,
    /// Plain text of the node's name, for reports.
    pub label: String,
}

/// Counts used by the brakes (§8.3) and the report.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MergeStats {
    /// Visible nodes (items) in `base_doc`, root excluded.
    pub base_visible: usize,
    /// Non-marker items in `local_doc`.
    pub local_items: usize,
    /// Nodes deleted, visible ones (top-most and descendants). Re-listed pending
    /// deletes are not counted.
    pub deleted_visible: usize,
    /// Hidden descendants deleted with them.
    pub deleted_hidden: usize,
    /// Ticks (including new items typed ticked).
    pub ticked: usize,
    /// Creates.
    pub created: usize,
    /// Pushed fields (name, note and layout counted separately).
    pub pushed_fields: usize,
    /// Conflicts recorded this run.
    pub conflicts: usize,
}

/// A human-readable event for the sync report and the status note (§9.1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReportEvent {
    /// A remotely deleted node edited locally was re-created (§6.2).
    Resurrected { old: NodeId, new: NodeId },
    /// Local edits/creates under a node completed remotely were pushed and vanish from
    /// view (§6.2 table).
    VanishedWork {
        node: NodeId,
        completed_ancestor: NodeId,
    },
    /// A hidden node became visible remotely (§4.5).
    Appeared { node: NodeId },
    /// A visible node became hidden remotely (§4.5).
    Disappeared { node: NodeId },
    /// A node moved out of the subtree remotely (§6.2).
    MovedOut {
        node: NodeId,
        new_parent: Option<NodeId>,
    },
    /// A node moved into the subtree remotely, or from another file (§6.2, §6.4).
    MovedIn {
        node: NodeId,
        from_file_root: Option<NodeId>,
    },
    /// A local move was dropped (remote move wins, or it would create a cycle).
    DroppedMove { node: NodeId, cycle: bool },
    /// A local delete lost against a remote edit or completion (§6.2).
    DeleteLost { node: NodeId },
    /// A tag that does not belong to this file was ignored (§4.4).
    ForeignTag { short: ShortId, line: usize },
    /// A duplicated tag: `kept_line` kept the identity, the others became new nodes.
    DuplicateTag {
        short: ShortId,
        lines: Vec<usize>,
        kept_line: usize,
    },
    /// An untagged item was matched to a base node by the fallback (§4.4).
    Rematched { node: NodeId, line: usize },
    /// An untagged item was matched to a node from an interrupted apply.
    Adopted { node: NodeId, line: usize },
    /// An edit of a read-only item or placeholder was refused (§5.3, §10).
    ReadOnlyEditRefused { node: NodeId, line: usize },
    /// A delete of a placeholder was refused without `force` (§10).
    PlaceholderDeleteRefused { node: NodeId },
    /// A local value equal to an earlier render was ignored (§9.1 guard 3).
    StaleEcho { node: NodeId, field: ConflictField },
    /// A tagged item hidden remotely and absent from the base was dropped (guard 3).
    ResurrectionDropped { node: NodeId, line: usize },
    /// A conflict was recorded as a marker (§6.1).
    Conflict { node: NodeId, field: ConflictField },
    /// A conflict marker vanished because its node is gone or hidden.
    ConflictLost { node: NodeId, field: ConflictField },
    /// A §5.2 write could not preserve what Markdown could not show.
    LossyWrite { node: NodeId, field: ConflictField },
    /// A tick of a node already completed remotely was skipped (DM§4.5).
    AlreadyCompleted { node: NodeId },
    /// A pending delete was cancelled because the node reappeared (§9.1 guard 2).
    PendingDeleteCancelled { node: NodeId },
    /// Safe mode suppressed this many structural changes (§9.1 guard 1).
    SafeMode { suppressed: usize },
}

impl ReportEvent {
    /// A one-line human-readable description (used in logs and the status note).
    /// `tree` (the remote tree) is used to name nodes by their plain-text names.
    pub fn describe(&self, tree: &Tree) -> String {
        let n = |id: &NodeId| node_label(tree, id);
        let f = |field: &ConflictField| match field {
            ConflictField::Name => "name",
            ConflictField::Note => "note",
        };
        match self {
            ReportEvent::Resurrected { old, new } => format!(
                "{} was deleted in Workflowy but edited in Markdown: re-created as a new node ({new})",
                n(old)
            ),
            ReportEvent::VanishedWork {
                node,
                completed_ancestor,
            } => format!(
                "{} was pushed but is hidden: {} was completed in Workflowy",
                n(node),
                n(completed_ancestor)
            ),
            ReportEvent::Appeared { node } => {
                format!("{} reappeared (uncompleted in Workflowy)", n(node))
            }
            ReportEvent::Disappeared { node } => {
                format!("{} was completed in Workflowy and left the file", n(node))
            }
            ReportEvent::MovedOut { node, new_parent } => match new_parent {
                Some(p) => format!(
                    "{} was moved out of this file in Workflowy (now under {})",
                    n(node),
                    n(p)
                ),
                None => format!(
                    "{} was moved out of this file in Workflowy (now top-level)",
                    n(node)
                ),
            },
            ReportEvent::MovedIn {
                node,
                from_file_root,
            } => match from_file_root {
                Some(r) => format!("{} moved into this file from the file of {}", n(node), n(r)),
                None => format!("{} moved into this file", n(node)),
            },
            ReportEvent::DroppedMove { node, cycle } => {
                if *cycle {
                    format!(
                        "the move of {} was dropped: with a Workflowy move it would create a cycle",
                        n(node)
                    )
                } else {
                    format!(
                        "the move of {} was dropped: it was moved in Workflowy too, or its destination is gone",
                        n(node)
                    )
                }
            }
            ReportEvent::DeleteLost { node } => format!(
                "the delete of {} was not sent: it changed in Workflowy",
                n(node)
            ),
            ReportEvent::ForeignTag { short, line } => {
                format!(
                    "line {line}: id tag {short} does not belong to this file; treated as a new item"
                )
            }
            ReportEvent::DuplicateTag {
                short,
                lines,
                kept_line,
            } => {
                let lines: Vec<String> = lines.iter().map(usize::to_string).collect();
                format!(
                    "id tag {short} appears on lines {}; line {kept_line} keeps it, the others are new items",
                    lines.join(", ")
                )
            }
            ReportEvent::Rematched { node, line } => {
                format!("line {line}: untagged item matched to {}", n(node))
            }
            ReportEvent::Adopted { node, line } => format!(
                "line {line}: matched to {} created by an interrupted sync",
                n(node)
            ),
            ReportEvent::ReadOnlyEditRefused { node, line } => format!(
                "line {line}: {} is read-only (or a mirror placeholder); the edit was not sent",
                n(node)
            ),
            ReportEvent::PlaceholderDeleteRefused { node } => format!(
                "{} is a mirror placeholder; its delete was not sent (use force)",
                n(node)
            ),
            ReportEvent::StaleEcho { node, field } => format!(
                "the {} of {} equals an older render; ignored as a stale echo",
                f(field),
                n(node)
            ),
            ReportEvent::ResurrectionDropped { node, line } => format!(
                "line {line}: {} is hidden in Workflowy and was not in the base; dropped",
                n(node)
            ),
            ReportEvent::Conflict { node, field } => format!(
                "conflict on the {} of {}: Workflowy's version kept, yours is in the file",
                f(field),
                n(node)
            ),
            ReportEvent::ConflictLost { node, field } => format!(
                "the conflict marker for the {} of {} was dropped: the node is gone or hidden",
                f(field),
                n(node)
            ),
            ReportEvent::LossyWrite { node, field } => format!(
                "the {} of {} lost formatting Markdown could not show",
                f(field),
                n(node)
            ),
            ReportEvent::AlreadyCompleted { node } => {
                format!("{} was already completed in Workflowy", n(node))
            }
            ReportEvent::PendingDeleteCancelled { node } => {
                format!("the pending delete of {} was cancelled", n(node))
            }
            ReportEvent::SafeMode { suppressed } => format!(
                "safe mode: {suppressed} structural change(s) not applied (the file's base is unknown)"
            ),
        }
    }
}

/// `"<plain text of the name>" (<short id>)`, or the id alone if unknown.
fn node_label(tree: &Tree, id: &NodeId) -> String {
    let short = id
        .short()
        .map(|s| s.to_string())
        .unwrap_or_else(|| id.to_string());
    match tree.get(id) {
        Some(node) => format!("\"{}\" ({short})", codec::plain_text_lossy(&node.name)),
        None => short,
    }
}

/// The merge's output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MergeResult {
    /// The sync root.
    pub root: NodeId,
    /// `M`: the root's subtree after the merge (see the postconditions).
    pub merged: Tree,
    /// Field writes to existing nodes.
    pub pushes: BTreeMap<NodeId, FieldPush>,
    /// Nodes to create, parents before children (file pre-order).
    pub creates: Vec<NewNode>,
    /// Re-parentings (real nodes), in file pre-order.
    pub moves: Vec<MoveIntent>,
    /// Nodes to complete (real ids; new items typed ticked are in `creates` with
    /// `ticked: true` instead).
    pub ticks: Vec<NodeId>,
    /// Local deletes (top-most only), including still-pending deletes (module docs).
    pub deletes: Vec<DeleteIntent>,
    /// Target order of the visible children of every visible parent in `M` (§6.3).
    pub orders: BTreeMap<NodeId, Vec<NodeId>>,
    /// Conflict markers to render (carried forward and new), sorted.
    pub conflicts: Vec<ConflictMarker>,
    /// Report events, in a deterministic order.
    pub report: Vec<ReportEvent>,
    /// Counts.
    pub stats: MergeStats,
    /// Pending deletes that are no longer pending: the node reappeared in the file,
    /// changed remotely (the delete lost), or left the subtree.
    pub cancelled_pending: BTreeSet<NodeId>,
}

impl MergeResult {
    /// True if there is nothing to push (no pushes, creates, moves, ticks, deletes, and
    /// every `orders` entry equals the remote visible order).
    pub fn is_noop(&self, remote: &Tree) -> bool {
        if !(self.pushes.is_empty()
            && self.creates.is_empty()
            && self.moves.is_empty()
            && self.ticks.is_empty()
            && self.deletes.is_empty())
        {
            return false;
        }
        self.orders.iter().all(|(parent, order)| {
            let members: BTreeSet<&NodeId> = order.iter().collect();
            let current: Vec<&NodeId> = remote
                .children(parent)
                .iter()
                .filter(|c| members.contains(c))
                .collect();
            current.len() == order.len() && current.into_iter().eq(order.iter())
        })
    }

    /// True if any field push, create, move, tick or delete came from the Markdown side
    /// (the order may still differ; see [`MergeResult::is_noop`]).
    pub fn has_local_changes(&self) -> bool {
        !(self.pushes.is_empty()
            && self.creates.is_empty()
            && self.moves.is_empty()
            && self.ticks.is_empty()
            && self.deletes.is_empty())
    }
}

/// Fatal merge problems.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum MergeError {
    /// The sync root is not in the remote tree (deleted, or the export is wrong).
    #[error("sync root {0} is not in the export")]
    RootMissing(NodeId),
    /// The two docs or the base disagree on the root.
    #[error("root mismatch: expected {expected}, found {found}")]
    RootMismatch {
        /// The root the merge was asked to merge.
        expected: NodeId,
        /// What an input names instead.
        found: String,
    },
}

/// Run the three-way merge for one sync root (module docs) with the production text
/// codec ([`MdInlineCodec`]).
pub fn merge(input: &MergeInput<'_>) -> Result<MergeResult, MergeError> {
    merge_with(input, &MdInlineCodec)
}

/// [`merge`] with an explicit [`TextCodec`] (tests use [`PlainTextCodec`]).
pub fn merge_with(
    input: &MergeInput<'_>,
    codec: &dyn TextCodec,
) -> Result<MergeResult, MergeError> {
    engine::run(*input, codec)
}

/// The nodes of `tree` visible from `root` under the file's projection rules (§4.5,
/// the same rules as `mdblock::View`): the root always; below it, nodes that are not
/// completed, not in `hidden` (pending deletes), not deeper than `cfg.max_depth`, and
/// not children of a mirror placeholder (§10). The root is never treated as a
/// placeholder, and nodes with layout `divider` are not placeholders (an empty divider
/// renders as `- ---`). UNVERIFIED design choice: both exceptions are this module's
/// reading of `mdblock`'s rules.
pub fn visible_nodes(
    tree: &Tree,
    root: &NodeId,
    cfg: &DocConfig,
    hidden: &BTreeSet<NodeId>,
) -> BTreeSet<NodeId> {
    let mut out = BTreeSet::new();
    if !tree.contains(root) {
        return out;
    }
    out.insert(root.clone());
    let mut stack: Vec<(NodeId, u32)> = vec![(root.clone(), 0)];
    while let Some((id, depth)) = stack.pop() {
        if &id != root && tree.get(&id).is_some_and(is_placeholder_node) {
            continue;
        }
        if cfg.max_depth.is_some_and(|max| depth + 1 > max) {
            continue;
        }
        for c in tree.children(&id) {
            let node = tree.get(c).expect("child in tree");
            if node.completed || hidden.contains(c) || out.contains(c) {
                continue;
            }
            out.insert(c.clone());
            stack.push((c.clone(), depth + 1));
        }
    }
    out
}

/// The §10 placeholder heuristic as the merge applies it (see [`visible_nodes`]).
pub(crate) fn is_placeholder_node(node: &crate::tree::Node) -> bool {
    node.looks_like_mirror_shell() && node.layout != "divider"
}
