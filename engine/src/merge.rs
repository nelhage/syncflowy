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
//!    the names, then same parent, then position) keeps it. Each other one takes the
//!    next valid tag on its line whose node no line wins and no other item took (so a
//!    line with its own tag and a copied one keeps its node), and only a line left
//!    without one is a new node ([`ReportEvent::DuplicateTag`]). Duplicates are
//!    resolved parents first, and "same parent" compares the parent item's resolved
//!    identity, so a pasted copy of a subtree is created whole and the original
//!    children stay under the original.
//! 2. A tag resolving to a node inside another managed root (`cross.other_roots`, the
//!    roots themselves excluded) is a cross-file move into this file (§6.4), unless the
//!    tag is still on its line in its own file (`cross.still_in_own_file`): then the
//!    line is a copy and a new node, as a duplicated tag within one file
//!    ([`ReportEvent::CopiedTag`]). Any other
//!    tag (unknown, ambiguous, or a node elsewhere in the account) is ignored with
//!    [`ReportEvent::ForeignTag`] and the item is new. UNVERIFIED design choice: §4.4
//!    would move any existing node; we only move nodes out of managed roots, to avoid
//!    pulling unmanaged content into a file.
//! 3. `adoptions` (nodes created by a crashed earlier apply, see `apply::recover`, or
//!    by an earlier pass of this cycle before a CAS rerun): an untagged item whose
//!    parent resolves to `adoption.parent` and whose `md_to_wf(name_md)` equals
//!    `adoption.name` takes `adoption.node` (first match in file order) instead of
//!    becoming a create ([`ReportEvent::Adopted`]). Every other item that would become
//!    a create gets the same chance when its fate is decided (§7.3: the journal never
//!    re-creates a node): items that lost a duplicated tag, items whose tags are all
//!    invalid, resurrections (matched on the values the resurrection would write), and
//!    untagged items under an adopted or resurrected parent; the parent is then the
//!    one the create would have in `M`. An adopted node takes its local position in
//!    the sibling order (the interrupted apply created it at the bottom).
//! 4. Deleted-tag fallback: an untagged item under the same parent as an unmatched base
//!    item (tagged in `base_doc`, tag missing from `local_doc`), at a nearby sibling
//!    position (at most [`REMATCH_MAX_DISTANCE`] apart), with similar text (similarity
//!    ratio >= [`REMATCH_MIN_RATIO`], or the item's text starts with the base item's:
//!    text appended to the name), is that base item edited
//!    ([`ReportEvent::Rematched`]). Same for an item that reappears where a
//!    pending-delete node was (§9.1 guard 2). Pairs are chosen greedily by best ratio,
//!    then nearest position, parents before children. When no such pair is left, an
//!    untagged item that is the only unmatched item under its parent is paired with the
//!    only unmatched base item there (still in the subtree remotely, not pasted into
//!    another file), whatever the text, if both are at the same sibling index: a tag
//!    removed together with a larger edit (§1: a delete and create would lose the
//!    node's Workflowy-only attributes). UNVERIFIED design choice: a line deleted and
//!    another typed in its place becomes a rename, the smaller loss.
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
//! `bullets`), quote added/removed (`quote-block` / `bullets`), `- ---` typed over an
//! item or replaced by text (`divider` / `bullets`), all visible children of a plain
//! item whose layout is `bullets` switched to ordered markers (`numbered`), or of a
//! `numbered` one switched back (`bullets`). Every other layout (`todo`, `h1`, `board`,
//! `p`, …) is kept as is (§4.3 table), and code, quote and divider items never take a
//! layout from their children. Checkbox forms never change layouts; new items typed
//! with `[ ]` get `todo` only if [`MergeConfig::todo_from_checkbox`]. A layout conflict
//! has no marker form: remote wins silently (UNVERIFIED design choice); if that leaves
//! a `- ---` line on a node that is not a divider, its empty name is not pushed either.
//! Read-only items (base item had the `ro` flag or the base value classifies as
//! `Tier::ReadOnly`) and placeholders (§10) are never pushed unless `force`
//! ([`ReportEvent::ReadOnlyEditRefused`]); the remote value is kept and the file is
//! re-rendered. Items typed (or moved) under an existing placeholder are refused the
//! same way: creates are lifted to the placeholder's parent, moves are dropped.
//! Guard 3 (stale echo): if the local value equals the node's value in one of
//! `prior_docs` and differs from `base_doc`'s, it is not an edit
//! ([`ReportEvent::StaleEcho`]).
//! Nodes without a base item in this file: a cross-file move (§6.4) is merged with the
//! same rules against the source file's base ([`CrossFile::bases`]: its line in the
//! source file's base render, the node's value in that file's `base_wf`, and its lines
//! in older renders for guard 3), so a stale paste never reverts a remote edit (§1);
//! without such a base its fields are never pushed. The same holds once the move is
//! applied: when a cycle merges again after a skipped write (§9, `sync`), the node is in
//! this root remotely but still has no base item here, and its line is still merged
//! against its `CrossFile::bases` entry, so a skipped edit of it becomes a §6.1 conflict
//! instead of being treated like a node hidden in the base (and dropped). Adopted nodes
//! are brought to the local values; reappearing pending deletes and other tagged nodes
//! that were hidden in the base never push fields (their text may be stale, guard 3).
//!
//! **Completion (§4.3, §6.2 table).** `[x]` on a visible item is a tick: `ticks` gets the
//! node (unless it is already completed remotely: [`ReportEvent::AlreadyCompleted`]), and
//! other edits to it and its subtree are still pushed (planned before the completion).
//! A field of a ticked node that was also changed remotely keeps the remote value with
//! no conflict marker (the node leaves the file); the local text is kept in the report
//! ([`ReportEvent::TickedConflict`]). A new item typed as `[x]` is created
//! and then completed. Ticks are never brakes. A tick of a placeholder (§10: the line
//! is or was a placeholder, or the node is an orphan shell remotely) is a write to an
//! orphan shell: refused without `force` ([`ReportEvent::ReadOnlyEditRefused`]).
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
//! the subtree removes the node from the file ([`ReportEvent::MovedOut`]), but the node
//! still exists, so its local field edits and its tick are merged and pushed where it is
//! now, exactly as for a node inside the root (§6.1; like the "edited vs completed" row
//! of §6.2: the nodes exist). Such nodes are listed in [`MergeResult::outside`], the only
//! nodes outside the root that `pushes` and `ticks` may name. A conflict on one cannot
//! be kept as a marker (the line leaves the file): remote wins and the local text is
//! reported ([`ReportEvent::MovedOutConflict`]). UNVERIFIED design choice: the design does
//! not say whether edits follow a node moved out; pushing them is the only choice that
//! loses no typed text. Local moves of it are dropped (move vs move: remote wins) and
//! local creates under it go to its nearest surviving local ancestor. A remote move in
//! adds it. A local move that would
//! create a cycle with a remote move is dropped (remote wins, [`ReportEvent::DroppedMove`]
//! with `cycle: true`). Hidden nodes are never moved, edited or deleted except as a side
//! effect the user asked for (a tick, or the delete of a visible ancestor, which
//! `deletes` reports with its hidden descendants). Deletes are decided per node: a
//! locally deleted node is kept if it or anything below it changed remotely or must
//! survive; its other deleted children are still deleted.
//! Placeholders are never deleted unless `force` ([`ReportEvent::PlaceholderDeleteRefused`]).
//! A node missing from `local_doc` whose tag is in `cross.claimed_elsewhere` moved to
//! another file: it is neither deleted nor moved here (the other file's merge moves
//! it); it stays in `M` at its remote position. So does a locally deleted (or pending)
//! node with such a node below it that was hidden in the base: its delete loses
//! ([`ReportEvent::DeleteLost`]), as when that node's line was visible.
//!
//! **Mirrors (§15.1, `crate::mirrors`).** On a view tree (base and remote alike) a
//! shell placement that renders its origin in full is an ordinary item whose edits,
//! ticks, creates and moves name the placement; `mirrors::to_real` maps them to the
//! content node afterwards. A **stub** placement (`ItemKind::MirrorStub`, `stub` on the
//! node) and an orphan shell are treated like §10 placeholders: read-only, no children,
//! items typed or moved under them refused. Deleting a shell placement is an ordinary
//! delete of that node (the planner removes just the mirror); its descendants in the
//! view are the content's children and are neither deleted nor counted. A delete is
//! refused ([`ReportEvent::MirrorDeleteRefused`]) when the deleted node, or a node below
//! it other than through a shell, is an origin that still has mirrors (live shells, or
//! entries in its registry, DM§10.5: the API would answer 400, and removing a mirror
//! that Workflowy users rely on is their call); the refused subtree is kept whole, and
//! `force` does not override this.
//!
//! **Pending deletes (§9.1 guard 2).** Nodes in `pending_deletes` that are still in the
//! remote subtree and were neither cancelled nor changed remotely are listed in
//! `deletes` again on every merge (and removed from `M`), so that `sync` can keep
//! deferring them and finally send them; they do not count in [`MergeStats`] (they were
//! counted when first deleted). A pending node that reappears in the file, or that
//! changed remotely (delete vs edit), or that left the subtree, or whose tag is in
//! `cross.claimed_elsewhere` (a cut and paste into another file of the run, which that
//! file's merge turns into a move), is in `cancelled_pending`.
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
//!   sibling order, given by `orders`), plus the pushes and ticks of the nodes in
//!   `outside`, which are not in `M`. Every pushed name/note is canonical WF-HTML.
//! * With `local_doc == base_doc` there are no pushes, creates, moves, ticks or deletes,
//!   and `M` equals the remote subtree up to priorities.
//! * Determinism: same inputs, same output (including temp id numbering).

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::apply::Adoption;
use crate::mdblock::{ConflictMarker, DocConfig, LocalDoc, LocalItem};
use crate::mdinline::ConflictField;
use crate::tree::{IdMap, Node, NodeId, ShortId, Tree};

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
    /// Nodes of the other roots whose tags are still in their own file's current text: a
    /// line carrying one here is a copy, so it is a new node (as a duplicated tag within
    /// one file, §4.4), not a move.
    #[serde(default)]
    pub still_in_own_file: BTreeSet<NodeId>,
    /// For nodes of the other roots whose tags appear in this file (a cut from another
    /// file pasted here): that file's base for them, against which the pasted line is
    /// merged. A node without an entry never has its fields pushed.
    #[serde(default)]
    pub bases: BTreeMap<NodeId, CrossBase>,
}

/// The source file's base for a node pasted into another file of the run (§6.4): the
/// pasted line is an old render from the source file, so its local changes are its
/// differences from that render (§5.1), merged with §6.1 against the node's value in
/// the source file's `base_wf`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CrossBase {
    /// The node in the source file's `base_wf`.
    pub wf: Node,
    /// The node's line in the source file's base render (`children` left empty).
    pub item: LocalItem,
    /// Its lines in older renders of the source file, for guard 3 (§9.1: a paste that
    /// arrives late may carry an older render).
    pub prior: Vec<LocalItem>,
}

/// The [`CrossBase`]s of the nodes in `wanted` that a source file shows: `base` and
/// `base_doc` are that file's merge base (§9.1 guard 1, as its own merge chooses it),
/// `prior_docs` its older renders, `ids` resolves tags. A node gets an entry only if its
/// line is tagged in `base_doc` and the node is in `base`. Pure; `sync` calls it for
/// every file of a run whose nodes another file claims.
pub fn cross_bases(
    base: &Tree,
    base_doc: &LocalDoc,
    prior_docs: &[LocalDoc],
    ids: &IdMap,
    wanted: &BTreeSet<NodeId>,
) -> BTreeMap<NodeId, CrossBase> {
    let lines = |doc: &LocalDoc| -> BTreeMap<NodeId, LocalItem> {
        let mut out = BTreeMap::new();
        for (_, it) in doc.walk() {
            if it.is_marker() {
                continue;
            }
            if let Some(n) = it.tag.as_ref().and_then(|t| ids.get(&t.short))
                && wanted.contains(n)
                && !out.contains_key(n)
            {
                let mut line = it.clone();
                line.children = Vec::new();
                out.insert(n.clone(), line);
            }
        }
        out
    };
    let priors: Vec<BTreeMap<NodeId, LocalItem>> = prior_docs.iter().map(lines).collect();
    lines(base_doc)
        .into_iter()
        .filter_map(|(n, item)| {
            let wf = base.get(&n)?.clone();
            let prior = priors.iter().filter_map(|p| p.get(&n).cloned()).collect();
            Some((n, CrossBase { wf, item, prior }))
        })
        .collect()
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
    /// Plain text of the names of the first [`HIDDEN_LABELS_MAX`] hidden descendants, in
    /// file order, for reports.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hidden_labels: Vec<String>,
}

/// How many hidden descendants of a delete the report names (§4.5).
pub const HIDDEN_LABELS_MAX: usize = 10;

impl DeleteIntent {
    /// `deleting "Old ideas" (<short id>) also deletes 1 hidden item: "Hedge maze"`, or
    /// `None` if the delete takes no hidden descendant (§4.5: the report names them).
    pub fn describe_hidden(&self) -> Option<String> {
        let n = self.hidden_descendants;
        if n == 0 {
            return None;
        }
        let mut names: Vec<String> = self.hidden_labels.iter().map(|l| quoted(l)).collect();
        if n > names.len() {
            names.push(format!("and {} more", n - names.len()));
        }
        let short = self.node.short().map(|s| s.to_string()).unwrap_or_default();
        Some(format!(
            "deleting {} ({short}) also deletes {n} hidden item{}: {}",
            quoted(&self.label),
            if n == 1 { "" } else { "s" },
            names.join(", ")
        ))
    }
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
    Resurrected {
        old: NodeId,
        new: NodeId,
        /// Plain text of the deleted node's base name (it is in no tree any more).
        #[serde(default)]
        old_name: String,
    },
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
    /// A field of a node moved out of the subtree remotely was edited on both sides:
    /// Workflowy's value is kept and, since the line leaves the file and cannot carry a
    /// conflict marker, the local text is kept here (§6.1, §6.2).
    MovedOutConflict {
        node: NodeId,
        field: ConflictField,
        local_md: String,
    },
    /// A field of a node ticked locally was edited on both sides: the completion is
    /// pushed and Workflowy's value kept (§6.2 table), and since the ticked line leaves
    /// the file, the local text is kept here.
    TickedConflict {
        node: NodeId,
        field: ConflictField,
        local_md: String,
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
    /// A tag of another file of the run that is still on its line there: the line here
    /// is a copy and became a new node (§4.4, §6.4).
    CopiedTag { short: ShortId, line: usize },
    /// A duplicated tag: `kept_line` kept the identity, the lines in `other_tag_lines`
    /// kept the node of another tag they carry (§4.4), the others became new nodes.
    DuplicateTag {
        short: ShortId,
        lines: Vec<usize>,
        kept_line: usize,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        other_tag_lines: Vec<usize>,
    },
    /// An untagged item was matched to a base node by the fallback (§4.4).
    Rematched { node: NodeId, line: usize },
    /// An untagged item was matched to a node from an interrupted apply.
    Adopted { node: NodeId, line: usize },
    /// An edit of a read-only item or placeholder was refused (§5.3, §10), or an item
    /// moved under a placeholder was. `line` 0 stands for the preamble (the root's note).
    ReadOnlyEditRefused { node: NodeId, line: usize },
    /// An item typed under a placeholder or stub (§10, §15.1) was created under
    /// `parent`, the nearest ancestor that can hold it, instead.
    Lifted {
        line: usize,
        /// Plain text of the new item's name.
        name: String,
        /// The (nearest) placeholder it was typed under.
        placeholder: NodeId,
        parent: NodeId,
    },
    /// A delete of a placeholder was refused without `force` (§10).
    PlaceholderDeleteRefused { node: NodeId },
    /// A delete was refused because `node` is, or holds, the mirror origin `origin`,
    /// which still has mirrors (§15.1, DM§10.5). Remove the mirrors in Workflowy first.
    MirrorDeleteRefused { node: NodeId, origin: NodeId },
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
    /// The typed Markdown held a link title (`[a](u "t")`), which a Workflowy link
    /// cannot store: the link was written without it.
    LinkTitleDropped { node: NodeId, field: ConflictField },
    /// A tick of a node already completed remotely was skipped (DM§4.5).
    AlreadyCompleted { node: NodeId },
    /// A pending delete was cancelled because the node reappeared (§9.1 guard 2).
    PendingDeleteCancelled { node: NodeId },
    /// Safe mode suppressed this many structural changes (§9.1 guard 1).
    SafeMode { suppressed: usize },
}

/// Where [`ReportEvent::describe_with`] finds the names of the nodes it mentions.
#[derive(Debug, Clone, Copy)]
pub struct Labels<'a> {
    /// The remote tree (after the apply, when describing a finished sync).
    pub tree: &'a Tree,
    /// Temp id -> real id of the nodes created (`ApplyOutcome::created`).
    pub created: Option<&'a BTreeMap<NodeId, NodeId>>,
    /// Looked up for nodes not (yet) in `tree`: the merged tree `M` of a dry run, where
    /// new nodes still have their temp ids.
    pub planned: Option<&'a Tree>,
}

impl<'a> Labels<'a> {
    /// Names from `tree` alone.
    pub fn new(tree: &'a Tree) -> Labels<'a> {
        Labels {
            tree,
            created: None,
            planned: None,
        }
    }

    /// `"<plain text of the name>" (<short id>)`, with `, new` for a node this sync
    /// creates; the short id alone if the node is unknown. A temp id is never shown.
    pub fn label(&self, id: &NodeId) -> String {
        let name = |n: &Node| quoted(&codec::plain_text_lossy(&n.name));
        if id.is_temp() {
            let real = self.created.and_then(|c| c.get(id));
            if let Some((real, node)) = real.and_then(|r| Some((r, self.tree.get(r)?))) {
                let short = real.short().map(|s| s.to_string()).unwrap_or_default();
                return format!("{} ({short}, new)", name(node));
            }
            return match self.planned.and_then(|t| t.get(id)) {
                Some(node) => format!("{} (new)", name(node)),
                None => "a new item".to_owned(),
            };
        }
        let short = id
            .short()
            .map(|s| s.to_string())
            .unwrap_or_else(|| id.to_string());
        match self.tree.get(id).or_else(|| self.planned?.get(id)) {
            Some(node) => format!("{} ({short})", name(node)),
            None => short,
        }
    }
}

impl ReportEvent {
    /// A one-line human-readable description (used in logs and the status note).
    /// `tree` (the remote tree) is used to name nodes by their plain-text names.
    pub fn describe(&self, tree: &Tree) -> String {
        self.describe_with(&Labels::new(tree))
    }

    /// [`ReportEvent::describe`], naming nodes from `labels`.
    pub fn describe_with(&self, labels: &Labels<'_>) -> String {
        let n = |id: &NodeId| labels.label(id);
        let f = |field: &ConflictField| match field {
            ConflictField::Name => "name",
            ConflictField::Note => "note",
        };
        match self {
            ReportEvent::Resurrected { old, new, old_name } => {
                let short = old.short().map(|s| s.to_string()).unwrap_or_default();
                format!(
                    "{} ({short}) was deleted in Workflowy but edited in Markdown: re-created as {} \
                     (the original is in Workflowy's Trash)",
                    quoted(old_name),
                    n(new)
                )
            }
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
            ReportEvent::MovedOutConflict {
                node,
                field,
                local_md,
            } => format!(
                "conflict on the {} of {}, which was moved out of this file in Workflowy: Workflowy's version kept; yours was {local_md:?}",
                f(field),
                n(node)
            ),
            ReportEvent::TickedConflict {
                node,
                field,
                local_md,
            } => format!(
                "conflict on the {} of {}, which you ticked: Workflowy's version kept; yours was {local_md:?}",
                f(field),
                n(node)
            ),
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
            ReportEvent::CopiedTag { short, line } => format!(
                "line {line}: id tag {short} is still in the file it comes from; treated as a copy (a new item)"
            ),
            ReportEvent::DuplicateTag {
                short,
                lines,
                kept_line,
                other_tag_lines,
            } => {
                let join = |ls: &[usize]| -> String {
                    let ls: Vec<String> = ls.iter().map(usize::to_string).collect();
                    ls.join(", ")
                };
                let new_items = lines.len().saturating_sub(1 + other_tag_lines.len());
                let other = match other_tag_lines.as_slice() {
                    [] => String::new(),
                    [one] => format!("line {one} keeps its other id tag"),
                    many => format!("lines {} keep their other id tags", join(many)),
                };
                let rest = match (other.is_empty(), new_items) {
                    (true, _) => "the others are new items".to_owned(),
                    (false, 0) => other,
                    (false, _) => format!("{other}, the others are new items"),
                };
                format!(
                    "id tag {short} appears on lines {}; line {kept_line} keeps it, {rest}",
                    join(lines)
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
            ReportEvent::Lifted {
                line, name, parent, ..
            } => format!(
                "line {line}: {} was typed under a read-only placeholder; created under {} instead",
                quoted(name),
                n(parent)
            ),
            ReportEvent::PlaceholderDeleteRefused { node } => format!(
                "{} is a mirror placeholder; its delete was not sent (use force)",
                n(node)
            ),
            ReportEvent::MirrorDeleteRefused { node, origin } if node == origin => format!(
                "{} still has mirrors in Workflowy; its delete was not sent (remove the mirrors first)",
                n(node)
            ),
            ReportEvent::MirrorDeleteRefused { node, origin } => format!(
                "{} contains {}, which still has mirrors in Workflowy; its delete was not sent (remove the mirrors first)",
                n(node),
                n(origin)
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
            ReportEvent::LinkTitleDropped { node, field } => format!(
                "the {} of {} was written without its link title: Workflowy links have none",
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

/// Plain text in double quotes, on one line: line breaks are shown as ` ⏎ `, as the
/// renderer does for a multi-line name.
fn quoted(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    format!("\"{}\"", lines.join(" ⏎ "))
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
    /// Nodes outside the root (moved out remotely, §6.2) whose local edits or tick are
    /// pushed where they are now: the only nodes outside `merged` that `pushes` and
    /// `ticks` may name.
    pub outside: BTreeSet<NodeId>,
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
/// not children of a placeholder (§10: an orphan shell) or stub (§15.1). The root is
/// never treated as a placeholder. UNVERIFIED design choice: this module's reading of
/// `mdblock`'s rules.
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

/// A node the merge treats as a read-only placeholder without children (see
/// [`visible_nodes`]): a §15.1 stub or an orphan shell (§10).
pub(crate) fn is_placeholder_node(node: &crate::tree::Node) -> bool {
    node.stub || node.is_orphan_shell()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn describe_names_nodes_by_plain_text() {
        let a = NodeId::from_api("00000000-0000-4000-8000-0000000000aa");
        let b = NodeId::from_api("00000000-0000-4000-8000-0000000000bb");
        let tree =
            Tree::from_nodes([Node::new(a.clone(), None, "<b>Plan</b> &amp; do")]).expect("tree");
        let short = a.short().expect("short");
        let events = vec![
            ReportEvent::Resurrected {
                old: a.clone(),
                new: NodeId::temp(1),
                old_name: "Two\nlines".to_owned(),
            },
            ReportEvent::VanishedWork {
                node: a.clone(),
                completed_ancestor: b.clone(),
            },
            ReportEvent::Appeared { node: a.clone() },
            ReportEvent::Disappeared { node: a.clone() },
            ReportEvent::MovedOut {
                node: a.clone(),
                new_parent: None,
            },
            ReportEvent::MovedOutConflict {
                node: a.clone(),
                field: ConflictField::Name,
                local_md: "typed".to_owned(),
            },
            ReportEvent::TickedConflict {
                node: a.clone(),
                field: ConflictField::Name,
                local_md: "typed".to_owned(),
            },
            ReportEvent::MovedIn {
                node: a.clone(),
                from_file_root: Some(b.clone()),
            },
            ReportEvent::DroppedMove {
                node: a.clone(),
                cycle: true,
            },
            ReportEvent::DeleteLost { node: a.clone() },
            ReportEvent::ForeignTag {
                short: short.clone(),
                line: 3,
            },
            ReportEvent::CopiedTag {
                short: short.clone(),
                line: 3,
            },
            ReportEvent::DuplicateTag {
                short,
                lines: vec![3, 9],
                kept_line: 9,
                other_tag_lines: Vec::new(),
            },
            ReportEvent::Rematched {
                node: a.clone(),
                line: 4,
            },
            ReportEvent::Adopted {
                node: a.clone(),
                line: 4,
            },
            ReportEvent::ReadOnlyEditRefused {
                node: a.clone(),
                line: 4,
            },
            ReportEvent::Lifted {
                line: 5,
                name: "typed".to_owned(),
                placeholder: b.clone(),
                parent: a.clone(),
            },
            ReportEvent::PlaceholderDeleteRefused { node: a.clone() },
            ReportEvent::MirrorDeleteRefused {
                node: a.clone(),
                origin: b.clone(),
            },
            ReportEvent::StaleEcho {
                node: a.clone(),
                field: ConflictField::Name,
            },
            ReportEvent::ResurrectionDropped {
                node: a.clone(),
                line: 4,
            },
            ReportEvent::Conflict {
                node: a.clone(),
                field: ConflictField::Note,
            },
            ReportEvent::ConflictLost {
                node: a.clone(),
                field: ConflictField::Note,
            },
            ReportEvent::LossyWrite {
                node: a.clone(),
                field: ConflictField::Name,
            },
            ReportEvent::LinkTitleDropped {
                node: a.clone(),
                field: ConflictField::Name,
            },
            ReportEvent::AlreadyCompleted { node: a.clone() },
            ReportEvent::PendingDeleteCancelled { node: a.clone() },
            ReportEvent::SafeMode { suppressed: 2 },
        ];
        for e in &events {
            let s = e.describe(&tree);
            assert!(!s.is_empty() && !s.contains('\n'), "{s}");
        }
        assert_eq!(
            events[3].describe(&tree),
            "\"Plan & do\" (0000000000aa) was completed in Workflowy and left the file"
        );
        // Unknown nodes are named by their short id.
        assert!(events[1].describe(&tree).contains("0000000000bb"));
        // A remotely deleted node is named by its base name, on one line; a temp id is
        // never shown.
        assert_eq!(
            events[0].describe(&tree),
            "\"Two ⏎ lines\" (0000000000aa) was deleted in Workflowy but edited in Markdown: \
             re-created as a new item (the original is in Workflowy's Trash)"
        );
    }

    #[test]
    fn labels_resolve_created_and_planned_nodes() {
        let a = NodeId::from_api("00000000-0000-4000-8000-0000000000aa");
        let c = NodeId::from_api("00000000-0000-4000-8000-0000000000cc");
        let t = NodeId::temp(1);
        let tree = Tree::from_nodes([
            Node::new(a.clone(), None, "Two\nline name"),
            Node::new(c.clone(), None, "a child I added"),
        ])
        .expect("tree");
        let planned =
            Tree::from_nodes([Node::new(t.clone(), None, "typed &amp; new")]).expect("tree");
        let created = BTreeMap::from([(t.clone(), c.clone())]);
        let refused = ReportEvent::ReadOnlyEditRefused {
            node: a.clone(),
            line: 5,
        };
        assert_eq!(
            refused.describe(&tree),
            "line 5: \"Two ⏎ line name\" (0000000000aa) is read-only (or a mirror placeholder); \
             the edit was not sent"
        );
        let vanished = ReportEvent::VanishedWork {
            node: t.clone(),
            completed_ancestor: a.clone(),
        };
        // After the apply: the created map gives the real node.
        let after = Labels {
            tree: &tree,
            created: Some(&created),
            planned: None,
        };
        assert!(
            vanished
                .describe_with(&after)
                .starts_with("\"a child I added\" (0000000000cc, new) was pushed but is hidden"),
            "{}",
            vanished.describe_with(&after)
        );
        // A dry run: the merged tree names it.
        let dry = Labels {
            tree: &tree,
            created: None,
            planned: Some(&planned),
        };
        assert!(
            vanished
                .describe_with(&dry)
                .starts_with("\"typed & new\" (new) was pushed"),
            "{}",
            vanished.describe_with(&dry)
        );
        assert!(!vanished.describe(&tree).contains("temp-"));
    }

    #[test]
    fn field_push_counts() {
        let mut p = FieldPush::default();
        assert!(p.is_empty());
        p.layout = Some(Change {
            expected: "bullets".into(),
            new: "todo".into(),
        });
        p.name = Some(Change {
            expected: "a".into(),
            new: "b".into(),
        });
        assert_eq!(p.count(), 2);
    }
}
