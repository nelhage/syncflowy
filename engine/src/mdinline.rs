//! WF-HTML inline text <-> Markdown inline text, representability tiers, the lossy-write
//! refinement, and the id-tag syntax shared with `mdblock`.
//!
//! Responsibility (design §4.2 inline mapping and escaping, §4.4 tag forms, §5.2 writing an
//! edited field without dropping what Markdown could not show, §5.3 representability
//! tiers, §5.4 convergence). Pure: no I/O.
//!
//! Consumes: [`crate::wfhtml`] (`parse`, `serialize`, `escape_text`, `canonicalize`,
//! `to_plain_text`), [`crate::tree::ShortId`], and the `similar` crate for §5.2.
//! Consumed by: `mdblock` (rendering names/notes, finding id tags, escaping), `merge`
//! (`md_to_wf` of edited text, [`write_value`] for §5.2, [`classify`]).
//!
//! # The mapping (§4.2)
//!
//! | WF-HTML | Markdown |
//! |---|---|
//! | text (entities decoded) | the text, backslash-escaped where Markdown or our block parser would misread it |
//! | `<b>` `<i>` `<s>` `<code>` | `**…**` `*…*` `~~…~~` `` `…` `` (code spans pick a backtick run longer than any inside, with padding spaces when the content starts or ends with a backtick) |
//! | `<u>…</u>` | passthrough `<u>…</u>` |
//! | `<a href="u">u'</a>` where the decoded text equals the href (UI autolink, UI:E5) | bare `u` |
//! | `<a href="u">text</a>` | `[text](u)`; destinations containing spaces, `(`/`)` or `<`/`>` use the angle form `<u>` with escapes |
//! | `<span class="colored …">` | passthrough `<span class="colored …">…</span>` |
//! | `<time …>T</time>` | `📅 YYYY-MM-DD` or `📅 YYYY-MM-DD HH:MM` **only if** [`InlineConfig::dates`] and `T` equals the text the API's create pipeline generates for those attributes (DM§6.2 step 11: `Thu, Jan 15, 2026`, ` at 2:30 PM`), otherwise passthrough |
//! | any other well-formed element | passthrough of its canonical form; if canonical serialisation drops it the value is [`Tier::Lossy`] |
//! | `#tag`, `@mention` | the same text |
//!
//! Parsing ([`md_to_wf`]) accepts `__x__` and `_x_` as bold/italic, both escaped and
//! unescaped forms wherever unambiguous, inline HTML for the passthrough tags (`<u> <span
//! class="colored…"> <time …> <b> <i> <s> <code> <a href>`; anything else, or unbalanced
//! HTML, is literal text), autolinks `<https://…>` and bare `http(s)://` URLs (both become
//! `<a href="u">u</a>` with the UI quirk: raw `&` in `href`, `&amp;` in the text), and
//! `📅 YYYY-MM-DD[ HH:MM]` (only when [`InlineConfig::dates`]). Markdown with no WF meaning
//! (`[[wikilinks]]`, `==hl==`, `$math$`, footnotes) is literal text.
//!
//! Character references in Markdown: `&amp; &lt; &gt; &quot; &#NN; &#xHH;` are decoded by
//! [`md_to_wf`]; any other `&` is a literal `&`. [`wf_to_md`] writes a literal `&` that is
//! followed by something that looks like a reference as `\&`.
//!
//! # The escaping contract (EC). `mdblock` relies on all of it.
//!
//! For `kind` in [`FieldKind::Name`] / [`FieldKind::Note`] and every config:
//!
//! 1. **Round trip.** If `wf_to_md(x).tier` is [`Tier::Exact`] or [`Tier::Passthrough`]
//!    then `md_to_wf(&wf_to_md(x).text) == x`. ([`classify`] is defined by this test.)
//! 2. **Canonical output.** For every string `m`, `md_to_wf(m)` is well-formed and
//!    `wfhtml::canonicalize(md_to_wf(m)) == md_to_wf(m)`. `md_to_wf` is total.
//! 3. **Convergence.** For every `m`: `md_to_wf(&wf_to_md(&md_to_wf(m)).text) ==
//!    md_to_wf(m)` (normalising Markdown never changes meaning, §5.4).
//! 4. **Block safety** of `wf_to_md(x).text`, for every `x`:
//!    * `Name`: always a single line (no `\n`/`\r`); for a [`Tier::ReadOnly`] value with
//!      newlines, each newline is shown as ` ⏎ ` in the best-effort text;
//!    * `Note`: lines separated by `\n`, one per WF line;
//!    * no line starts with something `mdblock` reads as structure: a list marker
//!      (`- `, `* `, `+ `, `1. `, `1) `), a checkbox (`[ ] `, `[x] `), a heading (`#`), a
//!      quote (`>`), a thematic break (`---`, `***`, `___`), a code fence
//!      (```` ``` ````, `~~~`); such starts are backslash-escaped (`\- `, `1\. `, `\# `,
//!      …). (The `🔒`, `⧉` and `⚠` prefixes need no escaping: `mdblock` only treats them
//!      as markup together with an `ro` flag or a conflict tag on the same line.)
//!    * leading and trailing whitespace of a line is preserved by encoding it as
//!      `&#32;` / `&#9;` (Markdown and our parser both strip it otherwise);
//!    * no line contains a substring that [`find_tags`] recognises, nor `%%` (the
//!      Obsidian comment delimiter): a `^` in `^wf-` is written `\^`, `%%` as `%\%`,
//!      `[wid::` as `\[wid::`, `(wid::` as `\(wid::`, `<!--` as `\<!--`.
//! 5. **Minimal escaping.** Escape only where needed (`a * b`, `x_y_z`, `3 > 2` stay
//!    unescaped); WF text `&lt;b&gt;` renders as `\<b>` so it stays distinct from real
//!    markup (UI:E2).
//!
//! For [`FieldKind::Code`] (the name of a `code-block` node, rendered inside a fence by
//! `mdblock`): `text` is `to_plain_text(x)` verbatim (no escaping), and `md_to_wf` is
//! `escape_text(m)`; the tier is `Exact` iff that round-trips, `ReadOnly` if a line of the
//! code would close the fence (a line starting with ```` ``` ````), else `Lossy`.
//! UNVERIFIED: how the UI stores code-block names (§13 Q2); this mapping is a guess.
//!
//! # Id tags (§4.4)
//!
//! A tag carries a [`ShortId`] plus optional flags, written `<12hex>` followed by
//! `-<flag>` for each flag (only `ro` exists: read-only, §5.3 and §10). Five forms are
//! parsed everywhere on a line; [`TagForm`] lists them. A conflict-marker tag
//! `%%wf-conflict:name%%` / `%%wf-conflict:note%%` marks a conflict marker item (§6.1, see
//! `mdblock`). User text can never produce either (EC 4).

#![allow(unused_variables, dead_code)] // stubs; remove when implemented

use std::collections::BTreeSet;
use std::ops::Range;

use serde::{Deserialize, Serialize};

use crate::tree::ShortId;

/// Inline rendering options. Persisted in the file's frontmatter via
/// `mdblock::DocConfig`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InlineConfig {
    /// Render API-shaped `<time>` dates as `📅 YYYY-MM-DD[ HH:MM]` and parse that form
    /// back (§4.2). Default `true`. When `false`, dates are passthrough HTML and `📅 …` is
    /// literal text.
    pub dates: bool,
}

impl Default for InlineConfig {
    fn default() -> Self {
        InlineConfig { dates: true }
    }
}

/// Which field a text belongs to; escaping and line rules differ.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FieldKind {
    /// A node name rendered on an item's first line (or a heading line).
    Name,
    /// A note (or the sync root's note, the preamble): several lines.
    Note,
    /// The name of a `code-block` node, rendered as the body of a fence.
    Code,
}

/// Representability of one WF value under our dialect (§5.3). Ordered from best to worst.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Tier {
    /// Tier 1: `md_to_wf(wf_to_md(x)) == x` using Markdown constructs only.
    Exact,
    /// Tier 2: round-trips exactly, but only because parts are emitted as raw inline HTML.
    Passthrough,
    /// Well-formed, but the round trip differs (non-canonical markup from another API
    /// client, an unknown element, …). Still editable: edits go through [`write_value`]
    /// (§5.2), which preserves what the rendering could not show where possible.
    Lossy,
    /// Tier 3: not representable (malformed WF-HTML, a newline in a non-code name, a code
    /// line that would close the fence). Rendered as a best-effort, read-only form
    /// (`mdblock` adds a `🔒` and the `ro` tag flag); local edits are refused (§5.3).
    ReadOnly,
}

/// Markdown text produced from a WF value, with its tier.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MdText {
    /// The Markdown. For `Note` and `Code`, lines are separated by `\n` (no trailing
    /// newline). Satisfies the escaping contract (module docs).
    pub text: String,
    /// The representability of the source value.
    pub tier: Tier,
}

/// WF-HTML -> Markdown (§4.2) for one field, with its tier. Total: malformed input gives a
/// best-effort text (`to_plain_text`, escaped) with [`Tier::ReadOnly`].
pub fn wf_to_md(wf: &str, kind: FieldKind, cfg: &InlineConfig) -> MdText {
    todo!()
}

/// Markdown -> canonical WF-HTML (§4.2) for one field. Total, and always canonical
/// (EC 2). For `Note`, `\n` separates WF lines (raw newlines in the note, UI:E8).
pub fn md_to_wf(md: &str, kind: FieldKind, cfg: &InlineConfig) -> String {
    todo!()
}

/// The tier of `wf` (= `wf_to_md(wf, kind, cfg).tier`).
pub fn classify(wf: &str, kind: FieldKind, cfg: &InlineConfig) -> Tier {
    todo!()
}

/// Result of [`write_value`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WriteValue {
    /// The canonical WF-HTML value to write.
    pub wf: String,
    /// True if the three-way merge could not be applied and the naive write
    /// `md_to_wf(new_md)` was used although `base_wf` was not representable exactly, so
    /// something Markdown could not show was dropped (§5.2: "log what was lost").
    pub lost: bool,
}

/// The §5.2 refinement for writing an edited field. Let `b = base_wf` and
/// `b' = md_to_wf(wf_to_md(b).text)`. If `b' == b`, return `md_to_wf(new_md)`. Otherwise
/// compute the edit from `b'` to `md_to_wf(new_md)` and apply it to `b` with a three-way
/// text merge (character-level for `Name`/`Code`, line-level for `Note`, using `similar`).
/// The merged result must be well-formed and canonical (EC 2) to be used; if the merge
/// conflicts or the result is not canonical, fall back to `md_to_wf(new_md)` with
/// `lost: true`.
pub fn write_value(base_wf: &str, new_md: &str, kind: FieldKind, cfg: &InlineConfig) -> WriteValue {
    todo!()
}

// ---------------------------------------------------------------------------------------
// Id tags (§4.4)
// ---------------------------------------------------------------------------------------

/// The written form of an id tag. `wf_to_md` guarantees user text never contains one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum TagForm {
    /// `^wf-8e1f00aa2b3c` (an Obsidian block id; not allowed on headings).
    BlockId,
    /// `%%wf:8e1f00aa2b3c%%` (an Obsidian comment).
    Comment,
    /// `[wid:: 8e1f00aa2b3c]` (Dataview inline field).
    Dataview,
    /// `(wid:: 8e1f00aa2b3c)` (Dataview inline field, value only).
    DataviewHidden,
    /// `<!-- wf:8e1f00aa2b3c -->` (HTML comment).
    Html,
}

/// A flag carried in a tag after the short id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum TagFlag {
    /// `-ro`: the item is a read-only rendering (§5.3 tier 3, §10 placeholders).
    ReadOnly,
}

impl TagFlag {
    /// The flag's spelling inside a tag (`ro`).
    pub fn as_str(self) -> &'static str {
        match self {
            TagFlag::ReadOnly => "ro",
        }
    }
}

/// A parsed id tag.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct IdTag {
    /// The node's short id.
    pub short: ShortId,
    /// Known flags. Unknown flags are dropped by [`find_tags`] (the caller may warn).
    pub flags: BTreeSet<TagFlag>,
    /// How it was written.
    pub form: TagForm,
}

/// Which field a conflict marker preserves (§6.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum ConflictField {
    /// The node's name.
    Name,
    /// The node's note.
    Note,
}

/// Something [`find_tags`] recognised on a line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum LineTag {
    /// A node id tag.
    Id(IdTag),
    /// A conflict-marker tag `%%wf-conflict:name%%` / `%%wf-conflict:note%%`.
    Conflict(ConflictField),
}

/// One tag occurrence on a line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TagMatch {
    /// What was found.
    pub tag: LineTag,
    /// Byte range of the whole tag in the line (not including surrounding spaces).
    pub range: Range<usize>,
}

/// Every syntactically valid tag anywhere in `line`, left to right (§9.1 guard 4: a tag
/// can end up mid-line after a text merge). Escaped forms (`\^wf-…`, `%\%…`) are not tags.
/// Matching is case-insensitive in the hex digits.
pub fn find_tags(line: &str) -> Vec<TagMatch> {
    todo!()
}

/// Write an id tag: for example `format_tag(&s, &{ReadOnly}, TagForm::BlockId)` is
/// `^wf-8e1f00aa2b3c-ro`, and with `TagForm::Comment` it is `%%wf:8e1f00aa2b3c-ro%%`.
pub fn format_tag(short: &ShortId, flags: &BTreeSet<TagFlag>, form: TagForm) -> String {
    todo!()
}

/// Write a conflict-marker tag: `%%wf-conflict:name%%` or `%%wf-conflict:note%%`.
pub fn format_conflict_tag(field: ConflictField) -> String {
    todo!()
}
