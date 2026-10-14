//! # syncflowy: a Workflowy <-> Markdown sync engine
//!
//! The specification is `docs/sync-design.md` (cited as §x.y). Facts about the Workflowy
//! API are in `docs/workflowy/data-model.md` (DM§x), facts about the web UI in
//! `docs/workflowy/ui-findings.md` (UI:xx). The Python mock server (`src/syncflowy/mock`,
//! `docs/mock-server.md`) is the test oracle; [`testsupport::MockServer`] runs it.
//! `src/syncflowy/wfhtml.py` is the oracle of [`wfhtml`].
//!
//! This comment is the briefing for everyone implementing a module: it is the module map,
//! the data flow and the contract between modules. Each module's own doc comment holds
//! its detailed rules; behaviour chosen without evidence is marked `// UNVERIFIED:`.
//!
//! ## Data flow (§3)
//!
//! ```text
//!             ┌──────────── checkpoint (state) ────────────┐
//!             │  base_wf: subtree as of last sync           │
//!             │  base_md: exact text of the .md we wrote    │
//!             └──────────────┬──────────────────────────────┘
//!                            │
//!  nodes-export ──► Remote tree R ──┐                   ┌── mdblock::parse(current .md) ◄── file
//!  (wfclient)       (tree::Tree)    ├─► merge::merge ◄──┤
//!           base_wf ───────────────►┘     │             └── mdblock::parse(base_md)
//!                                         ▼
//!                                   MergeResult (M + provenance)
//!                                         │
//!                  ┌──────────────────────┼──────────────────────────────┐
//!                  ▼                      ▼                              ▼
//!         plan::plan(M vs R)     mdblock::View::build(R') + render   state::Checkpoint
//!                  │              → new .md (CAS write, sync)        (gen + 1, history)
//!         apply::Applier + Journal ──► re-export R' ──► verify ≈ M
//! ```
//!
//! **The core is pure** (§3): [`tree`], [`wfhtml`], [`mdinline`], [`mdblock`], [`merge`]
//! and [`plan`] take values and return values, with no I/O, no clock and no randomness
//! (temp ids are deterministic). I/O lives in [`wfclient`], [`apply`], [`state`],
//! [`sync`] and [`daemon`]. Property tests (`proptest`) and snapshot tests (`insta`) are
//! expected for the pure modules; I/O modules are tested against the mock.
//!
//! ## Modules
//!
//! | module | responsibility (design §§) | key public items | consumes |
//! |---|---|---|---|
//! | [`tree`] | outline model; ids; visibility; sibling order; structural edits with the API's priority algorithm (§3, §4.5, §6.3, §10, DM§1, DM§7). **Implemented.** | [`tree::NodeId`] (full UUID or `temp-<n>`), [`tree::ShortId`], [`tree::IdMap`], [`tree::Node`], [`tree::ApiNode`] (exact API JSON), [`tree::Position`], [`tree::Tree`] (`from_nodes`, `subtree`, `children`, `ancestors`, `descendants`, `is_visible`, `visible_children`, `count_hidden_descendants`, `priority_ties`, `place`, `move_node`, `remove_subtree`, `set_*`, `reorder_children`, `remap_ids`), [`tree::outline_diff`] | – |
//! | [`wfhtml`] | WF-HTML escape / parse / canonicalise / well-formedness; byte-for-byte port of `wfhtml.py` (§2, §5.4) | `escape_text`, `parse` -> [`wfhtml::Inline`] tree, `serialize` (canonical), `is_well_formed`, `canonicalize`, `to_plain_text`, `append_typed_text` | – |
//! | [`mdinline`] | WF-HTML inline <-> Markdown inline, tiers, §5.2 lossy writes, id-tag syntax (§4.2, §4.4, §5.2, §5.3) | `wf_to_md`, `md_to_wf`, `classify` -> [`mdinline::Tier`], `write_value`, `find_tags`/`format_tag` ([`mdinline::IdTag`], [`mdinline::TagForm`], `TagFlag::ReadOnly`), `InlineConfig`, the escaping contract EC | `wfhtml`, `tree` |
//! | [`mdblock`] | the file format: frontmatter, parse to [`mdblock::LocalDoc`], [`mdblock::View`] of a tree, `render` (§4.1, §4.3–§4.5, §5.3, §6.1 markers, §9.1 `gen`/controls, §10) | `parse`, `parse_frontmatter`, `render`, `normalize`, `equivalent`, `View::build`, [`mdblock::Frontmatter`], [`mdblock::DocConfig`] (headings, tag style, checkbox policy, dates, depth), `LocalItem`, `ItemKind`, `Checkbox`, `ConflictMarker`, `ViewExtras`, `Sticky` | `tree`, `mdinline` |
//! | [`merge`] | three-way merge of fields, structure, completion, order; identity repair; conflicts; report (§4.4, §4.5, §5.1, §5.2, §6, §9.1 guards 1–3, §10) | `merge(&MergeInput) -> MergeResult` (`merged` tree M with temp ids, `pushes`, `creates`, `moves`, `ticks`, `deletes` with hidden counts, `orders`, `conflicts`, `report`, `stats`), `MergeConfig`, `CrossFile`, `ReportEvent` | `tree`, `mdblock`, `mdinline`, `apply::Adoption` (type only) |
//! | [`plan`] | API ops from M vs R in §7.1 order; §8.1 reorders; §8.2 guards; §8.3 brakes; simulation | `plan(&MergeResult, &Tree, &PlanConfig) -> Result<Plan, PlanError>`, [`plan::Op`], `CreateName` (§7.2), `reorder_moves`, `create_name_is_safe`, `simulate`/`simulate_op`, `BrakeConfig`, `Brake` | `merge`, `tree` |
//! | [`apply`] | execute a plan: journal, pre-write GET checks, two-step creates with nonces, temp->real ids, verification export; crash recovery (§7.1–§7.3, §8.2) | `Applier::apply`, `Journal` (JSONL, `JournalEntry`), `ApplyOutcome`, `recover -> Recovery { adoptions }`, `Adoption` | `plan`, `wfclient`, `tree` |
//! | [`state`] | checkpoint + render history + pending deletes on disk; atomic writes; per-root lock (§3, §5.1, §9.1 guards 1–2) | `Checkpoint`, `HistoryEntry`, `HistoryLimits`, `PendingDeletes`, `fingerprint`, `StateStore` (`load_checkpoint`, `save_checkpoint`, `put_history`, `get_history`, `prune_history`, `journal_path`, `lock`), `atomic_write` | `tree`, `mdblock::DocConfig` |
//! | [`wfclient`] | blocking API client: 10 req/s, retries/backoff, full-UUID enforcement, typed errors, request log (§3, DM§1–§4) | trait [`wfclient::Api`] (`export`, `get`, `get_short`, `list`, `create`, `update`, `move_node`, `complete`, `uncomplete`, `delete`), `Client`, `ClientConfig`, `ApiError`, `CreateRequest`, `UpdateRequest` | `tree` |
//! | [`sync`] | one cycle for one file and for several files; render command; status (§1, §5.1, §6.4, §7, §8.3, §9, §9.1) | `render_once`, `sync_once`, `sync_all`, `status`, `SyncConfig`, `Clock`, `SyncOutcome`, `SyncReport` | everything above |
//! | [`daemon`] | watch mode: discovery by root, debounce, polling, guard 6 Tasks fields, status note (§9.1, §12.1, §14 step 6) | `run`, `Daemon` (`new`, `step`, `next_wakeup`, `status`), `DaemonConfig`, `Time`/`RealTime`/`FakeTime`, `discover`, `is_conflict_copy`, `Debouncer`, `strip_tasks_fields`, `StatusReport`, `render_status` | `sync`, `mdblock`, `state`, `wfclient` |
//! | [`testsupport`] | run the Python mock from tests. **Implemented.** | `MockServer::start`, `base_url`, `api_key`, `client_config`, `get`/`post`/`delete`, `reset`, `ui`, `dump`, `config`, `clock` | `wfclient::ClientConfig` |
//! | [`error`] | engine-level error aggregating module errors | `Error`, `Result` | all error types |
//!
//! ## The seams (what each side may rely on)
//!
//! * **tree -> everyone.** Names and notes are raw WF-HTML strings, never unescaped
//!   in `Tree`. Sibling order is `(priority, id)`; compare orders as id sequences, never
//!   priorities. `Tree::is_visible` is the completion rule only; `mdblock::View` adds the
//!   other hiding rules and is the single definition of "visible in the file".
//! * **mdinline -> mdblock.** `wf_to_md` output satisfies the escaping contract EC
//!   (single-line names, no line-leading block syntax, no tag-shaped text, preserved
//!   edge whitespace), so `mdblock` never escapes text itself; it only adds markers,
//!   prefixes, indentation and tags. Tags are recognised only by `mdinline::find_tags`.
//! * **mdblock -> merge.** `LocalDoc` items keep raw Markdown (`name_md`, `note_md`);
//!   merge converts with `mdinline::md_to_wf` / `write_value` using
//!   `MergeConfig::doc.inline`. Identity is the winning tag (`LocalItem::tag`), resolved
//!   through `IdMap`. Conflict markers are `LocalItem`s with
//!   `ItemKind::ConflictMarker`, attached to their parent item's node.
//! * **merge -> plan.** `MergeResult` lists exactly what to push; `merged` (M) is the
//!   expected outcome (temp ids for creates), and `orders` the visible sibling orders.
//!   Every pushed text is canonical WF-HTML.
//! * **merge -> mdblock (render).** `MergeResult::conflicts` go into
//!   `ViewExtras::conflicts`; `sync` renders from the **re-exported** remote tree (with
//!   real ids), not from M.
//! * **plan -> apply.** `Plan::ops` run in order; ids may be temp ids of earlier creates;
//!   `Update*` carry the expected remote value for the pre-write GET; `Move` carries
//!   the expected parent; `plan::simulate_op` is the shared step function for the
//!   "latest known remote tree".
//! * **apply -> merge.** After a crash, `apply::recover` returns `Adoption`s that the
//!   next merge pairs with untagged items, so a create is never duplicated.
//! * **state -> sync.** A cycle commits in this order: history entry, Markdown file
//!   (CAS), checkpoint, journal commit. The checkpoint changes only when the file is
//!   written, so the file's `gen` always names its base (§9.1 guard 1).
//!
//! ## Conventions
//!
//! * Unix seconds are `i64`. Short ids are lowercase. Everything written to Workflowy is
//!   a fixed point of `wfhtml::canonicalize`.
//! * `Debug, Clone, PartialEq, Eq` and serde are derived on plain data where possible;
//!   enums are matched exhaustively (§12.1).
//! * Never add dependencies: `Cargo.toml` already declares everything the engine needs.

pub mod apply;
pub mod daemon;
pub mod error;
pub mod mdblock;
pub mod mdinline;
pub mod merge;
pub mod plan;
pub mod state;
pub mod sync;
pub mod testsupport;
pub mod tree;
pub mod wfclient;
pub mod wfhtml;

pub use error::{Error, Result};
