//! WF inline tree -> Markdown (§4.2). The renderer works at three levels of caution;
//! `wf_to_md` tries them in order and keeps the first whose output round-trips through
//! `md_to_wf` (EC 1) and is block-safe (EC 4):
//!
//! * [`Level::Minimal`]: Markdown constructs, and backslash escapes only where Markdown
//!   (or our parser) would misread the text (EC 5). A delimiter character (`*`, `_`,
//!   `~`, or a backtick when there is no code element) that occurs in a single run in the
//!   whole value cannot pair with anything and is not escaped.
//! * [`Level::Careful`]: the same, without the single-run exception.
//! * [`Level::Aggressive`]: the same constructs, but every ASCII punctuation character in
//!   text is escaped.
//! * [`Level::Html`]: every element except links is written as passthrough HTML, and
//!   text is escaped aggressively. This is the form of last resort; it round-trips for
//!   everything `md_to_wf` produces.

use super::InlineConfig;
use super::block::line_start_escape_at;
use super::chars::{
    Class, angle_autolink_len, bare_url_len, can_open_close, class, entity_like_len, is_ws,
    starts_with_ci, url_boundary,
};
use super::dates::{self, CALENDAR, Date};
use super::node::{El, N, plain_text};
use super::tags::has_hazard;

/// How cautious the renderer is (see the module docs).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Level {
    Minimal,
    Careful,
    Aggressive,
    Html,
}

/// Nesting depth beyond which only passthrough HTML is tried (bounds the search).
const MAX_DEPTH: usize = 12;

/// What is known about the output next to a node.
#[derive(Debug, Clone, Copy)]
enum Side {
    /// Start or end of the value: counts as whitespace (and as a line start).
    Edge,
    /// A known character.
    Char(char),
    /// An element whose rendering is not decided yet: which delimiter characters it may
    /// start or end with, whether it may start with an alphanumeric (a bare URL), and
    /// whether it may start with `[` (a link).
    Elem {
        delims: &'static str,
        alnum: bool,
        bracket: bool,
    },
}

impl Side {
    fn class(self) -> Class {
        match self {
            Side::Edge => Class::Ws,
            Side::Char(c) => class(Some(c)),
            Side::Elem { alnum: true, .. } => Class::Other,
            Side::Elem { .. } => Class::Punct,
        }
    }

    /// True if the neighbour is, or may be, the delimiter character `c`.
    fn may_be(self, c: char) -> bool {
        match self {
            Side::Edge => false,
            Side::Char(x) => x == c,
            Side::Elem { delims, .. } => delims.contains(c),
        }
    }

    fn line_start(self) -> bool {
        matches!(self, Side::Edge | Side::Char('\n'))
    }
}

fn is_autolink_shaped(href: &str, kids: &[N]) -> bool {
    matches!(kids, [N::Text(t)] if t == href)
}

fn side_of(node: &N) -> Side {
    match node {
        N::Text(t) => t.chars().next().map_or(Side::Edge, Side::Char),
        N::El(el, kids) => match el {
            El::B | El::I => Side::Elem {
                delims: "*_",
                alnum: false,
                bracket: false,
            },
            El::S => Side::Elem {
                delims: "~",
                alnum: false,
                bracket: false,
            },
            El::Code => Side::Elem {
                delims: "`",
                alnum: false,
                bracket: false,
            },
            El::A(h) => Side::Elem {
                delims: "",
                alnum: is_autolink_shaped(h, kids) && h.starts_with("http"),
                bracket: true,
            },
            El::U | El::Span(_) | El::Time(_) => Side::Elem {
                delims: "",
                alnum: false,
                bracket: false,
            },
        },
        N::Auto { .. } => Side::Elem {
            delims: "",
            alnum: true,
            bracket: true,
        },
    }
}

/// The renderer for one value at one [`Level`].
pub(crate) struct Renderer<'a> {
    cfg: &'a InlineConfig,
    level: Level,
    /// Escape every `$` (the value has two or more, which Obsidian could read as math).
    dollars: bool,
    /// Delimiter characters that occur in a single run in the whole value (and so cannot
    /// pair); only used at [`Level::Minimal`].
    lone: Vec<char>,
    /// Set when raw inline HTML was emitted (tier `Passthrough`, §5.3).
    pub(crate) used_html: bool,
}

impl<'a> Renderer<'a> {
    pub(crate) fn new(cfg: &'a InlineConfig, level: Level, nodes: &[N]) -> Self {
        Renderer {
            cfg,
            level,
            dollars: plain_text(nodes).matches('$').count() >= 2,
            lone: if level == Level::Minimal {
                lone_delimiters(nodes)
            } else {
                Vec::new()
            },
            used_html: false,
        }
    }

    /// Render a value (without the line-level post-pass).
    pub(crate) fn render(&mut self, nodes: &[N]) -> String {
        self.seq(nodes, Side::Edge, Side::Edge, false, 0)
    }

    fn seq(&mut self, nodes: &[N], prev: Side, next: Side, in_link: bool, depth: usize) -> String {
        let mut buf = String::new();
        for (i, node) in nodes.iter().enumerate() {
            let p = buf.chars().next_back().map_or(prev, Side::Char);
            let rest = &nodes[i + 1..];
            let n = rest.first().map_or(next, side_of);
            self.node(node, p, n, rest, in_link, depth, &mut buf);
        }
        buf
    }

    #[allow(clippy::too_many_arguments)]
    fn node(
        &mut self,
        node: &N,
        p: Side,
        n: Side,
        rest: &[N],
        in_link: bool,
        depth: usize,
        buf: &mut String,
    ) {
        match node {
            N::Text(t) => {
                let s = self.text(t, p, n, in_link);
                buf.push_str(&s);
            }
            N::Auto { url, .. } => {
                let kids = vec![N::Text(url.clone())];
                self.link(url, &kids, p, n, rest, in_link, depth, buf);
            }
            N::El(el, kids) => match el {
                El::B => self.emph(&["**", "__"], el, kids, p, n, in_link, depth, buf),
                El::I => self.emph(&["*", "_"], el, kids, p, n, in_link, depth, buf),
                El::S => self.emph(&["~~"], el, kids, p, n, in_link, depth, buf),
                El::Code => {
                    if self.level < Level::Html
                        && let [N::Text(c)] = kids.as_slice()
                        && code_span_ok(c, p, n)
                    {
                        buf.push_str(&code_span(c));
                        return;
                    }
                    self.passthrough(el, kids, in_link, depth, buf);
                }
                El::U | El::Span(_) => self.passthrough(el, kids, in_link, depth, buf),
                El::Time(attrs) => {
                    if self.cfg.dates
                        && self.level < Level::Html
                        && let Some(d) = Date::of_element(attrs, kids)
                    {
                        // `📅 2026-01-15` followed by text ` 14:30` would read as a time.
                        let ambiguous = d.time.is_none()
                            && matches!(rest.first(), Some(N::Text(t)) if dates::starts_like_time(t));
                        if !ambiguous {
                            buf.push_str(&d.compact());
                            return;
                        }
                    }
                    self.passthrough(el, kids, in_link, depth, buf);
                }
                El::A(h) => self.link(h, kids, p, n, rest, in_link, depth, buf),
            },
        }
    }

    fn passthrough(&mut self, el: &El, kids: &[N], in_link: bool, depth: usize, buf: &mut String) {
        self.used_html = true;
        let in_link = in_link || matches!(el, El::A(_));
        buf.push_str(&safe_start_tag(el));
        let inner = self.seq(kids, Side::Char('>'), Side::Char('<'), in_link, depth + 1);
        buf.push_str(&inner);
        buf.push_str(&el.end_tag());
    }

    /// `<b>`/`<i>`/`<s>` as delimiter runs when CommonMark's flanking rules allow it in
    /// this context, else passthrough HTML (§4.2 row 2).
    #[allow(clippy::too_many_arguments)]
    fn emph(
        &mut self,
        delims: &[&str],
        el: &El,
        kids: &[N],
        p: Side,
        n: Side,
        in_link: bool,
        depth: usize,
        buf: &mut String,
    ) {
        if self.level < Level::Html && depth < MAX_DEPTH {
            for d in delims {
                let c = d.chars().next().unwrap_or('*');
                let saved = self.used_html;
                let content = self.seq(kids, Side::Char(c), Side::Char(c), in_link, depth + 1);
                if delim_ok(c, p, &content, n) {
                    buf.push_str(d);
                    buf.push_str(&content);
                    buf.push_str(d);
                    return;
                }
                self.used_html = saved;
            }
        }
        self.passthrough(el, kids, in_link, depth, buf);
    }

    /// Links (§4.2 rows 4–5): a bare URL or `<u>` for UI autolinks (UI:E5), else
    /// `[text](u)`. Inside a link (Markdown links cannot nest) a link is passthrough HTML.
    #[allow(clippy::too_many_arguments)]
    fn link(
        &mut self,
        h: &str,
        kids: &[N],
        p: Side,
        n: Side,
        rest: &[N],
        in_link: bool,
        depth: usize,
        buf: &mut String,
    ) {
        let el = El::A(h.to_owned());
        if in_link {
            self.passthrough(&el, kids, in_link, depth, buf);
            return;
        }
        if is_autolink_shaped(h, kids) {
            if self.level < Level::Html && bare_ok(h, p, n, rest) {
                buf.push_str(h);
                return;
            }
            if angle_ok(h) {
                buf.push('<');
                buf.push_str(h);
                buf.push('>');
                return;
            }
        }
        let saved = self.used_html;
        let text = self.seq(kids, Side::Char('['), Side::Char(']'), true, depth + 1);
        let form = format!("[{text}]({})", dest(h));
        // A link at a line start that reads as block structure (a checkbox `[ ] `, a
        // link reference definition `[x]: …`) cannot be escaped without breaking it.
        if text.contains("\n\n")
            || text.starts_with('\n')
            || text.ends_with('\n')
            || (p.line_start() && line_start_escape_at(&form).is_some())
        {
            self.used_html = saved;
            self.passthrough(&el, kids, in_link, depth, buf);
            return;
        }
        buf.push_str(&form);
    }

    /// Escape decoded text (§4.2 row 1, EC 5).
    fn text(&self, t: &str, p: Side, n: Side, in_link: bool) -> String {
        let mut out = String::with_capacity(t.len() + 8);
        if self.level >= Level::Aggressive {
            for c in t.chars() {
                if c.is_ascii_punctuation() {
                    out.push('\\');
                    out.push(c);
                } else if c == CALENDAR && self.cfg.dates {
                    out.push_str("&#128197;");
                } else {
                    out.push(c);
                }
            }
            return out;
        }
        let chars: Vec<(usize, char)> = t.char_indices().collect();
        let len = chars.len();
        let mut k = 0;
        while k < len {
            let (i, c) = chars[k];
            let rest = &t[i + c.len_utf8()..];
            let prev_c = k.checked_sub(1).map(|j| chars[j].1);
            let next_c = chars.get(k + 1).map(|x| x.1);
            let last = k + 1 == len;
            let esc = match c {
                // A backslash before punctuation (or a line end, a hard break) is an escape.
                '\\' => last || next_c.is_some_and(|x| x.is_ascii_punctuation() || x == '\n'),
                // Backticks could pair into a code span anywhere.
                '`' => !self.lone.contains(&'`'),
                '*' | '_' | '~' => {
                    let run = chars[k..].iter().take_while(|x| x.1 == c).count();
                    let before = if k == 0 { Err(p) } else { Ok(prev_c) };
                    let after = match chars.get(k + run) {
                        Some(x) => Ok(Some(x.1)),
                        None => Err(n),
                    };
                    let esc = run_needs_escape(c, run, before, after, self.lone.contains(&c));
                    for _ in 0..run {
                        if esc {
                            out.push('\\');
                        }
                        out.push(c);
                    }
                    k += run;
                    continue;
                }
                // Obsidian wikilinks/embeds `[[`, footnotes `[^`, Dataview `[wid::` (EC 4).
                '[' => {
                    in_link
                        || next_c == Some('[')
                        || prev_c == Some('[')
                        || next_c == Some('^')
                        || starts_with_ci(rest, "wid::")
                }
                ']' => in_link || next_c == Some('('),
                // Could start an HTML tag, a comment or an autolink: `&lt;b&gt;` -> `\<b>`.
                '<' => match next_c {
                    Some(x) => x.is_ascii_alphabetic() || matches!(x, '/' | '!' | '?'),
                    None => match n {
                        Side::Char(x) => x.is_ascii_alphabetic(),
                        Side::Elem { alnum, .. } => alnum,
                        Side::Edge => false,
                    },
                },
                '&' => entity_like_len(&t[i..]).is_some(),
                // `![` would be an image or an embed.
                '!' => {
                    next_c == Some('[')
                        || (last
                            && match n {
                                Side::Char(x) => x == '[',
                                Side::Elem { bracket, .. } => bracket,
                                Side::Edge => false,
                            })
                }
                // UNVERIFIED: Obsidian math (`$x$`) and highlight (`==x==`) rules are
                // approximated: escape every `$` when there are two or more in the value,
                // and every `=` in a run of two or more.
                '$' => self.dollars,
                '=' => prev_c == Some('=') || next_c == Some('='),
                '%' => prev_c == Some('%'),
                '^' => starts_with_ci(rest, "wf-"),
                '(' => starts_with_ci(rest, "wid::"),
                ':' => !in_link && url_colon(t, i, p),
                CALENDAR if self.cfg.dates && dates::looks_like_compact_tail(rest) => {
                    out.push_str("&#128197;");
                    k += 1;
                    continue;
                }
                _ => false,
            };
            if esc {
                out.push('\\');
            }
            out.push(c);
            k += 1;
        }
        out
    }
}

/// Whether a run of `run` delimiter characters `c` in text could act as an emphasis
/// delimiter (and so must be escaped). `before`/`after` are the neighbouring characters
/// inside the text (`Ok(None)` = none), or the side outside it (`Err`).
fn run_needs_escape(
    c: char,
    run: usize,
    before: Result<Option<char>, Side>,
    after: Result<Option<char>, Side>,
    lone: bool,
) -> bool {
    let classify = |x: Result<Option<char>, Side>| -> Option<Class> {
        match x {
            Ok(ch) => Some(class(ch)),
            Err(side) if side.may_be(c) => None,
            Err(side) => Some(side.class()),
        }
    };
    let (Some(pc), Some(nc)) = (classify(before), classify(after)) else {
        return true; // it would merge with an adjacent delimiter run
    };
    if lone {
        return false;
    }
    let (open, close) = can_open_close(c, pc, nc);
    (open || close) && (c != '~' || run == 2)
}

/// True if the `:` at byte `i` of `t` is the colon of a bare `http(s)://` URL that our
/// parser (and GFM) would autolink.
fn url_colon(t: &str, i: usize, p: Side) -> bool {
    if !t[i..].starts_with("://") {
        return false;
    }
    let head = &t[..i];
    let start = if head.ends_with("https") {
        i - 5
    } else if head.ends_with("http") {
        i - 4
    } else {
        return false;
    };
    match t[..start].chars().next_back() {
        Some(c) => url_boundary(Some(c)),
        None => match p {
            Side::Edge => true,
            Side::Char(c) => url_boundary(Some(c)),
            Side::Elem { .. } => true,
        },
    }
}

/// The canonical start tag of `el`, with tag-like text in attribute values written as
/// character references (`%&#37;`, `&#94;wf-`, `&#91;wid::`, `&#40;wid::`), which HTML
/// decodes but `find_tags` and Obsidian do not see (EC 4). Backslash escapes do not work
/// inside HTML.
fn safe_start_tag(el: &El) -> String {
    let tag = el.start_tag();
    if !has_hazard(&tag) {
        return tag;
    }
    let mut out = String::with_capacity(tag.len() + 8);
    let mut prev = None;
    for (i, c) in tag.char_indices() {
        let rest = &tag[i + c.len_utf8()..];
        match c {
            '%' if prev == Some('%') => out.push_str("&#37;"),
            '^' if starts_with_ci(rest, "wf-") => out.push_str("&#94;"),
            '[' if starts_with_ci(rest, "wid::") => out.push_str("&#91;"),
            '(' if starts_with_ci(rest, "wid::") => out.push_str("&#40;"),
            _ => out.push(c),
        }
        prev = Some(c);
    }
    out
}

/// Delimiter form validity for an emphasis element with rendered `content` between
/// sides `p` and `n` (CommonMark flanking; no merging with adjacent runs; no blank line).
fn delim_ok(c: char, p: Side, content: &str, n: Side) -> bool {
    let (Some(first), Some(last)) = (content.chars().next(), content.chars().next_back()) else {
        return false;
    };
    if content.contains("\n\n") || first == c || last == c || p.may_be(c) || n.may_be(c) {
        return false;
    }
    let (open, _) = can_open_close(c, p.class(), class(Some(first)));
    let (_, close) = can_open_close(c, class(Some(last)), n.class());
    open && close
}

fn code_span_ok(c: &str, p: Side, n: Side) -> bool {
    !c.is_empty()
        && !c.contains('\n')
        && !has_hazard(c)
        && !p.may_be('`')
        && !n.may_be('`')
        && !(p.line_start() && longest_backtick_run(c) >= 2)
}

fn longest_backtick_run(c: &str) -> usize {
    let mut best = 0;
    let mut cur = 0;
    for ch in c.chars() {
        if ch == '`' {
            cur += 1;
            best = best.max(cur);
        } else {
            cur = 0;
        }
    }
    best
}

/// A CommonMark code span for `c`: a fence longer than any backtick run inside, padded
/// with spaces when the content starts or ends with a backtick or would lose a space.
fn code_span(c: &str) -> String {
    let fence = "`".repeat(longest_backtick_run(c) + 1);
    let pad = c.starts_with('`')
        || c.ends_with('`')
        || (c.len() >= 2 && c.starts_with(' ') && c.ends_with(' ') && !c.chars().all(|x| x == ' '));
    let sp = if pad { " " } else { "" };
    format!("{fence}{sp}{c}{sp}{fence}")
}

/// Can `h` be written as a bare URL here: our parser reads back exactly `h` given the
/// neighbours, and (for Obsidian) it holds nothing GFM would parse as emphasis or an
/// entity, and nothing tag-like (EC 4).
fn bare_ok(h: &str, p: Side, n: Side, rest: &[N]) -> bool {
    if !(h.starts_with("http://") || h.starts_with("https://")) || has_hazard(h) {
        return false;
    }
    let b = h.as_bytes();
    for (i, c) in h.char_indices() {
        let ok = match c {
            '*' => false,
            '_' => {
                i > 0
                    && b[i - 1].is_ascii_alphanumeric()
                    && b.get(i + 1).is_some_and(u8::is_ascii_alphanumeric)
            }
            '~' => !h[i + 1..].starts_with('~') && !h[..i].ends_with('~'),
            '&' => entity_like_len(&h[i..]).is_none(),
            _ => true,
        };
        if !ok {
            return false;
        }
    }
    let prev_ok = match p {
        Side::Edge => true,
        Side::Char(c) => url_boundary(Some(c)),
        Side::Elem { .. } => false,
    };
    if !prev_ok {
        return false;
    }
    // What our parser would see after the URL, up to the first whitespace.
    let side_follow = || match n {
        Side::Edge => Some(String::new()),
        Side::Char(c) => Some(format!("{c}{c}")),
        Side::Elem { .. } => None,
    };
    let follow = match rest.first() {
        Some(N::Text(t)) => match t.char_indices().find(|&(_, c)| is_ws(c)) {
            Some((i, c)) => t[..i + c.len_utf8()].to_owned(),
            None if rest.len() == 1 => match side_follow() {
                Some(f) => format!("{t}{f}"),
                None => return false,
            },
            None => return false,
        },
        Some(_) => return false,
        None => match side_follow() {
            Some(f) => f,
            None => return false,
        },
    };
    bare_url_len(&format!("{h}{follow}")) == Some(h.len())
}

/// Delimiter characters with a single run in the value's text (see [`Level::Minimal`]).
fn lone_delimiters(nodes: &[N]) -> Vec<char> {
    fn has_code(nodes: &[N]) -> bool {
        nodes.iter().any(|n| match n {
            N::El(El::Code, _) => true,
            N::El(_, kids) => has_code(kids),
            _ => false,
        })
    }
    let text = plain_text(nodes);
    ['*', '_', '~', '`']
        .into_iter()
        .filter(|&c| {
            let runs = text
                .char_indices()
                .filter(|&(i, x)| x == c && !text[..i].ends_with(c))
                .count();
            runs == 1 && (c != '`' || !has_code(nodes))
        })
        .collect()
}

/// Can `h` be written as a CommonMark autolink `<h>`.
fn angle_ok(h: &str) -> bool {
    angle_autolink_len(&format!("<{h}>")) == Some(h.len() + 2)
        && !has_hazard(h)
        && !h
            .char_indices()
            .any(|(i, c)| c == '&' && entity_like_len(&h[i..]).is_some())
}

/// A link destination: plain, or the angle form `<…>` when it contains spaces, parens,
/// angle brackets or control characters (§4.2 row 5); backslash escapes for `\`, `<`,
/// `>`, entity-like `&` and tag-like text (EC 4).
fn dest(h: &str) -> String {
    if h.is_empty() {
        return "<>".to_owned();
    }
    let angle = h
        .chars()
        .any(|c| matches!(c, ' ' | '(' | ')' | '<' | '>') || c.is_ascii_control());
    let mut out = String::with_capacity(h.len() + 4);
    if angle {
        out.push('<');
    }
    let mut prev = None;
    for (i, c) in h.char_indices() {
        let rest = &h[i + c.len_utf8()..];
        let esc = match c {
            '\\' => true,
            '<' | '>' => angle,
            '&' => entity_like_len(&h[i..]).is_some(),
            '%' => prev == Some('%'),
            '^' => starts_with_ci(rest, "wf-"),
            '[' | '(' => starts_with_ci(rest, "wid::"),
            _ => false,
        };
        if esc {
            out.push('\\');
        }
        out.push(c);
        prev = Some(c);
    }
    if angle {
        out.push('>');
    }
    out
}
