//! WF-HTML: Workflowy's rich-text format for `name` and `note`.
//!
//! Responsibility (design §2 "Text is WF-HTML", §3 "wfhtml", §5.4 last bullet): parse,
//! escape, canonicalise and check well-formedness of WF-HTML, and expose a small inline
//! element tree so that `mdinline` can walk it. This is a **port of
//! `src/syncflowy/wfhtml.py`** (read its docstrings: they cite the UI findings E1–E8,
//! F1/F2 that justify every rule). The Python module is the oracle: for every input
//! string, [`escape_text`], [`is_well_formed`], [`canonicalize`], [`to_plain_text`] and
//! [`append_typed_text`] must return **byte-for-byte** what the Python functions return.
//! The implementer should add a differential test that runs a corpus through both (for
//! example `uv run python -c 'from syncflowy import wfhtml; …'` from the repository root,
//! the same way `testsupport` runs the mock), including the E and F tables of
//! docs/workflowy/ui-findings.md and the vectors in tests/test_wfhtml.py.
//!
//! # Facts the port must reproduce (from wfhtml.py and Python's expat)
//!
//! * A value is an **XML fragment**: it is parsed as the content of a synthetic root
//!   element. Only XML's five predefined entities (`&amp; &lt; &gt; &quot; &apos;`) and
//!   numeric character references (`&#39;`, `&#x27;`) are accepted; `&nbsp;` and any
//!   other named entity make the value malformed (UI:F15). A raw `<` or a bare `&` in
//!   text is malformed (UI:F13); unclosed or mismatched tags are malformed (UI:F08);
//!   void HTML tags like `<br>` and `<img …>` are malformed (UI:F05/F06).
//! * Exception: a bare `&` *inside a start tag's attribute values* that does not begin
//!   an entity reference is tolerated (the UI writes `href="…?b=1&c=2"` itself, UI:E5);
//!   wfhtml.py rewrites such `&` to `&amp;` before parsing (`_fix_attr_amps`).
//! * Comments and processing instructions are dropped (UI:F11). CDATA sections are
//!   character data. Expat's XML rules apply: characters not allowed in XML 1.0 (for
//!   example U+0001) make the value malformed; `\r\n` and lone `\r` in text become `\n`;
//!   tabs and newlines inside attribute values become spaces (attribute-value
//!   normalisation); attribute names are case-sensitive in parsing.
//! * [`canonicalize`] = parse, then serialise with: text escaped by [`escape_text`];
//!   tag names lowercased; `strong` -> `b`, `em` -> `i`; `b i u s code` written bare (no
//!   attributes); `<a>` keeps only `href` (escaped with `"` -> `&quot;`, `<` -> `&lt;`, a
//!   raw `&` kept), `javascript:` links dropped together with their text (UI:F07), `<a>`
//!   without `href` unwrapped; `<span>` kept (with only its `class`) iff its class's
//!   first word is `colored`; `<time>` kept with all attributes in source order; every
//!   other element unwrapped (its text kept). Malformed input canonicalises to `""`.
//!
//! # Contract for writers
//!
//! Everything the engine writes to Workflowy must be well-formed and canonical:
//! `is_well_formed(w) && canonicalize(w) == w` (§1, §5.4). [`serialize`] produces
//! canonical output from an [`Inline`] tree by construction.

use thiserror::Error;

/// Tags kept verbatim (without attributes) by [`canonicalize`].
pub const FORMAT_TAGS: [&str; 5] = ["b", "i", "u", "s", "code"];

/// Aliases rewritten on canonicalisation: `strong` -> `b` (UI:F10) and `em` -> `i`
/// (UNVERIFIED in wfhtml.py, kept for byte compatibility).
pub const TAG_ALIASES: [(&str, &str); 2] = [("strong", "b"), ("em", "i")];

/// One node of a parsed WF-HTML fragment.
///
/// Text is stored **decoded** (entities resolved: `a &amp; b` is `Text("a & b")`).
/// Adjacent text runs are always merged into one `Text`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Inline {
    /// Decoded character data.
    Text(String),
    /// An element as written in the source: tag name and attribute names exactly as in
    /// the source (not lowercased, not aliased), attribute values decoded, in source
    /// order.
    Element {
        /// Tag name as written.
        tag: String,
        /// `(name, decoded value)` pairs in source order.
        attrs: Vec<(String, String)>,
        /// Child nodes.
        children: Vec<Inline>,
    },
}

/// A parsed fragment: the children of the synthetic root.
pub type Fragment = Vec<Inline>;

/// Why a value is not well-formed WF-HTML.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("malformed WF-HTML at byte {offset}: {message}")]
pub struct MalformedError {
    /// Byte offset into the input where the problem was detected (best effort).
    pub offset: usize,
    /// Human-readable reason.
    pub message: String,
}

/// Escape plain text the way the UI stores it (UI:E1): `&` `<` `>` `"` become
/// `&amp; &lt; &gt; &quot;`; `'` stays raw. Same as wfhtml.py `escape_text`.
pub fn escape_text(text: &str) -> String {
    todo!("port of wfhtml.escape_text: {text:?}")
}

/// Parse `s` as an XML fragment with the rules in the module docs. `Err` exactly when
/// wfhtml.py's `_parse` returns `None`.
pub fn parse(s: &str) -> Result<Fragment, MalformedError> {
    todo!("parse WF-HTML fragment {s:?}")
}

/// True if the UI can parse `s` (otherwise it renders an empty bullet and the next UI
/// edit wipes it, UI:F1/F2). Same as wfhtml.py `is_well_formed`.
pub fn is_well_formed(s: &str) -> bool {
    todo!("is_well_formed({s:?})")
}

/// Serialise a fragment in **canonical** form (wfhtml.py `_serialize`): the rules in the
/// module docs. For every `s`: `canonicalize(s) == parse(s).map(|f| serialize(&f))
/// .unwrap_or_default()`, and `serialize` output is always a fixed point of
/// [`canonicalize`].
pub fn serialize(fragment: &[Inline]) -> String {
    todo!("serialize {} nodes", fragment.len())
}

/// What the UI stores after any edit of a node whose current value is `s`; malformed
/// input becomes `""`. Same as wfhtml.py `canonicalize`.
pub fn canonicalize(s: &str) -> String {
    todo!("canonicalize({s:?})")
}

/// The visible text of a name or note with entities decoded. For malformed input:
/// best-effort text (tags stripped with the regex `<[^>]*>`, then HTML entities
/// decoded with Python `html.unescape` semantics; the full HTML5 named-entity table is
/// not required, but `&amp; &lt; &gt; &quot; &#NN; &#xHH; &nbsp; &apos;` must work).
/// Same as wfhtml.py `to_plain_text` for well-formed input.
pub fn to_plain_text(s: &str) -> String {
    todo!("to_plain_text({s:?})")
}

/// Simulate typing plain text `typed` at the very end of a name in the UI: canonicalise
/// (malformed counts as empty), escape `typed`, and insert it inside any trailing run of
/// extending tags (`b i u s code span`, not `a`/`time`). Same as wfhtml.py
/// `append_typed_text`. Used by tests that imitate UI edits.
pub fn append_typed_text(s: &str, typed: &str) -> String {
    todo!("append_typed_text({s:?}, {typed:?})")
}

impl Inline {
    /// The canonical start tag of this element as [`serialize`] would write it (for
    /// example `<b>`, `<a href="u">`, `<span class="colored c-red">`, `<time a="1">`),
    /// or `None` if canonical serialisation drops the tag (unknown elements, `<a>`
    /// without href, `javascript:` links, non-`colored` spans) or if this is text.
    /// `mdinline` uses it to emit passthrough HTML that round-trips exactly.
    pub fn canonical_start_tag(&self) -> Option<String> {
        todo!("canonical_start_tag of {self:?}")
    }

    /// The canonical (lowercased, aliased) tag name of an element; `None` for text.
    pub fn canonical_tag(&self) -> Option<String> {
        todo!("canonical_tag of {self:?}")
    }

    /// The value of attribute `name` (exact, case-sensitive match first, then
    /// case-insensitive, as wfhtml.py lowercases attribute names for lookups).
    pub fn attr(&self, name: &str) -> Option<&str> {
        todo!("attr {name:?} of {self:?}")
    }
}
