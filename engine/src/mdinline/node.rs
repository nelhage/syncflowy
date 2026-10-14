//! The inline tree shared by the renderer and the parser: the canonical WF-HTML
//! vocabulary (UI:E3, E5, F02, DM§6.2 step 11) and nothing else.

use crate::wfhtml::{self, Inline};

/// A canonical WF-HTML element (what `wfhtml::serialize` keeps).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum El {
    B,
    I,
    S,
    U,
    Code,
    /// `<a href="…">` with the decoded href.
    A(String),
    /// `<span class="colored …">` with the decoded class.
    Span(String),
    /// `<time …>` with its attributes (names as written, decoded values) in source order.
    Time(Vec<(String, String)>),
}

/// One node of a parsed or to-be-rendered inline value. Text is decoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum N {
    Text(String),
    El(El, Vec<N>),
    /// Parser only: an autolink (`<scheme:…>` or a bare `http(s)://` URL) that becomes
    /// `<a href="u">u</a>`, or literal text again (`raw`) inside a link (no nested links).
    Auto {
        url: String,
        raw: String,
    },
}

impl El {
    pub(crate) fn tag(&self) -> &'static str {
        match self {
            El::B => "b",
            El::I => "i",
            El::S => "s",
            El::U => "u",
            El::Code => "code",
            El::A(_) => "a",
            El::Span(_) => "span",
            El::Time(_) => "time",
        }
    }

    fn attrs(&self) -> Vec<(String, String)> {
        match self {
            El::A(h) => vec![("href".to_owned(), h.clone())],
            El::Span(c) => vec![("class".to_owned(), c.clone())],
            El::Time(a) => a.clone(),
            El::B | El::I | El::S | El::U | El::Code => Vec::new(),
        }
    }

    fn to_inline(&self, children: Vec<Inline>) -> Inline {
        Inline::Element {
            tag: self.tag().to_owned(),
            attrs: self.attrs(),
            children,
        }
    }

    /// The canonical start tag, as `wfhtml::serialize` writes it.
    pub(crate) fn start_tag(&self) -> String {
        self.to_inline(Vec::new())
            .canonical_start_tag()
            .unwrap_or_else(|| format!("<{}>", self.tag()))
    }

    /// The end tag.
    pub(crate) fn end_tag(&self) -> String {
        format!("</{}>", self.tag())
    }

    /// True if an element with these attributes serialises to a well-formed fixed point
    /// of `canonicalize` (so it may be written, §5.4) and is not dropped (UI:F07).
    pub(crate) fn is_writable(&self) -> bool {
        let el = self.to_inline(vec![Inline::Text("x".to_owned())]);
        if el.is_dropped() || el.canonical_start_tag().is_none() {
            return false;
        }
        let s = wfhtml::serialize(std::slice::from_ref(&el));
        wfhtml::is_well_formed(&s) && wfhtml::canonicalize(&s) == s
    }
}

/// Append text to a node list, merging with a preceding text node.
pub(crate) fn push_text(out: &mut Vec<N>, s: &str) {
    if s.is_empty() {
        return;
    }
    if let Some(N::Text(t)) = out.last_mut() {
        t.push_str(s);
    } else {
        out.push(N::Text(s.to_owned()));
    }
}

/// Append a node, merging adjacent text.
pub(crate) fn push_node(out: &mut Vec<N>, n: N) {
    match n {
        N::Text(t) => push_text(out, &t),
        other => out.push(other),
    }
}

/// The inline tree of a parsed WF-HTML fragment, reduced to what canonical serialisation
/// keeps: unknown elements and `<a>` without href are unwrapped, `javascript:` links
/// dropped with their text (UI:F03/F04/F07/F12), names lowercased and aliased
/// (UI:F09/F10). A carriage return in text (only reachable through `&#13;`) becomes a
/// newline, as it does when the UI re-parses its own output.
pub(crate) fn from_fragment(frag: &[Inline]) -> Vec<N> {
    let mut out = Vec::new();
    lower(frag, &mut out);
    out
}

fn lower(frag: &[Inline], out: &mut Vec<N>) {
    for node in frag {
        match node {
            Inline::Text(t) => push_text(out, &t.replace('\r', "\n")),
            Inline::Element {
                children, attrs, ..
            } => {
                if node.is_dropped() {
                    continue;
                }
                if node.canonical_start_tag().is_none() {
                    lower(children, out);
                    continue;
                }
                let el = match node.canonical_tag().as_deref() {
                    Some("b") => El::B,
                    Some("i") => El::I,
                    Some("s") => El::S,
                    Some("u") => El::U,
                    Some("code") => El::Code,
                    Some("a") => El::A(node.attr("href").unwrap_or_default().to_owned()),
                    Some("span") => El::Span(node.attr("class").unwrap_or_default().to_owned()),
                    Some("time") => El::Time(attrs.clone()),
                    _ => {
                        lower(children, out);
                        continue;
                    }
                };
                let mut kids = Vec::new();
                lower(children, &mut kids);
                out.push(N::El(el, kids));
            }
        }
    }
}

/// Convert to the `wfhtml` element tree (autolinks become links).
pub(crate) fn to_inline(nodes: &[N]) -> Vec<Inline> {
    nodes
        .iter()
        .map(|n| match n {
            N::Text(t) => Inline::Text(t.clone()),
            N::El(el, kids) => el.to_inline(to_inline(kids)),
            N::Auto { url, .. } => El::A(url.clone()).to_inline(vec![Inline::Text(url.clone())]),
        })
        .collect()
}

/// Canonical WF-HTML of a node list.
pub(crate) fn serialize(nodes: &[N]) -> String {
    wfhtml::serialize(&to_inline(nodes))
}

/// The visible text of a node list.
pub(crate) fn plain_text(nodes: &[N]) -> String {
    fn walk(nodes: &[N], s: &mut String) {
        for n in nodes {
            match n {
                N::Text(t) => s.push_str(t),
                N::El(_, kids) => walk(kids, s),
                N::Auto { url, .. } => s.push_str(url),
            }
        }
    }
    let mut s = String::new();
    walk(nodes, &mut s);
    s
}

/// True if any text in the tree contains a newline.
pub(crate) fn has_newline(nodes: &[N]) -> bool {
    nodes.iter().any(|n| match n {
        N::Text(t) => t.contains('\n'),
        N::El(_, kids) => has_newline(kids),
        N::Auto { .. } => false,
    })
}
