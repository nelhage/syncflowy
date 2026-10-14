//! WF-HTML: Workflowy's rich-text format for `name` and `note`.
//!
//! Responsibility (design §2 "Text is WF-HTML", §3 "wfhtml", §5.4 last bullet): parse,
//! escape, canonicalise and check well-formedness of WF-HTML, and expose a small inline
//! element tree so that `mdinline` can walk it. This is a **port of
//! `src/syncflowy/wfhtml.py`** (read its docstrings: they cite the UI findings E1–E8,
//! F1/F2 that justify every rule). The Python module is the oracle: for every input
//! string, [`escape_text`], [`is_well_formed`], [`canonicalize`], [`to_plain_text`] and
//! [`append_typed_text`] must return **byte-for-byte** what the Python functions return.
//! The differential test `engine/tests/wfhtml_differential.rs` runs a hand-written
//! corpus (the E and F tables of docs/workflowy/ui-findings.md, the vectors of
//! tests/test_wfhtml.py and every parser rule) and a generated corpus through both.
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
//! canonical output from an [`Inline`] tree by construction, **except** for the
//! oracle quirks below, which a writer must avoid (check `canonicalize(w) == w`).
//!
//! # The parser
//!
//! Rust has no expat, so [`parse`] is a small hand-written XML-content parser whose
//! *acceptance* matches expat 2.x (as driven by Python's pyexpat on `str` input) for a
//! fragment wrapped in `<_wfroot>…</_wfroot>`:
//!
//! * elements must nest and end tags must match exactly (case-sensitive); an end tag
//!   at the top level closes the synthetic root, after which the appended
//!   `</_wfroot>` is junk, so it is always an error;
//! * start tags: `<Name (S Name S? = S? ("…"|'…'))* S? (>|/>)`, whitespace required
//!   between attributes, no duplicate attribute names, no `<` in attribute values;
//! * names use expat's XML 1.0 4th-edition name tables (`name_tables.rs`, probed from
//!   pyexpat by `engine/tests/fixtures/wfhtml/probe_names.py`); `:` is an ordinary name
//!   character (no namespace processing);
//! * references: `&amp; &lt; &gt; &quot; &apos;` and `&#N;` / `&#xH;` (lowercase `x`
//!   only) whose value is an XML 1.0 `Char`; anything else is an error;
//! * comments `<!-- … -->` (no `--` inside), processing instructions `<?target …?>`
//!   (a target spelled `xml` in any case is an error) and CDATA sections
//!   `<![CDATA[ … ]]>`; any other `<!` is an error;
//! * the text sequence `]]>` is an error; every character must be an XML 1.0 `Char`
//!   (U+0000, C0 controls other than tab/LF/CR, U+FFFE and U+FFFF are errors);
//! * end-of-line handling: `\r\n` and `\r` become `\n` in text and CDATA; in attribute
//!   values tab, LF, CR and CR LF each become one space (character references are
//!   not normalised).
//!
//! # Oracle quirks reproduced on purpose
//!
//! These follow from wfhtml.py's choices and are kept for byte compatibility:
//!
//! * Attribute values keep a raw `&` when serialised (UI:E5), so an attribute whose
//!   *decoded* value contains `&`-text that looks like a reference is not a fixed point:
//!   `<a href="&amp;amp;">` canonicalises to `<a href="&amp;">` (whose href is `&`), and
//!   `<a href="&amp;nbsp;">` canonicalises to `<a href="&nbsp;">`, which is
//!   **malformed** (undefined entity). So `canonicalize` is not idempotent and its
//!   output is not always well-formed for such inputs.
//! * Character references to CR (`&#13;`) in text, and to tab/LF/CR in attribute
//!   values, are serialised raw; re-parsing normalises them (to LF, or to a space), so
//!   these inputs are not fixed points either.
//! * `_fix_attr_amps` is a regex over the raw string, so it also rewrites `&` inside
//!   comments, CDATA sections and attribute values that merely look like start tags:
//!   `<![CDATA[<x &>]]>` yields the text `<x &amp;>`.
//! * [`append_typed_text`] uses Python's `$`, which also matches before a final `\n`:
//!   `append_typed_text("<b>x</b>\n", "Z") == "<b>xZ</b>\n"`.
//!
//! Python behaviour not reproduced: inputs that make Python raise instead of returning
//! (nesting deeper than Python's recursion limit in `_serialize`; `html.unescape` of a
//! decimal reference with more than 4300 digits) have no oracle value; here they
//! return normally.

use std::borrow::Cow;

use thiserror::Error;

mod html5_entities;
mod name_tables;

/// Tags kept verbatim (without attributes) by [`canonicalize`].
pub const FORMAT_TAGS: [&str; 5] = ["b", "i", "u", "s", "code"];

/// Aliases rewritten on canonicalisation: `strong` -> `b` (UI:F10) and `em` -> `i`
/// (UNVERIFIED in wfhtml.py, kept for byte compatibility).
pub const TAG_ALIASES: [(&str, &str); 2] = [("strong", "b"), ("em", "i")];

/// Tags whose trailing run is extended when the user types at the very end of a name
/// (wfhtml.py `_EXTENDING_TAGS`): observed for `u`, `span`, `b` (UI:F01/F02/F09/F10).
// UNVERIFIED: `i`, `s`, `code` extend by analogy; `a` and `time` deliberately do not
// (choices made in wfhtml.py, ported as is).
const EXTENDING_TAGS: [&str; 6] = ["b", "i", "u", "s", "code", "span"];

/// One node of a parsed WF-HTML fragment.
///
/// Text is stored **decoded** (entities resolved: `a &amp; b` is `Text("a & b")`).
/// Adjacent text runs are always merged into one `Text` (also across dropped comments
/// and processing instructions, and with CDATA sections); `Text` is never empty.
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
    /// Byte offset where the problem was detected (best effort). It is an offset into
    /// the input after the attribute-ampersand fix-up, which inserts `amp;` after each
    /// bare `&` inside a start tag; for inputs without such `&` it is an offset into
    /// the input itself.
    pub offset: usize,
    /// Human-readable reason.
    pub message: String,
}

// ---------------------------------------------------------------------------------
// Escaping
// ---------------------------------------------------------------------------------

/// Escape plain text the way the UI stores it (UI:E1): `&` `<` `>` `"` become
/// `&amp; &lt; &gt; &quot;`; `'` stays raw. Same as wfhtml.py `escape_text`.
pub fn escape_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    escape_text_into(text, &mut out);
    out
}

fn escape_text_into(text: &str, out: &mut String) {
    let mut last = 0;
    for (i, b) in text.bytes().enumerate() {
        let rep = match b {
            b'&' => "&amp;",
            b'<' => "&lt;",
            b'>' => "&gt;",
            b'"' => "&quot;",
            _ => continue,
        };
        out.push_str(&text[last..i]);
        out.push_str(rep);
        last = i + 1;
    }
    out.push_str(&text[last..]);
}

/// wfhtml.py `_escape_attr`: only `"` and `<` are escaped; a raw `&` is kept, as the UI
/// writes it in hrefs (UI:E5).
fn escape_attr_into(value: &str, out: &mut String) {
    let mut last = 0;
    for (i, b) in value.bytes().enumerate() {
        let rep = match b {
            b'"' => "&quot;",
            b'<' => "&lt;",
            _ => continue,
        };
        out.push_str(&value[last..i]);
        out.push_str(rep);
        last = i + 1;
    }
    out.push_str(&value[last..]);
}

// ---------------------------------------------------------------------------------
// _fix_attr_amps
// ---------------------------------------------------------------------------------

/// wfhtml.py `_fix_attr_amps`: in every match of `<[A-Za-z][^<>]*>` (a start-tag-shaped
/// run, found left to right without overlap), replace each `&` that does not begin
/// `[A-Za-z][A-Za-z0-9]*;`, `#[0-9]+;` or `#x[0-9A-Fa-f]+;` by `&amp;` (UI:E5).
fn fix_attr_amps(s: &str) -> Cow<'_, str> {
    let b = s.as_bytes();
    if !b.contains(&b'&') {
        return Cow::Borrowed(s);
    }
    let mut out = String::new();
    let mut last = 0;
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'<' && i + 1 < b.len() && b[i + 1].is_ascii_alphabetic() {
            // `[^<>]*` then `>`: the first `<` or `>` after the letter decides.
            if let Some(off) = b[i + 2..].iter().position(|&c| c == b'<' || c == b'>') {
                let j = i + 2 + off;
                if b[j] == b'>' {
                    let tag = &s[i..=j];
                    if tag.contains('&') {
                        out.push_str(&s[last..i]);
                        fix_amps_in(tag, &mut out);
                        last = j + 1;
                    }
                    i = j + 1;
                    continue;
                }
            }
        }
        i += 1;
    }
    if last == 0 {
        return Cow::Borrowed(s);
    }
    out.push_str(&s[last..]);
    Cow::Owned(out)
}

/// wfhtml.py `_BARE_AMP_RE` substitution inside one start-tag-shaped match.
fn fix_amps_in(tag: &str, out: &mut String) {
    let b = tag.as_bytes();
    let mut last = 0;
    for (i, &c) in b.iter().enumerate() {
        if c == b'&' && !starts_entity_shape(&b[i + 1..]) {
            out.push_str(&tag[last..=i]);
            out.push_str("amp;");
            last = i + 1;
        }
    }
    out.push_str(&tag[last..]);
}

/// Lookahead of `_BARE_AMP_RE`: `[A-Za-z][A-Za-z0-9]*;|#[0-9]+;|#x[0-9A-Fa-f]+;`.
fn starts_entity_shape(rest: &[u8]) -> bool {
    fn run_then_semicolon(rest: &[u8], pred: impl Fn(u8) -> bool) -> bool {
        let n = rest.iter().take_while(|&&c| pred(c)).count();
        n > 0 && rest.get(n) == Some(&b';')
    }
    match rest.first() {
        Some(c) if c.is_ascii_alphabetic() => {
            run_then_semicolon(&rest[1..], |c| c.is_ascii_alphanumeric())
        }
        Some(b'#') => {
            run_then_semicolon(&rest[1..], |c| c.is_ascii_digit())
                || (rest.get(1) == Some(&b'x')
                    && run_then_semicolon(&rest[2..], |c| c.is_ascii_hexdigit()))
        }
        _ => false,
    }
}

// ---------------------------------------------------------------------------------
// The XML fragment parser (expat-compatible acceptance)
// ---------------------------------------------------------------------------------

/// XML 1.0 `Char` (what expat accepts in a UTF-8 document and as a character
/// reference).
fn is_xml_char(c: char) -> bool {
    matches!(c, '\t' | '\n' | '\r' | '\u{20}'..='\u{D7FF}' | '\u{E000}'..='\u{FFFD}' | '\u{10000}'..)
}

fn in_ranges(c: char, ranges: &[(u32, u32)]) -> bool {
    let c = c as u32;
    ranges
        .binary_search_by(|&(lo, hi)| {
            if hi < c {
                std::cmp::Ordering::Less
            } else if lo > c {
                std::cmp::Ordering::Greater
            } else {
                std::cmp::Ordering::Equal
            }
        })
        .is_ok()
}

fn is_name_start(c: char) -> bool {
    if c.is_ascii() {
        c.is_ascii_alphabetic() || c == '_' || c == ':'
    } else {
        in_ranges(c, name_tables::NAME_START)
    }
}

fn is_name_char(c: char) -> bool {
    if c.is_ascii() {
        c.is_ascii_alphanumeric() || matches!(c, '_' | ':' | '-' | '.')
    } else {
        in_ranges(c, name_tables::NAME_CHAR)
    }
}

/// XML `S`.
fn is_s(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | b'\r')
}

/// An open element during parsing.
struct Frame {
    tag: String,
    attrs: Vec<(String, String)>,
    children: Vec<Inline>,
}

/// Append text to a child list, merging with a preceding text node (wfhtml.py `chars`).
fn push_text(children: &mut Vec<Inline>, text: &str) {
    if text.is_empty() {
        return;
    }
    if let Some(Inline::Text(t)) = children.last_mut() {
        t.push_str(text);
    } else {
        children.push(Inline::Text(text.to_owned()));
    }
}

fn push_char(children: &mut Vec<Inline>, c: char) {
    let mut buf = [0u8; 4];
    push_text(children, c.encode_utf8(&mut buf));
}

struct Parser<'a> {
    s: &'a str,
    b: &'a [u8],
    pos: usize,
    /// `stack[0]` is the synthetic root.
    stack: Vec<Frame>,
}

type PResult<T> = Result<T, MalformedError>;

impl<'a> Parser<'a> {
    fn new(s: &'a str) -> Self {
        Parser {
            s,
            b: s.as_bytes(),
            pos: 0,
            stack: vec![Frame {
                tag: String::new(),
                attrs: Vec::new(),
                children: Vec::new(),
            }],
        }
    }

    fn err<T>(&self, at: usize, message: impl Into<String>) -> PResult<T> {
        Err(MalformedError {
            offset: at,
            message: message.into(),
        })
    }

    fn peek(&self) -> Option<u8> {
        self.b.get(self.pos).copied()
    }

    fn peek_char(&self) -> Option<char> {
        self.s[self.pos..].chars().next()
    }

    fn rest(&self) -> &'a str {
        &self.s[self.pos..]
    }

    fn children(&mut self) -> &mut Vec<Inline> {
        &mut self.stack.last_mut().expect("root frame").children
    }

    fn run(mut self) -> PResult<Fragment> {
        while let Some(c) = self.peek() {
            match c {
                b'<' => self.markup()?,
                b'&' => {
                    let ch = self.reference()?;
                    push_char(self.children(), ch);
                }
                b'\r' => {
                    // End-of-line handling: CR LF and lone CR become LF.
                    self.pos += 1;
                    if self.peek() == Some(b'\n') {
                        self.pos += 1;
                    }
                    push_text(self.children(), "\n");
                }
                _ => self.char_data()?,
            }
        }
        if self.stack.len() > 1 {
            let tag = &self.stack.last().expect("frame").tag;
            return self.err(self.pos, format!("unclosed element <{tag}>"));
        }
        Ok(self.stack.pop().expect("root frame").children)
    }

    /// A run of plain character data (no `<`, `&` or CR).
    fn char_data(&mut self) -> PResult<()> {
        let start = self.pos;
        let mut end = start;
        for (i, c) in self.s[start..].char_indices() {
            match c {
                '<' | '&' | '\r' => break,
                ']' if self.s[start + i..].starts_with("]]>") => {
                    return self.err(start + i, "']]>' in character data");
                }
                c if !is_xml_char(c) => {
                    return self.err(start + i, format!("invalid character U+{:04X}", c as u32));
                }
                _ => end = start + i + c.len_utf8(),
            }
        }
        self.pos = end;
        let text = &self.s[start..end];
        push_text(self.children(), text);
        Ok(())
    }

    /// An XML name at the current position.
    fn name(&mut self) -> PResult<&'a str> {
        let start = self.pos;
        match self.peek_char() {
            Some(c) if is_name_start(c) => self.pos += c.len_utf8(),
            Some(c) => return self.err(start, format!("{c:?} cannot start a name")),
            None => return self.err(start, "unexpected end of input, expected a name"),
        }
        while let Some(c) = self.peek_char() {
            if !is_name_char(c) {
                break;
            }
            self.pos += c.len_utf8();
        }
        Ok(&self.s[start..self.pos])
    }

    /// Skip `S*`; true if anything was skipped.
    fn skip_s(&mut self) -> bool {
        let start = self.pos;
        while self.peek().is_some_and(is_s) {
            self.pos += 1;
        }
        self.pos > start
    }

    fn expect(&mut self, byte: u8) -> PResult<()> {
        if self.peek() == Some(byte) {
            self.pos += 1;
            Ok(())
        } else {
            self.err(self.pos, format!("expected {:?}", byte as char))
        }
    }

    /// A reference at `&` (in content or in an attribute value); returns its character.
    fn reference(&mut self) -> PResult<char> {
        let start = self.pos;
        self.pos += 1; // '&'
        if self.peek() == Some(b'#') {
            self.pos += 1;
            let hex = self.peek() == Some(b'x'); // expat: lowercase `x` only
            if hex {
                self.pos += 1;
            }
            let digits_start = self.pos;
            let mut value: u32 = 0;
            while let Some(d) = self.peek() {
                let v = if hex {
                    (d as char).to_digit(16)
                } else {
                    (d as char).to_digit(10)
                };
                let Some(v) = v else { break };
                // Saturate: anything >= 0x110000 is rejected below, like expat.
                value = value
                    .saturating_mul(if hex { 16 } else { 10 })
                    .saturating_add(v)
                    .min(0x110000);
                self.pos += 1;
            }
            if self.pos == digits_start {
                return self.err(start, "malformed character reference");
            }
            if self.peek() != Some(b';') {
                return self.err(start, "character reference without ';'");
            }
            self.pos += 1;
            return match char::from_u32(value).filter(|&c| is_xml_char(c)) {
                Some(c) => Ok(c),
                None => self.err(start, "reference to an invalid character"),
            };
        }
        let name = self.name()?;
        if self.peek() != Some(b';') {
            return self.err(start, "entity reference without ';'");
        }
        self.pos += 1;
        match name {
            "amp" => Ok('&'),
            "lt" => Ok('<'),
            "gt" => Ok('>'),
            "quot" => Ok('"'),
            "apos" => Ok('\''),
            _ => self.err(start, format!("undefined entity &{name};")),
        }
    }

    /// Markup at `<`.
    fn markup(&mut self) -> PResult<()> {
        let start = self.pos;
        let rest = self.rest();
        if rest.starts_with("</") {
            self.end_tag()
        } else if rest.starts_with("<!--") {
            self.comment()
        } else if rest.starts_with("<![CDATA[") {
            self.cdata()
        } else if rest.starts_with("<!") {
            self.err(start, "unsupported markup declaration")
        } else if rest.starts_with("<?") {
            self.pi()
        } else {
            self.start_tag()
        }
    }

    fn end_tag(&mut self) -> PResult<()> {
        let start = self.pos;
        self.pos += 2;
        let name = self.name()?;
        self.skip_s();
        self.expect(b'>')?;
        if self.stack.len() == 1 {
            // Closes the synthetic root: what follows (at least the appended
            // `</_wfroot>`) is junk after the document element.
            return self.err(start, format!("end tag </{name}> without a start tag"));
        }
        let top = self.stack.last().expect("frame");
        if top.tag != name {
            let msg = format!("mismatched end tag </{name}> for <{}>", top.tag);
            return self.err(start, msg);
        }
        let f = self.stack.pop().expect("frame");
        self.children().push(Inline::Element {
            tag: f.tag,
            attrs: f.attrs,
            children: f.children,
        });
        Ok(())
    }

    fn start_tag(&mut self) -> PResult<()> {
        let start = self.pos;
        self.pos += 1;
        let tag = self.name()?.to_owned();
        let mut attrs: Vec<(String, String)> = Vec::new();
        loop {
            let had_space = self.skip_s();
            match self.peek() {
                None => return self.err(start, "unclosed start tag"),
                Some(b'>') => {
                    self.pos += 1;
                    self.stack.push(Frame {
                        tag,
                        attrs,
                        children: Vec::new(),
                    });
                    return Ok(());
                }
                Some(b'/') => {
                    self.pos += 1;
                    self.expect(b'>')?;
                    self.children().push(Inline::Element {
                        tag,
                        attrs,
                        children: Vec::new(),
                    });
                    return Ok(());
                }
                Some(_) => {
                    if !had_space {
                        return self.err(self.pos, "expected whitespace, '>' or '/>'");
                    }
                    let name_at = self.pos;
                    let name = self.name()?;
                    self.skip_s();
                    self.expect(b'=')?;
                    self.skip_s();
                    let value = self.attr_value()?;
                    if attrs.iter().any(|(k, _)| k == name) {
                        return self.err(name_at, format!("duplicate attribute {name}"));
                    }
                    attrs.push((name.to_owned(), value));
                }
            }
        }
    }

    /// A quoted attribute value, normalised (CDATA attribute: no collapsing).
    fn attr_value(&mut self) -> PResult<String> {
        let quote = match self.peek() {
            Some(q @ (b'"' | b'\'')) => q,
            _ => return self.err(self.pos, "expected a quoted attribute value"),
        };
        self.pos += 1;
        let mut out = String::new();
        loop {
            let Some(c) = self.peek_char() else {
                return self.err(self.pos, "unterminated attribute value");
            };
            match c {
                _ if c as u32 == quote as u32 => {
                    self.pos += 1;
                    return Ok(out);
                }
                '<' => return self.err(self.pos, "'<' in attribute value"),
                '&' => out.push(self.reference()?),
                '\r' => {
                    self.pos += 1;
                    if self.peek() == Some(b'\n') {
                        self.pos += 1;
                    }
                    out.push(' ');
                }
                '\n' | '\t' => {
                    self.pos += 1;
                    out.push(' ');
                }
                c if !is_xml_char(c) => {
                    let msg = format!("invalid character U+{:04X}", c as u32);
                    return self.err(self.pos, msg);
                }
                c => {
                    self.pos += c.len_utf8();
                    out.push(c);
                }
            }
        }
    }

    /// Check that every character of `self.s[from..to]` is an XML `Char`.
    fn check_chars(&self, from: usize, to: usize) -> PResult<()> {
        match self.s[from..to]
            .char_indices()
            .find(|&(_, c)| !is_xml_char(c))
        {
            Some((i, c)) => self.err(from + i, format!("invalid character U+{:04X}", c as u32)),
            None => Ok(()),
        }
    }

    /// `<!-- … -->`, dropped (UI:F11). `--` may only appear as part of `-->`.
    fn comment(&mut self) -> PResult<()> {
        let start = self.pos;
        let body = start + 4;
        let Some(off) = self.s[body..].find("--") else {
            return self.err(start, "unterminated comment");
        };
        let dashes = body + off;
        self.check_chars(body, dashes)?;
        if self.b.get(dashes + 2) != Some(&b'>') {
            return self.err(dashes, "'--' inside a comment");
        }
        self.pos = dashes + 3;
        Ok(())
    }

    /// `<![CDATA[ … ]]>`: character data, with end-of-line handling.
    fn cdata(&mut self) -> PResult<()> {
        let start = self.pos;
        let body = start + 9;
        let Some(off) = self.s[body..].find("]]>") else {
            return self.err(start, "unterminated CDATA section");
        };
        let end = body + off;
        self.check_chars(body, end)?;
        let text = &self.s[body..end];
        if text.contains('\r') {
            let t = text.replace("\r\n", "\n").replace('\r', "\n");
            push_text(self.children(), &t);
        } else {
            push_text(self.children(), text);
        }
        self.pos = end + 3;
        Ok(())
    }

    /// `<?target …?>`, dropped. A target spelled `xml` (any case) is an error in content.
    fn pi(&mut self) -> PResult<()> {
        let start = self.pos;
        self.pos += 2;
        let target = self.name()?;
        if target.eq_ignore_ascii_case("xml") {
            return self.err(start, "XML declaration or reserved PI target in content");
        }
        if self.rest().starts_with("?>") {
            self.pos += 2;
            return Ok(());
        }
        if !self.peek().is_some_and(is_s) {
            return self.err(self.pos, "malformed processing instruction");
        }
        let body = self.pos;
        let Some(off) = self.s[body..].find("?>") else {
            return self.err(start, "unterminated processing instruction");
        };
        self.check_chars(body, body + off)?;
        self.pos = body + off + 2;
        Ok(())
    }
}

/// Parse `s` as an XML fragment with the rules in the module docs. `Err` exactly when
/// wfhtml.py's `_parse` returns `None`.
pub fn parse(s: &str) -> Result<Fragment, MalformedError> {
    let fixed = fix_attr_amps(s);
    Parser::new(&fixed).run()
}

/// True if the UI can parse `s` (otherwise it renders an empty bullet and the next UI
/// edit wipes it, UI:F1/F2). Same as wfhtml.py `is_well_formed`.
pub fn is_well_formed(s: &str) -> bool {
    parse(s).is_ok()
}

// ---------------------------------------------------------------------------------
// Canonical serialisation
// ---------------------------------------------------------------------------------

/// Python's `str.isspace()` (what `str.split()` and `str.strip()` use).
fn py_isspace(c: char) -> bool {
    matches!(
        c,
        '\t'..='\r'
            | '\u{1C}'..='\u{20}'
            | '\u{85}'
            | '\u{A0}'
            | '\u{1680}'
            | '\u{2000}'..='\u{200A}'
            | '\u{2028}'
            | '\u{2029}'
            | '\u{202F}'
            | '\u{205F}'
            | '\u{3000}'
    )
}

/// Python's `str.lower()` for the comparisons wfhtml.py makes. Rust's `to_lowercase`
/// uses the same Unicode default case mapping (both map U+0130 to `i̇` and U+212A to
/// `k`, the only non-ASCII characters whose lowercase contains ASCII).
fn py_lower(s: &str) -> String {
    s.to_lowercase()
}

/// The canonical tag name: lowercased, then aliased (`strong` -> `b`, `em` -> `i`).
fn canonical_tag_name(tag: &str) -> String {
    let lower = py_lower(tag);
    for (from, to) in TAG_ALIASES {
        if lower == from {
            return to.to_owned();
        }
    }
    lower
}

/// wfhtml.py `attrs = {k.lower(): v ...}; attrs.get(name)`: the value of the **last**
/// attribute whose lowercased name is `name`.
fn py_attr<'x>(attrs: &'x [(String, String)], name: &str) -> Option<&'x str> {
    attrs
        .iter()
        .rev()
        .find(|(k, _)| py_lower(k) == name)
        .map(|(_, v)| v.as_str())
}

/// How [`serialize`] treats an element.
enum Canon {
    /// Write `start`, the children, then `</end>`.
    Wrap { start: String, end: &'static str },
    /// Write only the children.
    Unwrap,
    /// Write nothing (`javascript:` link, UI:F07).
    Drop,
}

fn canon_of(tag: &str, attrs: &[(String, String)]) -> Canon {
    let ctag = canonical_tag_name(tag); // UI:F09 <B> -> <b>, UI:F10 <strong> -> <b>
    if let Some(&ft) = FORMAT_TAGS.iter().find(|&&t| t == ctag) {
        // UNVERIFIED (wfhtml.py): attributes of format tags are dropped.
        return Canon::Wrap {
            start: format!("<{ft}>"),
            end: ft,
        };
    }
    match ctag.as_str() {
        "a" => match py_attr(attrs, "href") {
            Some(href) if py_lower(href.trim_matches(py_isspace)).starts_with("javascript:") => {
                Canon::Drop // UI:F07: the link and its text are dropped
            }
            // UNVERIFIED (wfhtml.py): <a> without href is unwrapped.
            None => Canon::Unwrap,
            Some(href) => {
                // UI:E5: only href is kept.
                let mut start = String::from("<a href=\"");
                escape_attr_into(href, &mut start);
                start.push_str("\">");
                Canon::Wrap { start, end: "a" }
            }
        },
        "span" => {
            let class = py_attr(attrs, "class").unwrap_or("");
            // UI:F02 colour spans survive. UNVERIFIED (wfhtml.py): any class whose
            // first word is "colored" is kept.
            if class.split(py_isspace).find(|w| !w.is_empty()) == Some("colored") {
                let mut start = String::from("<span class=\"");
                escape_attr_into(class, &mut start);
                start.push_str("\">");
                Canon::Wrap { start, end: "span" }
            } else {
                Canon::Unwrap
            }
        }
        "time" => {
            // UNVERIFIED (wfhtml.py): dates keep all attributes, in source order.
            let mut start = String::from("<time");
            for (k, v) in attrs {
                start.push(' ');
                start.push_str(k);
                start.push_str("=\"");
                escape_attr_into(v, &mut start);
                start.push('"');
            }
            start.push('>');
            Canon::Wrap { start, end: "time" }
        }
        // UI:F03 <script>, F04 <div>, F12 <mark>: unknown tags are unwrapped, their
        // text kept; void tags (<br/>, <img/>) therefore vanish.
        _ => Canon::Unwrap,
    }
}

fn serialize_into(nodes: &[Inline], out: &mut String) {
    for n in nodes {
        match n {
            Inline::Text(t) => escape_text_into(t, out),
            Inline::Element {
                tag,
                attrs,
                children,
            } => match canon_of(tag, attrs) {
                Canon::Wrap { start, end } => {
                    out.push_str(&start);
                    serialize_into(children, out);
                    out.push_str("</");
                    out.push_str(end);
                    out.push('>');
                }
                Canon::Unwrap => serialize_into(children, out),
                Canon::Drop => {}
            },
        }
    }
}

/// Serialise a fragment in **canonical** form (wfhtml.py `_serialize`): the rules in the
/// module docs. For every `s`: `canonicalize(s) == parse(s).map(|f| serialize(&f))
/// .unwrap_or_default()`, and `serialize` output is a fixed point of [`canonicalize`]
/// except for the oracle quirks listed in the module docs (attribute values containing
/// reference-shaped `&` text, CR in text, tab/LF/CR in attribute values).
pub fn serialize(fragment: &[Inline]) -> String {
    let mut out = String::new();
    serialize_into(fragment, &mut out);
    out
}

/// What the UI stores after any edit of a node whose current value is `s`; malformed
/// input becomes `""` (UI:F05/F06/F08/F13/F15). Same as wfhtml.py `canonicalize`.
pub fn canonicalize(s: &str) -> String {
    match parse(s) {
        Ok(f) => serialize(&f),
        Err(_) => String::new(),
    }
}

// ---------------------------------------------------------------------------------
// Plain text
// ---------------------------------------------------------------------------------

fn text_into(nodes: &[Inline], out: &mut String) {
    for n in nodes {
        match n {
            Inline::Text(t) => out.push_str(t),
            Inline::Element { children, .. } => text_into(children, out),
        }
    }
}

/// wfhtml.py `_TAG_RE.sub("", s)` with `_TAG_RE = <[^>]*>`.
fn strip_tags(s: &str) -> Cow<'_, str> {
    if !s.contains('<') {
        return Cow::Borrowed(s);
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(lt) = rest.find('<') {
        let Some(gt) = rest[lt..].find('>') else {
            break; // no '>' after this '<', hence none after any later '<' either
        };
        out.push_str(&rest[..lt]);
        rest = &rest[lt + gt + 1..];
    }
    out.push_str(rest);
    Cow::Owned(out)
}

/// Python `html._invalid_charrefs` (HTML5 numeric reference fix-ups).
fn invalid_charref(num: u32) -> Option<char> {
    const C1: [char; 32] = [
        '\u{20AC}', '\u{81}', '\u{201A}', '\u{192}', '\u{201E}', '\u{2026}', '\u{2020}',
        '\u{2021}', '\u{2C6}', '\u{2030}', '\u{160}', '\u{2039}', '\u{152}', '\u{8D}', '\u{17D}',
        '\u{8F}', '\u{90}', '\u{2018}', '\u{2019}', '\u{201C}', '\u{201D}', '\u{2022}', '\u{2013}',
        '\u{2014}', '\u{2DC}', '\u{2122}', '\u{161}', '\u{203A}', '\u{153}', '\u{9D}', '\u{17E}',
        '\u{178}',
    ];
    match num {
        0x00 => Some('\u{FFFD}'),
        0x0D => Some('\r'),
        0x80..=0x9F => Some(C1[(num - 0x80) as usize]),
        _ => None,
    }
}

/// Python `html._invalid_codepoints`: references to these are removed.
fn invalid_codepoint(num: u32) -> bool {
    matches!(num, 0x1..=0x8 | 0xB | 0xE..=0x1F | 0x7F..=0x9F | 0xFDD0..=0xFDEF)
        || (num & 0xFFFE == 0xFFFE && num <= 0x10FFFF)
}

fn html5_lookup(name: &str) -> Option<&'static str> {
    html5_entities::HTML5
        .binary_search_by(|(k, _)| (*k).cmp(name))
        .ok()
        .map(|i| html5_entities::HTML5[i].1)
}

/// Python's `html.unescape` (HTML5 rules, full named-reference table).
fn html_unescape(s: &str) -> Cow<'_, str> {
    if !s.contains('&') {
        return Cow::Borrowed(s);
    }
    let b = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut last = 0;
    let mut i = 0;
    while i < b.len() {
        if b[i] != b'&' {
            i += 1;
            continue;
        }
        // _charref = &(#[0-9]+;?|#[xX][0-9a-fA-F]+;?|[^\t\n\f <&#;]{1,32};?)
        let start = i + 1;
        let (end, rep): (usize, Cow<'static, str>) = if b.get(start) == Some(&b'#') {
            let (radix, digits_at) = if b.get(start + 1).is_some_and(u8::is_ascii_digit) {
                (10, start + 1)
            } else if matches!(b.get(start + 1), Some(b'x' | b'X'))
                && b.get(start + 2).is_some_and(u8::is_ascii_hexdigit)
            {
                (16, start + 2)
            } else {
                i += 1;
                continue; // no alternative matches at this '&'
            };
            let mut j = digits_at;
            let mut num: u32 = 0;
            while let Some(v) = b.get(j).and_then(|&d| (d as char).to_digit(radix)) {
                num = num.saturating_mul(radix).saturating_add(v).min(0x0011_0000);
                j += 1;
            }
            if b.get(j) == Some(&b';') {
                j += 1;
            }
            let rep: Cow<'static, str> = if let Some(c) = invalid_charref(num) {
                Cow::Owned(c.to_string())
            } else if (0xD800..=0xDFFF).contains(&num) || num > 0x10FFFF {
                Cow::Borrowed("\u{FFFD}")
            } else if invalid_codepoint(num) {
                Cow::Borrowed("")
            } else {
                Cow::Owned(char::from_u32(num).expect("valid scalar").to_string())
            };
            (j, rep)
        } else {
            let mut j = start;
            for (n, c) in s[start..].chars().enumerate() {
                if n == 32 || matches!(c, '\t' | '\n' | '\x0C' | ' ' | '<' | '&' | '#' | ';') {
                    break;
                }
                j += c.len_utf8();
            }
            if j == start {
                i += 1;
                continue;
            }
            if b.get(j) == Some(&b';') {
                j += 1;
            }
            let name = &s[start..j];
            let rep: Cow<'static, str> = if let Some(v) = html5_lookup(name) {
                Cow::Borrowed(v)
            } else {
                // The longest prefix (of at least 2 characters, shorter than the whole)
                // that is a known name.
                let bounds: Vec<usize> = name.char_indices().map(|(k, _)| k).skip(2).collect();
                match bounds
                    .iter()
                    .rev()
                    .find_map(|&k| html5_lookup(&name[..k]).map(|v| (k, v)))
                {
                    Some((k, v)) => Cow::Owned(format!("{v}{}", &name[k..])),
                    None => Cow::Owned(format!("&{name}")),
                }
            };
            (j, rep)
        };
        out.push_str(&s[last..i]);
        out.push_str(&rep);
        last = end;
        i = end;
    }
    out.push_str(&s[last..]);
    Cow::Owned(out)
}

/// The visible text of a name or note with entities decoded. For malformed input:
/// best-effort text (tags stripped with the regex `<[^>]*>`, then HTML entities
/// decoded with Python `html.unescape` semantics, including the full HTML5
/// named-reference table). Same as wfhtml.py `to_plain_text`.
pub fn to_plain_text(s: &str) -> String {
    match parse(s) {
        Ok(f) => {
            let mut out = String::new();
            text_into(&f, &mut out);
            out
        }
        Err(_) => html_unescape(&strip_tags(s)).into_owned(),
    }
}

// ---------------------------------------------------------------------------------
// Typing simulation
// ---------------------------------------------------------------------------------

/// The closers of `_TRAILING_CLOSERS_RE = (?:</([a-z]+)>)+$` searched in `base`: the
/// start of the match and the closer names, or `None`. Python's `$` also matches just
/// before a final `\n`.
fn trailing_closers(base: &str) -> Option<(usize, Vec<&str>)> {
    let end = if base.ends_with('\n') {
        base.len() - 1
    } else {
        base.len()
    };
    let b = base.as_bytes();
    let mut pos = end;
    let mut names = Vec::new();
    // Closers are self-delimiting, so parsing backwards from the end yields the same
    // maximal run as the leftmost-starting forward match.
    while pos > 0 && b[pos - 1] == b'>' {
        let mut k = pos - 1;
        while k > 0 && b[k - 1].is_ascii_lowercase() {
            k -= 1;
        }
        if k == pos - 1 || k < 2 || &b[k - 2..k] != b"</" {
            break;
        }
        names.push(&base[k..pos - 1]);
        pos = k - 2;
    }
    if names.is_empty() {
        return None;
    }
    names.reverse();
    Some((pos, names))
}

/// Simulate typing plain text `typed` at the very end of a name in the UI: canonicalise
/// (malformed counts as empty), escape `typed`, and insert it inside any trailing run of
/// extending tags (`b i u s code span`, not `a`/`time`) (UI:F01/F02/F09/F10). Same as
/// wfhtml.py `append_typed_text`. Used by tests that imitate UI edits.
pub fn append_typed_text(s: &str, typed: &str) -> String {
    let base = canonicalize(s);
    let esc = escape_text(typed);
    if let Some((start, closers)) = trailing_closers(&base) {
        // Extend only through a suffix of extending tags.
        let mut k = closers.len();
        while k > 0 && EXTENDING_TAGS.contains(&closers[k - 1]) {
            k -= 1;
        }
        if k < closers.len() {
            let split = start + closers[..k].iter().map(|t| t.len() + 3).sum::<usize>();
            return format!("{}{}{}", &base[..split], esc, &base[split..]);
        }
    }
    base + &esc
}

impl Inline {
    /// The canonical start tag of this element as [`serialize`] would write it (for
    /// example `<b>`, `<a href="u">`, `<span class="colored c-red">`, `<time a="1">`),
    /// or `None` if canonical serialisation drops the tag (unknown elements, `<a>`
    /// without href, `javascript:` links, non-`colored` spans) or if this is text.
    /// `mdinline` uses it to emit passthrough HTML that round-trips exactly.
    pub fn canonical_start_tag(&self) -> Option<String> {
        match self {
            Inline::Text(_) => None,
            Inline::Element { tag, attrs, .. } => match canon_of(tag, attrs) {
                Canon::Wrap { start, .. } => Some(start),
                Canon::Unwrap | Canon::Drop => None,
            },
        }
    }

    /// The canonical (lowercased, aliased) tag name of an element; `None` for text.
    pub fn canonical_tag(&self) -> Option<String> {
        match self {
            Inline::Text(_) => None,
            Inline::Element { tag, .. } => Some(canonical_tag_name(tag)),
        }
    }

    /// The value of attribute `name` (exact, case-sensitive match first, then
    /// case-insensitive, as wfhtml.py lowercases attribute names for lookups; among
    /// several case-insensitive matches the last wins, like wfhtml.py's dict). `None`
    /// for text.
    pub fn attr(&self, name: &str) -> Option<&str> {
        match self {
            Inline::Text(_) => None,
            Inline::Element { attrs, .. } => attrs
                .iter()
                .find(|(k, _)| k == name)
                .map(|(_, v)| v.as_str())
                .or_else(|| py_attr(attrs, &py_lower(name))),
        }
    }

    /// True if this element is dropped *with its contents* by canonical serialisation
    /// (a `javascript:` link, UI:F07). `false` for text and every other element.
    pub fn is_dropped(&self) -> bool {
        match self {
            Inline::Text(_) => false,
            Inline::Element { tag, attrs, .. } => matches!(canon_of(tag, attrs), Canon::Drop),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fix_attr_amps_matches_regex() {
        assert_eq!(fix_attr_amps("a & b"), "a & b");
        assert_eq!(
            fix_attr_amps(r#"<a href="x?b=1&c=2">t & u</a>"#),
            r#"<a href="x?b=1&amp;c=2">t & u</a>"#
        );
        assert_eq!(
            fix_attr_amps("<a b='&amp;&#1;&#x1;&#X1;&a-b;'>"),
            "<a b='&amp;&#1;&#x1;&amp;#X1;&amp;a-b;'>"
        );
        assert_eq!(fix_attr_amps("<a &<b &>"), "<a &<b &amp;>");
        assert_eq!(fix_attr_amps("<1 &>"), "<1 &>");
    }

    #[test]
    fn trailing_closers_cases() {
        assert_eq!(trailing_closers("<b>x</b>"), Some((4, vec!["b"])));
        assert_eq!(trailing_closers("<b>x</b>\n"), Some((4, vec!["b"])));
        assert_eq!(trailing_closers("x</i></b>"), Some((1, vec!["i", "b"])));
        assert_eq!(trailing_closers("x</>"), None);
        assert_eq!(trailing_closers("x"), None);
        assert_eq!(trailing_closers(">"), None);
        assert_eq!(trailing_closers("</a>"), Some((0, vec!["a"])));
    }

    #[test]
    fn unescape_rules() {
        assert_eq!(
            html_unescape("&amp &ampx &notit; &#128;&#0;&#x1;&#xD800;&#99999999999;"),
            "& &x ¬it; €\u{FFFD}\u{FFFD}\u{FFFD}"
        );
        assert_eq!(html_unescape("&#; &#x; &zzz; &"), "&#; &#x; &zzz; &");
    }
}
