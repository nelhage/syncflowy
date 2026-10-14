//! Turn a merge result into an ordered list of API operations, with the structural guards
//! and the mass-change brakes.
//!
//! Responsibility (design §7.1 order of operations, §7.2 safe create and its whitelist
//! fast path, §8.1 reordering with only top and bottom, §8.2 cycle and boundary guards,
//! §8.3 mass-change brakes). Pure: no I/O.
//!
//! Consumes: [`crate::merge::MergeResult`] (pushes, creates, moves, ticks, deletes,
//! orders, stats), the remote [`Tree`] (the full export the merge saw), and
//! [`crate::tree`]'s structural helpers (`place`, `move_node`, … for simulation).
//! Consumed by: `apply` (executes [`Plan::ops`] in order), `sync` (brakes, summary).
//!
//! # Algorithm (§7.1)
//!
//! Work on a simulated copy `S` of the remote tree, updated after every op (that is the
//! "latest known remote tree" the guards check against):
//!
//! 1. **Creates**, in `merge.creates` order (parents first), each at `Position::Bottom`
//!    of its parent (a real id or an earlier temp id). Use [`CreateName::Direct`] when
//!    [`create_name_is_safe`] holds and [`PlanConfig::fast_path_creates`] is on, else
//!    [`CreateName::Deferred`] (§7.2).
//! 2. **Moves** (re-parenting), each to `Position::Bottom` of the new parent, each guarded
//!    (§8.2, below).
//! 3. **Field updates** from `merge.pushes`, with the remote value as `expected`.
//! 4. **Reorders** (§8.1): for every parent in `merge.orders`, `T` = the target visible
//!    order and `C` = the visible order of that parent in `S` after steps 1–2 (completed
//!    siblings are never in `T` or `C` and are never moved). Emit [`reorder_moves`] as
//!    same-parent `Move` ops.
//! 5. **Completions**: `merge.ticks`, then new nodes created `ticked`.
//! 6. **Deletes**: `merge.deletes` (top-most nodes only; delete is recursive), except
//!    those in [`PlanConfig::deferred_deletes`] (§9.1 guard 2).
//!
//! # Guards (§8.2), checked against `S` for every op
//!
//! * A move's destination must not be the node or any of its descendants
//!   ([`GuardReason::Cycle`]).
//! * A move's destination must be inside one of [`PlanConfig::sync_roots`]
//!   ([`GuardReason::OutsideSyncRoots`]). Updates, completions and deletes must target
//!   nodes inside `merge.root`'s subtree, except moves' sources (cross-file moves come
//!   from another root) and the updates and completions of `merge.outside` (nodes moved
//!   out of the root remotely whose local edits follow them, §6.2), which
//!   [`Plan::outside`] passes on to `apply`.
//! * Every parent sent to the API is explicit (create and move always carry a parent).
//!
//! A guard violation is a bug or a race; the whole plan is refused
//! ([`PlanError::Guard`]). `apply` re-checks the same guards against its own latest tree.
//!
//! # Brakes (§8.3)
//!
//! Unless [`PlanConfig::force`]: refuse with [`PlanError::Brake`] when
//! * the deletes (visible + hidden descendants, from `merge.stats`) exceed
//!   [`BrakeConfig::max_deletes`], or are at least
//!   [`BrakeConfig::min_deletes_for_fraction`] and exceed
//!   [`BrakeConfig::max_delete_fraction`] of `merge.stats.base_visible`;
//! * the base is non-empty and either the file has no (non-marker) item while the merge
//!   deletes something, or the merge deletes every visible node of the base
//!   ([`Brake::EmptyFile`]);
//! * completions exceed the (higher) completion thresholds.
//!
//! # Postconditions (property tests)
//!
//! * `simulate(&plan, remote)` succeeds, and its outline below the root equals
//!   `merge.merged` with `tree::DiffScope::Visible` after mapping temp ids (they are kept
//!   as temp ids by `simulate`), ignoring nodes deferred from deletion.
//! * Hidden nodes are never the subject of a `Move` or an `Update*` unless the merge
//!   asked for it: reorders never touch them, but a node completed remotely and edited
//!   locally is still pushed (§6.2 table, `ReportEvent::VanishedWork`), so pushes and
//!   moves from the merge are taken as given. [`GuardReason::HiddenNode`] is raised for
//!   a completed node in a target order.
//! * The number of reorder moves for a parent is `n − |longest block|` (§8.1).
//!
//! # Decisions made here (not dictated by the design)
//!
//! * **Re-parenting order.** The merge lists moves in its own order, but a set of moves
//!   whose end state is acyclic can still pass through a cycle in a bad order (swap a
//!   parent and its child). The planner emits, at each step, the first pending move that
//!   passes the cycle guard against `S`; such a move always exists when the end state is
//!   acyclic (take a blocked move; the path from its node down to its destination must
//!   contain another pending node with a strictly smaller subtree, and so on). Only when
//!   every pending move is blocked is the plan refused with [`GuardReason::Cycle`]: that
//!   is a genuine cycle, including one that only appears after earlier moves.
//! * **Reorders only touch the target's members.** `C` is the visible children of the
//!   parent in `S` *restricted to the nodes of `T`*. Other visible siblings in `S` (nodes
//!   still to be ticked or deleted in steps 5–6, deferred deletes, children hidden by
//!   `mdblock::View`'s other rules) stay where they are, exactly like completed ones:
//!   top and bottom moves of the others keep the relative order exact.
//! * **Priority ties (§6.3).** `C` is `S`'s `(priority, id)` order, ties included, and
//!   the block uses positions in it as §8.1 says. So when the merge keeps a tied remote
//!   order (`T == C`), nothing is written because of the tie; a tie inside an unmoved
//!   block stays a tie, exactly as unstable as it was before the sync. (Requiring
//!   strictly increasing priorities instead would make the result tie-free, but would
//!   move tied nodes the user never touched.)
//! * **Boundary roots.** The boundary guard uses [`PlanConfig::sync_roots`] plus
//!   `merge.root` (so an empty set means "this root only"). The node of a move must lie
//!   strictly inside one of them (a root itself is never moved).
//! * **Brake counts.** Deletes are counted from `merge.stats`, which covers this
//!   cycle's local deletes whether or not they are deferred: the brake is about the
//!   local diff (§8.3), and a truncated file must stop the run at once, not ten minutes
//!   later. The [`crate::merge::DeleteIntent`]s are not summed: they also re-list the
//!   pending deletes of earlier cycles (§9.1 guard 2), which the merge leaves out of
//!   `stats` because they were counted when first deleted; counting them again would
//!   engage the brake when a pending delete falls due (with a base that no longer shows
//!   the node, a single due delete looked like an emptied file). Completions count
//!   `merge.ticks` (existing nodes); new items typed ticked are not a mass change.
//! * **Empty file.** [`Brake::EmptyFile`] also looks at the file itself
//!   (`merge.stats.local_items`), not only at the deletes that survive the merge: the
//!   delete-vs-edit rule (§6.2) and refused placeholder deletes (§10) keep nodes the
//!   file dropped, so a truncated file with one such node would otherwise slip under it.
//!   An empty file only brakes when the merge deletes something, though: §8.3 guards
//!   against a truncated file deleting nodes, and an empty file that deletes nothing
//!   loses nothing (every item was pasted into another file of the run, which moves it,
//!   §6.4; or moved, deleted or kept remotely, and the render brings kept ones back).
//!   Braking such a file would stop nothing (the destination file still moves the
//!   nodes), and since a braked cycle commits nothing, its stale base would brake it
//!   again on every later cycle and keep remote changes out of it. A split arrival of
//!   the same cut (§9.1 guard 2: the paste has not reached the other file yet) deletes
//!   the nodes, so it still brakes. UNVERIFIED design choice: §8.3 reads "if the file is
//!   ... empty, refuse" without this exception.
//! * **Consistency.** Pushes must name the remote value as `expected`; orders must list
//!   children of their parent in `S`. Anything else is [`PlanError::Inconsistent`].
//!   Redundant intents (a move to the current parent, a tick of a completed node, a
//!   delete under an already deleted node, a push of the current value) are dropped.
//! * **Creates under a node moving in.** Creates run before moves (§7.1), so a create
//!   whose parent a cross-file move brings in (§6.4) happens in the other root and the
//!   child travels with the move. The create guard therefore asks that the parent be in
//!   `merge.merged` and inside one of the managed roots in `S`.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::merge::{MergeResult, MoveIntent};
use crate::tree::{Node, NodeId, Position, Tree, TreeError};

/// How a create sends the name (§7.2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CreateName {
    /// The name is a fixed point of the create pipeline ([`create_name_is_safe`]): send
    /// it in the create request itself (1 request).
    Direct(String),
    /// Create with a nonce as the name (`apply` picks it, e.g. `⟳syncflowy:<hex>`; it is
    /// itself a fixed point of the create pipeline), then `update` the name (2 requests).
    /// The nonce lets crash recovery find a half-created node (§7.3). UNVERIFIED design
    /// choice: §7.3 suggests the nonce in the note; putting it in the name keeps a `null`
    /// note exact (the API cannot write `null` back, DM§6.4).
    Deferred(String),
}

impl CreateName {
    /// The final WF-HTML name either way.
    pub fn value(&self) -> &str {
        match self {
            CreateName::Direct(s) | CreateName::Deferred(s) => s,
        }
    }
}

/// One API operation. Node ids may be temp ids of earlier creates; `apply` maps them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Op {
    /// Create a node at the bottom of `parent` (§7.1 step 1, §7.2).
    Create {
        /// The temp id `merge` gave the node.
        temp: NodeId,
        /// Real or temp id of the parent.
        parent: NodeId,
        /// Name and how to send it.
        name: CreateName,
        /// Note, sent verbatim in the create request (`None` = omitted, stored null).
        note: Option<String>,
        /// Layout (verbatim on create, DM§4.3).
        layout: String,
    },
    /// Update the name; skip if the current remote name is not `expected` (§7.1).
    UpdateName {
        /// Node (real id).
        node: NodeId,
        /// Remote value the plan was made against.
        expected: String,
        /// New canonical WF-HTML name.
        name: String,
    },
    /// Update the note; skip if the current remote note is not `expected`.
    UpdateNote {
        /// Node (real id).
        node: NodeId,
        /// Remote value the plan was made against.
        expected: Option<String>,
        /// New canonical WF-HTML note.
        note: String,
    },
    /// Update the layout; skip if the current remote layout is not `expected`.
    UpdateLayout {
        /// Node (real id).
        node: NodeId,
        /// Remote value the plan was made against.
        expected: String,
        /// New layout.
        layout: String,
    },
    /// Move `node` under `parent` at `position` (re-parenting or reordering).
    Move {
        /// Node (real or temp id).
        node: NodeId,
        /// Destination parent (real or temp id), always explicit (§8.2).
        parent: NodeId,
        /// Top or bottom.
        position: Position,
        /// The node's parent in the simulated tree just before this op (`None` for temp
        /// nodes); `apply` may skip the move if the node's live parent differs.
        expected_parent: Option<NodeId>,
    },
    /// Complete the node (never re-completes: `apply` skips completed nodes, DM§4.5).
    Complete {
        /// Node (real or temp id).
        node: NodeId,
    },
    /// Delete the node and its subtree (to the WF Trash).
    Delete {
        /// Node (real id).
        node: NodeId,
        /// Hidden descendants deleted with it (for logs and the journal).
        hidden_descendants: usize,
    },
}

impl Op {
    /// The node the op acts on (the temp id for creates).
    pub fn node(&self) -> &NodeId {
        match self {
            Op::Create { temp, .. } => temp,
            Op::UpdateName { node, .. }
            | Op::UpdateNote { node, .. }
            | Op::UpdateLayout { node, .. }
            | Op::Move { node, .. }
            | Op::Complete { node }
            | Op::Delete { node, .. } => node,
        }
    }

    /// Requests this op costs at most (create deferred: 2; everything else: 1; the
    /// optional pre-write GET is not counted).
    pub fn requests(&self) -> usize {
        match self {
            Op::Create {
                name: CreateName::Deferred(_),
                ..
            } => 2,
            _ => 1,
        }
    }
}

/// Counts for logs and the status note.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanSummary {
    /// Create ops.
    pub creates: usize,
    /// Update ops (name, note and layout counted separately).
    pub updates: usize,
    /// Re-parenting moves.
    pub moves: usize,
    /// Same-parent reorder moves.
    pub reorders: usize,
    /// Complete ops.
    pub completes: usize,
    /// Delete ops.
    pub deletes: usize,
    /// Hidden descendants removed by the deletes.
    pub hidden_deleted: usize,
    /// Upper bound of API requests (sum of [`Op::requests`]).
    pub requests: usize,
}

/// An ordered list of operations for one sync root.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Plan {
    /// The sync root.
    pub root: NodeId,
    /// Operations, in execution order (§7.1).
    pub ops: Vec<Op>,
    /// Counts.
    pub summary: PlanSummary,
    /// Nodes outside the root that `Update*` and `Complete` ops may target
    /// (`MergeResult::outside`: moved out remotely, edited or ticked locally, §6.2).
    #[serde(default)]
    pub outside: BTreeSet<NodeId>,
}

impl Plan {
    /// True if there is nothing to do.
    pub fn is_empty(&self) -> bool {
        self.ops.is_empty()
    }
}

/// Thresholds of the mass-change brakes (§8.3). UNVERIFIED tuning; the design gives
/// "for example 20%, or more than 50 nodes".
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BrakeConfig {
    /// Absolute delete limit (visible + hidden). Default 50.
    pub max_deletes: usize,
    /// Fractional delete limit relative to the base's visible nodes. Default 0.2.
    pub max_delete_fraction: f64,
    /// The fractional delete limit applies from this many deletes on. Default 5.
    pub min_deletes_for_fraction: usize,
    /// Absolute completion limit. Default 200.
    pub max_completes: usize,
    /// Fractional completion limit. Default 0.5.
    pub max_complete_fraction: f64,
    /// The fractional completion limit applies from this many completions on. Default 10.
    pub min_completes_for_fraction: usize,
}

impl Default for BrakeConfig {
    fn default() -> Self {
        BrakeConfig {
            max_deletes: 50,
            max_delete_fraction: 0.2,
            min_deletes_for_fraction: 5,
            max_completes: 200,
            max_complete_fraction: 0.5,
            min_completes_for_fraction: 10,
        }
    }
}

/// A mass-change brake that engaged (§8.3). The run must stop before any write and ask
/// for `--force` (or `force: true` in the frontmatter).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Error)]
pub enum Brake {
    /// Too many deletes.
    #[error(
        "refusing to delete {visible} visible and {hidden} hidden nodes out of {base_visible} (use --force)"
    )]
    TooManyDeletes {
        /// Visible nodes deleted.
        visible: usize,
        /// Hidden descendants deleted with them.
        hidden: usize,
        /// Visible nodes in the base file.
        base_visible: usize,
    },
    /// Every visible node would be deleted: the file looks truncated or emptied.
    #[error("the file is empty but {base_visible} nodes were visible (use --force)")]
    EmptyFile {
        /// Visible nodes in the base file.
        base_visible: usize,
    },
    /// Too many completions.
    #[error("refusing to complete {completes} nodes out of {base_visible} (use --force)")]
    TooManyCompletes {
        /// Completions.
        completes: usize,
        /// Visible nodes in the base file.
        base_visible: usize,
    },
}

/// Why a guard refused an op (§8.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GuardReason {
    /// The move would put a node under itself or its descendant.
    Cycle,
    /// The destination is outside every sync root.
    OutsideSyncRoots,
    /// The target node is outside this root's subtree.
    OutsideRoot,
    /// The node or parent is unknown in the simulated tree.
    UnknownNode,
    /// The op would touch a hidden node (§4.5) other than by a delete of a visible
    /// ancestor or a tick.
    HiddenNode,
}

/// Planning failures.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum PlanError {
    /// A brake engaged.
    #[error("brake: {0}")]
    Brake(Brake),
    /// A guard refused an op.
    #[error("guard {reason:?} refused {op:?}")]
    Guard {
        /// The refused op.
        op: Box<Op>,
        /// Why.
        reason: GuardReason,
    },
    /// The merge result is inconsistent with the remote tree (a bug).
    #[error("inconsistent merge result: {0}")]
    Inconsistent(String),
}

/// Planner options.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlanConfig {
    /// Every sync root managed in this run (the boundary guard, §8.2).
    pub sync_roots: BTreeSet<NodeId>,
    /// Brake thresholds.
    pub brakes: BrakeConfig,
    /// Skip the brakes (`--force` / `force: true`).
    pub force: bool,
    /// Use [`CreateName::Direct`] for whitelisted names (§7.2). Default true.
    pub fast_path_creates: bool,
    /// Deletes to leave out because they are deferred (§9.1 guard 2).
    pub deferred_deletes: BTreeSet<NodeId>,
}

impl Default for PlanConfig {
    fn default() -> Self {
        PlanConfig {
            sync_roots: BTreeSet::new(),
            brakes: BrakeConfig::default(),
            force: false,
            fast_path_creates: true,
            deferred_deletes: BTreeSet::new(),
        }
    }
}

/// Build the plan (module docs). `remote` is the export the merge saw.
pub fn plan(merge: &MergeResult, remote: &Tree, cfg: &PlanConfig) -> Result<Plan, PlanError> {
    if !remote.contains(&merge.root) {
        return Err(PlanError::Inconsistent(format!(
            "sync root {} is not in the remote tree",
            merge.root
        )));
    }
    if !cfg.force {
        check_brakes(merge, &cfg.brakes).map_err(PlanError::Brake)?;
    }
    let mut roots = cfg.sync_roots.clone();
    roots.insert(merge.root.clone());
    let mut b = Builder {
        root: &merge.root,
        roots: &roots,
        s: remote.clone(),
        ops: Vec::new(),
        summary: PlanSummary::default(),
    };
    b.creates(merge, cfg)?;
    b.moves(merge)?;
    b.updates(merge)?;
    b.reorders(merge)?;
    b.completions(merge)?;
    b.deletes(merge, cfg)?;
    let mut summary = b.summary;
    summary.requests = b.ops.iter().map(Op::requests).sum();
    Ok(Plan {
        root: merge.root.clone(),
        ops: b.ops,
        summary,
        outside: merge.outside.clone(),
    })
}

/// §8.3: the brakes, evaluated on the merge's local diff (module docs, "Brake counts").
fn check_brakes(merge: &MergeResult, cfg: &BrakeConfig) -> Result<(), Brake> {
    let base_visible = merge.stats.base_visible;
    // `stats`, not the intents: re-listed pending deletes are not new (module docs).
    let visible = merge.stats.deleted_visible;
    let hidden = merge.stats.deleted_hidden;
    // "If the file is missing or empty, refuse" (§8.3), judged on the local side: the
    // file shows no item at all and deletes something, or every visible node is deleted
    // (the file was replaced by new items). The delete count alone is not enough: the
    // merge keeps a deleted node a collaborator edited (§6.2) or a placeholder (§10), so
    // an emptied file can delete fewer nodes than the base showed, and a small file is
    // under the fraction brake. An empty file that deletes nothing (its items were cut
    // into another file of the run) loses nothing and is not braked (module docs).
    let emptied = merge.stats.local_items == 0 && visible + hidden > 0;
    if base_visible > 0 && (emptied || visible >= base_visible) {
        return Err(Brake::EmptyFile { base_visible });
    }
    // Hidden descendants count toward the threshold: the user cannot see them (§4.5).
    let total = visible + hidden;
    if exceeds(
        total,
        base_visible,
        cfg.max_deletes,
        cfg.max_delete_fraction,
        cfg.min_deletes_for_fraction,
    ) {
        return Err(Brake::TooManyDeletes {
            visible,
            hidden,
            base_visible,
        });
    }
    // Ticks are not deletes, but a separate, higher threshold is cheap insurance (§8.3).
    let completes = merge.ticks.len();
    if exceeds(
        completes,
        base_visible,
        cfg.max_completes,
        cfg.max_complete_fraction,
        cfg.min_completes_for_fraction,
    ) {
        return Err(Brake::TooManyCompletes {
            completes,
            base_visible,
        });
    }
    Ok(())
}

/// `count` is over the absolute limit, or at least `min_for_fraction` and over `fraction`
/// of `base`.
fn exceeds(count: usize, base: usize, max: usize, fraction: f64, min_for_fraction: usize) -> bool {
    count > max || (count >= min_for_fraction && count as f64 > fraction * base as f64)
}

/// Which §7.1 step a move belongs to (for the summary).
#[derive(Clone, Copy, PartialEq, Eq)]
enum MoveKind {
    Reparent,
    Reorder,
}

/// The planner's state: the simulated remote tree `S` and the ops emitted so far.
struct Builder<'a> {
    root: &'a NodeId,
    roots: &'a BTreeSet<NodeId>,
    s: Tree,
    ops: Vec<Op>,
    summary: PlanSummary,
}

impl Builder<'_> {
    /// Append `op` and apply it to `S`. Guards have been checked by the caller; a tree
    /// error here means the merge result disagrees with the remote tree.
    fn emit(&mut self, op: Op) -> Result<(), PlanError> {
        simulate_op(&op, &mut self.s)
            .map_err(|e| PlanError::Inconsistent(format!("{e} (while planning {op:?})")))?;
        self.ops.push(op);
        Ok(())
    }

    fn guard(op: &Op, reason: GuardReason) -> PlanError {
        PlanError::Guard {
            op: Box::new(op.clone()),
            reason,
        }
    }

    /// `id` is the sync root or inside its subtree in `S`.
    fn in_root(&self, id: &NodeId) -> bool {
        id == self.root || self.s.is_descendant(id, self.root)
    }

    /// `id` is one of the sync roots or inside one in `S`.
    fn in_sync_roots(&self, id: &NodeId) -> bool {
        self.roots
            .iter()
            .any(|r| id == r || self.s.is_descendant(id, r))
    }

    /// `id` is strictly inside one of the sync roots in `S`.
    fn strictly_in_sync_roots(&self, id: &NodeId) -> bool {
        self.roots.iter().any(|r| self.s.is_descendant(id, r))
    }

    /// §8.2 for a move of `node` under `dest`, against `S`.
    fn check_move(&self, node: &NodeId, dest: &NodeId) -> Result<(), GuardReason> {
        if !self.s.contains(node) || !self.s.contains(dest) {
            return Err(GuardReason::UnknownNode);
        }
        // The API would answer 200 and detach the subtree (DM§4.7.5).
        if dest == node || self.s.is_descendant(dest, node) {
            return Err(GuardReason::Cycle);
        }
        // We never move nodes into parts of the account we do not manage, and never move
        // a sync root (or anything outside the managed roots) itself.
        if !self.in_sync_roots(dest) || !self.strictly_in_sync_roots(node) {
            return Err(GuardReason::OutsideSyncRoots);
        }
        Ok(())
    }

    /// Emit a move after checking the guards.
    fn emit_move(
        &mut self,
        node: &NodeId,
        parent: &NodeId,
        position: Position,
        kind: MoveKind,
    ) -> Result<(), PlanError> {
        let op = Op::Move {
            node: node.clone(),
            parent: parent.clone(),
            position,
            expected_parent: if node.is_temp() {
                None
            } else {
                self.s.parent(node).cloned()
            },
        };
        self.check_move(node, parent)
            .map_err(|r| Self::guard(&op, r))?;
        self.emit(op)?;
        match kind {
            MoveKind::Reparent => self.summary.moves += 1,
            MoveKind::Reorder => self.summary.reorders += 1,
        }
        Ok(())
    }

    /// §7.1 step 1 and §7.2: creates in the merge's order (parents first), each at the
    /// bottom of its parent, which in pre-order gives the right order with no extra moves.
    fn creates(&mut self, merge: &MergeResult, cfg: &PlanConfig) -> Result<(), PlanError> {
        for n in &merge.creates {
            let name = if cfg.fast_path_creates && create_name_is_safe(&n.name) {
                CreateName::Direct(n.name.clone())
            } else {
                CreateName::Deferred(n.name.clone())
            };
            let op = Op::Create {
                temp: n.temp.clone(),
                parent: n.parent.clone(),
                name,
                note: n.note.clone(),
                layout: n.layout.clone(),
            };
            if !self.s.contains(&n.parent) {
                return Err(Self::guard(&op, GuardReason::UnknownNode));
            }
            // The parent belongs to this root in M. Creates run before moves (§7.1), so a
            // parent that a cross-file move brings in (§6.4) is still in its old root:
            // in S it only has to be inside one of the managed roots.
            if !merge.merged.contains(&n.parent) || !self.in_sync_roots(&n.parent) {
                return Err(Self::guard(&op, GuardReason::OutsideRoot));
            }
            if self.s.contains(&n.temp) {
                return Err(PlanError::Inconsistent(format!(
                    "create {} reuses an existing id",
                    n.temp
                )));
            }
            self.emit(op)?;
            self.summary.creates += 1;
        }
        Ok(())
    }

    /// §7.1 step 2 and §8.2: re-parentings, each to the bottom of its new parent, each
    /// guarded against `S`. Order: the first pending move that passes the cycle guard
    /// (module docs).
    fn moves(&mut self, merge: &MergeResult) -> Result<(), PlanError> {
        let mut pending: Vec<&MoveIntent> = Vec::new();
        for m in &merge.moves {
            if !self.s.contains(&m.node) || !self.s.contains(&m.to) {
                let op = Op::Move {
                    node: m.node.clone(),
                    parent: m.to.clone(),
                    position: Position::Bottom,
                    expected_parent: self.s.parent(&m.node).cloned(),
                };
                return Err(Self::guard(&op, GuardReason::UnknownNode));
            }
            // Already there: the reorder step places it.
            if self.s.parent(&m.node) != Some(&m.to) {
                pending.push(m);
            }
        }
        while !pending.is_empty() {
            let next = pending
                .iter()
                .position(|m| self.check_move(&m.node, &m.to) != Err(GuardReason::Cycle))
                .unwrap_or(0);
            let m = pending.remove(next);
            self.emit_move(&m.node, &m.to, Position::Bottom, MoveKind::Reparent)?;
        }
        Ok(())
    }

    /// §7.1 step 3: field updates, with the remote value as `expected` for the pre-write
    /// GET. Name, note, layout per node, nodes in id order.
    fn updates(&mut self, merge: &MergeResult) -> Result<(), PlanError> {
        for (id, push) in &merge.pushes {
            let Some(cur) = self.s.get(id).cloned() else {
                let op = Op::UpdateName {
                    node: id.clone(),
                    expected: String::new(),
                    name: String::new(),
                };
                return Err(Self::guard(&op, GuardReason::UnknownNode));
            };
            let mut ops = Vec::new();
            if let Some(c) = &push.name {
                check_expected(id, "name", &c.expected, &cur.name)?;
                if c.new != cur.name {
                    ops.push(Op::UpdateName {
                        node: id.clone(),
                        expected: c.expected.clone(),
                        name: c.new.clone(),
                    });
                }
            }
            if let Some(c) = &push.note {
                check_expected(id, "note", &c.expected, &cur.note)?;
                let Some(new) = &c.new else {
                    // The API cannot write null (DM§6.4).
                    return Err(PlanError::Inconsistent(format!(
                        "push of a null note to {id}"
                    )));
                };
                if c.new != cur.note {
                    ops.push(Op::UpdateNote {
                        node: id.clone(),
                        expected: c.expected.clone(),
                        note: new.clone(),
                    });
                }
            }
            if let Some(c) = &push.layout {
                check_expected(id, "layout", &c.expected, &cur.layout)?;
                if c.new != cur.layout {
                    ops.push(Op::UpdateLayout {
                        node: id.clone(),
                        expected: c.expected.clone(),
                        layout: c.new.clone(),
                    });
                }
            }
            for op in ops {
                if !self.in_root(id) && !merge.outside.contains(id) {
                    return Err(Self::guard(&op, GuardReason::OutsideRoot));
                }
                self.emit(op)?;
                self.summary.updates += 1;
            }
        }
        Ok(())
    }

    /// §7.1 step 4 and §8.1: same-parent top/bottom moves that realise `merge.orders`
    /// over the visible members of each target order. Parents in pre-order of `S`.
    fn reorders(&mut self, merge: &MergeResult) -> Result<(), PlanError> {
        let mut parents: Vec<NodeId> = self
            .s
            .preorder(self.root)
            .into_iter()
            .filter(|p| merge.orders.contains_key(p))
            .collect();
        let seen: BTreeSet<&NodeId> = parents.iter().collect();
        let rest: Vec<NodeId> = merge
            .orders
            .keys()
            .filter(|p| !seen.contains(p))
            .cloned()
            .collect();
        for p in &rest {
            if !self.s.contains(p) {
                return Err(PlanError::Inconsistent(format!(
                    "order given for {p}, which is not in the tree"
                )));
            }
        }
        parents.extend(rest);
        for p in &parents {
            let target = &merge.orders[p];
            let members: BTreeSet<&NodeId> = target.iter().collect();
            if members.len() != target.len() {
                return Err(PlanError::Inconsistent(format!(
                    "order of {p} lists a node twice"
                )));
            }
            for t in target {
                if self.s.parent(t) != Some(p) {
                    return Err(PlanError::Inconsistent(format!(
                        "order of {p} lists {t}, which is not its child"
                    )));
                }
                if self.s.get(t).is_some_and(|n| n.completed) {
                    // Completed siblings are never in T and never moved (§4.5, §8.1).
                    let op = Op::Move {
                        node: t.clone(),
                        parent: p.clone(),
                        position: Position::Top,
                        expected_parent: Some(p.clone()),
                    };
                    return Err(Self::guard(&op, GuardReason::HiddenNode));
                }
            }
            // C: the visible children of p in S, restricted to T's members (module docs).
            let current: Vec<NodeId> = self
                .s
                .visible_children(p)
                .into_iter()
                .filter(|c| members.contains(c))
                .collect();
            let moves = reorder_moves(target, &current);
            if let Some((node, position)) = moves.first()
                && !self.in_root(p)
            {
                let op = Op::Move {
                    node: node.clone(),
                    parent: p.clone(),
                    position: *position,
                    expected_parent: Some(p.clone()),
                };
                return Err(Self::guard(&op, GuardReason::OutsideRoot));
            }
            for (node, position) in moves {
                self.emit_move(&node, p, position, MoveKind::Reorder)?;
            }
        }
        Ok(())
    }

    /// §7.1 step 5: ticks, then new nodes typed ticked. After every edit, so a
    /// half-applied sync never hides work that was not pushed yet.
    fn completions(&mut self, merge: &MergeResult) -> Result<(), PlanError> {
        let new_ticked = merge.creates.iter().filter(|n| n.ticked).map(|n| &n.temp);
        for id in merge.ticks.iter().chain(new_ticked) {
            let op = Op::Complete { node: id.clone() };
            let Some(node) = self.s.get(id) else {
                return Err(Self::guard(&op, GuardReason::UnknownNode));
            };
            if node.completed {
                // The API never re-completes (DM§4.5); nothing to do.
                continue;
            }
            if !self.in_root(id) && !merge.outside.contains(id) {
                return Err(Self::guard(&op, GuardReason::OutsideRoot));
            }
            self.emit(op)?;
            self.summary.completes += 1;
        }
        Ok(())
    }

    /// §7.1 step 6: deletes, top-most only (delete is recursive), minus deferred ones
    /// (§9.1 guard 2).
    fn deletes(&mut self, merge: &MergeResult, cfg: &PlanConfig) -> Result<(), PlanError> {
        let mut removed: BTreeSet<NodeId> = BTreeSet::new();
        for d in &merge.deletes {
            if cfg.deferred_deletes.contains(&d.node) {
                continue;
            }
            if !self.s.contains(&d.node) {
                if removed.contains(&d.node) {
                    continue; // below an earlier delete
                }
                let op = Op::Delete {
                    node: d.node.clone(),
                    hidden_descendants: d.hidden_descendants,
                };
                return Err(Self::guard(&op, GuardReason::UnknownNode));
            }
            let op = Op::Delete {
                node: d.node.clone(),
                hidden_descendants: self.s.count_hidden_descendants(&d.node),
            };
            if &d.node == self.root || !self.in_root(&d.node) {
                return Err(Self::guard(&op, GuardReason::OutsideRoot));
            }
            removed.extend(self.s.preorder(&d.node));
            if let Op::Delete {
                hidden_descendants, ..
            } = &op
            {
                self.summary.hidden_deleted += hidden_descendants;
            }
            self.emit(op)?;
            self.summary.deletes += 1;
        }
        Ok(())
    }
}

fn check_expected<T: PartialEq + std::fmt::Debug>(
    id: &NodeId,
    field: &str,
    expected: &T,
    current: &T,
) -> Result<(), PlanError> {
    if expected == current {
        Ok(())
    } else {
        Err(PlanError::Inconsistent(format!(
            "push of {field} of {id} expects {expected:?} but the remote has {current:?}"
        )))
    }
}

/// §8.1: the moves that turn the order `current` into `target` using only top and
/// bottom placements. Both are sequences of the same set of ids (visible siblings).
/// The unmoved nodes form the longest block that is contiguous in `target` and
/// increasing in `current`; nodes before it go to the top (emitted in reverse target
/// order), nodes after it go to the bottom (in target order). Cost `n − |block|`;
/// empty when `current == target`.
///
/// Ids of `target` missing from `current` are never part of the block (they are moved);
/// ids of `current` missing from `target` are ignored (left in place).
pub fn reorder_moves(target: &[NodeId], current: &[NodeId]) -> Vec<(NodeId, Position)> {
    let pos: BTreeMap<&NodeId, usize> = current.iter().enumerate().map(|(i, id)| (id, i)).collect();
    let ranks: Vec<Option<usize>> = target.iter().map(|id| pos.get(id).copied()).collect();
    let (start, len) = longest_block(&ranks);
    let mut out = Vec::with_capacity(target.len() - len);
    // Before the block: to the top, last first, so they end up in target order.
    for id in target[..start].iter().rev() {
        out.push((id.clone(), Position::Top));
    }
    // After the block: to the bottom, in target order.
    for id in &target[start + len..] {
        out.push((id.clone(), Position::Bottom));
    }
    out
}

/// The first longest run of strictly increasing positions (`None`: absent from the
/// current order, never in a block): `(start, len)`. A linear scan (§8.1).
fn longest_block(ranks: &[Option<usize>]) -> (usize, usize) {
    let (mut best_start, mut best_len) = (0, 0);
    let mut i = 0;
    while i < ranks.len() {
        if ranks[i].is_none() {
            i += 1;
            continue;
        }
        let start = i;
        i += 1;
        while i < ranks.len() && matches!((ranks[i - 1], ranks[i]), (Some(a), Some(b)) if b > a) {
            i += 1;
        }
        if i - start > best_len {
            (best_start, best_len) = (start, i - start);
        }
    }
    (best_start, best_len)
}

/// Characters never accepted by [`create_name_is_safe`]: entities (`&`), the inline
/// markdown of DM§6.2 step 10 (`*`, `[`, `]`, backtick, `~`, and `\` escapes), heading
/// prefixes (`#`), markup and quotes (`<`, `>`, `"`) and `_` (not converted today, but
/// markdown-shaped; conservative).
const CREATE_RISKY_CHARS: &[char] = &['&', '*', '[', ']', '`', '~', '#', '<', '>', '\\', '_', '"'];

/// §7.2 whitelist: true only if sending `name_wf` as a create `name` stores exactly
/// `name_wf` (the create pipeline, DM§6.2, leaves it unchanged). Must be conservative:
/// non-empty; no leading or trailing whitespace (Python `str.strip` semantics, including
/// NBSP); no control characters, line or paragraph separators; none of the characters
/// `&`, `*`, `[`, `]`, backtick, `~`, `#`, `<`, `>`, `\`, `_`, `"` anywhere; does not start
/// with `-`, `+`, `>` or a digit run followed by `.`; no `http` (autolink-like text).
///
/// Checked differentially against the mock's `text.py` and through the mock's create
/// endpoint (`tests/plan_create_oracle.rs`). Why each rule holds, by pipeline step
/// (DM§6.2): no control characters (step 1 strip; step 6 splits on `\v`, `\f`,
/// `\x1c`–`\x1e`, `\x85`); no `&` (step 2 `html.unescape`); no `\r`/`\n` (steps 3–5 join
/// and split); no whitespace other than U+0020 anywhere and no space at either end
/// (step 6 splits on U+2028/U+2029; step 7 `str.strip` also strips NBSP, U+2003,
/// U+3000); no backtick (fences, code); no `-`/`#` at the start (step 8 prefixes, step 9
/// completion leak); no `*`, `` ` ``, `~`, `[`, `]`, `\` (step 10 inline markdown, step 11
/// dates, which need `[`).
pub fn create_name_is_safe(name_wf: &str) -> bool {
    // UNVERIFIED: checked against the mock's model of the create pipeline, which DM§6.2
    // fitted to ~200 observed cases; production was not probed with these exact rules.
    // Rejecting everything markdown- or entity-shaped leaves a wide margin.
    if name_wf.is_empty() || name_wf.starts_with(' ') || name_wf.ends_with(' ') {
        return false;
    }
    let bad_char = |c: char| {
        c.is_control() || (c.is_whitespace() && c != ' ') || CREATE_RISKY_CHARS.contains(&c)
    };
    if name_wf.chars().any(bad_char) {
        return false;
    }
    if name_wf.starts_with(['-', '+', '>']) {
        return false;
    }
    // "1." looks like an ordered-list item (plain today, DM§6.2 step 8; conservative).
    let after_digits = name_wf.trim_start_matches(char::is_numeric);
    if after_digits.len() < name_wf.len() && after_digits.starts_with('.') {
        return false;
    }
    // Autolink-like text (not converted today, DM§6.2 step 10; conservative).
    if name_wf.to_lowercase().contains("http") {
        return false;
    }
    true
}

/// Simulation failures.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SimError {
    /// A structural op failed on the tree.
    #[error("op {index}: {source}")]
    Tree {
        /// Index into `plan.ops`.
        index: usize,
        /// The tree error.
        source: TreeError,
    },
}

/// Apply `plan` to a copy of `tree` with the API's semantics (DM§4: creates and moves
/// use `Tree::place`/`Tree::move_node` priorities; updates set fields; complete sets
/// `completed` and leaves `completed_at: None`; delete removes the subtree). Created
/// nodes keep their temp ids. Used for property tests and by `apply` as its latest known
/// remote tree.
pub fn simulate(plan: &Plan, tree: &Tree) -> Result<Tree, SimError> {
    let mut t = tree.clone();
    for (index, op) in plan.ops.iter().enumerate() {
        simulate_op(op, &mut t).map_err(|source| SimError::Tree { index, source })?;
    }
    Ok(t)
}

/// Apply one op to `tree` in place (the step function of [`simulate`]). `apply` calls it
/// after each successful request, with temp ids already mapped to real ids.
///
/// A move into the node itself or a descendant fails with [`TreeError::Cycle`] instead
/// of reproducing the API's detached cycle (DM§4.7.5); the planner never emits one.
/// Timestamps (`createdAt`, `modifiedAt`, `completedAt`) are not simulated.
pub fn simulate_op(op: &Op, tree: &mut Tree) -> Result<(), TreeError> {
    match op {
        Op::Create {
            temp,
            parent,
            name,
            note,
            layout,
        } => {
            // The final name either way: a deferred create's nonce is replaced by the
            // update that follows it (§7.2).
            let mut node = Node::new(temp.clone(), Some(parent.clone()), name.value());
            node.note = note.clone();
            // Verbatim, except that an empty layoutMode gives `bullets` (DM§4.3; the
            // names the planner sends directly or as a nonce have no prefix layout).
            if !layout.is_empty() {
                node.layout = layout.clone();
            }
            tree.place(node, Some(parent), Position::Bottom)
        }
        Op::UpdateName { node, name, .. } => tree.set_name(node, name.clone()),
        Op::UpdateNote { node, note, .. } => tree.set_note(node, Some(note.clone())),
        Op::UpdateLayout { node, layout, .. } => tree.set_layout(node, layout.clone()),
        Op::Move {
            node,
            parent,
            position,
            ..
        } => tree.move_node(node, Some(parent), *position),
        Op::Complete { node } => {
            let already = tree
                .get(node)
                .ok_or_else(|| TreeError::UnknownNode(node.clone()))?
                .completed;
            if already {
                Ok(()) // never re-completes (DM§4.5)
            } else {
                tree.set_completed(node, true, None)
            }
        }
        Op::Delete { node, .. } => tree.remove_subtree(node).map(|_| ()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn ids(n: usize) -> Vec<NodeId> {
        (0..n)
            .map(|i| NodeId::from_api(format!("00000000-0000-4000-8000-{i:012x}")))
            .collect()
    }

    /// Apply `moves` to a plain sequence with top/bottom semantics.
    fn apply_moves(current: &[NodeId], moves: &[(NodeId, Position)]) -> Vec<NodeId> {
        let mut v = current.to_vec();
        for (id, pos) in moves {
            v.retain(|x| x != id);
            match pos {
                Position::Top => v.insert(0, id.clone()),
                Position::Bottom => v.push(id.clone()),
            }
        }
        v
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

    fn cost(target: &[NodeId], current: &[NodeId]) -> usize {
        let moves = reorder_moves(target, current);
        assert_eq!(
            apply_moves(current, &moves),
            target,
            "moves realise the target"
        );
        moves.len()
    }

    // ----- the §8.1 cost table ---------------------------------------------------------

    #[test]
    fn cost_append_is_zero() {
        // A new node created at the bottom (§7.1) that belongs at the end.
        for n in 0..8 {
            let order = ids(n + 1);
            assert_eq!(cost(&order, &order), 0);
        }
    }

    #[test]
    fn cost_insert_at_k_is_min_k_n_minus_k() {
        // A new node x created at the bottom (§7.1) that belongs at 1-based position k
        // of the final list of n siblings (k − 1 siblings before it, n − k after): move x
        // to the top and the k − 1 before it above it (k moves), or the n − k after it
        // to the bottom (n − k moves).
        for n in 1..10 {
            for k in 1..=n {
                let old = ids(n - 1);
                let x = NodeId::from_api("ffffffff-0000-4000-8000-000000000000");
                let mut current = old.clone();
                current.push(x.clone());
                let mut target = old.clone();
                target.insert(k - 1, x.clone());
                assert_eq!(cost(&target, &current), k.min(n - k), "n={n} k={k}");
            }
        }
    }

    #[test]
    fn cost_end_to_front_is_one() {
        for n in 2..10 {
            let current = ids(n);
            let mut target = current.clone();
            let last = target.pop().unwrap();
            target.insert(0, last);
            assert_eq!(cost(&target, &current), 1);
        }
    }

    #[test]
    fn cost_front_into_middle_is_min_k_n_minus_k() {
        // The first item moved to 0-based index k of n.
        for n in 2..10 {
            for k in 1..n {
                let current = ids(n);
                let mut target = current.clone();
                let first = target.remove(0);
                target.insert(k, first);
                assert_eq!(cost(&target, &current), k.min(n - k), "n={n} k={k}");
            }
        }
    }

    #[test]
    fn cost_reverse_is_n_minus_one() {
        for n in 1..10 {
            let current = ids(n);
            let mut target = current.clone();
            target.reverse();
            assert_eq!(cost(&target, &current), n - 1);
        }
    }

    #[test]
    fn identical_orders_need_no_moves() {
        assert!(reorder_moves(&[], &[]).is_empty());
        let o = ids(5);
        assert!(reorder_moves(&o, &o).is_empty());
    }

    #[test]
    fn moves_before_block_go_top_in_reverse_after_go_bottom_in_order() {
        let [a, b, c, d, e] = <[NodeId; 5]>::try_from(ids(5)).unwrap();
        let current = vec![a.clone(), b.clone(), c.clone(), d.clone(), e.clone()];
        // Target d e b c a: blocks "d e" (positions 3, 4) and "b c" (1, 2) tie; the first
        // one in target order wins.
        let target = vec![d.clone(), e.clone(), b.clone(), c.clone(), a.clone()];
        assert_eq!(
            reorder_moves(&target, &current),
            vec![
                (b.clone(), Position::Bottom),
                (c.clone(), Position::Bottom),
                (a.clone(), Position::Bottom)
            ]
        );
        // Target c a b d e: "a b d e" is increasing (0, 1, 3, 4).
        let target = vec![c.clone(), a.clone(), b.clone(), d.clone(), e.clone()];
        assert_eq!(
            reorder_moves(&target, &current),
            vec![(c.clone(), Position::Top)]
        );
        // Two before the block: emitted in reverse so they end up in target order.
        let target = vec![d.clone(), e.clone(), a.clone(), b.clone(), c.clone()];
        assert_eq!(
            reorder_moves(&target, &current),
            vec![(e.clone(), Position::Top), (d.clone(), Position::Top)]
        );
    }

    #[test]
    fn unknown_target_ids_are_moved() {
        let [a, b, c] = <[NodeId; 3]>::try_from(ids(3)).unwrap();
        let moves = reorder_moves(&[a.clone(), b.clone(), c.clone()], &[a.clone(), c.clone()]);
        assert_eq!(
            moves,
            vec![(b.clone(), Position::Bottom), (c.clone(), Position::Bottom)]
        );
    }

    fn permutation_pair() -> impl Strategy<Value = (Vec<NodeId>, Vec<NodeId>)> {
        (0usize..14).prop_flat_map(|n| {
            let base = ids(n);
            (Just(base.clone()).prop_shuffle(), Just(base).prop_shuffle())
        })
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(512))]

        /// §8.1: the moves realise the target, cost n − |longest valid block| ≤ n − 1.
        #[test]
        fn reorder_moves_realise_target_at_optimal_block_cost(
            (target, current) in permutation_pair()
        ) {
            let moves = reorder_moves(&target, &current);
            prop_assert_eq!(apply_moves(&current, &moves), target.clone());
            let n = target.len();
            prop_assert!(moves.len() <= n.saturating_sub(1));
            prop_assert_eq!(moves.len(), n - longest_valid_block(&target, &current));
            // Every node is moved at most once.
            let moved: BTreeSet<&NodeId> = moves.iter().map(|(id, _)| id).collect();
            prop_assert_eq!(moved.len(), moves.len());
        }

        /// Hidden siblings interleaved anywhere stay in place and the visible order is
        /// exact, when the moves run on a real tree with the API's priority algorithm.
        #[test]
        fn reorder_moves_on_a_tree_keep_hidden_siblings_in_place(
            (target, current) in permutation_pair(),
            hidden_slots in proptest::collection::vec(0usize..15, 0..6),
        ) {
            let parent = NodeId::from_api("aaaaaaaa-0000-4000-8000-000000000000");
            let mut nodes = vec![Node::new(parent.clone(), None, "p")];
            let mut full: Vec<(NodeId, bool)> =
                current.iter().map(|id| (id.clone(), false)).collect();
            for (i, slot) in hidden_slots.iter().enumerate() {
                let h = NodeId::from_api(format!("bbbbbbbb-0000-4000-8000-{i:012x}"));
                full.insert((*slot).min(full.len()), (h, true));
            }
            for (i, (id, hidden)) in full.iter().enumerate() {
                let mut n = Node::new(id.clone(), Some(parent.clone()), "");
                n.priority = 100 * (i as i64 + 1);
                n.completed = *hidden;
                nodes.push(n);
            }
            let mut tree = Tree::from_nodes(nodes).unwrap();
            let hidden_of = |t: &Tree| -> Vec<NodeId> {
                t.children(&parent)
                    .iter()
                    .filter(|c| t.get(c).unwrap().completed)
                    .cloned()
                    .collect()
            };
            let hidden_before = hidden_of(&tree);
            for (id, pos) in reorder_moves(&target, &current) {
                tree.move_node(&id, Some(&parent), pos).unwrap();
            }
            prop_assert_eq!(tree.visible_children(&parent), target);
            prop_assert_eq!(hidden_of(&tree), hidden_before);
        }
    }

    // ----- the create-name whitelist (more in tests/plan_create_oracle.rs) -------------

    #[test]
    fn whitelist_accepts_plain_names() {
        for s in [
            "Plain",
            "two words",
            "Buy milk, eggs and bread.",
            "Résumé – naïve café ✓ 中文",
            "a  b",
            "What? Yes! (maybe)",
            "100% of 3/4",
            "x - y",
            "1 2 3",
        ] {
            assert!(create_name_is_safe(s), "{s:?}");
        }
    }

    #[test]
    fn whitelist_rejects_risky_names() {
        for s in [
            "",
            " lead",
            "trail ",
            "\u{a0}nbsp",
            "nbsp\u{a0}",
            "inner\u{a0}nbsp",
            "line\nbreak",
            "cr\rx",
            "tab\tx",
            "ls\u{2028}x",
            "ps\u{2029}x",
            "nel\u{85}x",
            "vt\u{b}x",
            "- item",
            "- [x] done",
            "- [ ] todo",
            "-dash",
            "+ plus",
            "> quote",
            "# heading",
            "1. first",
            "12.5",
            "a &amp; b",
            "&#42;x&#42;",
            "**bold**",
            "*it*",
            "`code`",
            "~~strike~~",
            "[link](x)",
            "[2026-01-15]",
            "a\\*b",
            "snake_case",
            "say \"hi\"",
            "<b>x</b>",
            "see http://x",
            "HTTPS://X",
        ] {
            assert!(!create_name_is_safe(s), "{s:?}");
        }
    }

    // ----- simulation -----------------------------------------------------------------

    #[test]
    fn simulate_op_semantics() {
        let p = NodeId::from_api("aaaaaaaa-0000-4000-8000-000000000000");
        let a = NodeId::from_api("aaaaaaaa-0000-4000-8000-000000000001");
        let mut root = Node::new(p.clone(), None, "p");
        root.priority = 100;
        let mut na = Node::new(a.clone(), Some(p.clone()), "a");
        na.priority = 100;
        let mut t = Tree::from_nodes([root, na]).unwrap();
        let x = NodeId::temp(1);
        let create = Op::Create {
            temp: x.clone(),
            parent: p.clone(),
            name: CreateName::Deferred("X &amp; y".into()),
            note: Some("n".into()),
            layout: "todo".into(),
        };
        simulate_op(&create, &mut t).unwrap();
        assert_eq!(t.children(&p), &[a.clone(), x.clone()]);
        let n = t.get(&x).unwrap();
        assert_eq!(
            (n.name.as_str(), n.note.as_deref(), n.layout.as_str()),
            ("X &amp; y", Some("n"), "todo")
        );
        let to_top = Op::Move {
            node: x.clone(),
            parent: p.clone(),
            position: Position::Top,
            expected_parent: None,
        };
        simulate_op(&to_top, &mut t).unwrap();
        assert_eq!(t.children(&p), &[x.clone(), a.clone()]);
        simulate_op(&Op::Complete { node: a.clone() }, &mut t).unwrap();
        assert!(t.get(&a).unwrap().completed);
        assert_eq!(t.get(&a).unwrap().completed_at, None);
        // Completing again is a no-op, not an error.
        simulate_op(&Op::Complete { node: a.clone() }, &mut t).unwrap();
        let cycle = Op::Move {
            node: p.clone(),
            parent: x.clone(),
            position: Position::Bottom,
            expected_parent: None,
        };
        assert!(matches!(
            simulate_op(&cycle, &mut t),
            Err(TreeError::Cycle { .. })
        ));
        let delete = Op::Delete {
            node: x.clone(),
            hidden_descendants: 0,
        };
        simulate_op(&delete, &mut t).unwrap();
        assert!(!t.contains(&x));
        let plan = Plan {
            root: p.clone(),
            ops: vec![Op::Complete { node: x.clone() }],
            summary: PlanSummary::default(),
            outside: Default::default(),
        };
        assert_eq!(
            simulate(&plan, &t),
            Err(SimError::Tree {
                index: 0,
                source: TreeError::UnknownNode(x)
            })
        );
    }

    #[test]
    fn brake_threshold_arithmetic() {
        // > max, or >= min and > fraction * base.
        assert!(!exceeds(50, 1000, 50, 0.2, 5));
        assert!(exceeds(51, 1000, 50, 0.2, 5));
        assert!(!exceeds(4, 10, 50, 0.2, 5));
        assert!(exceeds(5, 10, 50, 0.2, 5));
        assert!(!exceeds(5, 25, 50, 0.2, 5));
        assert!(exceeds(6, 25, 50, 0.2, 5));
    }
}
