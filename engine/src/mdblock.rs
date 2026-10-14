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
//!   `force` (bool; consumed by the engine: `sync` clears it before rendering, and
//!   [`Frontmatter::render`] writes what it is given), `gen` (u64, always written once
//!   known; an unparseable value reads as `None`, not as an error). Unknown keys inside
//!   `syncflowy:` produce a warning and are dropped on the next render. Booleans are
//!   `true`/`false` (any case); values may be quoted; `# comments` are ignored; a flow
//!   mapping may continue on the following lines. An invalid value of a known key is
//!   [`ParseError::BadConfig`] (skip the cycle rather than guess).
//! * Every other frontmatter line is preserved verbatim, in order, after the two engine
//!   keys. The `workflowy:` value is kept as written ([`RootRef::raw`], comment
//!   included); a second `workflowy:` key naming another root is [`ParseError::BadRoot`].
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
//!   dropped silently. Only frontmatter problems are [`ParseError`]s; a missing, empty
//!   or frontmatter-less file is [`ParseError::NoFrontmatter`], never an empty document.
//!
//! ## Details fixed by the implementation
//!
//! These refine the rules above where the design leaves a choice; each is consistent
//! between [`render`] and [`parse`] (the property tests check it).
//!
//! * **Structure by indentation width**: a list item is a child of the nearest open item
//!   whose marker is indented less (a stack, as in CommonMark), with tabs counting to the
//!   next multiple of 4 columns. An item whose indentation differs from its earlier
//!   siblings' is kept where the stack puts it, with [`WarningKind::Indentation`].
//! * **Empty note lines**: a blank line before or between continuation lines of a list
//!   item is an empty note line. Empty lines at the *end* of a note are rendered as the
//!   continuation indentation alone (a whitespace-only line), because a truly blank line
//!   there would read as the gap before the next item; the parser counts whitespace-only
//!   lines indented deeper than the marker as empty note lines.
//!   UNVERIFIED: whether Obsidian (or a formatter on a client) keeps such lines.
//! * **Root and heading notes** lose leading and trailing empty lines on render (a WF
//!   note `"\nx\n"` shows as `x`); this is lossy only for those edge lines, and a field
//!   the Markdown user does not edit is never written (§5.1).
//! * **Fences inside notes**: a note line that opens a fence (typed by the user; rendered
//!   text is escaped, EC 4) keeps the following lines as note text up to its closing
//!   fence, if that fence comes before the note's item ends; an unclosed fence is plain
//!   text and never swallows the items after it.
//! * **Code at heading depth**: `# ``` %%wf:…%%`, the code at column 0, a closing fence
//!   at column 0. UNVERIFIED (our choice): Obsidian shows it as a heading followed by a
//!   code block.
//! * **Fence length**: code is fenced with one backtick more than the longest backtick run
//!   that starts one of its lines (at least three), so a `ReadOnly` code value with a
//!   ```` ``` ```` line still reads back exactly.
//! * **Kinds without a box**: headings, code, quotes, dividers and placeholders never
//!   render `[ ]` (UNVERIFIED, our choice for quotes: `- [ ] > x` reads badly).
//! * **Dividers are not shells**: a `divider` node with an empty name renders as `- ---`
//!   even though it matches the shell heuristic (the UI gives it `note ""`, UI:G2, while
//!   shells always get layout `bullets`, UI:H2).
//! * **Text without an `ro` flag is a fixed point**: a `Lossy` value (and a `ReadOnly`
//!   root note, which cannot carry a flag) is rendered as its own normal form
//!   `wf_to_md(md_to_wf(text))`, so re-rendering the parsed file gives the same bytes.
//! * **Read-only items** carry the `ro` flag and the `🔒 ` prefix when any of their
//!   fields is `ReadOnly`; their texts are kept verbatim by [`normalize`].
//! * **Conflict markers** of a code block's name (several lines) are written as a fenced
//!   block: `- ⚠ ``` %%wf-conflict:name%%`, the local code, the closing fence. Markers
//!   with children, or written as headings, are kept with
//!   [`WarningKind::MisplacedMarker`]; an id tag on a marker line is moved to
//!   `extra_tags` with [`WarningKind::MultipleTags`].
//!
//! # Invariants (§5.4); implementers must property-test them
//!
//! 1. **Render fixed point**: for every [`View`] `v` and frontmatter `f`,
//!    `normalize(&parse(&render(&v, &f))?) == render(&v, &f)`.
//! 2. **Normalisation keeps meaning**: for every text `m` that parses,
//!    `equivalent(&parse(&normalize(&parse(m)?))?, &parse(m)?, cfg)`.
//! 3. Rendering never emits a line that the parser would read differently from the View
//!    (a direct consequence of the escaping contract EC of `mdinline`).

mod emit;
mod frontmatter;
mod parse;
mod view;

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::mdinline::{ConflictField, FieldKind, IdTag, InlineConfig, MdText, TagFlag, md_to_wf};
use crate::tree::{IdMap, NodeId, ShortId, Tree};

use emit::RItem;

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
        RootRef::parse_impl(s)
    }

    /// The full id: `full` if present, else the [`IdMap`] lookup of `short`.
    pub fn resolve(&self, ids: &IdMap) -> Option<NodeId> {
        self.resolve_impl(ids)
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
        frontmatter::render(self)
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
        fn rec<'a>(
            items: &'a [LocalItem],
            path: &mut Vec<usize>,
            out: &mut Vec<(ItemPath, &'a LocalItem)>,
        ) {
            for (i, it) in items.iter().enumerate() {
                path.push(i);
                out.push((ItemPath(path.clone()), it));
                rec(&it.children, path, out);
                path.pop();
            }
        }
        let mut out = Vec::new();
        rec(&self.items, &mut Vec::new(), &mut out);
        out
    }

    /// The item at `path`.
    pub fn get(&self, path: &ItemPath) -> Option<&LocalItem> {
        let (first, rest) = path.0.split_first()?;
        let mut it = self.items.get(*first)?;
        for &i in rest {
            it = it.children.get(i)?;
        }
        Some(it)
    }

    /// Paths of the non-marker items carrying each short id (winning tags only), in
    /// file order. More than one path means a duplicated tag (§4.4).
    pub fn tags(&self) -> BTreeMap<ShortId, Vec<ItemPath>> {
        let mut out: BTreeMap<ShortId, Vec<ItemPath>> = BTreeMap::new();
        for (path, it) in self.walk() {
            if let (false, Some(t)) = (it.is_marker(), &it.tag) {
                out.entry(t.short.clone()).or_default().push(path);
            }
        }
        out
    }
}

/// Parse a whole file. Never fails on body content (module docs).
pub fn parse(text: &str) -> Result<LocalDoc, ParseError> {
    parse::parse(text)
}

/// Parse only the frontmatter (cheap; used by `daemon` discovery, §9.1 guard 5).
pub fn parse_frontmatter(text: &str) -> Result<Frontmatter, ParseError> {
    let lines = frontmatter::split_lines(text);
    frontmatter::parse(&lines).map(|p| p.fm)
}

/// Re-render a parsed document in canonical form without a tree: same items, kinds,
/// tags and texts, canonical markers, indentation, tag form (per the doc's own
/// config), checkbox kept as written (`Ticked` stays ticked). Used for invariant 2 and
/// tests; `render` of a View built from the same content must produce the same bytes.
pub fn normalize(doc: &LocalDoc) -> String {
    let inline = &doc.frontmatter.config.inline;
    let root_note = canonical_note(&doc.root_note, inline);
    let items: Vec<RItem> = doc.items.iter().map(|i| local_ritem(i, inline)).collect();
    emit::emit(&doc.frontmatter, &root_note, &items)
}

fn has_ro(item: &LocalItem) -> bool {
    item.tag
        .as_ref()
        .is_some_and(|t| t.flags.contains(&TagFlag::ReadOnly))
}

fn name_kind(kind: ItemKind) -> FieldKind {
    if kind == ItemKind::Code {
        FieldKind::Code
    } else {
        FieldKind::Name
    }
}

/// The canonical Markdown of a note: `wf_to_md(md_to_wf(lines))` (empty if the value is).
fn canonical_note(lines: &[String], cfg: &InlineConfig) -> Vec<String> {
    let wf = md_to_wf(&lines.join("\n"), FieldKind::Note, cfg);
    if wf.is_empty() {
        return Vec::new();
    }
    view::md_text(&wf, FieldKind::Note, cfg, false)
        .text
        .split('\n')
        .map(str::to_owned)
        .collect()
}

/// The writer's form of a parsed item (see [`normalize`]): texts of editable items are
/// canonicalised; read-only items and conflict markers keep their text as written.
fn local_ritem(it: &LocalItem, cfg: &InlineConfig) -> RItem {
    let ro = has_ro(it) || it.kind == ItemKind::Placeholder;
    let verbatim = ro || it.is_marker();
    let name = if verbatim {
        it.name_md.clone()
    } else {
        let kind = name_kind(it.kind);
        view::md_text(&md_to_wf(&it.name_md, kind, cfg), kind, cfg, false).text
    };
    let note = if verbatim {
        it.note_md.clone()
    } else {
        canonical_note(&it.note_md, cfg)
    };
    RItem {
        tag: it.tag.as_ref().map(|t| (t.short.clone(), t.flags.clone())),
        kind: it.kind,
        heading: it.heading,
        checkbox: it.checkbox,
        ordered: it.ordered,
        read_only: ro && it.kind != ItemKind::Placeholder,
        name,
        note,
        children: it.children.iter().map(|c| local_ritem(c, cfg)).collect(),
    }
}

/// Semantic equality of two parsed documents (§5.4 "≅"): same frontmatter (root, config,
/// gen, paused, force, other), same root note and item trees, comparing per item the
/// winning tag's short id and flags, kind, heading level, checkbox *class* (`Ticked` vs
/// not ticked: `Open` ≡ `None`), ordered flag, and the WF values `md_to_wf(name_md)` /
/// `md_to_wf(note_md)` (so escaping variants are equal). Line numbers and warnings are
/// ignored.
pub fn equivalent(a: &LocalDoc, b: &LocalDoc, cfg: &InlineConfig) -> bool {
    let (fa, fb) = (&a.frontmatter, &b.frontmatter);
    let same_fm = fa.root.short == fb.root.short
        && fa.root.full == fb.root.full
        && fa.config == fb.config
        && fa.generation == fb.generation
        && fa.paused == fb.paused
        && fa.force == fb.force
        && fa.other == fb.other;
    let note = |l: &[String]| md_to_wf(&l.join("\n"), FieldKind::Note, cfg);
    same_fm && note(&a.root_note) == note(&b.root_note) && items_equivalent(&a.items, &b.items, cfg)
}

fn items_equivalent(a: &[LocalItem], b: &[LocalItem], cfg: &InlineConfig) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| item_equivalent(x, y, cfg))
}

fn item_equivalent(a: &LocalItem, b: &LocalItem, cfg: &InlineConfig) -> bool {
    let tag = |i: &LocalItem| i.tag.as_ref().map(|t| (t.short.clone(), t.flags.clone()));
    let ticked = |i: &LocalItem| i.checkbox == Checkbox::Ticked;
    let name = |i: &LocalItem| md_to_wf(&i.name_md, name_kind(i.kind), cfg);
    let note = |i: &LocalItem| md_to_wf(&i.note_md.join("\n"), FieldKind::Note, cfg);
    tag(a) == tag(b)
        && a.kind == b.kind
        && a.heading == b.heading
        && ticked(a) == ticked(b)
        && a.ordered == b.ordered
        && name(a) == name(b)
        && note(a) == note(b)
        && items_equivalent(&a.children, &b.children, cfg)
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
        view::build(tree, root, cfg, extras)
    }

    /// Every node in the view (the root and all items), i.e. the visible set.
    pub fn ids(&self) -> BTreeSet<NodeId> {
        let mut out: BTreeSet<NodeId> =
            self.walk().into_iter().map(|(_, i)| i.id.clone()).collect();
        out.insert(self.root.clone());
        out
    }

    /// True if `id` is the root or an item of the view.
    pub fn contains(&self, id: &NodeId) -> bool {
        *id == self.root || self.walk().iter().any(|(_, i)| i.id == *id)
    }

    /// Every item with its parent id (the root for top-level items), in pre-order.
    pub fn walk(&self) -> Vec<(&NodeId, &ViewItem)> {
        fn rec<'a>(
            parent: &'a NodeId,
            items: &'a [ViewItem],
            out: &mut Vec<(&'a NodeId, &'a ViewItem)>,
        ) {
            for it in items {
                out.push((parent, it));
                rec(&it.id, &it.children, out);
            }
        }
        let mut out = Vec::new();
        rec(&self.root, &self.items, &mut out);
        out
    }
}

/// Render a whole file: `frontmatter.render()`, then the root note, root conflict
/// markers, and items (module docs). The output ends with exactly one `\n`. Rendering
/// is deterministic and a fixed point (invariant 1). The frontmatter is written as given
/// (`sync` sets `gen` and clears `force` first); `frontmatter.config` drives the tag form
/// (the View already embodies the other config choices).
pub fn render(view: &View, frontmatter: &Frontmatter) -> String {
    let root_note: Vec<String> = view
        .root_note
        .as_ref()
        .map(|n| n.text.split('\n').map(str::to_owned).collect())
        .unwrap_or_default();
    let mut items: Vec<RItem> = view.root_conflicts.iter().map(marker_ritem).collect();
    items.extend(view.items.iter().map(view_ritem));
    emit::emit(frontmatter, &root_note, &items)
}

/// The writer's form of a conflict marker (§6.1). Texts are trimmed the way the parser
/// trims them, so that the marker reads back exactly.
fn marker_ritem(m: &ConflictMarker) -> RItem {
    let trimmed = m.local_md.trim_matches([' ', '\t']);
    let (name, note) = match m.field {
        ConflictField::Name if emit::marker_needs_fence(trimmed) => {
            (m.local_md.clone(), Vec::new())
        }
        ConflictField::Name => (trimmed.to_owned(), Vec::new()),
        ConflictField::Note if m.local_md.is_empty() => (String::new(), Vec::new()),
        ConflictField::Note => (
            String::new(),
            m.local_md
                .split('\n')
                .map(|l| l.trim_matches([' ', '\t']).to_owned())
                .collect(),
        ),
    };
    RItem {
        tag: None,
        kind: ItemKind::ConflictMarker(m.field),
        heading: None,
        checkbox: Checkbox::None,
        ordered: false,
        read_only: false,
        name,
        note,
        children: Vec::new(),
    }
}

fn view_ritem(v: &ViewItem) -> RItem {
    let flags = if v.read_only {
        BTreeSet::from([TagFlag::ReadOnly])
    } else {
        BTreeSet::new()
    };
    let mut children: Vec<RItem> = v.conflicts.iter().map(marker_ritem).collect();
    children.extend(v.children.iter().map(view_ritem));
    RItem {
        tag: v.tag.clone().map(|t| (t, flags)),
        kind: v.kind,
        heading: v.heading,
        checkbox: v.checkbox,
        ordered: v.ordered && v.heading.is_none(),
        read_only: v.read_only && v.kind != ItemKind::Placeholder,
        name: v.name.text.clone(),
        note: v
            .note
            .as_ref()
            .map(|n| n.text.split('\n').map(str::to_owned).collect())
            .unwrap_or_default(),
        children,
    }
}
