//! Markdown inline -> WF inline tree (§4.2). A single left-to-right scan in the manner of
//! CommonMark's inline parser: code spans, autolinks, the passthrough HTML tags,
//! backslash escapes and character references bind first; then links (a bracket stack,
//! with the "no links inside links" rule) and passthrough elements (an opener stack) form
//! when their closer is seen; emphasis is resolved last within each range by CommonMark's
//! `process_emphasis` (flanking, rule of 3), with GFM's `~~` strikethrough.
//!
//! Deliberate differences from CommonMark (documented in the module docs of
//! `mdinline`): code spans do not cross line breaks; only the WF tag vocabulary is inline
//! HTML (anything else is literal text) and it must be balanced; images (`![…](…)`) are
//! literal text; link titles are not supported (`[a](b "t")` is literal); only `&amp;
//! &lt; &gt; &quot;` and numeric references are decoded; bare URLs are atomic tokens.

use super::chars::{
    angle_autolink_len, bare_url_len, can_open_close, class, decode_entity, url_boundary,
};
use super::dates::{self, CALENDAR};
use super::node::{El, N, push_node, push_text};
use super::{FieldKind, InlineConfig};
use crate::wfhtml::{self, Inline};

#[derive(Debug)]
enum Item {
    Text(String),
    Delim {
        ch: char,
        count: usize,
        orig: usize,
        can_open: bool,
        can_close: bool,
    },
    Node(N),
    /// The raw source of an opener (`[`, `![`, a start tag); literal text unless it is
    /// closed while still on the opener stack.
    Open(String),
}

#[derive(Debug)]
enum OpKind {
    Bracket { image: bool, active: bool },
    Html(El),
}

#[derive(Debug)]
struct Opener {
    item: usize,
    /// Byte offset of the opener in the source.
    src: usize,
    kind: OpKind,
}

struct Parser<'a> {
    s: &'a str,
    pos: usize,
    cfg: &'a InlineConfig,
    kind: FieldKind,
    items: Vec<Item>,
    stack: Vec<Opener>,
}

/// The tags `md_to_wf` accepts as inline HTML (§4.2).
const HTML_TAGS: [&str; 8] = ["code", "span", "time", "b", "i", "s", "u", "a"];

/// Parse preprocessed Markdown (lines already trimmed and joined with `\n`).
pub(crate) fn parse(src: &str, cfg: &InlineConfig, kind: FieldKind) -> Vec<N> {
    let mut p = Parser {
        s: src,
        pos: 0,
        cfg,
        kind,
        items: Vec::new(),
        stack: Vec::new(),
    };
    p.run();
    let nodes = process_emphasis(std::mem::take(&mut p.items));
    finalize(nodes)
}

impl Parser<'_> {
    fn prev_char(&self) -> Option<char> {
        self.s[..self.pos].chars().next_back()
    }

    fn text(&mut self, s: &str) {
        if let Some(Item::Text(t)) = self.items.last_mut() {
            t.push_str(s);
        } else {
            self.items.push(Item::Text(s.to_owned()));
        }
    }

    fn run(&mut self) {
        while self.pos < self.s.len() {
            let rest = &self.s[self.pos..];
            let c = rest.chars().next().unwrap_or('\0');
            match c {
                '\\' => {
                    let next = rest[1..].chars().next();
                    match next {
                        Some(n) if n.is_ascii_punctuation() => {
                            self.text(&rest[1..2]);
                            self.pos += 2;
                        }
                        _ => {
                            self.text("\\");
                            self.pos += 1;
                        }
                    }
                }
                '`' => self.code_span(),
                '<' => self.lt(),
                '&' => match decode_entity(rest, self.kind) {
                    Some((ch, len)) => {
                        self.text(ch.encode_utf8(&mut [0; 4]));
                        self.pos += len;
                    }
                    None => {
                        self.text("&");
                        self.pos += 1;
                    }
                },
                '*' | '_' | '~' => self.delim_run(c),
                '[' => self.open_bracket(false, 1),
                '!' if rest[1..].starts_with('[') => self.open_bracket(true, 2),
                ']' => self.close_bracket(),
                CALENDAR if self.cfg.dates => match dates::parse_compact(rest) {
                    Some((d, len)) => {
                        self.items.push(Item::Node(d.node()));
                        self.pos += len;
                    }
                    None => {
                        self.text(&rest[..c.len_utf8()]);
                        self.pos += c.len_utf8();
                    }
                },
                'h' if url_boundary(self.prev_char()) && bare_url_len(rest).is_some() => {
                    let len = bare_url_len(rest).unwrap_or(1);
                    let url = &rest[..len];
                    if El::A(url.to_owned()).is_writable() {
                        self.items.push(Item::Node(N::Auto {
                            url: url.to_owned(),
                            raw: url.to_owned(),
                        }));
                    } else {
                        let url = url.to_owned();
                        self.text(&url);
                    }
                    self.pos += len;
                }
                _ => {
                    // Plain text up to the next character that may be special.
                    let end = rest
                        .char_indices()
                        .skip(1)
                        .find(|&(_, x)| {
                            matches!(
                                x,
                                '\\' | '`' | '<' | '&' | '*' | '_' | '~' | '[' | '!' | ']' | 'h'
                            ) || x == CALENDAR
                        })
                        .map_or(rest.len(), |(i, _)| i);
                    let chunk = rest[..end].to_owned();
                    self.text(&chunk);
                    self.pos += end;
                }
            }
        }
    }

    fn code_span(&mut self) {
        let rest = &self.s[self.pos..];
        let n = rest.bytes().take_while(|&b| b == b'`').count();
        let mut j = n;
        let b = rest.as_bytes();
        while j < b.len() {
            match b[j] {
                b'\n' => break,
                b'`' => {
                    let m = b[j..].iter().take_while(|&&x| x == b'`').count();
                    if m == n {
                        let mut content = &rest[n..j];
                        if content.len() >= 2
                            && content.starts_with(' ')
                            && content.ends_with(' ')
                            && !content.bytes().all(|x| x == b' ')
                        {
                            content = &content[1..content.len() - 1];
                        }
                        self.items.push(Item::Node(N::El(
                            El::Code,
                            vec![N::Text(content.to_owned())],
                        )));
                        self.pos += j + n;
                        return;
                    }
                    j += m;
                }
                _ => j += 1,
            }
        }
        let run = rest[..n].to_owned();
        self.text(&run);
        self.pos += n;
    }

    fn lt(&mut self) {
        let rest = &self.s[self.pos..];
        if let Some(len) = angle_autolink_len(rest) {
            let url = &rest[1..len - 1];
            if El::A(url.to_owned()).is_writable() {
                self.items.push(Item::Node(N::Auto {
                    url: url.to_owned(),
                    raw: rest[..len].to_owned(),
                }));
                self.pos += len;
                return;
            }
        }
        for name in HTML_TAGS {
            let close = format!("</{name}>");
            if rest.starts_with(&close) {
                self.close_html(name, close.len());
                return;
            }
        }
        if let Some((el, len)) = open_tag(rest) {
            let raw = rest[..len].to_owned();
            self.stack.push(Opener {
                item: self.items.len(),
                src: self.pos,
                kind: OpKind::Html(el),
            });
            self.items.push(Item::Open(raw));
            self.pos += len;
            return;
        }
        self.text("<");
        self.pos += 1;
    }

    fn close_html(&mut self, name: &str, len: usize) {
        let found = self
            .stack
            .iter()
            .rposition(|o| matches!(&o.kind, OpKind::Html(el) if el.tag() == name));
        let Some(k) = found else {
            let raw = self.s[self.pos..self.pos + len].to_owned();
            self.text(&raw);
            self.pos += len;
            return;
        };
        let op = self.stack.swap_remove(k);
        self.stack.truncate(k);
        let OpKind::Html(el) = op.kind else {
            unreachable!("found an Html opener")
        };
        let content: Vec<Item> = self.items.drain(op.item + 1..).collect();
        let mut children = process_emphasis(content);
        if matches!(el, El::A(_)) {
            children = deautolink(children);
        }
        self.items[op.item] = Item::Node(N::El(el, children));
        self.pos += len;
    }

    fn open_bracket(&mut self, image: bool, len: usize) {
        let raw = self.s[self.pos..self.pos + len].to_owned();
        self.stack.push(Opener {
            item: self.items.len(),
            src: self.pos,
            kind: OpKind::Bracket {
                image,
                active: true,
            },
        });
        self.items.push(Item::Open(raw));
        self.pos += len;
    }

    fn close_bracket(&mut self) {
        let found = self
            .stack
            .iter()
            .rposition(|o| matches!(o.kind, OpKind::Bracket { .. }));
        let literal = |p: &mut Self| {
            p.text("]");
            p.pos += 1;
        };
        let Some(k) = found else {
            return literal(self);
        };
        let OpKind::Bracket { image, active } = self.stack[k].kind else {
            unreachable!("found a bracket")
        };
        let src_open = self.stack[k].src;
        // Images have no WF meaning: `![alt](src)` stays literal text (§4.2).
        let spans_blank_line = self.s[src_open..self.pos].contains("\n\n");
        let dest = if !active || image || spans_blank_line {
            None
        } else {
            link_dest(&self.s[self.pos + 1..], self.kind)
        };
        let Some((href, dest_len)) = dest.filter(|(h, _)| El::A(h.clone()).is_writable()) else {
            self.stack.remove(k);
            return literal(self);
        };
        let op = self.stack.swap_remove(k);
        self.stack.truncate(k);
        let content: Vec<Item> = self.items.drain(op.item + 1..).collect();
        let children = deautolink(process_emphasis(content));
        self.items[op.item] = Item::Node(N::El(El::A(href), children));
        // No links inside links: earlier openers can no longer form one.
        for o in &mut self.stack {
            if let OpKind::Bracket { active, .. } = &mut o.kind {
                *active = false;
            }
        }
        self.pos += 1 + dest_len;
    }

    fn delim_run(&mut self, c: char) {
        let rest = &self.s[self.pos..];
        let n = rest.bytes().take_while(|&b| b == c as u8).count();
        let prev = class(self.prev_char());
        let next = class(rest[n..].chars().next());
        let run = rest[..n].to_owned();
        if c == '~' && n != 2 {
            self.text(&run);
        } else {
            let (can_open, can_close) = can_open_close(c, prev, next);
            self.items.push(Item::Delim {
                ch: c,
                count: n,
                orig: n,
                can_open,
                can_close,
            });
        }
        self.pos += n;
    }
}

/// A recognised inline HTML start tag at the start of `s` (§4.2): one of [`HTML_TAGS`],
/// lowercase, well-formed (checked by `wfhtml::parse`, so attribute values may use
/// character references) and writable in canonical form. Attributes other than `href`
/// (`<a>`) and `class` (`<span>`) are dropped, as canonicalisation does.
fn open_tag(s: &str) -> Option<(El, usize)> {
    let name = HTML_TAGS.into_iter().find(|name| {
        s.strip_prefix('<')
            .and_then(|r| r.strip_prefix(name))
            .is_some_and(|r| r.starts_with('>') || r.starts_with(|c: char| c.is_ascii_whitespace()))
    })?;
    let mut quote = None;
    let mut end = None;
    for (i, c) in s.char_indices().skip(1 + name.len()) {
        match (quote, c) {
            (None, '>') => {
                end = Some(i);
                break;
            }
            (None, '<' | '\n') => return None,
            (None, '"' | '\'') => quote = Some(c),
            (Some(q), c) if c == q => quote = None,
            (Some(_), '\n') => return None,
            _ => {}
        }
    }
    let len = end? + 1;
    let frag = wfhtml::parse(&format!("{}</{name}>", &s[..len])).ok()?;
    let [
        elem @ Inline::Element {
            tag,
            attrs,
            children,
        },
    ] = frag.as_slice()
    else {
        return None;
    };
    if tag != name || !children.is_empty() {
        return None;
    }
    let el = match name {
        "b" => El::B,
        "i" => El::I,
        "s" => El::S,
        "u" => El::U,
        "code" => El::Code,
        "a" => El::A(elem.attr("href")?.to_owned()),
        "span" => El::Span(elem.attr("class")?.to_owned()),
        "time" => El::Time(attrs.clone()),
        _ => return None,
    };
    el.is_writable().then_some((el, len))
}

/// Parse a link destination `(dest)` at the start of `s` (no title). Returns the decoded
/// href and the bytes consumed.
fn link_dest(s: &str, kind: FieldKind) -> Option<(String, usize)> {
    let b = s.as_bytes();
    if b.first() != Some(&b'(') {
        return None;
    }
    let skip_sp = |mut i: usize| {
        while matches!(b.get(i), Some(b' ' | b'\t')) {
            i += 1;
        }
        i
    };
    let mut i = skip_sp(1);
    let mut href = String::new();
    let decode_at = |i: usize, href: &mut String| -> usize {
        let rest = &s[i..];
        let c = rest.chars().next().unwrap_or('\0');
        if c == '\\' && rest[1..].starts_with(|x: char| x.is_ascii_punctuation()) {
            href.push_str(&rest[1..2]);
            return 2;
        }
        if c == '&'
            && let Some((ch, len)) = decode_entity(rest, kind)
        {
            href.push(ch);
            return len;
        }
        href.push(c);
        c.len_utf8()
    };
    if b.get(i) == Some(&b'<') {
        i += 1;
        loop {
            match b.get(i)? {
                b'>' => {
                    i += 1;
                    break;
                }
                b'<' | b'\n' => return None,
                _ => i += decode_at(i, &mut href),
            }
        }
    } else {
        let mut depth = 0usize;
        while let Some(&c) = b.get(i) {
            if c == b' ' || c.is_ascii_control() {
                break;
            }
            if c == b'(' {
                depth += 1;
            } else if c == b')' {
                if depth == 0 {
                    break;
                }
                depth -= 1;
            }
            i += decode_at(i, &mut href);
        }
        if depth != 0 {
            return None;
        }
    }
    let i = skip_sp(i);
    (b.get(i) == Some(&b')')).then_some((href, i + 1))
}

/// CommonMark's `process_emphasis` over one range of items, then conversion to nodes
/// (unmatched delimiters and openers become literal text).
fn process_emphasis(mut items: Vec<Item>) -> Vec<N> {
    let mut c = 0;
    while c < items.len() {
        let Item::Delim {
            ch,
            count: c_count,
            orig: c_orig,
            can_open: c_open,
            can_close: true,
        } = items[c]
        else {
            c += 1;
            continue;
        };
        let opener = (0..c).rev().find(|&o| match items[o] {
            Item::Delim {
                ch: och,
                orig,
                can_open: true,
                can_close: o_close,
                ..
            } if och == ch => {
                // Rule of 3 (CommonMark rules 9 and 10); not for `~~`.
                ch == '~'
                    || !((o_close || c_open)
                        && (orig + c_orig) % 3 == 0
                        && !(orig % 3 == 0 && c_orig % 3 == 0))
            }
            _ => false,
        });
        let Some(o) = opener else {
            if !c_open
                && let Item::Delim {
                    can_open,
                    can_close,
                    ..
                } = &mut items[c]
            {
                *can_open = false;
                *can_close = false;
            }
            c += 1;
            continue;
        };
        let Item::Delim { count: o_count, .. } = items[o] else {
            unreachable!("opener is a delimiter")
        };
        let used = if ch == '~' || (o_count >= 2 && c_count >= 2) {
            2
        } else {
            1
        };
        let inner: Vec<Item> = items.drain(o + 1..c).collect();
        let el = match (ch, used) {
            ('~', _) => El::S,
            (_, 2) => El::B,
            _ => El::I,
        };
        items.insert(o + 1, Item::Node(N::El(el, to_nodes(inner))));
        let mut closer = o + 2;
        for idx in [o, closer] {
            if let Item::Delim { count, .. } = &mut items[idx] {
                *count -= used;
            }
        }
        if matches!(items[o], Item::Delim { count: 0, .. }) {
            items.remove(o);
            closer -= 1;
        }
        if matches!(items[closer], Item::Delim { count: 0, .. }) {
            items.remove(closer);
        }
        c = closer;
    }
    to_nodes(items)
}

fn to_nodes(items: Vec<Item>) -> Vec<N> {
    let mut out = Vec::new();
    for it in items {
        match it {
            Item::Text(t) | Item::Open(t) => push_text(&mut out, &t),
            Item::Delim { ch, count, .. } => {
                push_text(&mut out, &ch.to_string().repeat(count));
            }
            Item::Node(n) => push_node(&mut out, n),
        }
    }
    out
}

/// Inside a link, autolinks are literal text (links do not nest, §4.2).
fn deautolink(nodes: Vec<N>) -> Vec<N> {
    let mut out = Vec::new();
    for n in nodes {
        match n {
            N::Auto { raw, .. } => push_text(&mut out, &raw),
            N::El(el, kids) => out.push(N::El(el, deautolink(kids))),
            N::Text(t) => push_text(&mut out, &t),
        }
    }
    out
}

/// Turn the remaining autolinks into links.
fn finalize(nodes: Vec<N>) -> Vec<N> {
    let mut out = Vec::new();
    for n in nodes {
        match n {
            N::Text(t) => push_text(&mut out, &t),
            N::Auto { url, .. } => out.push(N::El(El::A(url.clone()), vec![N::Text(url)])),
            N::El(el, kids) => out.push(N::El(el, finalize(kids))),
        }
    }
    out
}
