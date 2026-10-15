//! Line-level escaping (§4.2 "Escaping is the trickiest part", EC 4): what a line of
//! inline Markdown must never start with, edge whitespace, trailing block ids.

use std::borrow::Cow;

use super::chars::{backslashes_before, is_ws, numeric_ref};
use super::tags::has_hazard;

/// If `line` starts with something a Markdown block parser (Obsidian's, or `mdblock`'s)
/// would read as structure, the byte offset where a backslash must be inserted to make it
/// text. Recognised starts:
///
/// * list markers `-`, `+`, `*` followed by whitespace or the end of the line;
/// * ordered markers `1.` / `1)` (1–9 digits) followed by whitespace or the end (the
///   backslash goes before the `.` / `)`);
/// * ATX headings: a run of `#` followed by whitespace or the end (`#tag` is a tag, not
///   a heading, and stays unescaped);
/// * block quotes `>`;
/// * checkboxes `[ ]`, `[x]` (any one character between the brackets) followed by
///   whitespace or the end, and link reference / footnote definitions `[…]:`;
/// * thematic breaks (`---`, `***`, `___`, `- - -`) and setext underlines (`=`/`-` runs);
/// * code fences ```` ``` ```` and `~~~`.
pub fn line_start_escape_at(line: &str) -> Option<usize> {
    let b = line.as_bytes();
    let first = *b.first()?;
    let ws_or_end = |i: usize| line[i..].chars().next().is_none_or(is_ws);
    match first {
        b'-' | b'+' | b'*' if ws_or_end(1) => return Some(0),
        b'>' => return Some(0),
        b'#' => {
            let n = b.iter().take_while(|&&c| c == b'#').count();
            if ws_or_end(n) {
                return Some(0);
            }
        }
        b'0'..=b'9' => {
            let n = b.iter().take_while(|c| c.is_ascii_digit()).count();
            if n <= 9 && matches!(b.get(n), Some(b'.' | b')')) && ws_or_end(n + 1) {
                return Some(n);
            }
        }
        b'[' => {
            let mut it = line[1..].char_indices();
            if let (Some((_, mid)), Some((i, ']'))) = (it.next(), it.next())
                && mid != '\\'
                && ws_or_end(1 + i + 1)
            {
                return Some(0);
            }
            let close = line
                .match_indices(']')
                .map(|(i, _)| i)
                .find(|&i| backslashes_before(line, i).is_multiple_of(2));
            if close.is_some_and(|i| b.get(i + 1) == Some(&b':')) {
                return Some(0);
            }
        }
        _ => {}
    }
    if line.starts_with("```") || line.starts_with("~~~") {
        return Some(0);
    }
    if matches!(first, b'-' | b'*' | b'_' | b'=') {
        let body = line.trim_end_matches([' ', '\t']);
        let runs = body.bytes().filter(|&c| c == first).count();
        let only = body.bytes().all(|c| c == first || c == b' ' || c == b'\t');
        let thematic = first != b'=' && only && runs >= 3;
        let setext = matches!(first, b'-' | b'=') && body.bytes().all(|c| c == first);
        if thematic || setext {
            return Some(0);
        }
    }
    None
}

/// Escape a structural line start (see [`line_start_escape_at`]) with a backslash.
pub fn escape_line_start(line: &str) -> Cow<'_, str> {
    match line_start_escape_at(line) {
        Some(i) => Cow::Owned(format!("{}\\{}", &line[..i], &line[i..])),
        None => Cow::Borrowed(line),
    }
}

/// Byte offset of a `^` that Obsidian would read as a block id: `^` + `[A-Za-z0-9-]+`
/// at the end of the line (trailing whitespace allowed), at the line start or after
/// whitespace, and not already escaped.
// UNVERIFIED: Obsidian's block-id syntax as documented (`^id` after whitespace at the end
// of a paragraph); not checked against the app.
fn trailing_block_id_at(line: &str) -> Option<usize> {
    let body = line.trim_end_matches(is_ws);
    let id = body
        .bytes()
        .rev()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == b'-')
        .count();
    if id == 0 || body.len() < id + 1 {
        return None;
    }
    let caret = body.len() - id - 1;
    if body.as_bytes()[caret] != b'^' || backslashes_before(body, caret) % 2 == 1 {
        return None;
    }
    let before = body[..caret].chars().next_back();
    before.is_none_or(is_ws).then_some(caret)
}

/// Encode leading and trailing whitespace of a line as character references (`&#32;`,
/// `&#9;`, …), since Markdown and `md_to_wf` strip it (EC 4). A backslash left
/// unescaped right before an encoded character is doubled so that it stays literal.
pub fn encode_edge_whitespace(line: &str) -> Cow<'_, str> {
    let start = line.len() - line.trim_start_matches(is_ws).len();
    if start == line.len() {
        return Cow::Owned(line.chars().map(numeric_ref).collect());
    }
    let end = line.trim_end_matches(is_ws).len();
    if start == 0 && end == line.len() {
        return Cow::Borrowed(line);
    }
    let mut out: String = line[..start].chars().map(numeric_ref).collect();
    out.push_str(&line[start..end]);
    if end < line.len() && backslashes_before(line, end) % 2 == 1 {
        out.push('\\');
    }
    out.extend(line[end..].chars().map(numeric_ref));
    Cow::Owned(out)
}

/// Apply the line-level escapes to one rendered line: structural starts, a trailing
/// Obsidian block id, then edge whitespace.
pub(crate) fn finish_line(line: &str) -> String {
    let mut s = escape_line_start(line).into_owned();
    if let Some(i) = trailing_block_id_at(&s) {
        s.insert(i, '\\');
    }
    encode_edge_whitespace(&s).into_owned()
}

/// True if a line satisfies the block-safety part of the escaping contract (EC 4): no
/// structural start, no edge whitespace, no trailing block id, no id tag or conflict tag
/// and no `%%` that starts one, no line break.
pub fn line_is_block_safe(line: &str) -> bool {
    !line.contains(['\n', '\r'])
        && line_start_escape_at(line).is_none()
        && trailing_block_id_at(line).is_none()
        && line.trim_matches(is_ws).len() == line.len()
        && !has_hazard(line)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_starts() {
        for (line, want) in [
            ("- x", "\\- x"),
            ("-", "\\-"),
            ("+ x", "\\+ x"),
            ("* x", "\\* x"),
            ("1. foo", "1\\. foo"),
            ("12) foo", "12\\) foo"),
            ("1.", "1\\."),
            ("# x", "\\# x"),
            ("### x", "\\### x"),
            ("#", "\\#"),
            ("> x", "\\> x"),
            (">x", "\\>x"),
            ("[ ] x", "\\[ ] x"),
            ("[x] x", "\\[x] x"),
            ("[x]", "\\[x]"),
            ("[foo]: bar", "\\[foo]: bar"),
            ("[^1]: note", "\\[^1]: note"),
            ("[a\\]b]: x", "\\[a\\]b]: x"),
            ("---", "\\---"),
            ("--", "\\--"),
            ("***", "\\***"),
            ("___", "\\___"),
            ("_ _ _", "\\_ _ _"),
            ("===", "\\==="),
            ("```", "\\```"),
            ("```rust", "\\```rust"),
            ("~~~", "\\~~~"),
        ] {
            assert_eq!(escape_line_start(line), want, "{line:?}");
        }
        for line in [
            "#tag",
            "-x",
            "[\\]",
            "[a\\]: b",
            "1.5 kg",
            "1990",
            "[link](u)",
            "[ab] x",
            "*x*",
            "**b**",
            "x - y",
            "a > b",
            "--x",
            "__init__",
            "***bi***",
            "=x",
            "~~s~~",
            "``x``",
        ] {
            assert_eq!(escape_line_start(line), line, "{line:?}");
        }
    }

    #[test]
    fn edges_and_block_ids() {
        assert_eq!(encode_edge_whitespace(" x\t"), "&#32;x&#9;");
        assert_eq!(encode_edge_whitespace("  "), "&#32;&#32;");
        assert_eq!(encode_edge_whitespace("x"), "x");
        assert_eq!(encode_edge_whitespace("a\\ "), "a\\\\&#32;");
        assert_eq!(encode_edge_whitespace("\u{a0}x"), "&#160;x");
        assert_eq!(finish_line("see ^abc"), "see \\^abc");
        assert_eq!(finish_line("^abc-1 "), "\\^abc-1&#32;");
        assert_eq!(finish_line("x^2"), "x^2");
        assert_eq!(finish_line("see \\^abc"), "see \\^abc");
        assert_eq!(finish_line("- "), "\\-&#32;");
    }

    #[test]
    fn safety() {
        assert!(line_is_block_safe("plain **text**"));
        assert!(!line_is_block_safe("x ^wf-8e1f00aa2b3c"));
        assert!(line_is_block_safe("x \\^wf-8e1f00aa2b3c"));
        // An Obsidian comment delimiter is safe; one that starts a tag is not.
        assert!(line_is_block_safe("a %% b"));
        assert!(!line_is_block_safe("a %%wf: b"));
        assert!(!line_is_block_safe("a %% \twf-conflict:x"));
        assert!(line_is_block_safe("a %\\%wf: b"));
        assert!(!line_is_block_safe(" x"));
        assert!(!line_is_block_safe("- x"));
        assert!(!line_is_block_safe("x [wid:: 8e1f00aa2b3c]"));
        assert!(line_is_block_safe("x \\[wid:: 8e1f00aa2b3c]"));
    }
}
