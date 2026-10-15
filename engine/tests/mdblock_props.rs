//! Property tests for mdblock (§5.4 and the module invariants).
//!
//! * Render fixed point: for random trees of visible nodes with UI-style text (plus a few
//!   malformed and non-canonical values), random configurations, pending deletes and
//!   conflict markers, `normalize(parse(render(v))) == render(v)`, the parse has no
//!   warnings, and every parsed item carries the node's tag, kind, heading level,
//!   checkbox, ordered flag and (for representable fields) the node's exact WF value;
//!   markers read back as written.
//! * Normalisation keeps meaning, on rendered-then-perturbed documents (reindented with
//!   spaces, id tags moved mid-line, `- ` and `- [ ] ` swapped, blank lines added, CRLF):
//!   the perturbed document means the same as the rendering, and
//!   `parse(normalize(parse(m))) ≅ parse(m)`.
//! * The same for arbitrary Markdown-ish documents (stray lines, odd indentation, tags,
//!   prefixes and fences anywhere): parsing never fails on the body, normalising keeps the
//!   meaning and is idempotent.
//! * Every id tag encoding of §4.4, anywhere on a line, parses to the same short id.
//! * A completed node and its whole subtree never appear in the output; every other node
//!   does.
//! * The root's note becomes the preamble.
//!
//! `MDBLOCK_CASES` sets the number of cases per property (default 300).

use std::collections::BTreeSet;

use proptest::prelude::*;
use proptest::test_runner::Config;

use syncflowy::mdblock::{
    Checkbox, CheckboxPolicy, ConflictMarker, DocConfig, Frontmatter, ItemKind, LocalDoc,
    LocalItem, RootRef, TagStyle, View, ViewExtras, ViewItem, equivalent, normalize, parse, render,
};
use syncflowy::mdinline::{
    ConflictField, FieldKind, InlineConfig, TagFlag, TagForm, Tier, find_tags, format_tag,
    md_to_wf, wf_to_md,
};
use syncflowy::tree::{Node, NodeId, ShortId, Tree};
use syncflowy::wfhtml::{self, Inline};

fn cases() -> u32 {
    std::env::var("MDBLOCK_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(300)
}

fn config() -> Config {
    Config {
        cases: cases(),
        failure_persistence: None,
        ..Config::default()
    }
}

// ---------------------------------------------------------------------------------------
// Generators: UI-style text (UI:E1–E8, F02, DM§6.2 step 11) and trees
// ---------------------------------------------------------------------------------------

/// Text pieces that matter to Markdown, to our block parser or to WF-HTML.
const PIECES: &[&str] = &[
    "*",
    "**",
    "_",
    "__",
    "~~",
    "`",
    "[",
    "]",
    "(",
    ")",
    "<",
    ">",
    "&",
    "\"",
    "'",
    "\\",
    "#",
    "!",
    "%",
    "%%",
    "=",
    "^",
    ":",
    "/",
    ".",
    "-",
    "+",
    "|",
    "1.",
    "1)",
    "- ",
    "* ",
    "> ",
    "# ",
    "## ",
    "[ ] ",
    "[x] ",
    "---",
    "```",
    "~~~",
    "🔒 ",
    "⧉ (mirror of a deleted item)",
    "⚠ ",
    "📅",
    "📅 2026-01-15",
    "https://e.com/a?b=1&c=2",
    "^wf-8e1f00aa2b3c",
    "%%wf:8e1f00aa2b3c%%",
    "[wid:: 8e1f00aa2b3c]",
    "(wid:: 8e1f00aa2b3c)",
    "<!-- wf:8e1f00aa2b3c -->",
    "%%wf-conflict:name%%",
    "^abc",
    "[[",
    "]]",
    "&amp;",
    "&lt;b&gt;",
    "#tag",
    "@who",
    " ",
    "  ",
    "\t",
    "\u{a0}",
    "é",
    "日本",
    "word",
    "x",
    "a b",
    "0",
];

fn piece() -> impl Strategy<Value = String> {
    prop_oneof![
        5 => prop::sample::select(PIECES).prop_map(str::to_owned),
        4 => "[a-z]{1,8}",
        1 => any::<char>().prop_map(|c| c.to_string()),
    ]
}

fn text(newlines: bool) -> impl Strategy<Value = String> {
    let p = prop_oneof![
        10 => piece(),
        if newlines { 3 } else { 0 } => Just("\n".to_owned()),
    ];
    prop::collection::vec(p, 1..6).prop_map(|v| v.concat())
}

fn el(tag: &str, attrs: Vec<(String, String)>, children: Vec<Inline>) -> Inline {
    Inline::Element {
        tag: tag.to_owned(),
        attrs,
        children,
    }
}

fn date() -> impl Strategy<Value = Inline> {
    (
        2000u32..2100,
        1u32..=12,
        1u32..=28,
        prop::option::of((0u32..24, 0u32..60)),
    )
        .prop_map(|(y, m, d, t)| {
            let md = match t {
                Some((h, mi)) => format!("📅 {y:04}-{m:02}-{d:02} {h:02}:{mi:02}"),
                None => format!("📅 {y:04}-{m:02}-{d:02}"),
            };
            // The API's exact element, as md_to_wf produces it.
            let wf = md_to_wf(&md, FieldKind::Name, &InlineConfig::default());
            wfhtml::parse(&wf).expect("date element").remove(0)
        })
}

fn href() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("https://example.com/".to_owned()),
        Just("https://example.com/a?b=1&c=2".to_owned()),
        Just("https://en.wikipedia.org/wiki/Foo_(bar)".to_owned()),
        Just("https://e.com/a b".to_owned()),
        "https://[a-z]{1,5}\\.com(/[a-z0-9_.-]{0,6}){0,2}",
    ]
}

/// A fragment of the UI's constructs.
fn fragment(newlines: bool) -> impl Strategy<Value = Vec<Inline>> {
    let leaf = prop_oneof![
        8 => text(newlines).prop_map(Inline::Text),
        1 => date(),
        1 => href().prop_map(|h| el("a", vec![("href".to_owned(), h.clone())], vec![Inline::Text(h)])),
    ];
    let node = leaf.prop_recursive(3, 12, 3, |inner| {
        let kids = prop::collection::vec(inner, 1..3);
        prop_oneof![
            (
                prop::sample::select(&["b", "i", "s", "u", "code"][..]),
                kids.clone()
            )
                .prop_map(|(t, k)| el(t, vec![], k)),
            (href(), kids.clone()).prop_map(|(h, k)| el("a", vec![("href".to_owned(), h)], k)),
            kids.prop_map(|k| el(
                "span",
                vec![("class".to_owned(), "colored c-red".to_owned())],
                k
            )),
        ]
    });
    prop::collection::vec(node, 0..4)
}

/// Canonical WF-HTML built from the UI's constructs, plus a few non-UI values (malformed,
/// non-canonical, a newline in a name) for the read-only and lossy paths.
fn wf_value(newlines: bool) -> impl Strategy<Value = String> {
    prop_oneof![
        12 => fragment(newlines)
            .prop_map(|f| wfhtml::serialize(&f))
            .prop_filter("canonical", |w| wfhtml::is_well_formed(w)
                && wfhtml::canonicalize(w) == *w),
        1 => prop::sample::select(&[
            "broken <b>bold",
            "a\nb",
            "<b>x</b> <foo>y</foo>",
            "<B>upper</B>",
            "&lt;b&gt;literal&lt;/b&gt;",
        ][..]).prop_map(str::to_owned),
    ]
}

/// A code-block name: plain lines, some indented, some structural-looking.
fn code_value() -> impl Strategy<Value = String> {
    let line = prop_oneof![
        4 => "[a-z =(){};]{0,12}",
        1 => prop::sample::select(&[
            "\tindented", "    four", "- item", "# head", "> q", "", "``` inner", "~~~", "  ",
            "x &amp; y", "^wf-8e1f00aa2b3c",
        ][..]).prop_map(str::to_owned),
    ];
    prop::collection::vec(line, 1..5).prop_map(|ls| wfhtml::escape_text(&ls.join("\n")))
}

#[derive(Debug, Clone)]
struct G {
    name: String,
    note: Option<String>,
    layout: &'static str,
    completed: bool,
    children: Vec<G>,
}

const LAYOUTS: &[&str] = &[
    "bullets",
    "bullets",
    "bullets",
    "bullets",
    "bullets",
    "todo",
    "numbered",
    "code-block",
    "quote-block",
    "divider",
    "p",
    "h2",
    "board",
];

fn g_leaf(completion: bool) -> impl Strategy<Value = G> {
    (
        prop::sample::select(LAYOUTS),
        wf_value(false),
        code_value(),
        prop::option::weighted(0.3, wf_value(true)),
        prop::bool::weighted(if completion { 0.2 } else { 0.0 }),
    )
        .prop_map(|(layout, name, code, note, completed)| {
            let name = match layout {
                "code-block" => code,
                "divider" => String::new(),
                _ => name,
            };
            G {
                name,
                note,
                layout,
                completed,
                children: Vec::new(),
            }
        })
}

fn g_tree(completion: bool) -> impl Strategy<Value = Vec<G>> {
    let node = g_leaf(completion).prop_recursive(4, 40, 4, move |inner| {
        (g_leaf(completion), prop::collection::vec(inner, 0..4)).prop_map(|(mut g, kids)| {
            g.children = kids;
            g
        })
    });
    prop::collection::vec(node, 0..5)
}

fn nid(n: usize) -> NodeId {
    NodeId::from_api(format!("00000000-0000-4000-8000-{n:012x}"))
}

fn root_id() -> NodeId {
    nid(1)
}

/// Build a tree: the root (node 1) with `kids`, ids numbered in pre-order.
fn build(root_note: Option<String>, kids: &[G]) -> Tree {
    fn add(nodes: &mut Vec<Node>, parent: &NodeId, gs: &[G]) {
        for (i, g) in gs.iter().enumerate() {
            let id = nid(nodes.len() + 1);
            let mut n = Node::new(id.clone(), Some(parent.clone()), g.name.clone());
            n.note = g.note.clone();
            n.layout = g.layout.to_owned();
            n.completed = g.completed;
            n.priority = i as i64 * 100;
            nodes.push(n);
            add(nodes, &id, &g.children);
        }
    }
    let mut root = Node::new(root_id(), None, "ROOTNAME unrendered");
    root.note = root_note;
    let mut nodes = vec![root];
    add(&mut nodes, &root_id(), kids);
    Tree::from_nodes(nodes).expect("tree")
}

fn doc_config() -> impl Strategy<Value = DocConfig> {
    (
        0u8..=3,
        prop::sample::select(
            &[
                TagStyle::Auto,
                TagStyle::BlockId,
                TagStyle::Comment,
                TagStyle::Dataview,
                TagStyle::DataviewHidden,
                TagStyle::Html,
            ][..],
        ),
        prop::sample::select(
            &[
                CheckboxPolicy::TodosAndLeaves,
                CheckboxPolicy::TodosOnly,
                CheckboxPolicy::All,
                CheckboxPolicy::None,
            ][..],
        ),
        any::<bool>(),
        prop::option::weighted(0.2, 1u32..5),
    )
        .prop_map(|(headings, tags, checkboxes, dates, max_depth)| DocConfig {
            headings,
            tags,
            checkboxes,
            inline: InlineConfig { dates },
            max_depth,
        })
}

fn frontmatter(cfg: &DocConfig) -> Frontmatter {
    let mut fm = Frontmatter::new(RootRef::parse("000000000001").unwrap());
    fm.config = cfg.clone();
    fm.generation = Some(42);
    fm
}

// ---------------------------------------------------------------------------------------
// Checks
// ---------------------------------------------------------------------------------------

fn name_kind(kind: ItemKind) -> FieldKind {
    if kind == ItemKind::Code {
        FieldKind::Code
    } else {
        FieldKind::Name
    }
}

/// `s` without its leading and trailing empty lines.
fn trim_empty_lines(s: &str) -> String {
    let lines: Vec<&str> = s.split('\n').collect();
    let start = lines
        .iter()
        .position(|l| !l.is_empty())
        .unwrap_or(lines.len());
    let end = lines
        .iter()
        .rposition(|l| !l.is_empty())
        .map_or(start, |e| e + 1);
    lines[start..end].join("\n")
}

/// The parsed items say exactly what the view says.
fn check_items(tree: &Tree, cfg: &DocConfig, view: &[ViewItem], parsed: &[LocalItem]) {
    let inline = &cfg.inline;
    assert_eq!(view.len(), parsed.len());
    for (v, p) in view.iter().zip(parsed) {
        let node = tree.get(&v.id).unwrap();
        let tag = p.tag.as_ref().expect("tag");
        assert_eq!(Some(&tag.short), v.tag.as_ref());
        assert_eq!(tag.flags.contains(&TagFlag::ReadOnly), v.read_only);
        assert_eq!(p.kind, v.kind);
        assert_eq!(p.heading, v.heading);
        assert_eq!(p.checkbox, v.checkbox);
        assert_eq!(p.ordered, v.ordered);
        assert!(p.extra_tags.is_empty());
        if v.read_only {
            if v.kind != ItemKind::Placeholder {
                assert_eq!(p.name_md, v.name.text);
            }
        } else {
            let kind = name_kind(v.kind);
            let back = md_to_wf(&p.name_md, kind, inline);
            if matches!(v.name.tier, Tier::Exact | Tier::Passthrough) {
                assert_eq!(back, node.name, "name of {}", v.id);
            } else {
                assert_eq!(back, md_to_wf(&v.name.text, kind, inline));
            }
            let note_back = md_to_wf(&p.note_md.join("\n"), FieldKind::Note, inline);
            match &v.note {
                Some(n) if matches!(n.tier, Tier::Exact | Tier::Passthrough) => {
                    let expect = node.note.clone().unwrap();
                    // Heading notes are at column 0 and lose their edge empty lines.
                    let expect = if v.heading.is_some() {
                        trim_empty_lines(&expect)
                    } else {
                        expect
                    };
                    assert_eq!(note_back, expect, "note of {}", v.id);
                }
                Some(n) if v.heading.is_some() => assert_eq!(
                    note_back,
                    trim_empty_lines(&md_to_wf(&n.text, FieldKind::Note, inline))
                ),
                Some(n) => assert_eq!(note_back, md_to_wf(&n.text, FieldKind::Note, inline)),
                None => assert!(p.note_md.is_empty()),
            }
        }
        let rest = check_markers(&v.conflicts, &p.children);
        check_items(tree, cfg, &v.children, rest);
    }
}

/// Conflict markers come first among the parsed items and read back as written (§6.1);
/// returns the remaining items.
fn check_markers<'a>(markers: &[ConflictMarker], parsed: &'a [LocalItem]) -> &'a [LocalItem] {
    assert!(parsed.len() >= markers.len());
    for (m, p) in markers.iter().zip(parsed) {
        assert_eq!(p.kind, ItemKind::ConflictMarker(m.field));
        assert!(p.tag.is_none() && p.extra_tags.is_empty() && p.children.is_empty());
        let trimmed = m.local_md.trim_matches([' ', '\t']);
        match m.field {
            // Several lines: a fenced block, verbatim.
            ConflictField::Name if trimmed.contains('\n') => assert_eq!(p.name_md, m.local_md),
            // One line: trimmed on the marker line, or verbatim when it had to be fenced
            // (fence-like or tag-shaped text).
            ConflictField::Name => {
                assert!(
                    p.name_md == trimmed || p.name_md == m.local_md,
                    "{:?} vs {:?}",
                    p.name_md,
                    m.local_md
                );
            }
            ConflictField::Note => {
                let lines: Vec<&str> = if m.local_md.is_empty() {
                    Vec::new()
                } else {
                    m.local_md
                        .split('\n')
                        .map(|l| l.trim_matches([' ', '\t']))
                        .collect()
                };
                assert_eq!(p.name_md, "");
                assert_eq!(p.note_md, lines);
            }
        }
    }
    &parsed[markers.len()..]
}

/// Render `tree`, check invariant 1 and the item-by-item reading; return the Markdown
/// and its parse.
fn render_checked(tree: &Tree, cfg: &DocConfig) -> (String, LocalDoc) {
    render_checked_with(tree, cfg, &ViewExtras::default())
}

fn render_checked_with(tree: &Tree, cfg: &DocConfig, extras: &ViewExtras) -> (String, LocalDoc) {
    let view = View::build(tree, &root_id(), cfg, extras).unwrap();
    let md = render(&view, &frontmatter(cfg));
    let doc = parse(&md).unwrap();
    assert!(
        doc.warnings.is_empty(),
        "warnings {:#?} in\n{md}",
        doc.warnings
    );
    assert_eq!(normalize(&doc), md, "not a fixed point");
    let rest = check_markers(&view.root_conflicts, &doc.items);
    check_items(tree, cfg, &view.items, rest);
    (md, doc)
}

/// Local Markdown for a conflict marker, as `merge` would take it from a parsed file: a
/// name's Markdown (one line), a code block's lines, or a note's lines.
fn marker_text(field: ConflictField) -> BoxedStrategy<String> {
    let cfg = InlineConfig::default();
    match field {
        ConflictField::Name => prop_oneof![
            3 => wf_value(false).prop_map(move |w| wf_to_md(&w, FieldKind::Name, &cfg).text),
            1 => code_value().prop_map(|c| wfhtml::to_plain_text(&c)),
        ]
        .boxed(),
        ConflictField::Note => wf_value(true)
            .prop_map(move |w| wf_to_md(&w, FieldKind::Note, &cfg).text)
            .boxed(),
    }
}

fn marker_spec() -> impl Strategy<Value = (prop::sample::Index, ConflictField, String)> {
    prop::sample::select(&[ConflictField::Name, ConflictField::Note][..])
        .prop_flat_map(|f| (any::<prop::sample::Index>(), Just(f), marker_text(f)))
}

// ---------------------------------------------------------------------------------------
// Perturbations (a deterministic generator driven by a seed)
// ---------------------------------------------------------------------------------------

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        // xorshift64*
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    fn chance(&mut self, percent: u64) -> bool {
        self.next() % 100 < percent
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

/// What a body line is, for the perturbations: `Item(is_heading)`, or code content with
/// the number of leading tabs that are structure (the rest belong to the code).
#[derive(Clone, Copy)]
enum LineKind {
    Item(bool),
    Code(usize),
}

fn classify_lines(items: &[LocalItem], depth: usize, out: &mut Vec<(usize, LineKind)>) {
    for it in items {
        let heading = it.heading.is_some();
        out.push((it.line, LineKind::Item(heading)));
        if it.kind == ItemKind::Code {
            let n = it.name_md.split('\n').count();
            let structural = if heading { 0 } else { depth + 1 };
            for k in 1..=n + 1 {
                out.push((it.line + k, LineKind::Code(structural)));
            }
        }
        let child_depth = if heading { 0 } else { depth + 1 };
        classify_lines(&it.children, child_depth, out);
    }
}

/// The byte length of a list item's prefix (indentation, marker, one space).
fn list_prefix(line: &str) -> Option<usize> {
    let ws = line.len() - line.trim_start_matches([' ', '\t']).len();
    let rest = &line[ws..];
    let m = if rest.starts_with("- ") {
        1
    } else {
        let d = rest.bytes().take_while(u8::is_ascii_digit).count();
        if d > 0 && rest[d..].starts_with(". ") {
            d + 1
        } else {
            return None;
        }
    };
    Some(ws + m + 1)
}

fn heading_prefix(line: &str) -> usize {
    line.bytes().take_while(|&b| b == b'#').count() + 1
}

fn perturb(md: &str, doc: &LocalDoc, seed: u64, spaces: usize, crlf: bool) -> String {
    let mut rng = Rng(seed | 1);
    let mut kinds = Vec::new();
    classify_lines(&doc.items, 0, &mut kinds);
    let kinds: std::collections::BTreeMap<usize, LineKind> = kinds.into_iter().collect();
    let mut out: Vec<String> = Vec::new();
    for (i, line) in md.split_terminator('\n').enumerate() {
        let no = i + 1;
        let mut line = line.to_owned();
        let kind = kinds.get(&no).copied();
        if let Some(LineKind::Item(heading)) = kind {
            let prefix = if heading {
                Some(heading_prefix(&line))
            } else {
                list_prefix(&line)
            };
            if let Some(prefix) = prefix.filter(|&p| p <= line.len()) {
                // `- ` <-> `- [ ] ` (cosmetic, §4.3).
                if !heading && rng.chance(50) {
                    let rest = &line[prefix..];
                    if rest == "[ ]" {
                        line.truncate(prefix);
                    } else if let Some(r) = rest.strip_prefix("[ ] ") {
                        line = format!("{}{r}", &line[..prefix]);
                    } else if !rest.starts_with("[x]") {
                        line = format!("{}[ ] {rest}", &line[..prefix]);
                    }
                }
                // Move the tag to a random word boundary (§9.1 guard 4).
                if rng.chance(60)
                    && let Some(m) = find_tags(&line).pop()
                {
                    let tag = line[m.range.clone()].to_owned();
                    // Remove the tag with one adjacent space (never the marker's).
                    let (mut start, mut end) = (m.range.start, m.range.end);
                    if start > prefix && line[..start].ends_with(' ') {
                        start -= 1;
                    } else if line[end..].starts_with(' ') {
                        end += 1;
                    }
                    let mut rest = format!("{}{}", &line[..start], &line[end..]);
                    let prefix = prefix.min(rest.len());
                    let spaces: Vec<usize> = rest[prefix..]
                        .match_indices(' ')
                        .map(|(k, _)| prefix + k)
                        .collect();
                    let pick = rng.below(spaces.len() + 1);
                    if pick == spaces.len() {
                        rest.insert_str(prefix, &format!("{tag} "));
                    } else {
                        rest.insert_str(spaces[pick], &format!(" {tag}"));
                    }
                    line = rest;
                }
            }
            if !heading && rng.chance(15) {
                out.push(String::new());
            }
        }
        // Reindent: structural tabs become `spaces` spaces each.
        let tabs = line.bytes().take_while(|&b| b == b'\t').count();
        let structural = match kind {
            Some(LineKind::Code(n)) => tabs.min(n),
            _ => tabs,
        };
        if spaces > 0 && structural > 0 {
            line = format!("{}{}", " ".repeat(spaces * structural), &line[structural..]);
        }
        out.push(line);
    }
    let eol = if crlf { "\r\n" } else { "\n" };
    let mut s = out.join(eol);
    s.push_str(eol);
    s
}

/// A Markdown-ish line as a person (or a bad text merge) might leave it: indentation,
/// list or heading markers, checkboxes, the prefixes the parser knows, tags anywhere,
/// fences and text.
fn wild_line() -> impl Strategy<Value = String> {
    let indent = prop::sample::select(&["", "", "", "\t", "\t\t", "  ", "    ", " ", "\t  "][..]);
    let marker = prop::sample::select(
        &[
            "- ", "- ", "- ", "* ", "+ ", "1. ", "12) ", "-", "", "", "# ", "## ", "#### ",
        ][..],
    );
    let token = prop_oneof![
        6 => piece(),
        2 => prop::sample::select(&[
            "[ ] ", "[x] ", "[X] ", "🔒 ", "⚠ ", "> ", "```", "````", "~~~", "---",
            "⧉ (mirror of a deleted item)", "local note",
        ][..]).prop_map(str::to_owned),
        2 => (
            prop::sample::select(&["000000000002", "000000000003", "8E1F00AA2B3C"][..]),
            prop::sample::select(&["", "-ro", "-zz"][..]),
            0usize..5,
        ).prop_map(|(h, f, form)| match form {
            0 => format!(" ^wf-{h}{f}"),
            1 => format!(" %%wf:{h}{f}%%"),
            2 => format!(" [wid:: {h}{f}]"),
            3 => format!(" (wid:: {h}{f})"),
            _ => format!(" <!-- wf:{h}{f} -->"),
        }),
        1 => prop::sample::select(&[" %%wf-conflict:name%%", " %%wf-conflict:note%%"][..])
            .prop_map(str::to_owned),
    ];
    (indent, marker, prop::collection::vec(token, 0..5))
        .prop_map(|(i, m, t)| format!("{i}{m}{}", t.concat()))
}

fn wild_doc() -> impl Strategy<Value = String> {
    (
        0u8..=3,
        prop::sample::select(&["auto", "comment", "dataview", "html"][..]),
        prop::collection::vec(wild_line(), 0..16),
    )
        .prop_map(|(h, ids, lines)| {
            let mut s = format!(
                "---\nworkflowy: 000000000001\nsyncflowy: {{headings: {h}, ids: {ids}}}\n---\n"
            );
            for l in lines {
                s.push_str(&l);
                s.push('\n');
            }
            s
        })
}

// ---------------------------------------------------------------------------------------
// Properties
// ---------------------------------------------------------------------------------------

proptest! {
    #![proptest_config(config())]

    /// Invariant 1 and invariant 3 (§5.4): render(parse(render(t))) == render(t), and the
    /// file reads back as exactly the view.
    #[test]
    fn render_is_a_fixed_point(
        kids in g_tree(false),
        root_note in prop::option::weighted(0.3, wf_value(true)),
        cfg in doc_config(),
        hidden in prop::collection::vec(any::<prop::sample::Index>(), 0..2),
        markers in prop::collection::vec(marker_spec(), 0..4),
    ) {
        let tree = build(root_note, &kids);
        // Pending deletes (§9.1 guard 2) hide their subtrees.
        let others: Vec<NodeId> = tree.ids().filter(|i| **i != root_id()).cloned().collect();
        let mut extras = ViewExtras::default();
        if !others.is_empty() {
            extras.hidden = hidden.iter().map(|i| others[i.index(others.len())].clone()).collect();
        }
        // Conflict markers on visible nodes (root included).
        let visible: Vec<NodeId> = View::build(&tree, &root_id(), &cfg, &extras)
            .unwrap()
            .ids()
            .into_iter()
            .collect();
        extras.conflicts = markers
            .into_iter()
            .map(|(i, field, local_md)| ConflictMarker {
                node: visible[i.index(visible.len())].clone(),
                field,
                local_md,
            })
            .collect();
        let view = View::build(&tree, &root_id(), &cfg, &extras).unwrap();
        prop_assert!(view.dropped_conflicts.is_empty());
        prop_assert!(view.ids().iter().all(|i| !extras.hidden.contains(i)));
        render_checked_with(&tree, &cfg, &extras);
    }

    /// Invariant 2 on rendered-then-perturbed documents.
    #[test]
    fn perturbed_renderings_keep_their_meaning(
        kids in g_tree(false),
        cfg in doc_config(),
        seed in any::<u64>(),
        spaces in prop_oneof![Just(0usize), 1usize..=8],
        crlf in any::<bool>(),
    ) {
        let tree = build(None, &kids);
        let (md, doc) = render_checked(&tree, &cfg);
        let m = perturb(&md, &doc, seed, spaces, crlf);
        let pd = parse(&m).unwrap();
        prop_assert!(pd.warnings.is_empty(), "warnings {:#?} in\n{}", pd.warnings, m);
        prop_assert!(equivalent(&pd, &doc, &cfg.inline), "meaning changed:\n{}\n--- vs ---\n{}", m, md);
        let n = normalize(&pd);
        let nd = parse(&n).unwrap();
        prop_assert!(equivalent(&nd, &pd, &cfg.inline), "normalize changed meaning:\n{}", n);
        prop_assert_eq!(normalize(&nd), n);
    }

    /// Invariant 2 for any text that parses: normalising keeps the meaning, and is
    /// idempotent. The parser never fails on body content (§9.1 guard 4).
    #[test]
    fn normalising_any_document_keeps_its_meaning(m in wild_doc()) {
        let d = parse(&m).unwrap();
        let cfg = d.frontmatter.config.inline.clone();
        let n = normalize(&d);
        let nd = parse(&n).unwrap();
        prop_assert!(equivalent(&nd, &d, &cfg), "meaning changed:\n{}\n--- normalised to ---\n{}", m, n);
        prop_assert_eq!(normalize(&nd), n);
    }

    /// Every encoding of §4.4, anywhere on an item's first line (any case of the hex
    /// digits), parses to the same short id and flags, and leaves the name intact.
    #[test]
    fn every_tag_encoding_parses_to_the_same_short_id(
        hex in "[0-9a-f]{12}",
        upper in any::<bool>(),
        ro in any::<bool>(),
        form in prop::sample::select(&[
            TagForm::BlockId, TagForm::Comment, TagForm::Dataview,
            TagForm::DataviewHidden, TagForm::Html,
        ][..]),
        words in prop::collection::vec("[a-zA-Z0-9*_]{1,6}", 1..5),
        pos in any::<prop::sample::Index>(),
        heading in any::<bool>(),
    ) {
        let short = ShortId::parse(&hex).unwrap();
        let flags = if ro { BTreeSet::from([TagFlag::ReadOnly]) } else { BTreeSet::new() };
        let mut tag = format_tag(&short, &flags, form);
        if upper {
            tag = tag.replace(&hex, &hex.to_ascii_uppercase());
        }
        let k = pos.index(words.len() + 1);
        let mut parts: Vec<String> = words.clone();
        parts.insert(k, tag);
        let marker = if heading { "#" } else { "-" };
        let text = format!("---\nworkflowy: 000000000001\n---\n{marker} {}\n", parts.join(" "));
        let d = parse(&text).unwrap();
        let it = &d.items[0];
        let t = it.tag.as_ref().unwrap();
        prop_assert_eq!(&t.short, &short);
        prop_assert_eq!(t.form, form);
        prop_assert_eq!(&t.flags, &flags);
        prop_assert_eq!(&it.name_md, &words.join(" "));
        prop_assert!(d.warnings.iter().all(|w| w.kind == syncflowy::mdblock::WarningKind::HeadingBeyondConfig));
    }

    /// A completed node and everything under it never appear in the file (§4.3, §4.5);
    /// every node visible by the completion rule does.
    #[test]
    fn hidden_subtrees_never_appear(
        kids in g_tree(true),
        cfg in doc_config(),
    ) {
        // Unique, searchable names (no shells, dividers or code, so every visible node
        // renders its name).
        fn tokenise(gs: &mut [G], next: &mut usize) {
            for g in gs {
                *next += 1;
                g.name = format!("tok{next}x");
                g.layout = match g.layout {
                    "code-block" | "divider" => "bullets",
                    l => l,
                };
                tokenise(&mut g.children, next);
            }
        }
        let mut kids = kids;
        tokenise(&mut kids, &mut 1);
        let tree = build(None, &kids);
        let cfg = DocConfig { max_depth: None, ..cfg };
        let (md, _) = render_checked(&tree, &cfg);
        for node in tree.iter() {
            if node.id == root_id() {
                continue;
            }
            let visible = tree.is_visible(&node.id, &root_id());
            let name_shown = md
                .split(|c: char| !c.is_ascii_alphanumeric())
                .any(|w| w == node.name);
            let tag_shown = md.contains(node.id.short().unwrap().as_str());
            prop_assert_eq!(name_shown, visible, "{} in\n{}", node.name, md);
            prop_assert_eq!(tag_shown, visible, "{} in\n{}", node.name, md);
        }
    }

    /// The root's note becomes the preamble (§4.1), its name is not rendered.
    #[test]
    fn root_note_is_the_preamble(
        note in wf_value(true),
        kids in g_tree(false),
        dates in any::<bool>(),
    ) {
        let cfg = DocConfig { inline: InlineConfig { dates }, ..DocConfig::default() };
        let tree = build(Some(note.clone()), &kids);
        let (md, doc) = render_checked(&tree, &cfg);
        prop_assert!(!md.contains("ROOTNAME"));
        let md_note = wf_to_md(&note, FieldKind::Note, &cfg.inline);
        let lines: Vec<&str> = md_note.text.split('\n').collect();
        let start = lines.iter().position(|l| !l.is_empty()).unwrap_or(lines.len());
        let end = lines.iter().rposition(|l| !l.is_empty()).map_or(start, |e| e + 1);
        if matches!(md_note.tier, Tier::Exact | Tier::Passthrough) {
            prop_assert_eq!(doc.root_note.clone(), lines[start..end].iter().map(|s| s.to_string()).collect::<Vec<_>>());
            // The value comes back exactly, up to the empty lines at its edges.
            let wf_lines: Vec<&str> = note.split('\n').collect();
            let s = wf_lines.iter().position(|l| !l.is_empty()).unwrap_or(wf_lines.len());
            let e = wf_lines.iter().rposition(|l| !l.is_empty()).map_or(s, |e| e + 1);
            prop_assert_eq!(
                md_to_wf(&doc.root_note.join("\n"), FieldKind::Note, &cfg.inline),
                wf_lines[s..e].join("\n")
            );
        }
        if !doc.root_note.is_empty() {
            let body = md.split("\n---\n").nth(1).unwrap();
            prop_assert!(body.starts_with(&doc.root_note[0]));
        }
        prop_assert_eq!(doc.items.iter().filter(|i| i.checkbox == Checkbox::Ticked).count(), 0);
    }
}
