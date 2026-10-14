//! Property tests for the escaping contract of mdinline (EC 1–4, §5.3, §5.4).
//!
//! * Tier 1/2 round trip: canonical WF-HTML built from the UI's constructs (UI:E1–E8,
//!   F02, DM§6.2 step 11) renders as `Exact` or `Passthrough` and `md_to_wf` gives it
//!   back.
//! * For any Markdown `m`: `md_to_wf(m)` is well-formed and canonical; normalising
//!   converges after one step (`md_to_wf(wf_to_md(md_to_wf(m))) == md_to_wf(m)`, and
//!   rendering is then a fixed point).
//! * `md_to_wf` never produces malformed WF-HTML for any string, and `wf_to_md` output is
//!   always block-safe, for any string (malformed WF-HTML included).
//!
//! `MDINLINE_CASES` raises the number of cases (default 2000 per property).

use proptest::prelude::*;
use proptest::test_runner::Config;

use syncflowy::mdinline::{
    FieldKind, InlineConfig, Tier, find_tags, line_is_block_safe, md_to_wf, wf_to_md,
};
use syncflowy::wfhtml::{self, Inline};

fn cases() -> u32 {
    std::env::var("MDINLINE_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(2000)
}

fn config() -> Config {
    Config {
        cases: cases(),
        failure_persistence: None,
        ..Config::default()
    }
}

const KINDS: [FieldKind; 2] = [FieldKind::Name, FieldKind::Note];

fn configs() -> [InlineConfig; 2] {
    [InlineConfig { dates: true }, InlineConfig { dates: false }]
}

#[track_caller]
fn assert_block_safe(md: &str, kind: FieldKind, src: &str) {
    if kind == FieldKind::Name {
        assert!(
            !md.contains(['\n', '\r']),
            "name with a line break: {src:?} -> {md:?}"
        );
    }
    for line in md.split('\n') {
        assert!(
            line_is_block_safe(line),
            "unsafe line {line:?} from {src:?}"
        );
        assert!(find_tags(line).is_empty(), "tag in {line:?} from {src:?}");
    }
}

// ---------------------------------------------------------------------------------------
// Generators
// ---------------------------------------------------------------------------------------

/// Characters that matter to Markdown, our block parser, Obsidian or WF-HTML.
const SPECIAL: &[&str] = &[
    "*",
    "**",
    "_",
    "__",
    "~",
    "~~",
    "`",
    "``",
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
    "$",
    "%",
    "%%",
    "=",
    "==",
    "^",
    ":",
    "/",
    ".",
    "-",
    "+",
    "|",
    "{",
    "}",
    "@",
    "1.",
    "1)",
    "- ",
    "> ",
    "# ",
    "[ ] ",
    "[x] ",
    "---",
    "```",
    "~~~",
    "http://",
    "https://",
    "https://e.com/a?b=1&c=2",
    "www.",
    "&amp;",
    "&lt;",
    "&#42;",
    "&#x2a;",
    "&copy;",
    "&nbsp;",
    "&#32;",
    "📅",
    "📅 2026-01-15",
    " 14:30",
    "^wf-8e1f00aa2b3c",
    "%%wf:8e1f00aa2b3c%%",
    "[wid:: 8e1f00aa2b3c]",
    "(wid:: 8e1f00aa2b3c)",
    "<!-- wf:8e1f00aa2b3c -->",
    "%%wf-conflict:name%%",
    "^abc",
    "[[",
    "]]",
    "![",
    "<b>",
    "</b>",
    "<u>",
    "</u>",
    "<i>",
    "<code>",
    "</code>",
    "<s>",
    "<a href=\"u\">",
    "</a>",
    "<span class=\"colored c-red\">",
    "</span>",
    "<div>",
    "<br>",
    "#tag",
    "@who",
    " ",
    "  ",
    "\t",
    "\u{a0}",
    "\u{2003}",
    "é",
    "日本",
    "x",
    "y",
    "word",
    "a",
    "0",
    "9",
];

fn special() -> impl Strategy<Value = String> {
    prop::sample::select(SPECIAL).prop_map(str::to_owned)
}

fn piece() -> impl Strategy<Value = String> {
    prop_oneof![
        6 => special(),
        3 => "[a-z]{1,6}",
        1 => any::<char>().prop_map(|c| c.to_string()),
    ]
}

/// Text for a WF text node (decoded). `newlines` allows `\n` (notes).
fn text(newlines: bool) -> impl Strategy<Value = String> {
    let p = prop_oneof![
        10 => piece(),
        if newlines { 2 } else { 0 } => Just("\n".to_owned()),
    ];
    prop::collection::vec(p, 1..5).prop_map(|v| v.concat())
}

fn href() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("https://example.com/".to_owned()),
        Just("https://example.com/a?b=1&c=2".to_owned()),
        Just("https://en.wikipedia.org/wiki/Foo_(bar)".to_owned()),
        Just("https://e.com/a b".to_owned()),
        Just("mailto:a@b.c".to_owned()),
        Just("/rel/path".to_owned()),
        Just("".to_owned()),
        Just("https://e.com/*x*_y_".to_owned()),
        Just("x<y>\"z\"".to_owned()),
        Just("a\\b".to_owned()),
        Just("https://e.com/%%wf:8e1f00aa2b3c%%".to_owned()),
        "https?://[a-z]{1,5}\\.[a-z]{2,3}(/[a-z0-9_~.()-]{0,6}){0,2}",
        "[ -~]{0,12}",
    ]
}

fn date_el() -> impl Strategy<Value = Inline> {
    (
        1u32..=9999,
        1u32..=12,
        1u32..=28,
        prop::option::of((0u32..24, 0u32..60)),
        any::<bool>(),
    )
        .prop_map(|(y, m, d, t, exact)| {
            let mut attrs = vec![
                ("startYear".to_owned(), y.to_string()),
                ("startMonth".to_owned(), m.to_string()),
                ("startDay".to_owned(), d.to_string()),
            ];
            if let Some((h, mi)) = t {
                attrs.push(("startHour".to_owned(), h.to_string()));
                attrs.push(("startMinute".to_owned(), mi.to_string()));
            }
            // The exact API text is produced by md_to_wf itself, so take it from there.
            let api = md_to_wf(
                &match t {
                    Some((h, mi)) => format!("📅 {y:04}-{m:02}-{d:02} {h:02}:{mi:02}"),
                    None => format!("📅 {y:04}-{m:02}-{d:02}"),
                },
                FieldKind::Name,
                &InlineConfig::default(),
            );
            let text = if exact {
                wfhtml::to_plain_text(&api)
            } else {
                "someday".to_owned()
            };
            Inline::Element {
                tag: "time".to_owned(),
                attrs,
                children: vec![Inline::Text(text)],
            }
        })
}

fn el(tag: &str, attrs: Vec<(String, String)>, children: Vec<Inline>) -> Inline {
    Inline::Element {
        tag: tag.to_owned(),
        attrs,
        children,
    }
}

/// A fragment built from the UI's constructs.
fn fragment(newlines: bool) -> impl Strategy<Value = Vec<Inline>> {
    let leaf = prop_oneof![
        6 => text(newlines).prop_map(Inline::Text),
        1 => date_el(),
        1 => href().prop_map(|h| el("a", vec![("href".to_owned(), h.clone())], vec![Inline::Text(h)])),
    ];
    let node = leaf.prop_recursive(4, 24, 4, |inner| {
        let kids = prop::collection::vec(inner, 0..4);
        prop_oneof![
            (
                prop::sample::select(&["b", "i", "s", "u", "code"][..]),
                kids.clone()
            )
                .prop_map(|(t, k)| el(t, vec![], k)),
            (href(), kids.clone()).prop_map(|(h, k)| el("a", vec![("href".to_owned(), h)], k)),
            (
                prop::sample::select(&["colored c-red", "colored bc-yellow", "colored"][..]),
                kids
            )
                .prop_map(|(c, k)| el(
                    "span",
                    vec![("class".to_owned(), c.to_owned())],
                    k
                )),
        ]
    });
    prop::collection::vec(node, 0..5)
}

/// Canonical WF-HTML from the UI's constructs (non-canonical draws are discarded).
fn canonical_wf(newlines: bool) -> impl Strategy<Value = String> {
    fragment(newlines)
        .prop_map(|f| wfhtml::serialize(&f))
        .prop_filter("canonical", |w| {
            wfhtml::is_well_formed(w) && wfhtml::canonicalize(w) == *w
        })
}

/// Markdown-ish input.
fn markdown(newlines: bool) -> impl Strategy<Value = String> {
    let p = prop_oneof![
        10 => piece(),
        if newlines { 2 } else { 0 } => prop_oneof![Just("\n".to_owned()), Just("\r\n".to_owned())],
    ];
    prop::collection::vec(p, 0..14).prop_map(|v| v.concat())
}

// ---------------------------------------------------------------------------------------
// Properties
// ---------------------------------------------------------------------------------------

/// EC 1 for UI-shaped values, plus EC 4.
fn check_round_trip(w: &str, kind: FieldKind, cfg: &InlineConfig) {
    let md = wf_to_md(w, kind, cfg);
    assert_block_safe(&md.text, kind, w);
    assert!(
        matches!(md.tier, Tier::Exact | Tier::Passthrough),
        "tier {:?} for {w:?} ({kind:?}, dates {}): {:?} -> {:?}",
        md.tier,
        cfg.dates,
        md.text,
        md_to_wf(&md.text, kind, cfg)
    );
    assert_eq!(
        md_to_wf(&md.text, kind, cfg),
        w,
        "round trip of {w:?} via {:?}",
        md.text
    );
}

/// EC 2, EC 3, render fixed point, EC 4.
fn check_markdown(m: &str, kind: FieldKind, cfg: &InlineConfig) {
    let w = md_to_wf(m, kind, cfg);
    assert!(wfhtml::is_well_formed(&w), "malformed {w:?} from {m:?}");
    assert_eq!(wfhtml::canonicalize(&w), w, "not canonical: {m:?}");
    if kind == FieldKind::Name {
        assert!(!w.contains('\n'), "newline in name from {m:?}");
    }
    let r = wf_to_md(&w, kind, cfg);
    assert_block_safe(&r.text, kind, m);
    let w2 = md_to_wf(&r.text, kind, cfg);
    assert_eq!(
        w2, w,
        "convergence: {m:?} -> {w:?} -> {:?} ({:?}) -> {w2:?}",
        r.text, r.tier
    );
    assert!(
        matches!(r.tier, Tier::Exact | Tier::Passthrough),
        "{m:?} -> {w:?}: {:?}",
        r.tier
    );
    assert_eq!(
        wf_to_md(&w2, kind, cfg).text,
        r.text,
        "render fixed point for {m:?}"
    );
}

proptest! {
    #![proptest_config(config())]

    #[test]
    fn ui_names_round_trip(w in canonical_wf(false).prop_filter("one line", |w| !w.contains('\n'))) {
        for cfg in configs() {
            check_round_trip(&w, FieldKind::Name, &cfg);
        }
    }

    #[test]
    fn ui_notes_round_trip(w in canonical_wf(true)) {
        for cfg in configs() {
            check_round_trip(&w, FieldKind::Note, &cfg);
        }
    }

    #[test]
    fn markdown_converges(m in markdown(true)) {
        for kind in KINDS {
            for cfg in configs() {
                check_markdown(&m, kind, &cfg);
            }
        }
    }

    #[test]
    fn any_string_converges(m in any::<String>()) {
        for kind in KINDS {
            check_markdown(&m, kind, &InlineConfig::default());
        }
    }

    #[test]
    fn code_names(m in markdown(true)) {
        let cfg = InlineConfig::default();
        let w = md_to_wf(&m, FieldKind::Code, &cfg);
        prop_assert!(wfhtml::is_well_formed(&w));
        prop_assert_eq!(wfhtml::canonicalize(&w), w.clone());
        let r = wf_to_md(&w, FieldKind::Code, &cfg);
        if r.tier != Tier::ReadOnly {
            prop_assert_eq!(r.tier, Tier::Exact);
            prop_assert_eq!(md_to_wf(&r.text, FieldKind::Code, &cfg), w);
        }
    }

    #[test]
    fn any_wf_renders_block_safe(w in prop_oneof![markdown(true), any::<String>()]) {
        for kind in KINDS {
            let r = wf_to_md(&w, kind, &InlineConfig::default());
            assert_block_safe(&r.text, kind, &w);
            if matches!(r.tier, Tier::Exact | Tier::Passthrough) {
                prop_assert_eq!(md_to_wf(&r.text, kind, &InlineConfig::default()), w.clone());
            }
            prop_assert_eq!(r.tier == Tier::ReadOnly && kind == FieldKind::Note, !wfhtml::is_well_formed(&w) && kind == FieldKind::Note);
        }
    }
}
