//! Character classes and small scanners shared by the renderer and the parser.

use super::FieldKind;

/// Whitespace for flanking rules and for the line-edge trimming of the parser (and so
/// for the `&#32;` encoding of the renderer, EC 4). CommonMark uses Unicode `Zs` plus
/// tab/LF/FF/CR; Rust's `White_Space` property is a close superset and is used on both
/// sides, so the round trip is consistent.
// UNVERIFIED: Obsidian's exact whitespace set; only consistency between the two
// directions matters for the round trip.
pub(crate) fn is_ws(c: char) -> bool {
    c.is_whitespace()
}

/// Punctuation for flanking rules: ASCII punctuation, or any non-ASCII character that is
/// neither alphanumeric nor whitespace (CommonMark 0.31 counts Unicode `P` and `S`; this
/// approximation also counts marks, which is harmless because both directions agree).
pub(crate) fn is_punct(c: char) -> bool {
    c.is_ascii_punctuation() || (!c.is_ascii() && !c.is_alphanumeric() && !c.is_whitespace())
}

/// Flanking class of a neighbouring character (`None` = start or end of the text, which
/// counts as whitespace).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Class {
    Ws,
    Punct,
    Other,
}

pub(crate) fn class(c: Option<char>) -> Class {
    match c {
        None => Class::Ws,
        Some(c) if is_ws(c) => Class::Ws,
        Some(c) if is_punct(c) => Class::Punct,
        Some(_) => Class::Other,
    }
}

/// CommonMark left- and right-flanking of a delimiter run between `prev` and `next`.
pub(crate) fn flanking(prev: Class, next: Class) -> (bool, bool) {
    let left = next != Class::Ws && (next != Class::Punct || prev != Class::Other);
    let right = prev != Class::Ws && (prev != Class::Punct || next != Class::Other);
    (left, right)
}

/// Whether a delimiter run of `ch` between `prev` and `next` can open / close emphasis
/// (CommonMark rules 1–8; `~` follows `*`, as in GFM).
pub(crate) fn can_open_close(ch: char, prev: Class, next: Class) -> (bool, bool) {
    let (left, right) = flanking(prev, next);
    if ch == '_' {
        (
            left && (!right || prev == Class::Punct),
            right && (!left || next == Class::Punct),
        )
    } else {
        (left, right)
    }
}

/// Characters allowed by XML 1.0 (what `wfhtml::parse` accepts in text).
pub(crate) fn is_xml_char(c: char) -> bool {
    matches!(c, '\t' | '\n' | '\r' | '\u{20}'..='\u{D7FF}' | '\u{E000}'..='\u{FFFD}' | '\u{10000}'..)
}

/// Normalise line endings (`\r\n`, `\r` -> `\n`) and drop characters that XML forbids
/// (the API strips the same C0 controls on update, DM§6.1), so that whatever follows can
/// only produce well-formed WF-HTML (EC 2).
pub(crate) fn sanitize(md: &str) -> String {
    let mut out = String::with_capacity(md.len());
    let mut chars = md.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                out.push('\n');
            }
            c if is_xml_char(c) => out.push(c),
            _ => {}
        }
    }
    out
}

/// Length in bytes of something at the start of `s` (which starts with `&`) that
/// CommonMark (and so Obsidian) reads as a character reference: `&name;` (1–32
/// alphanumerics starting with a letter), `&#1234567;` or `&#x10FFFF;`. The renderer
/// escapes a literal `&` followed by such text as `\&` (§4.2).
pub(crate) fn entity_like_len(s: &str) -> Option<usize> {
    let b = s.as_bytes();
    if b.first() != Some(&b'&') {
        return None;
    }
    let (start, max, pred): (usize, usize, fn(u8) -> bool) = match (b.get(1), b.get(2)) {
        (Some(b'#'), Some(b'x' | b'X')) => (3, 6, |c| c.is_ascii_hexdigit()),
        (Some(b'#'), _) => (2, 7, |c| c.is_ascii_digit()),
        (Some(c), _) if c.is_ascii_alphabetic() => (1, 32, |c| c.is_ascii_alphanumeric()),
        _ => return None,
    };
    let n = b[start..].iter().take_while(|&&c| pred(c)).count();
    if n == 0 || n > max || b.get(start + n) != Some(&b';') {
        return None;
    }
    Some(start + n + 1)
}

/// Decode a character reference at the start of `s` the way `md_to_wf` does (§4.2):
/// `&amp; &lt; &gt; &quot;` and numeric references. A numeric reference is decoded only
/// if it names a character that can be stored (an XML character, not CR, and not LF in a
/// name); otherwise it is literal text. Returns the decoded text and the bytes consumed.
pub(crate) fn decode_entity(s: &str, kind: FieldKind) -> Option<(char, usize)> {
    for (name, c) in [
        ("&amp;", '&'),
        ("&lt;", '<'),
        ("&gt;", '>'),
        ("&quot;", '"'),
    ] {
        if s.starts_with(name) {
            return Some((c, name.len()));
        }
    }
    let len = entity_like_len(s)?;
    let b = s.as_bytes();
    if b[1] != b'#' {
        return None;
    }
    let v = if matches!(b[2], b'x' | b'X') {
        u32::from_str_radix(&s[3..len - 1], 16).ok()?
    } else {
        s[2..len - 1].parse::<u32>().ok()?
    };
    let c = char::from_u32(v)?;
    // UNVERIFIED (our choice): CommonMark would decode these (to U+FFFD for 0); we keep
    // them literal because the decoded character could not be stored or would not survive
    // a UI save (CR becomes LF on re-parse; a name must not hold LF).
    if !is_xml_char(c) || c == '\r' || (c == '\n' && kind == FieldKind::Name) {
        return None;
    }
    Some((c, len))
}

/// The character reference the renderer writes for an edge whitespace character.
pub(crate) fn numeric_ref(c: char) -> String {
    format!("&#{};", c as u32)
}

/// ASCII case-insensitive `starts_with`.
pub(crate) fn starts_with_ci(s: &str, prefix: &str) -> bool {
    s.len() >= prefix.len() && s.as_bytes()[..prefix.len()].eq_ignore_ascii_case(prefix.as_bytes())
}

/// Number of consecutive backslashes immediately before byte offset `i` of `s`.
pub(crate) fn backslashes_before(s: &str, i: usize) -> usize {
    s.as_bytes()[..i]
        .iter()
        .rev()
        .take_while(|&&c| c == b'\\')
        .count()
}

/// Characters after which a bare `http(s)://` URL is recognised (GFM's extended autolink
/// rule: start of line, whitespace, `*`, `_`, `~`, `(`; plus `>` so that a URL right
/// after a passthrough tag such as `<u>` is recognised).
// UNVERIFIED: modelled on cmark-gfm's extended autolinks, not checked against Obsidian.
pub(crate) fn url_boundary(prev: Option<char>) -> bool {
    match prev {
        None => true,
        Some(c) => is_ws(c) || matches!(c, '*' | '_' | '~' | '(' | '>'),
    }
}

/// Length of a bare `http://` / `https://` URL at the start of `s` (GFM extended
/// autolinks, simplified): the scheme, an ASCII alphanumeric, then everything up to
/// whitespace or one of ``< > \ [ ] ` "``; then trailing `? ! . , : * _ ~ '`, an unbalanced
/// `)` and a trailing entity-like `&name;` are trimmed off, repeatedly.
// UNVERIFIED: GFM also autolinks `www.` and is case-insensitive in the scheme; we only
// read lowercase `http://`/`https://` (the UI autolinks pasted URLs, UI:E5).
pub(crate) fn bare_url_len(s: &str) -> Option<usize> {
    let scheme = if s.starts_with("https://") {
        8
    } else if s.starts_with("http://") {
        7
    } else {
        return None;
    };
    if !s[scheme..].starts_with(|c: char| c.is_ascii_alphanumeric()) {
        return None;
    }
    let mut end = s
        .char_indices()
        .find(|&(_, c)| is_ws(c) || matches!(c, '<' | '>' | '\\' | '[' | ']' | '`' | '"'))
        .map_or(s.len(), |(i, _)| i);
    loop {
        let u = &s[..end];
        let last = u.chars().next_back()?;
        let unbalanced = last == ')' && u.matches('(').count() < u.matches(')').count();
        if unbalanced || matches!(last, '?' | '!' | '.' | ',' | ':' | '*' | '_' | '~' | '\'') {
            end -= 1;
        } else if last == ';' {
            let alnum = u[..end - 1]
                .bytes()
                .rev()
                .take_while(|c| c.is_ascii_alphanumeric())
                .count();
            if alnum > 0 && u.as_bytes().get(end - 2 - alnum) == Some(&b'&') {
                end -= alnum + 2;
            } else {
                break;
            }
        } else {
            break;
        }
    }
    (end > scheme).then_some(end)
}

/// Length of a CommonMark autolink `<scheme:…>` at the start of `s`.
pub(crate) fn angle_autolink_len(s: &str) -> Option<usize> {
    let b = s.as_bytes();
    if b.first() != Some(&b'<') || !b.get(1)?.is_ascii_alphabetic() {
        return None;
    }
    let scheme = 1 + b[1..]
        .iter()
        .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, b'+' | b'.' | b'-'))
        .count();
    if !(3..=33).contains(&scheme) || b.get(scheme) != Some(&b':') {
        return None;
    }
    for (i, c) in s[scheme + 1..].char_indices() {
        match c {
            '>' => return Some(scheme + 1 + i + 1),
            '<' => return None,
            c if c.is_ascii_control() || is_ws(c) => return None,
            _ => {}
        }
    }
    None
}
