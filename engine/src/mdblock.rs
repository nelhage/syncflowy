//! Tree <-> Markdown document: the file format, its parser and its renderer.
//!
//! Responsibility (design §4.1 shape, §4.3 node kinds and completion, §4.4 id tags,
//! §4.5 projections, §5.3 read-only forms, §5.4 convergence, §6.1 conflict markers,
//! §9.1 guard 1 (`gen`) and the frontmatter controls, §10 placeholders). Pure: no I/O.
//!
//! Consumes: [`crate::tree`] (`Tree`, `NodeId`, `ShortId`, visibility), [`crate::mdinline`]
//! (`wf_to_md`/`md_to_wf`, tiers, `find_tags`/`format_tag`, the escaping contract EC).
//! Consumed by: `merge` (parses base and current files into [`LocalDoc`]s; uses
//! [`View`] for remote visibility), `sync` (builds a [`View`] from the fresh export and
//! [`render`]s it), `state` ([`DocConfig`] is echoed in the checkpoint), `daemon`
//! ([`parse_frontmatter`] for discovery).
//!
//! # The file (§4.1)
//!
//! ```markdown
//! ---
//! workflowy: a846cfce4c6d
//! syncflowy: {headings: 1, gen: 412}
//! tags: [other, keys, are, preserved]
//! ---
//! Free text here is the sync root's note.
//!
//! # Planning %%wf:1a2b3c4d5e6f%%
//! Note on "Planning": lines right under the heading.
//!
//! - Draft the spec ^wf-8e1f00aa2b3c
//!     A note line on "Draft the spec".
//!     - [ ] Write the mock ^wf-0c0ffee12345
//!     - ⚠ my local wording %%wf-conflict:name%%
//! - ``` ^wf-1234567890ab
//!     x = 1
//!     ```
//! - ⧉ (empty or mirror) ^wf-77aa12cd0e9f-ro
//! - 🔒 best-effort text of a malformed name ^wf-0e9f77aa12cd-ro
//! ```
//!
//! (Indentation is shown as four spaces here; the renderer writes one tab per level.)
//!
//! ## Frontmatter
//!
//! Must start on line 1 (`---` … `---`). Keys the engine owns:
//!
//! * `workflowy:` the sync root: a 12-hex short id, a full UUID, or a
//!   `https://workflowy.com/#/<short>` URL (a trailing `# comment` is ignored). Required.
//! * `syncflowy:` a flow mapping `{k: v, …}` (also accepted as an indented block
//!   mapping) with the keys, written in this order and only when not default:
//!   `headings` (0–6, default 0), `ids` (`auto` | `block` | `comment` | `dataview` |
//!   `dataview-hidden` | `html`, default `auto`), `checkboxes` (`todos-and-leaves` |
//!   `todos` | `all` | `none`, default `todos-and-leaves`), `dates` (bool, default
//!   true), `depth` (positive int, default unlimited), `paused` (bool, default false),
//!   `force` (bool; consumed by the engine and never written back), `gen` (u64, always
//!   written once known). Unknown keys inside `syncflowy:` produce a warning and are
//!   dropped on the next render.
//! * Every other frontmatter line is preserved verbatim, in order, after the two engine
//!   keys.
//!
//! ## Body
//!
//! * Lines before the first item are the sync root's note (leading/trailing blank lines
//!   trimmed). The root's name is not rendered.
//! * **Headings** (`DocConfig::headings = k`, structural, §4.1/§12.3): nodes at depth
//!   1…k render as `#`…`#`×k headings; their note is the non-blank lines right under the
//!   heading; deeper nodes are lists under them. A heading that skips a level (`###`
//!   directly under `#`) is coerced to parent level + 1 with a warning. Headings never
//!   show a checkbox, but `# [x] Title` (and `# [ ] Title`) parse as a tick (§4.3).
//! * **List items**: indentation by tabs when rendering; the parser accepts tabs or any
//!   consistent number of spaces per level (detected from the file; inconsistent input is
//!   coerced with a warning). Markers `-`, `*`, `+` and ordered `N.`/`N)` are accepted;
//!   rendering uses `- `, and `1. 2. …` for the children of a `numbered` parent. The
//!   first line holds the name; continuation lines indented one level deeper, before
//!   the first child, are the note (one Markdown line per WF line, §4.1). A blank line
//!   between two continuation lines of the same item is an empty note line.
//! * **Checkboxes** (§4.3): `- [ ] ` and `- ` both mean "incomplete"; `- [x] ` / `- [X] `
//!   (on any item kind, including headings and ordered items) means "complete this
//!   node". Which incomplete items show `[ ]` is cosmetic ([`CheckboxPolicy`],
//!   [`Sticky`]).
//! * **Kinds** (§4.3): a fenced block as the item text (```` - ``` ````, content indented
//!   one level, closing fence at the same indentation as the content) is a `code-block`
//!   node, its name being the code; a non-empty info string on the opening fence is
//!   folded into the code as its first line (as the API's create pipeline does, DM§6.2
//!   step 7; rendering always writes an empty info string). `- > text` is a
//!   `quote-block`. `- ---` is a `divider` (only rendered for dividers with an empty
//!   name). All other layouts render as plain items and are preserved in the checkpoint.
//! * **Id tags** (§4.4): found anywhere on an item's first line with
//!   [`crate::mdinline::find_tags`]; if several id tags are present the **last** one
//!   wins and the others are kept in [`LocalItem::extra_tags`] with a warning. Tags are
//!   removed from the text together with one adjacent space. Rendering puts the tag at
//!   the end of the first line, in the form chosen by [`TagStyle`] (`Auto`: `^wf-…` on
//!   list items, `%%wf:…%%` on headings, where block ids are not allowed).
//! * **Read-only forms** (§5.3, §10): an item whose id tag has the `ro` flag is rendered
//!   from a value of tier `ReadOnly` (text prefixed with `🔒 `) or is a mirror
//!   placeholder (text exactly [`PLACEHOLDER_TEXT`], children never rendered). The parser
//!   strips the `🔒 ` prefix only when the `ro` flag is present, and recognises a
//!   placeholder only by `ro` + the exact text.
//! * **Conflict markers** (§6.1, §12.5): the local version of a conflicted field is kept
//!   in the file only, as an item placed **before the real children** of the conflicted
//!   node (at the top of the body for the root): `- ⚠ <local name md>
//!   %%wf-conflict:name%%`, or `- ⚠ local note %%wf-conflict:note%%` with the local note
//!   as its continuation lines. A marker has no id tag, is never synced, and stays until
//!   the user deletes it (then the conflict counts as resolved). The parser strips the
//!   `⚠ ` prefix only on lines with a conflict tag.
//! * **Leniency** (§9.1 guard 4): the parser never fails on body content. Lines that fit
//!   nowhere (for example column-0 text after a list, a CommonMark lazy continuation)
//!   are attached to the nearest preceding item's note with a warning; nothing is ever
//!   dropped silently. Only frontmatter problems are [`ParseError`]s.
//!
//! # Invariants (§5.4); implementers must property-test them
//!
//! 1. **Render fixed point**: for every [`View`] `v` and frontmatter `f`,
//!    `normalize(&parse(&render(&v, &f))?) == render(&v, &f)`.
//! 2. **Normalisation keeps meaning**: for every text `m` that parses,
//!    `equivalent(&parse(&normalize(&parse(m)?))?, &parse(m)?, cfg)`.
//! 3. Rendering never emits a line that the parser would read differently from the View
//!    (a direct consequence of the escaping contract EC of `mdinline`).

#![allow(unused_variables, dead_code)] // stubs; remove when implemented

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::mdinline::{ConflictField, IdTag, InlineConfig, MdText};
use crate::tree::{IdMap, NodeId, ShortId, Tree};

/// The text of a mirror placeholder item (§10).
pub const PLACEHOLDER_TEXT: &str = "⧉ (empty or mirror)";
/// Prefix of a read-only best-effort rendering (§5.3).
pub const READ_ONLY_PREFIX: &str = "🔒 ";
/// Prefix of a conflict marker item (§6.1).
pub const CONFLICT_PREFIX: &str = "⚠ ";

// ---------------------------------------------------------------------------------------
// Configuration and frontmatter
// ---------------------------------------------------------------------------------------

/// Which id-tag form to write (all forms are always parsed). §4.4 recommends `Auto`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum TagStyle {
    /// `^wf-…` on list items, `%%wf:…%%` on headings (the design's recommendation).
    #[default]
    Auto,
    /// `^wf-…` everywhere except headings (which get `%%wf:…%%`).
    BlockId,
    /// `%%wf:…%%` everywhere.
    Comment,
    /// `[wid:: …]` everywhere.
    Dataview,
    /// `(wid:: …)` everywhere.
    DataviewHidden,
    /// `<!-- wf:… -->` everywhere.
    Html,
}

/// Which incomplete list items render with `[ ]` (§4.3, §12.4). Cosmetic only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum CheckboxPolicy {
    /// `todo` layout and visible leaves (the design's default).
    #[default]
    TodosAndLeaves,
    /// Only `todo` layout.
    TodosOnly,
    /// Every list item.
    All,
    /// None (ticking still works by typing `[x]`).
    None,
}

/// Rendering configuration of one file, persisted in its frontmatter (`syncflowy:`) and
/// echoed in the checkpoint. Defaults are the design's recommendations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocConfig {
    /// Depth 1..=headings renders as headings (§4.1). Default 0.
    pub headings: u8,
    /// Id tag form to write. Default [`TagStyle::Auto`].
    pub tags: TagStyle,
    /// Checkbox policy. Default [`CheckboxPolicy::TodosAndLeaves`].
    pub checkboxes: CheckboxPolicy,
    /// Inline options (compact dates). Default `dates: true`.
    pub inline: InlineConfig,
    /// Nodes deeper than this (root = 0) are hidden (§4.5). Default `None` (unlimited).
    pub max_depth: Option<u32>,
}

impl Default for DocConfig {
    fn default() -> Self {
        DocConfig {
            headings: 0,
            tags: TagStyle::Auto,
            checkboxes: CheckboxPolicy::TodosAndLeaves,
            inline: InlineConfig::default(),
            max_depth: None,
        }
    }
}

/// The `workflowy:` value: how the file names its sync root.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RootRef {
    /// The value as written (re-written verbatim on render).
    pub raw: String,
    /// The short id it denotes.
    pub short: ShortId,
    /// The full id, if the value was a full UUID.
    pub full: Option<NodeId>,
}

impl RootRef {
    /// Parse a short id, a full UUID or a `https://workflowy.com/#/<short>` URL
    /// (DM§3.2 URL form). Surrounding whitespace is trimmed.
    pub fn parse(s: &str) -> Result<RootRef, ParseError> {
        todo!()
    }

    /// The full id: `full` if present, else the [`IdMap`] lookup of `short`.
    pub fn resolve(&self, ids: &IdMap) -> Option<NodeId> {
        todo!()
    }
}

/// The parsed frontmatter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Frontmatter {
    /// The sync root.
    pub root: RootRef,
    /// Rendering configuration.
    pub config: DocConfig,
    /// The render generation this file came from (§9.1 guard 1), the `gen:` key; `None`
    /// if absent or unparseable (then `sync` uses safe mode).
    pub generation: Option<u64>,
    /// `paused: true`: do not sync this root (§9.1 controls).
    pub paused: bool,
    /// `force: true`: one-shot override of the mass-change brake and placeholder guards
    /// (§8.3, §10). `sync` clears it (sets `false`) before the next render.
    pub force: bool,
    /// Every other frontmatter line, verbatim, in order (without the `---` fences).
    pub other: Vec<String>,
}

impl Frontmatter {
    /// A frontmatter for a fresh file: given root, default config, no gen, not paused.
    pub fn new(root: RootRef) -> Frontmatter {
        Frontmatter {
            root,
            config: DocConfig::default(),
            generation: None,
            paused: false,
            force: false,
            other: Vec::new(),
        }
    }

    /// The frontmatter block including both `---` fences and a final newline, as
    /// [`render`] writes it (key order and defaults per the module docs).
    pub fn render(&self) -> String {
        todo!()
    }
}

// ---------------------------------------------------------------------------------------
// The parsed file
// ---------------------------------------------------------------------------------------

/// The structural kind of an item (§4.3). Headings are orthogonal ([`LocalItem::heading`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ItemKind {
    /// An ordinary item (any layout not listed below).
    Bullet,
    /// A fenced code block item (`code-block` layout).
    Code,
    /// `- > text` (`quote-block` layout).
    Quote,
    /// `- ---` (`divider` layout with an empty name).
    Divider,
    /// A read-only mirror placeholder (§10): `ro` tag and [`PLACEHOLDER_TEXT`].
    Placeholder,
    /// A conflict marker (§6.1); not a node.
    ConflictMarker(ConflictField),
}

/// The checkbox written on an item (§4.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum Checkbox {
    /// No box.
    #[default]
    None,
    /// `[ ]`: incomplete (cosmetic).
    Open,
    /// `[x]` or `[X]`: complete this node (a command, §4.3).
    Ticked,
}

/// One item of the parsed file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalItem {
    /// The winning (last) id tag on the first line, if any.
    pub tag: Option<IdTag>,
    /// Other id tags found on the first line (§4.4 "two tags on a line").
    pub extra_tags: Vec<IdTag>,
    /// The kind.
    pub kind: ItemKind,
    /// `Some(level)` for a heading item (`#` × level).
    pub heading: Option<u8>,
    /// The checkbox as written.
    pub checkbox: Checkbox,
    /// True if written with an ordered marker (`1.`, `2)`, …).
    pub ordered: bool,
    /// The name as Markdown: the first line with indentation, marker, checkbox, kind
    /// prefix (`> `, `🔒 ` with `ro`, `⚠ ` with a conflict tag) and tags removed, trailing
    /// whitespace trimmed; not unescaped (feed it to `mdinline::md_to_wf`). For
    /// [`ItemKind::Code`]: the code lines (info string folded in), joined with `\n`,
    /// dedented. For a marker: the local name text (empty for a note marker).
    pub name_md: String,
    /// Note lines (or a note marker's local note lines), with the item's continuation
    /// indentation removed; not unescaped. Empty if there is no note.
    pub note_md: Vec<String>,
    /// Child items in file order (conflict markers first, by construction of `render`,
    /// but the parser accepts them anywhere among the children).
    pub children: Vec<LocalItem>,
    /// 1-based line number of the first line.
    pub line: usize,
    /// 1-based line number of the last line belonging to this item (before children).
    pub end_line: usize,
}

impl LocalItem {
    /// True for conflict markers (never nodes).
    pub fn is_marker(&self) -> bool {
        matches!(self.kind, ItemKind::ConflictMarker(_))
    }
}

/// A path to an item: indices into [`LocalDoc::items`], then into `children`, …
/// (markers included in the indexing).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ItemPath(pub Vec<usize>);

/// What kind of problem a [`ParseWarning`] reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum WarningKind {
    /// A heading skipped a level and was coerced.
    HeadingLevelSkipped,
    /// A heading deeper than `headings` (kept as a heading at its structural depth).
    HeadingBeyondConfig,
    /// Indentation was inconsistent and was coerced.
    Indentation,
    /// Several id tags on one line; the last one won.
    MultipleTags,
    /// An id tag carried an unknown flag (ignored).
    UnknownTagFlag,
    /// An unknown key in `syncflowy:` (dropped on the next render).
    UnknownConfigKey,
    /// A line that fit nowhere was attached to the nearest item's note.
    StrayLine,
    /// A code fence was not closed before the item ended.
    UnclosedFence,
    /// A conflict marker outside any item list position that makes sense (kept).
    MisplacedMarker,
}

/// A non-fatal parse problem.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParseWarning {
    /// 1-based line.
    pub line: usize,
    /// Kind.
    pub kind: WarningKind,
    /// Human-readable detail.
    pub message: String,
}

/// A fatal parse problem. Only the frontmatter can fail (§9.1 guard 4: a parse failure
/// means "skip this cycle", never "everything deleted").
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ParseError {
    /// The file does not start with a `---` frontmatter block.
    #[error("no frontmatter")]
    NoFrontmatter,
    /// The frontmatter is not closed by `---`.
    #[error("unterminated frontmatter")]
    UnterminatedFrontmatter,
    /// No `workflowy:` key.
    #[error("frontmatter has no workflowy: root")]
    NoRoot,
    /// The `workflowy:` value is not a short id, UUID or Workflowy URL.
    #[error("bad workflowy: root {0:?}")]
    BadRoot(String),
    /// A `syncflowy:` value is invalid (for example `headings: 9`).
    #[error("bad syncflowy config {key}: {value:?}")]
    BadConfig {
        /// The key.
        key: String,
        /// The offending value.
        value: String,
    },
}

/// A parsed Markdown file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalDoc {
    /// The frontmatter.
    pub frontmatter: Frontmatter,
    /// The sync root's note as Markdown lines (not unescaped).
    pub root_note: Vec<String>,
    /// Top-level items (children of the sync root), markers included.
    pub items: Vec<LocalItem>,
    /// Non-fatal problems, in line order.
    pub warnings: Vec<ParseWarning>,
}

impl LocalDoc {
    /// Every item with its path, in pre-order (file order).
    pub fn walk(&self) -> Vec<(ItemPath, &LocalItem)> {
        todo!()
    }

    /// The item at `path`.
    pub fn get(&self, path: &ItemPath) -> Option<&LocalItem> {
        todo!()
    }

    /// Paths of the non-marker items carrying each short id (winning tags only), in
    /// file order. More than one path means a duplicated tag (§4.4).
    pub fn tags(&self) -> BTreeMap<ShortId, Vec<ItemPath>> {
        todo!()
    }
}

/// Parse a whole file. Never fails on body content (module docs).
pub fn parse(text: &str) -> Result<LocalDoc, ParseError> {
    todo!()
}

/// Parse only the frontmatter (cheap; used by `daemon` discovery, §9.1 guard 5).
pub fn parse_frontmatter(text: &str) -> Result<Frontmatter, ParseError> {
    todo!()
}

/// Re-render a parsed document in canonical form without a tree: same items, kinds,
/// tags and texts, canonical markers, indentation, tag form (per the doc's own
/// config), checkbox kept as written (`Ticked` stays ticked). Used for invariant 2 and
/// tests; `render` of a View built from the same content must produce the same bytes.
pub fn normalize(doc: &LocalDoc) -> String {
    todo!()
}

/// Semantic equality of two parsed documents (§5.4 "≅"): same frontmatter (root, config,
/// gen, paused, force, other), same root note and item trees, comparing per item the
/// winning tag's short id and flags, kind, heading level, checkbox *class* (`Ticked` vs
/// not ticked: `Open` ≡ `None`), ordered flag, and the WF values `md_to_wf(name_md)` /
/// `md_to_wf(note_md)` (so escaping variants are equal). Line numbers and warnings are
/// ignored.
pub fn equivalent(a: &LocalDoc, b: &LocalDoc, cfg: &InlineConfig) -> bool {
    todo!()
}

// ---------------------------------------------------------------------------------------
// The view: what gets rendered
// ---------------------------------------------------------------------------------------

/// A conflict marker to render (§6.1): the local version of `field` of `node`, as the
/// Markdown the user had typed (for a note: lines joined with `\n`).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ConflictMarker {
    /// The conflicted node (the sync root for a root-note conflict).
    pub node: NodeId,
    /// Which field.
    pub field: ConflictField,
    /// The local Markdown text to preserve.
    pub local_md: String,
}

/// Choices carried over from the previous file so that re-renders do not rewrite lines
/// nobody edited (§9.1 guard 4). Built by `sync` from the base `LocalDoc`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sticky {
    /// The checkbox each item had in the previous file (`Ticked` never appears: ticked
    /// nodes are gone). For a list item whose layout is not `todo`, a recorded value wins
    /// over [`CheckboxPolicy`]; `todo` always renders `Open`.
    pub checkbox: BTreeMap<NodeId, Checkbox>,
    /// Reserved for §15.1 (mirrors as origins): content node -> placement that gets the
    /// full rendering. Always empty for now.
    pub full_placement: BTreeMap<NodeId, NodeId>,
}

/// Inputs to [`View::build`] besides the tree.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ViewExtras {
    /// Conflict markers to render (from `merge::MergeResult::conflicts`).
    pub conflicts: Vec<ConflictMarker>,
    /// Extra nodes to hide with their subtrees: pending deletes (§9.1 guard 2).
    pub hidden: BTreeSet<NodeId>,
    /// Sticky cosmetic choices.
    pub sticky: Sticky,
}

/// One rendered item.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ViewItem {
    /// The node (a temporary id only in dry-run views of a merged tree).
    pub id: NodeId,
    /// Its short id for the tag (`None` for temporary ids: rendered without a tag).
    pub tag: Option<ShortId>,
    /// Kind (never `ConflictMarker`; markers are in `conflicts`).
    pub kind: ItemKind,
    /// `Some(depth)` if depth <= `headings`.
    pub heading: Option<u8>,
    /// `Open` or `None` (visible nodes are incomplete, so never `Ticked`), from the
    /// layout, [`Sticky`] and [`CheckboxPolicy`]; always `None` on headings, code,
    /// dividers and placeholders.
    pub checkbox: Checkbox,
    /// True if the parent's layout is `numbered` (rendered `1.`, `2.`, …).
    pub ordered: bool,
    /// The name (`FieldKind::Code` for code blocks, else `Name`).
    pub name: MdText,
    /// The note, if the node has a non-empty one.
    pub note: Option<MdText>,
    /// Rendered read-only (`ro` flag): a `ReadOnly` name or note tier, or a placeholder.
    pub read_only: bool,
    /// Conflict markers for this node, rendered before the children.
    pub conflicts: Vec<ConflictMarker>,
    /// Visible children (none for placeholders).
    pub children: Vec<ViewItem>,
}

/// Problems found while building a view.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ViewError {
    /// The sync root is not in the tree.
    #[error("sync root {0} not in tree")]
    RootMissing(NodeId),
}

/// The visible projection of a tree from a sync root (§4.5): exactly what the file
/// shows. Hiding rules, in order: completed nodes and everything under them (the root
/// itself is always shown, `root_completed` records it); [`ViewExtras::hidden`]; nodes
/// deeper than `max_depth`; children of placeholders (§10).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct View {
    /// The sync root.
    pub root: NodeId,
    /// True if the root itself is completed (rendered anyway; `sync` warns).
    pub root_completed: bool,
    /// The root's note (`None` if absent or empty).
    pub root_note: Option<MdText>,
    /// Conflict markers on the root (its note), rendered first in the body.
    pub root_conflicts: Vec<ConflictMarker>,
    /// Top-level items.
    pub items: Vec<ViewItem>,
    /// Markers whose node is not visible (dropped from the file; `merge` reports them).
    pub dropped_conflicts: Vec<ConflictMarker>,
}

impl View {
    /// Build the view of `tree` from `root` (§4.5) with the kinds, checkboxes, tiers and
    /// tags described in the module docs. `tree` may be a whole export or a subtree.
    pub fn build(
        tree: &Tree,
        root: &NodeId,
        cfg: &DocConfig,
        extras: &ViewExtras,
    ) -> Result<View, ViewError> {
        todo!()
    }

    /// Every node in the view (the root and all items), i.e. the visible set.
    pub fn ids(&self) -> BTreeSet<NodeId> {
        todo!()
    }

    /// True if `id` is the root or an item of the view.
    pub fn contains(&self, id: &NodeId) -> bool {
        todo!()
    }

    /// Every item with its parent id (the root for top-level items), in pre-order.
    pub fn walk(&self) -> Vec<(&NodeId, &ViewItem)> {
        todo!()
    }
}

/// Render a whole file: `frontmatter.render()`, then the root note, root conflict
/// markers, and items (module docs). The output ends with exactly one `\n`. Rendering
/// is deterministic and a fixed point (invariant 1). The frontmatter is written as given
/// (`sync` sets `gen` and clears `force` first); `frontmatter.config` drives the tag form
/// (the View already embodies the other config choices).
pub fn render(view: &View, frontmatter: &Frontmatter) -> String {
    todo!()
}
