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
//!   from another root).
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
//! * the merge deletes every visible node of a non-empty base ([`Brake::EmptyFile`]);
//! * completions exceed the (higher) completion thresholds.
//!
//! # Postconditions (property tests)
//!
//! * `simulate(&plan, remote)` succeeds, and its outline below the root equals
//!   `merge.merged` with `tree::DiffScope::Visible` after mapping temp ids (they are kept
//!   as temp ids by `simulate`), ignoring nodes deferred from deletion.
//! * Hidden nodes are never the subject of a `Move` or an `Update*`.
//! * The number of reorder moves for a parent is `n − |longest block|` (§8.1).

#![allow(unused_variables, dead_code)] // stubs; remove when implemented

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::merge::MergeResult;
use crate::tree::{NodeId, Position, Tree, TreeError};

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
    todo!()
}

/// §8.1: the moves that turn the order `current` into `target` using only top and
/// bottom placements. Both are sequences of the same set of ids (visible siblings).
/// The unmoved nodes form the longest block that is contiguous in `target` and
/// increasing in `current`; nodes before it go to the top (emitted in reverse target
/// order), nodes after it go to the bottom (in target order). Cost `n − |block|`;
/// empty when `current == target`.
pub fn reorder_moves(target: &[NodeId], current: &[NodeId]) -> Vec<(NodeId, Position)> {
    todo!()
}

/// §7.2 whitelist: true only if sending `name_wf` as a create `name` stores exactly
/// `name_wf` (the create pipeline, DM§6.2, leaves it unchanged). Must be conservative:
/// non-empty; no leading or trailing whitespace (Python `str.strip` semantics, including
/// NBSP); no control characters, line or paragraph separators; none of the characters
/// `&`, `*`, `[`, `]`, backtick, `~`, `#`, `<`, `>`, `\`, `_`, `"` anywhere; does not start
/// with `-`, `+`, `>` or a digit run followed by `.`; no `http` (autolink-like text).
/// The implementer must add a test that sends whitelisted names (and near misses)
/// through the mock's create endpoint and checks the stored name.
pub fn create_name_is_safe(name_wf: &str) -> bool {
    todo!()
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
    todo!()
}

/// Apply one op to `tree` in place (the step function of [`simulate`]). `apply` calls it
/// after each successful request, with temp ids already mapped to real ids.
pub fn simulate_op(op: &Op, tree: &mut Tree) -> Result<(), TreeError> {
    todo!()
}
