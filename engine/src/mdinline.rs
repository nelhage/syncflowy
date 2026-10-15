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
//! (`[[wikilinks]]`, `![[embeds]]`, `![images](…)`, `==hl==`, `$math$`, `%%comments%%`,
//! footnotes) is literal text; an image is kept exactly as written (no link or escape
//! inside it is read). [`wf_to_md`] writes wikilinks, embeds, images, highlights, math,
//! comments and footnote references back unescaped, so what the user typed keeps working
//! in Obsidian; it escapes only what would change meaning: a `%%` that would start a tag
//! (`%%wf:`, `%%wf-conflict:`, EC 4), every `%%` of a value whose `%%` do not pair up (a
//! lone one would pair with a `%%wf:…%%` tag on the line), a `![` that is not a whole
//! image, the tag forms, and a `$` before a digit when the value has another `$` (a price
//! such as `$10-$20` typed in WF must not display as math). A link title (`[a](u "t")`) has no WF form: the
//! link is kept and the title dropped ([`drops_link_title`]; `merge` reports the dropped
//! title). Trade-off: `%%x%%` typed by a Workflowy user is hidden in Obsidian's reading
//! view (design §11).
//!
//! Character references in Markdown: `&amp; &lt; &gt; &quot; &#NN; &#xHH;` are decoded by
//! [`md_to_wf`]; any other `&` is a literal `&` (a numeric reference to a character that
//! cannot be stored, such as `&#0;`, `&#13;` or `&#10;` in a name, stays literal too).
//! [`wf_to_md`] writes a literal `&` that is followed by something that looks like a
//! reference (`&name;`, `&#NN;`, `&#xHH;`) as `\&`.
//!
//! ## How the parser reads Markdown
//!
//! [`md_to_wf`] is a CommonMark-style inline parser (delimiter runs with the flanking
//! rules and the rule of 3, `~~` as in GFM, backslash escapes for ASCII punctuation, link
//! brackets with the "no links in links" rule). Deliberate simplifications, which the
//! renderer never relies on: code spans do not cross line breaks; inline HTML is only the
//! WF vocabulary above and must be balanced; link titles are dropped; images are
//! literal; bare URLs (after start of line, whitespace, `*`, `_`, `~`, `(`, `>`) are atomic
//! tokens that end at whitespace or one of ``< > \ [ ] ` "``, with GFM's trailing
//! punctuation trimming. Leading and trailing whitespace of every line is removed before
//! parsing (whitespace = Unicode `White_Space`); a name's line breaks become spaces.
//!
//! ## How the renderer writes Markdown
//!
//! [`wf_to_md`] renders with minimal escaping first (a delimiter character whose only run
//! in the value cannot pair, so `*args` stays as is), then, if the result does not
//! round-trip or is not block-safe, with every potential delimiter escaped, then with
//! every ASCII punctuation character escaped, then with every element but links written
//! as passthrough HTML. The first rendering that round-trips
//! decides the tier ([`Tier::Passthrough`] if it contains raw HTML). If none does, the
//! value is [`Tier::Lossy`] and the most readable block-safe rendering is shown.
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
//! 4. **Block safety** of `wf_to_md(x).text`, for every `x` ([`line_is_block_safe`]
//!    holds for every line):
//!    * `Name`: always a single line (no `\n`/`\r`); for a [`Tier::ReadOnly`] value with
//!      newlines, each newline is shown as ` ⏎ ` in the best-effort text;
//!    * `Note`: lines separated by `\n`, one per WF line;
//!    * no line starts with something `mdblock` reads as structure: a list marker
//!      (`- `, `* `, `+ `, `1. `, `1) `, or the marker alone), a checkbox (`[ ] `, `[x] `,
//!      any single character between the brackets), a heading (a run of `#` followed by
//!      whitespace or the end of the line; `#tag` is a tag and is not escaped), a quote
//!      (`>`), a thematic break (`---`, `***`, `___`, `- - -`) or setext underline
//!      (`--`, `===`), a code fence (```` ``` ````, `~~~`), a link reference or footnote
//!      definition (`[x]: …`); such starts are backslash-escaped (`\- `, `1\. `, `\# `,
//!      …) — see [`line_start_escape_at`]. (The `🔒`, `⧉` and `⚠` prefixes need no
//!      escaping: `mdblock` only treats them as markup together with an `ro` flag or a
//!      conflict tag on the same line.)
//!    * leading and trailing whitespace of a line is preserved by encoding it as
//!      `&#32;` / `&#9;` / `&#160;` … (Markdown and our parser both strip it otherwise);
//!    * no line contains a substring that [`find_tags`] recognises, nor a `%%` followed
//!      by `wf:` or `wf-conflict:` (spaces or tabs between allowed): a `^` in `^wf-` is
//!      written `\^`, such a `%%` as `%\%`, `[wid::` as `\[wid::`, `(wid::` as
//!      `\(wid::`, `<!--` as `\<!--`; a trailing Obsidian block id (`… ^abc` at the end
//!      of a line) is written `\^abc`. Any other `%%` (an Obsidian comment) is written
//!      as is: it can neither form a tag nor be read as part of one.
//! 5. **Minimal escaping.** Escape only where needed (`a * b`, `x_y_z`, `3 > 2`, `AT&T`,
//!    `#tag`, `1.5`, and the Obsidian forms `[[Page|alias]]`, `![[img.png]]`,
//!    `![alt](img.png)`, `==hl==`, `$x$`, `%%comment%%`, `text[^1]` stay unescaped); WF text `&lt;b&gt;` renders as `\<b>` so it stays
//!    distinct from real markup (UI:E2).
//!
//! For [`FieldKind::Code`] (the name of a `code-block` node, rendered inside a fence by
//! `mdblock`): `text` is `to_plain_text(x)` verbatim (no escaping; line breaks normalised
//! to `\n`), and `md_to_wf` is `escape_text(m)` (line breaks normalised, characters XML
//! forbids dropped); the tier is `ReadOnly` if the value is malformed, else `Exact` iff
//! that round-trips, else `Lossy`. A code line starting with ```` ``` ```` is not a
//! hazard here: `mdblock` fences code with a backtick run longer than any that starts
//! one of its lines, so such a value reads back byte for byte (§5.3 reserves tier 3 for
//! values the dialect cannot hold).
//! UNVERIFIED: how the UI stores code-block names (§13 Q2); this mapping is a guess.
//!
//! # Id tags (§4.4)
//!
//! A tag carries a [`ShortId`] plus optional flags, written `<12hex>` followed by
//! `-<flag>` for each flag (only `ro` exists: read-only, §5.3 and §10). Five forms are
//! parsed everywhere on a line; [`TagForm`] lists them. A conflict-marker tag
//! `%%wf-conflict:name%%` / `%%wf-conflict:note%%` marks a conflict marker item (§6.1, see
//! `mdblock`). User text can never produce either (EC 4).
//!
//! Recognition rules ([`find_tags`]): keywords (`wf`, `wid`, `wf-conflict`, `name`,
//! `note`) are case-sensitive, hex digits and the `ro` flag are not; spaces or tabs are
//! allowed inside the delimiters of the bracketed forms (`%% wf:… %%`, `[wid::…]`,
//! `<!--wf:…-->`); a block id must end at a word boundary (`^wf-<13 hex>` is not a tag);
//! a tag preceded by an odd number of backslashes is escaped and not a tag.

mod block;
mod chars;
mod dates;
mod merge3;
mod node;
mod parse;
mod render;
mod tags;

use std::collections::BTreeSet;
use std::ops::Range;

use serde::{Deserialize, Serialize};

use crate::tree::ShortId;
use crate::wfhtml;

pub use block::{
    encode_edge_whitespace, escape_line_start, line_is_block_safe, line_start_escape_at,
};

use node::N;
use render::{Level, Renderer};

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
    /// Tier 3: not representable (malformed WF-HTML, a newline in a non-code name).
    /// Rendered as a best-effort, read-only form
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

/// Render `nodes` at `level` and apply the line-level escapes (EC 4) to every line.
fn render_at(nodes: &[N], level: Level, cfg: &InlineConfig) -> (String, bool) {
    let mut r = Renderer::new(cfg, level, nodes);
    let raw = r.render(nodes);
    let text = raw
        .split('\n')
        .map(block::finish_line)
        .collect::<Vec<_>>()
        .join("\n");
    (text, r.used_html)
}

fn block_safe(md: &str, kind: FieldKind) -> bool {
    if kind == FieldKind::Name && md.contains(['\n', '\r']) {
        return false;
    }
    md.split('\n').all(line_is_block_safe)
}

/// The most readable block-safe rendering of `nodes` (for values that do not round-trip).
/// For a name, line breaks are shown as ` ⏎ ` (EC 4).
fn best_effort(nodes: &[N], kind: FieldKind, cfg: &InlineConfig) -> String {
    let join = |md: String| -> String {
        if kind == FieldKind::Name && md.contains('\n') {
            let joined = md.split('\n').collect::<Vec<_>>().join(" ⏎ ");
            encode_edge_whitespace(&joined).into_owned()
        } else {
            md
        }
    };
    for level in [
        Level::Minimal,
        Level::Careful,
        Level::Aggressive,
        Level::Html,
    ] {
        let md = join(render_at(nodes, level, cfg).0);
        if block_safe(&md, kind) {
            return md;
        }
    }
    // Plain text with every ASCII punctuation character escaped is always block-safe.
    let plain = vec![N::Text(node::plain_text(nodes))];
    join(render_at(&plain, Level::Aggressive, cfg).0)
}

/// WF-HTML -> Markdown (§4.2) for one field, with its tier. Total: malformed input gives a
/// best-effort text (`to_plain_text`, escaped) with [`Tier::ReadOnly`].
pub fn wf_to_md(wf: &str, kind: FieldKind, cfg: &InlineConfig) -> MdText {
    if kind == FieldKind::Code {
        return code_to_md(wf, cfg);
    }
    let Ok(frag) = wfhtml::parse(wf) else {
        // §5.3 tier 3: malformed WF-HTML (UI:F1/F2 shows it as an empty bullet).
        let text = chars::sanitize(&wfhtml::to_plain_text(wf));
        return MdText {
            text: best_effort(&[N::Text(text)], kind, cfg),
            tier: Tier::ReadOnly,
        };
    };
    let nodes = node::from_fragment(&frag);
    if kind == FieldKind::Name && node::has_newline(&nodes) {
        // §5.3 tier 3: a newline in a non-code name (§13 Q5).
        return MdText {
            text: best_effort(&nodes, kind, cfg),
            tier: Tier::ReadOnly,
        };
    }
    let mut readable = None;
    for level in [
        Level::Minimal,
        Level::Careful,
        Level::Aggressive,
        Level::Html,
    ] {
        let (md, used_html) = render_at(&nodes, level, cfg);
        if !block_safe(&md, kind) {
            continue;
        }
        if md_to_wf(&md, kind, cfg) == wf {
            let tier = if used_html {
                Tier::Passthrough
            } else {
                Tier::Exact
            };
            return MdText { text: md, tier };
        }
        readable.get_or_insert(md);
    }
    MdText {
        text: readable.unwrap_or_else(|| best_effort(&nodes, kind, cfg)),
        tier: Tier::Lossy,
    }
}

fn code_to_md(wf: &str, cfg: &InlineConfig) -> MdText {
    let text = chars::sanitize(&wfhtml::to_plain_text(wf));
    // §5.3: only malformed values are tier 3. A line starting with ``` needs no special
    // tier: `mdblock` writes a longer fence around it (mdblock `fence_for`).
    let tier = if !wfhtml::is_well_formed(wf) {
        Tier::ReadOnly
    } else if md_to_wf(&text, FieldKind::Code, cfg) == wf {
        Tier::Exact
    } else {
        Tier::Lossy
    };
    MdText { text, tier }
}

/// Markdown -> canonical WF-HTML (§4.2) for one field. Total, and always canonical
/// (EC 2). For `Note`, `\n` separates WF lines (raw newlines in the note, UI:E8). Line
/// endings `\r\n`/`\r` count as `\n`; characters XML forbids are dropped; each line's
/// leading and trailing whitespace is removed (encode it as `&#32;` to keep it); in a
/// `Name`, line breaks become spaces.
pub fn md_to_wf(md: &str, kind: FieldKind, cfg: &InlineConfig) -> String {
    if kind == FieldKind::Code {
        return wfhtml::escape_text(&chars::sanitize(md));
    }
    let nodes = parse::parse(&inline_source(md, kind), cfg, kind);
    let out = node::serialize(&nodes);
    if wfhtml::is_well_formed(&out) && wfhtml::canonicalize(&out) == out {
        out
    } else {
        // Defensive: every element the parser builds is checked to be writable, so this
        // is not expected to happen; plain text is always canonical.
        wfhtml::escape_text(&node::plain_text(&nodes))
    }
}

/// What [`md_to_wf`] parses for a `Name` or `Note`: sanitized, lines trimmed.
fn inline_source(md: &str, kind: FieldKind) -> String {
    let s = chars::sanitize(md);
    match kind {
        // UNVERIFIED: a line break typed into a name becomes a space (the UI's rendering of
        // a name with `\n` is unknown, §13 Q5, so names we write never contain one).
        FieldKind::Name => s.replace('\n', " ").trim_matches(chars::is_ws).to_owned(),
        _ => s
            .split('\n')
            .map(|l| l.trim_matches(chars::is_ws))
            .collect::<Vec<_>>()
            .join("\n"),
    }
}

/// True if [`md_to_wf`] drops part of `md` that WF-HTML cannot hold: the title of a link
/// (`[a](u "t")`, §4.2). The link itself is kept; `merge` reports such a write as lossy.
pub fn drops_link_title(md: &str, kind: FieldKind, cfg: &InlineConfig) -> bool {
    kind != FieldKind::Code && parse::parse_reporting(&inline_source(md, kind), cfg, kind).1
}

/// The tier of `wf` (= `wf_to_md(wf, kind, cfg).tier`).
pub fn classify(wf: &str, kind: FieldKind, cfg: &InlineConfig) -> Tier {
    wf_to_md(wf, kind, cfg).tier
}

/// Result of [`write_value`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WriteValue {
    /// The canonical WF-HTML value to write.
    pub wf: String,
    /// True if something of `base_wf` that the rendering could not show was dropped
    /// (§5.2: "log what was lost"): the three-way merge onto `base_wf` could not be
    /// applied and either the naive write `md_to_wf(new_md)` was used, or the merge was
    /// applied to `canonicalize(base_wf)` instead (dropping only non-canonical markup,
    /// which the UI itself drops on its next edit, UI:F step 2).
    pub lost: bool,
}

fn is_canonical(w: &str) -> bool {
    wfhtml::is_well_formed(w) && wfhtml::canonicalize(w) == w
}

/// Apply the edit `b' -> new` to `base` (§5.2): character-level for names and code,
/// line-level for notes. `None` if the merge conflicts or the result is not canonical.
fn merge_edit(b_prime: &str, new: &str, base: &str, kind: FieldKind) -> Option<String> {
    let merged = if kind == FieldKind::Note {
        let lines = |s: &str| s.split('\n').map(str::to_owned).collect::<Vec<_>>();
        merge3::merge3(&lines(b_prime), &lines(new), &lines(base))?.join("\n")
    } else {
        let chars = |s: &str| s.chars().collect::<Vec<_>>();
        merge3::merge3(&chars(b_prime), &chars(new), &chars(base))?
            .into_iter()
            .collect()
    };
    (is_canonical(&merged) && (kind != FieldKind::Name || !merged.contains('\n'))).then_some(merged)
}

/// The §5.2 refinement for writing an edited field. Let `b = base_wf` and
/// `b' = md_to_wf(wf_to_md(b).text)`. If `b' == b`, return `md_to_wf(new_md)`. Otherwise
/// compute the edit from `b'` to `md_to_wf(new_md)` and apply it to `b` with a three-way
/// text merge (character-level for `Name`/`Code`, line-level for `Note`, using `similar`).
/// The merged result must be well-formed and canonical (EC 2) to be used. If it is not
/// (a merge onto a non-canonical `b` never is), the same merge is tried onto
/// `canonicalize(b)`; if that fails too, or the merge conflicts, the naive write
/// `md_to_wf(new_md)` is used. `lost` reports whether anything of `b` was dropped.
pub fn write_value(base_wf: &str, new_md: &str, kind: FieldKind, cfg: &InlineConfig) -> WriteValue {
    let new = md_to_wf(new_md, kind, cfg);
    let round_trip = |b: &str| md_to_wf(&wf_to_md(b, kind, cfg).text, kind, cfg);
    let b_prime = round_trip(base_wf);
    if b_prime == base_wf {
        return WriteValue {
            wf: new,
            lost: false,
        };
    }
    if let Some(wf) = merge_edit(&b_prime, &new, base_wf, kind) {
        return WriteValue { wf, lost: false };
    }
    let bc = wfhtml::canonicalize(base_wf);
    if bc != base_wf && is_canonical(&bc) {
        let bc_prime = round_trip(&bc);
        if bc_prime != bc
            && let Some(wf) = merge_edit(&bc_prime, &new, &bc, kind)
        {
            return WriteValue { wf, lost: true };
        }
    }
    WriteValue {
        wf: new,
        lost: true,
    }
}

// ---------------------------------------------------------------------------------------
// Helpers for `mdblock` (additive)
// ---------------------------------------------------------------------------------------

/// Plain text -> Markdown satisfying the escaping contract (EC 4/5) for `kind`, as if it
/// were a WF value without markup. For `Code` the text is returned unchanged.
pub fn plain_to_md(text: &str, kind: FieldKind, cfg: &InlineConfig) -> String {
    if kind == FieldKind::Code {
        return text.to_owned();
    }
    wf_to_md(&wfhtml::escape_text(&chars::sanitize(text)), kind, cfg).text
}

/// The plain text a line of Markdown stands for (escapes resolved, markup dropped):
/// `wfhtml::to_plain_text(md_to_wf(md))`.
pub fn md_to_plain(md: &str, kind: FieldKind, cfg: &InlineConfig) -> String {
    wfhtml::to_plain_text(&md_to_wf(md, kind, cfg))
}

/// Remove the whitespace `md_to_wf` ignores at both ends of a line (Unicode
/// `White_Space`). `mdblock` may use it to trim exactly what the inline layer would.
pub fn trim_line(line: &str) -> &str {
    line.trim_matches(chars::is_ws)
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
/// Matching is case-insensitive in the hex digits. Overlapping matches are impossible
/// for well-formed tags; if they occur the leftmost (then longest) wins.
pub fn find_tags(line: &str) -> Vec<TagMatch> {
    tags::find_tags(line)
}

/// Write an id tag: for example `format_tag(&s, &{ReadOnly}, TagForm::BlockId)` is
/// `^wf-8e1f00aa2b3c-ro`, and with `TagForm::Comment` it is `%%wf:8e1f00aa2b3c-ro%%`.
pub fn format_tag(short: &ShortId, flags: &BTreeSet<TagFlag>, form: TagForm) -> String {
    tags::format_tag(short, flags, form)
}

/// Write a conflict-marker tag: `%%wf-conflict:name%%` or `%%wf-conflict:note%%`.
pub fn format_conflict_tag(field: ConflictField) -> String {
    match field {
        ConflictField::Name => "%%wf-conflict:name%%".to_owned(),
        ConflictField::Note => "%%wf-conflict:note%%".to_owned(),
    }
}
