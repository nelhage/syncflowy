//! Unit tests for mdblock, one group per rule of the file format (docs/sync-design.md
//! §4.1, §4.3, §4.4, §4.5, §5.3, §6.1, §9.1, §10; module docs of `syncflowy::mdblock`).

use std::collections::{BTreeMap, BTreeSet};

use syncflowy::mdblock::{
    Checkbox, CheckboxPolicy, ConflictMarker, DocConfig, Frontmatter, ItemKind, ItemPath, LocalDoc,
    PLACEHOLDER_TEXT, ParseError, RootRef, Sticky, TagStyle, View, ViewError, ViewExtras,
    WarningKind, equivalent, normalize, parse, parse_frontmatter, render,
};
use syncflowy::mdinline::{ConflictField, InlineConfig, TagFlag, TagForm, Tier};
use syncflowy::tree::{IdMap, Mirror, Node, NodeId, ShortId, Tree};

const FM: &str = "---\nworkflowy: 000000000001\n---\n";

fn doc(body: &str) -> LocalDoc {
    parse(&format!("{FM}{body}")).expect("parse")
}

fn short(s: &str) -> ShortId {
    ShortId::parse(s).unwrap()
}

fn id(n: u64) -> NodeId {
    NodeId::from_api(format!("00000000-0000-4000-8000-{n:012x}"))
}

fn kinds(d: &LocalDoc) -> Vec<WarningKind> {
    d.warnings.iter().map(|w| w.kind).collect()
}

/// One node of a test tree: (number, parent number, name, note, layout, completed).
type Spec<'a> = (u64, u64, &'a str, Option<&'a str>, &'a str, bool);

/// A tree below the root node 1.
fn tree(spec: &[Spec]) -> Tree {
    tree_with_orphans(spec, &[])
}

/// [`tree`] where the nodes numbered in `orphans` are orphan shells (DM§10.4).
fn tree_with_orphans(spec: &[Spec], orphans: &[u64]) -> Tree {
    let mut nodes = vec![Node::new(id(1), None, "Root")];
    for (i, &(n, p, name, note, layout, completed)) in spec.iter().enumerate() {
        let mut node = Node::new(id(n), Some(id(p)), name);
        node.note = note.map(str::to_owned);
        node.layout = layout.to_owned();
        node.completed = completed;
        node.priority = i as i64;
        if orphans.contains(&n) {
            node.mirror = Some(Mirror::Shell { origin: None });
        }
        nodes.push(node);
    }
    Tree::from_nodes(nodes).unwrap()
}

fn fm() -> Frontmatter {
    Frontmatter::new(RootRef::parse("000000000001").unwrap())
}

fn render_tree(t: &Tree, cfg: &DocConfig, extras: &ViewExtras) -> String {
    let v = View::build(t, &id(1), cfg, extras).unwrap();
    let mut f = fm();
    f.config = cfg.clone();
    let md = render(&v, &f);
    // Every rendering in this file must be a fixed point (invariant 1) and parse cleanly.
    let d = parse(&md).unwrap();
    assert_eq!(normalize(&d), md);
    assert!(d.warnings.is_empty(), "{:#?}", d.warnings);
    md
}

fn body(md: &str) -> &str {
    let rest = md.strip_prefix("---\n").unwrap();
    &rest[rest.find("\n---\n").unwrap() + 5..]
}

// ---------------------------------------------------------------------------------------
// Frontmatter (§4.1, §9.1)
// ---------------------------------------------------------------------------------------

#[test]
fn root_ref_forms() {
    let s = short("a846cfce4c6d");
    for raw in [
        "a846cfce4c6d",
        "A846CFCE4C6D",
        " a846cfce4c6d ",
        "https://workflowy.com/#/a846cfce4c6d",
        "http://beta.workflowy.com/#/A846cfce4c6d",
        "https://WorkFlowy.com/#/a846cfce4c6d/",
        "https://workflowy.com/#/a846cfce4c6d?q=x",
        "a846cfce4c6d   # sync root (short id or URL)",
        "\"https://workflowy.com/#/a846cfce4c6d\"",
        "'a846cfce4c6d'",
    ] {
        let r = RootRef::parse(raw).unwrap_or_else(|e| panic!("{raw:?}: {e}"));
        assert_eq!(r.short, s, "{raw:?}");
        assert_eq!(r.full, None);
        assert_eq!(r.raw, raw.trim());
    }
    let full = "6ed4b9ca-256c-4f1b-9e5a-a846cfce4c6d";
    let r = RootRef::parse(full).unwrap();
    assert_eq!(r.short, s);
    assert_eq!(r.full, Some(NodeId::parse(full).unwrap()));
    for bad in [
        "",
        "a846cfce4c6",
        "a846cfce4c6dd",
        "5e2e76e4a846cfce4c6d",
        "6ED4B9CA-256C-4F1B-9E5A-A846CFCE4C6D",
        "https://workflowy.com/#/a846cfce4c6dx",
        "https://www.workflowy.com/#/a846cfce4c6d",
        "https://workflowy.com/#/x/a846cfce4c6d",
        "workflowy.com/#/a846cfce4c6d",
    ] {
        assert!(
            matches!(RootRef::parse(bad), Err(ParseError::BadRoot(_))),
            "{bad:?}"
        );
    }
}

#[test]
fn root_ref_resolve() {
    let full = NodeId::parse("6ed4b9ca-256c-4f1b-9e5a-a846cfce4c6d").unwrap();
    let mut ids = IdMap::new();
    ids.insert(&full);
    assert_eq!(
        RootRef::parse("a846cfce4c6d").unwrap().resolve(&ids),
        Some(full.clone())
    );
    assert_eq!(
        RootRef::parse(full.as_str())
            .unwrap()
            .resolve(&IdMap::new()),
        Some(full)
    );
    assert_eq!(
        RootRef::parse("000000000001")
            .unwrap()
            .resolve(&IdMap::new()),
        None
    );
}

#[test]
fn frontmatter_config_flow_and_block() {
    let text = "---\ntitle: x\nworkflowy: a846cfce4c6d # root\nsyncflowy: {headings: 2, ids: comment, checkboxes: todos, dates: false, depth: 4, paused: true, force: true, gen: 412}\ntags:\n  - a\n  - b\n---\nbody\n";
    let f = parse_frontmatter(text).unwrap();
    assert_eq!(f.root.raw, "a846cfce4c6d # root");
    assert_eq!(f.config.headings, 2);
    assert_eq!(f.config.tags, TagStyle::Comment);
    assert_eq!(f.config.checkboxes, CheckboxPolicy::TodosOnly);
    assert!(!f.config.inline.dates);
    assert_eq!(f.config.max_depth, Some(4));
    assert!(f.paused && f.force);
    assert_eq!(f.generation, Some(412));
    assert_eq!(f.other, vec!["title: x", "tags:", "  - a", "  - b"]);
    // Render: engine keys first, in the documented order, then the other lines.
    assert_eq!(
        f.render(),
        "---\nworkflowy: a846cfce4c6d # root\nsyncflowy: {headings: 2, ids: comment, checkboxes: todos, dates: false, depth: 4, paused: true, force: true, gen: 412}\ntitle: x\ntags:\n  - a\n  - b\n---\n"
    );

    let block = "---\nworkflowy: a846cfce4c6d\nsyncflowy:\n  headings: 1   # comment\n  ids: \"dataview-hidden\"\n\n  gen: 3\nother: 1\n---\n";
    let f = parse_frontmatter(block).unwrap();
    assert_eq!(f.config.headings, 1);
    assert_eq!(f.config.tags, TagStyle::DataviewHidden);
    assert_eq!(f.generation, Some(3));
    assert_eq!(f.other, vec!["other: 1"]);
    assert_eq!(
        f.render(),
        "---\nworkflowy: a846cfce4c6d\nsyncflowy: {headings: 1, ids: dataview-hidden, gen: 3}\nother: 1\n---\n"
    );

    // A flow mapping continued on the next line.
    let f = parse_frontmatter(
        "---\nworkflowy: a846cfce4c6d\nsyncflowy: {headings: 1,\n  gen: 9}\n---\n",
    )
    .unwrap();
    assert_eq!((f.config.headings, f.generation), (1, Some(9)));
}

#[test]
fn frontmatter_defaults_are_not_written() {
    let f = fm();
    assert_eq!(f.render(), "---\nworkflowy: 000000000001\n---\n");
    let mut f = fm();
    f.generation = Some(0);
    assert_eq!(
        f.render(),
        "---\nworkflowy: 000000000001\nsyncflowy: {gen: 0}\n---\n"
    );
    // Every value round-trips.
    for tags in [
        TagStyle::Auto,
        TagStyle::BlockId,
        TagStyle::Comment,
        TagStyle::Dataview,
        TagStyle::DataviewHidden,
        TagStyle::Html,
    ] {
        for checkboxes in [
            CheckboxPolicy::TodosAndLeaves,
            CheckboxPolicy::TodosOnly,
            CheckboxPolicy::All,
            CheckboxPolicy::None,
        ] {
            let mut f = fm();
            f.config.tags = tags;
            f.config.checkboxes = checkboxes;
            f.config.max_depth = Some(2);
            f.config.inline = InlineConfig { dates: false };
            assert_eq!(parse_frontmatter(&f.render()).unwrap(), f);
        }
    }
}

#[test]
fn frontmatter_unknown_key_warns_and_gen_is_lenient() {
    let d = parse("---\nworkflowy: 000000000001\nsyncflowy: {colour: blue, gen: soon}\n---\n- x\n")
        .unwrap();
    assert_eq!(kinds(&d), vec![WarningKind::UnknownConfigKey]);
    assert_eq!(d.warnings[0].line, 3);
    assert_eq!(d.frontmatter.generation, None);
    assert!(!normalize(&d).contains("colour"));
}

#[test]
fn frontmatter_errors() {
    let e = |t: &str| parse(t).unwrap_err();
    assert_eq!(e(""), ParseError::NoFrontmatter);
    assert_eq!(e("\n"), ParseError::NoFrontmatter);
    assert_eq!(e("- item\n"), ParseError::NoFrontmatter);
    assert_eq!(
        e("\n---\nworkflowy: 000000000001\n---\n"),
        ParseError::NoFrontmatter
    );
    assert_eq!(
        e("---\nworkflowy: 000000000001\n- x\n"),
        ParseError::UnterminatedFrontmatter
    );
    assert_eq!(e("---\ntitle: x\n---\n- x\n"), ParseError::NoRoot);
    assert!(matches!(
        e("---\nworkflowy: nope\n---\n"),
        ParseError::BadRoot(_)
    ));
    assert!(matches!(
        e("---\nworkflowy: 000000000001\nworkflowy: 000000000002\n---\n"),
        ParseError::BadRoot(_)
    ));
    for bad in [
        "{headings: 7}",
        "{headings: -1}",
        "{ids: blocks}",
        "{checkboxes: some}",
        "{dates: maybe}",
        "{depth: 0}",
        "{paused: 1}",
        "{force: yes}",
        "{headings}",
        "{headings: 1",
        "{headings: 1} trailing",
        "headings",
    ] {
        let t = format!("---\nworkflowy: 000000000001\nsyncflowy: {bad}\n---\n");
        assert!(
            matches!(parse(&t), Err(ParseError::BadConfig { .. })),
            "{bad}: {:?}",
            parse(&t)
        );
    }
}

#[test]
fn frontmatter_line_endings_and_bom() {
    let d = parse(
        "\u{feff}---\r\nworkflowy: 000000000001\r\n---\r\n- a ^wf-000000000002\r\n\tnote\r\n",
    )
    .unwrap();
    assert_eq!(d.items[0].name_md, "a");
    assert_eq!(d.items[0].note_md, vec!["note"]);
    // A closing `...` is accepted too (YAML document end).
    assert!(parse("---\nworkflowy: 000000000001\n...\n").is_ok());
    // A lone `\r` ends a line too (CommonMark, and mdinline reads it so).
    let d = parse("---\rworkflowy: 000000000001\r---\r- a\r\tnote\r- b").unwrap();
    assert_eq!(d.items.len(), 2);
    assert_eq!(d.items[0].note_md, vec!["note"]);
    assert_eq!((d.items[1].name_md.as_str(), d.items[1].line), ("b", 6));
}

#[test]
fn unicode_whitespace_lines_are_blank() {
    // md_to_wf trims Unicode White_Space, so a line of NBSPs is blank, not a stray line.
    let d = doc("- a\n\tone\n\u{a0}\n\ttwo\n\u{2003}\n- b\n");
    assert_eq!(d.items[0].note_md, vec!["one", "", "two"]);
    assert!(d.warnings.is_empty());
}

#[test]
fn unsafe_verbatim_notes_are_canonicalised() {
    let cfg = InlineConfig::default();
    // A read-only heading's note line `# x` (indented in the source, so not a heading)
    // would be a heading at column 0: normalize writes the canonical form instead.
    let d = doc("# t %%wf:000000000002-ro%%\n\t# x\n");
    assert_eq!(d.items[0].note_md, vec!["# x"]);
    let n = normalize(&d);
    assert!(n.ends_with("# 🔒 t %%wf:000000000002-ro%%\n\\# x\n"), "{n}");
    assert!(equivalent(&parse(&n).unwrap(), &d, &cfg));
    // A read-only item's note opening a fence it never closes: a later code item's
    // closing fence must not swallow the child.
    let d = doc("- ^wf-000000000002-ro\n```\n\t- ```\n");
    let n = normalize(&d);
    assert!(equivalent(&parse(&n).unwrap(), &d, &cfg), "{n}");
    // A closed fence in a verbatim note is kept as written.
    let d = doc("- t ^wf-000000000002-ro\n\t```\n\t- x\n\t```\n");
    assert_eq!(d.items[0].note_md, vec!["```", "- x", "```"]);
    let n = normalize(&d);
    assert!(
        n.ends_with("- 🔒 t ^wf-000000000002-ro\n\t```\n\t- x\n\t```\n"),
        "{n}"
    );
}

// ---------------------------------------------------------------------------------------
// Items, markers and checkboxes (§4.1, §4.3)
// ---------------------------------------------------------------------------------------

#[test]
fn list_markers_and_checkboxes() {
    let d =
        doc("- a\n* b\n+ c\n1. d\n2) e\n- [ ] f\n- [x] g\n- [X] h\n-\n- [ ]\n- [-] i\n- [x]j\n");
    let got: Vec<(&str, bool, Checkbox)> = d
        .items
        .iter()
        .map(|i| (i.name_md.as_str(), i.ordered, i.checkbox))
        .collect();
    assert_eq!(
        got,
        vec![
            ("a", false, Checkbox::None),
            ("b", false, Checkbox::None),
            ("c", false, Checkbox::None),
            ("d", true, Checkbox::None),
            ("e", true, Checkbox::None),
            ("f", false, Checkbox::Open),
            ("g", false, Checkbox::Ticked),
            ("h", false, Checkbox::Ticked),
            ("", false, Checkbox::None),
            ("", false, Checkbox::Open),
            ("[-] i", false, Checkbox::None),
            ("[x]j", false, Checkbox::None),
        ]
    );
    assert!(d.items.iter().all(|i| i.kind == ItemKind::Bullet));
    // Not list items: no space after the marker, or a long number.
    let d = doc("-x\n1234567890. y\n");
    assert!(d.items.is_empty());
    assert_eq!(d.root_note, vec!["-x", "1234567890. y"]);
}

#[test]
fn headings_tick_and_levels() {
    let mut cfgd = String::from("---\nworkflowy: 000000000001\nsyncflowy: {headings: 2}\n---\n");
    cfgd.push_str("# [x] Done %%wf:000000000002%%\n## [ ] Open\n### Skipped\n# A\n### B\n#### C\n####### not a heading\n");
    let d = parse(&cfgd).unwrap();
    let a = &d.items[0];
    assert_eq!(
        (a.heading, a.checkbox, a.name_md.as_str()),
        (Some(1), Checkbox::Ticked, "Done")
    );
    assert_eq!(a.tag.as_ref().unwrap().short, short("000000000002"));
    let open = &a.children[0];
    assert_eq!((open.heading, open.checkbox), (Some(2), Checkbox::Open));
    // `###` directly under `##` would be 3 (beyond the config: warned, kept).
    assert_eq!(open.children[0].heading, Some(3));
    // `###` directly under `#` is coerced to 2; `####` under it to 3.
    let b = &d.items[1].children[0];
    assert_eq!((b.name_md.as_str(), b.heading), ("B", Some(2)));
    assert_eq!(b.children[0].heading, Some(3));
    assert_eq!(b.children[0].note_md, vec!["####### not a heading"]);
    let w = kinds(&d);
    assert_eq!(
        w.iter()
            .filter(|k| **k == WarningKind::HeadingLevelSkipped)
            .count(),
        2
    );
    assert_eq!(
        w.iter()
            .filter(|k| **k == WarningKind::HeadingBeyondConfig)
            .count(),
        2
    );
}

#[test]
fn heading_first_level_coerced() {
    let d = doc("## Two\n- child\n");
    assert_eq!(d.items[0].heading, Some(1));
    assert_eq!(d.items[0].children[0].name_md, "child");
    assert_eq!(
        kinds(&d),
        vec![
            WarningKind::HeadingLevelSkipped,
            WarningKind::HeadingBeyondConfig
        ]
    );
    // Indented `#` lines are not headings.
    let d = doc("- a\n\t# b\n");
    assert_eq!(d.items[0].note_md, vec!["# b"]);
}

#[test]
fn heading_notes_and_children() {
    let d = doc(
        "Preamble\n\n# H %%wf:000000000002%%\nnote 1\n\nnote 2\n\n- child ^wf-000000000003\n## Sub\n- grandchild\n# H2\n",
    );
    assert_eq!(d.root_note, vec!["Preamble"]);
    let h = &d.items[0];
    assert_eq!(h.note_md, vec!["note 1", "", "note 2"]);
    assert_eq!(h.children.len(), 2);
    assert_eq!(h.children[0].name_md, "child");
    assert_eq!(h.children[1].heading, Some(2));
    assert_eq!(h.children[1].children[0].name_md, "grandchild");
    assert_eq!(d.items[1].name_md, "H2");
    assert_eq!((h.line, h.end_line), (6, 9));
}

#[test]
fn kinds_code_quote_divider() {
    let d = doc(
        "- ```python ^wf-000000000002\n\tx = 1\n\n\t    y\n\t```\n\tnote after\n- > quoted ^wf-000000000003\n- >\n- ---\n- ----\n- [ ] --- \n- ```js```\n- ~~~\n\ta\n\t~~~~\n",
    );
    let c = &d.items[0];
    assert_eq!(c.kind, ItemKind::Code);
    assert_eq!(c.name_md, "python\nx = 1\n\n    y");
    assert_eq!(c.note_md, vec!["note after"]);
    assert_eq!((c.line, c.end_line), (4, 9));
    assert_eq!(
        (d.items[1].kind, d.items[1].name_md.as_str()),
        (ItemKind::Quote, "quoted")
    );
    assert_eq!(
        (d.items[2].kind, d.items[2].name_md.as_str()),
        (ItemKind::Quote, "")
    );
    assert_eq!(d.items[3].kind, ItemKind::Divider);
    assert_eq!(d.items[4].kind, ItemKind::Divider);
    assert_eq!(
        (d.items[5].kind, d.items[5].checkbox),
        (ItemKind::Divider, Checkbox::Open)
    );
    // An info string with a backtick: not a fence (an inline code span).
    assert_eq!(
        (d.items[6].kind, d.items[6].name_md.as_str()),
        (ItemKind::Bullet, "```js```")
    );
    assert_eq!(
        (d.items[7].kind, d.items[7].name_md.as_str()),
        (ItemKind::Code, "a")
    );
    assert!(d.warnings.is_empty());
}

#[test]
fn code_fence_content_is_not_structure() {
    let d = doc(
        "- ``` ^wf-000000000002\n\t- not a child\n\t# not a heading\n\t\tindented\n\t```\n- next\n",
    );
    assert_eq!(d.items.len(), 2);
    assert_eq!(
        d.items[0].name_md,
        "- not a child\n# not a heading\n\tindented"
    );
    // A longer fence is not closed by a shorter one.
    let d = doc("- ```` ^wf-000000000002\n\t```\n\t````\n");
    assert_eq!(d.items[0].name_md, "```");
}

#[test]
fn unclosed_fence() {
    let d = doc("- ``` ^wf-000000000002\n\tx\n\n- next\n");
    assert_eq!(d.items[0].name_md, "x");
    assert_eq!(d.items[1].name_md, "next");
    assert_eq!(kinds(&d), vec![WarningKind::UnclosedFence]);
    let d = doc("- ```\n");
    assert_eq!(
        (d.items[0].kind, d.items[0].name_md.as_str()),
        (ItemKind::Code, "")
    );
}

#[test]
fn heading_code_block() {
    let text = "---\nworkflowy: 000000000001\nsyncflowy: {headings: 1}\n---\n# ``` %%wf:000000000002%%\n- x\n```\nnote\n";
    let d = parse(text).unwrap();
    let h = &d.items[0];
    assert_eq!((h.kind, h.name_md.as_str()), (ItemKind::Code, "- x"));
    assert_eq!(h.note_md, vec!["note"]);
    assert_eq!(normalize(&d), text);
    // Unclosed: ends at the next column-0 item or heading.
    let d = doc("# ```\ncode\n\n- item\n");
    assert_eq!(d.items[0].name_md, "code");
    assert_eq!(d.items[0].children[0].name_md, "item");
    assert_eq!(
        kinds(&d),
        vec![WarningKind::HeadingBeyondConfig, WarningKind::UnclosedFence]
    );
}

// ---------------------------------------------------------------------------------------
// Notes, blank lines, indentation, stray lines (§4.1, §9.1 guard 4)
// ---------------------------------------------------------------------------------------

#[test]
fn notes_and_blank_lines() {
    let d = doc("- a\n\tone\n\n\ttwo\n\t\n- b\n\n\tlead\n- c\n\tx\n\t\n\t\n\n- d\n\tkept\n\n");
    assert_eq!(d.items[0].note_md, vec!["one", "", "two", ""]);
    assert_eq!(d.items[1].note_md, vec!["", "lead"]);
    assert_eq!(d.items[2].note_md, vec!["x", "", ""]);
    assert_eq!(d.items[3].note_md, vec!["kept"]);
    // Blank lines between items are not note lines.
    let d = doc("- a\n\n- b\n\n\n- c\n");
    assert!(d.items.iter().all(|i| i.note_md.is_empty()));
}

#[test]
fn note_fence_keeps_lines_as_note() {
    let d = doc("- a\n\t```\n\t- not a child\n\t```\n\t- child\n");
    assert_eq!(d.items[0].note_md, vec!["```", "- not a child", "```"]);
    assert_eq!(d.items[0].children[0].name_md, "child");
    // An unclosed fence in a note never swallows the items after it.
    let d = doc("Preamble\n```\n- item\n");
    assert_eq!(d.root_note, vec!["Preamble", "```"]);
    assert_eq!(d.items[0].name_md, "item");
}

#[test]
fn spaces_indentation() {
    for unit in [1, 2, 3, 4, 8] {
        let i = |n: usize| " ".repeat(unit * n);
        let text = format!(
            "- a\n{}note\n{}- b\n{}- c\n{}c note\n- d\n",
            i(1),
            i(1),
            i(2),
            i(3)
        );
        let d = doc(&text);
        assert!(d.warnings.is_empty(), "unit {unit}: {:?}", d.warnings);
        assert_eq!(d.items.len(), 2);
        assert_eq!(d.items[0].note_md, vec!["note"]);
        let b = &d.items[0].children[0];
        assert_eq!(b.children[0].name_md, "c");
        assert_eq!(b.children[0].note_md, vec!["c note"]);
    }
    // Tabs and spaces mixed consistently (a tab is 4 columns).
    let d = doc("- a\n\t- b\n    - c\n");
    assert_eq!(d.items[0].children.len(), 2);
    assert!(d.warnings.is_empty());
}

#[test]
fn inconsistent_indentation_is_coerced() {
    let d = doc("- a\n\t\t- b\n\t- c\n");
    assert_eq!(d.items.len(), 1);
    let names: Vec<&str> = d.items[0]
        .children
        .iter()
        .map(|c| c.name_md.as_str())
        .collect();
    assert_eq!(names, vec!["b", "c"]);
    assert_eq!(kinds(&d), vec![WarningKind::Indentation]);
    assert_eq!(d.warnings[0].line, 6);
}

#[test]
fn stray_lines_join_the_preceding_item() {
    let d = doc("Preamble line\n- a\n\t- b\nlazy continuation\n\nafter a blank\n");
    assert_eq!(d.root_note, vec!["Preamble line"]);
    let b = &d.items[0].children[0];
    assert_eq!(b.note_md, vec!["lazy continuation", "after a blank"]);
    assert_eq!(
        kinds(&d),
        vec![WarningKind::StrayLine, WarningKind::StrayLine]
    );
    assert_eq!(b.end_line, 9);
}

#[test]
fn root_note_trims_blank_edges() {
    let d = doc("\n\nfirst\n\n\nsecond\n\n");
    assert_eq!(d.root_note, vec!["first", "", "", "second"]);
    assert!(d.items.is_empty());
    let d = doc("");
    assert!(d.root_note.is_empty() && d.items.is_empty());
}

// ---------------------------------------------------------------------------------------
// Id tags (§4.4)
// ---------------------------------------------------------------------------------------

#[test]
fn every_tag_form_anywhere_on_the_line() {
    let s = short("8e1f00aa2b3c");
    for (tag, form) in [
        ("^wf-8e1f00aa2b3c", TagForm::BlockId),
        ("^wf-8E1F00AA2B3C", TagForm::BlockId),
        ("%%wf:8e1f00aa2b3c%%", TagForm::Comment),
        ("%% wf:8e1f00aa2b3c %%", TagForm::Comment),
        ("[wid:: 8e1f00aa2b3c]", TagForm::Dataview),
        ("[wid::8e1f00aa2b3c]", TagForm::Dataview),
        ("(wid:: 8e1f00aa2b3c)", TagForm::DataviewHidden),
        ("<!-- wf:8e1f00aa2b3c -->", TagForm::Html),
        ("<!--wf:8e1f00aa2b3c-->", TagForm::Html),
    ] {
        for line in [
            format!("- Draft the spec {tag}"),
            format!("- Draft {tag} the spec"),
            format!("- {tag} Draft the spec"),
            format!("- [ ] {tag} Draft the spec"),
            format!("# Draft the spec {tag}"),
        ] {
            let d = doc(&format!("{line}\n"));
            let it = &d.items[0];
            let t = it.tag.as_ref().unwrap_or_else(|| panic!("{line}"));
            assert_eq!((&t.short, t.form), (&s, form), "{line}");
            assert_eq!(it.name_md, "Draft the spec", "{line}");
            assert!(
                kinds(&d)
                    .iter()
                    .all(|k| *k == WarningKind::HeadingBeyondConfig)
            );
        }
    }
}

#[test]
fn several_tags_last_wins() {
    let d = doc("- a ^wf-000000000002 b %%wf:000000000003%%\n");
    let it = &d.items[0];
    assert_eq!(it.tag.as_ref().unwrap().short, short("000000000003"));
    assert_eq!(it.extra_tags.len(), 1);
    assert_eq!(it.extra_tags[0].short, short("000000000002"));
    assert_eq!(it.name_md, "a b");
    assert_eq!(kinds(&d), vec![WarningKind::MultipleTags]);
}

#[test]
fn escaped_and_malformed_tags_are_text() {
    let d = doc("- a \\^wf-000000000002\n- b ^wf-0000000000023\n- c %\\%wf:000000000002%%\n");
    assert!(d.items.iter().all(|i| i.tag.is_none()));
    assert_eq!(d.items[0].name_md, "a \\^wf-000000000002");
}

#[test]
fn tag_flags() {
    let d = doc("- x ^wf-000000000002-ro\n- y ^wf-000000000003-zz\n- z ^wf-000000000004-RO-zz\n");
    let ro = |i: usize| d.items[i].tag.as_ref().unwrap().flags.clone();
    assert_eq!(ro(0), BTreeSet::from([TagFlag::ReadOnly]));
    assert!(ro(1).is_empty());
    assert_eq!(ro(2), BTreeSet::from([TagFlag::ReadOnly]));
    assert_eq!(
        kinds(&d),
        vec![WarningKind::UnknownTagFlag, WarningKind::UnknownTagFlag]
    );
}

#[test]
fn duplicate_tags_are_reported_by_path() {
    let d = doc(
        "- a ^wf-000000000002\n\t- b ^wf-000000000003\n- a copy ^wf-000000000002\n- ⚠ x %%wf-conflict:name%%\n",
    );
    let tags = d.tags();
    assert_eq!(
        tags[&short("000000000002")],
        vec![ItemPath(vec![0]), ItemPath(vec![1])]
    );
    assert_eq!(tags[&short("000000000003")], vec![ItemPath(vec![0, 0])]);
    assert_eq!(tags.len(), 2);
    assert_eq!(d.get(&ItemPath(vec![0, 0])).unwrap().name_md, "b");
    assert!(d.get(&ItemPath(vec![0, 1])).is_none());
    assert!(d.get(&ItemPath(vec![])).is_none());
    let walk: Vec<Vec<usize>> = d.walk().into_iter().map(|(p, _)| p.0).collect();
    assert_eq!(walk, vec![vec![0], vec![0, 0], vec![1], vec![2]]);
}

// ---------------------------------------------------------------------------------------
// Read-only forms and placeholders (§5.3, §10)
// ---------------------------------------------------------------------------------------

#[test]
fn read_only_prefix_needs_the_flag() {
    let d = doc(
        "- 🔒 secret ^wf-000000000002-ro\n- 🔒 plain ^wf-000000000003\n- 🔒 ^wf-000000000004-ro\n",
    );
    assert_eq!(d.items[0].name_md, "secret");
    assert_eq!(d.items[1].name_md, "🔒 plain");
    assert_eq!(d.items[2].name_md, "");
}

#[test]
fn placeholder_needs_flag_and_exact_text() {
    let d = doc(&format!(
        "- {PLACEHOLDER_TEXT} ^wf-000000000002-ro\n\t- child\n- {PLACEHOLDER_TEXT} ^wf-000000000003\n- {PLACEHOLDER_TEXT}!  ^wf-000000000004-ro\n"
    ));
    assert_eq!(d.items[0].kind, ItemKind::Placeholder);
    assert_eq!(d.items[0].name_md, "");
    assert_eq!(d.items[0].children.len(), 1);
    assert_eq!(d.items[1].kind, ItemKind::Bullet);
    assert_eq!(d.items[1].name_md, PLACEHOLDER_TEXT);
    assert_eq!(d.items[2].kind, ItemKind::Bullet);
}

// ---------------------------------------------------------------------------------------
// Conflict markers (§6.1)
// ---------------------------------------------------------------------------------------

#[test]
fn conflict_markers_parse() {
    let d = doc(
        "- Node ^wf-000000000002\n\t- ⚠ my wording %%wf-conflict:name%%\n\t- ⚠ local note %%wf-conflict:note%%\n\t\tline 1\n\n\t\tline 3\n\t- ⚠ ``` %%wf-conflict:name%%\n\t\tcode\n\t\t```\n\t- child ^wf-000000000003\n\t- ⚠ late marker %%wf-conflict:name%%\n- ⚠ not a marker\n",
    );
    let n = &d.items[0];
    let m: Vec<(ItemKind, &str, Vec<String>)> = n
        .children
        .iter()
        .map(|c| (c.kind, c.name_md.as_str(), c.note_md.clone()))
        .collect();
    assert_eq!(
        m,
        vec![
            (
                ItemKind::ConflictMarker(ConflictField::Name),
                "my wording",
                vec![]
            ),
            (
                ItemKind::ConflictMarker(ConflictField::Note),
                "",
                vec!["line 1".into(), "".into(), "line 3".into()]
            ),
            (
                ItemKind::ConflictMarker(ConflictField::Name),
                "code",
                vec![]
            ),
            (ItemKind::Bullet, "child", vec![]),
            (
                ItemKind::ConflictMarker(ConflictField::Name),
                "late marker",
                vec![]
            ),
        ]
    );
    assert!(n.children[0].is_marker() && n.children[0].tag.is_none());
    assert_eq!(d.items[1].name_md, "⚠ not a marker");
    assert!(d.warnings.is_empty());
    assert_eq!(d.tags().len(), 2);
}

#[test]
fn misplaced_markers_warn() {
    let d =
        doc("- ⚠ x %%wf-conflict:name%% ^wf-000000000002\n\t- child\n# ⚠ y %%wf-conflict:name%%\n");
    assert!(d.items[0].tag.is_none());
    assert_eq!(d.items[0].extra_tags.len(), 1);
    assert_eq!(
        kinds(&d),
        vec![
            WarningKind::MultipleTags,
            WarningKind::MisplacedMarker,
            WarningKind::HeadingBeyondConfig,
            WarningKind::MisplacedMarker
        ]
    );
}

#[test]
fn name_markers_that_would_misread_are_fenced() {
    // A code block's name is never escaped, so a one-line code name can look like a tag
    // or a fence; on the marker line it would be read as one. Such markers are fenced.
    let t = tree(&[(2, 1, "x", None, "code-block", false)]);
    for local in [
        "^wf-8e1f00aa2b3c",
        "a %%wf-conflict:note%% b",
        "```py",
        "~~~",
        "x\ny",
    ] {
        let extras = ViewExtras {
            conflicts: vec![ConflictMarker {
                node: id(2),
                field: ConflictField::Name,
                local_md: local.to_owned(),
            }],
            ..ViewExtras::default()
        };
        let md = render_tree(&t, &DocConfig::default(), &extras);
        let d = parse(&md).unwrap();
        let m = &d.items[0].children[0];
        assert_eq!(
            m.kind,
            ItemKind::ConflictMarker(ConflictField::Name),
            "{md}"
        );
        assert_eq!(m.name_md, local, "{md}");
        assert!(m.extra_tags.is_empty());
    }
}

// ---------------------------------------------------------------------------------------
// The view (§4.3, §4.5, §5.3, §10)
// ---------------------------------------------------------------------------------------

#[test]
fn view_hiding_rules() {
    let spec = [
        (2, 1, "visible", None, "bullets", false),
        (3, 2, "done", None, "bullets", true),
        (4, 3, "under done", None, "bullets", false),
        (5, 1, "pending delete", None, "bullets", false),
        (6, 5, "under pending", None, "bullets", false),
        (7, 2, "depth 2", None, "bullets", false),
        (8, 7, "depth 3", None, "bullets", false),
        (9, 1, "", Some(""), "bullets", false),
        (10, 9, "shell child", None, "bullets", false),
    ];
    let t = tree_with_orphans(&spec, &[9]);
    let extras = ViewExtras {
        hidden: BTreeSet::from([id(5)]),
        ..ViewExtras::default()
    };
    let cfg = DocConfig {
        max_depth: Some(2),
        ..DocConfig::default()
    };
    let v = View::build(&t, &id(1), &cfg, &extras).unwrap();
    assert_eq!(v.ids(), BTreeSet::from([id(1), id(2), id(7), id(9)]));
    assert!(v.contains(&id(1)) && v.contains(&id(7)) && !v.contains(&id(8)));
    let walk: Vec<(NodeId, NodeId)> = v
        .walk()
        .into_iter()
        .map(|(p, i)| (p.clone(), i.id.clone()))
        .collect();
    assert_eq!(walk, vec![(id(1), id(2)), (id(2), id(7)), (id(1), id(9))]);
    assert_eq!(v.items[1].kind, ItemKind::Placeholder);
    assert!(v.items[1].read_only && v.items[1].children.is_empty());
    let md = render_tree(&t, &cfg, &extras);
    for hidden in ["done", "under", "pending", "depth 3", "shell child"] {
        assert!(!md.contains(hidden), "{hidden} in\n{md}");
    }
}

#[test]
fn view_root_is_always_rendered() {
    let mut t = tree(&[(2, 1, "child", None, "bullets", false)]);
    t.set_completed(&id(1), true, Some(1)).unwrap();
    t.set_note(&id(1), Some("root note\nline 2".to_owned()))
        .unwrap();
    let v = View::build(&t, &id(1), &DocConfig::default(), &ViewExtras::default()).unwrap();
    assert!(v.root_completed);
    assert_eq!(v.items.len(), 1);
    let md = render_tree(&t, &DocConfig::default(), &ViewExtras::default());
    assert_eq!(
        body(&md),
        "root note\nline 2\n\n- [ ] child ^wf-000000000002\n"
    );
    assert_eq!(
        View::build(&t, &id(99), &DocConfig::default(), &ViewExtras::default()),
        Err(ViewError::RootMissing(id(99)))
    );
}

#[test]
fn view_subtree_root() {
    // The sync root need not be a top node of the tree.
    let t = tree(&[
        (2, 1, "sync root", Some("its note"), "bullets", false),
        (3, 2, "child", None, "bullets", false),
        (4, 1, "outside", None, "bullets", false),
    ]);
    let v = View::build(&t, &id(2), &DocConfig::default(), &ViewExtras::default()).unwrap();
    assert_eq!(v.ids(), BTreeSet::from([id(2), id(3)]));
    assert_eq!(v.root_note.as_ref().unwrap().text, "its note");
}

#[test]
fn view_kinds_and_layouts() {
    let t = tree(&[
        (2, 1, "c&lt;1", Some("n"), "code-block", false),
        (3, 1, "q", None, "quote-block", false),
        (4, 1, "", Some(""), "divider", false),
        (5, 1, "named divider", None, "divider", false),
        (6, 1, "nums", None, "numbered", false),
        (7, 6, "one", None, "bullets", false),
        (8, 6, "two", None, "todo", false),
        (9, 1, "", None, "code-block", false),
        (10, 1, "", Some("a note"), "bullets", false),
    ]);
    let v = View::build(&t, &id(1), &DocConfig::default(), &ViewExtras::default()).unwrap();
    let k: Vec<ItemKind> = v.items.iter().map(|i| i.kind).collect();
    assert_eq!(
        k,
        vec![
            ItemKind::Code,
            ItemKind::Quote,
            ItemKind::Divider,
            ItemKind::Bullet,
            ItemKind::Bullet,
            ItemKind::Code,
            ItemKind::Bullet
        ]
    );
    assert_eq!(v.items[0].name.text, "c<1");
    assert!(v.items[4].children.iter().all(|c| c.ordered));
    let md = render_tree(&t, &DocConfig::default(), &ViewExtras::default());
    assert_eq!(
        body(&md),
        "- ``` ^wf-000000000002\n\tc<1\n\t```\n\tn\n- > q ^wf-000000000003\n- --- ^wf-000000000004\n- [ ] named divider ^wf-000000000005\n- nums ^wf-000000000006\n\t1. [ ] one ^wf-000000000007\n\t2. [ ] two ^wf-000000000008\n- ``` ^wf-000000000009\n\n\t```\n- [ ] ^wf-00000000000a\n\ta note\n"
    );
}

#[test]
fn view_empty_items_are_not_placeholders() {
    // Shells are known from the beta API's mirror metadata (DM§10.5), not guessed: an
    // empty node of any layout, a plain bullet included, is an ordinary item (editable,
    // its children shown). Only an orphan shell is a placeholder (§10).
    let spec = [
        (2, 1, "", None, "todo", false),
        (3, 2, "todo child", None, "bullets", false),
        (4, 1, "", None, "numbered", false),
        (5, 4, "first", None, "bullets", false),
        (6, 4, "second", None, "bullets", false),
        (7, 1, "", Some(""), "code-block", false),
        (8, 7, "code child", None, "bullets", false),
        (9, 1, "", None, "quote-block", false),
        (10, 9, "quote child", None, "bullets", false),
        (11, 1, "", None, "bullets", false),
        (12, 11, "empty child", None, "bullets", false),
        (13, 1, "", None, "bullets", false),
    ];
    let t = tree_with_orphans(&spec, &[13]);
    let cfg = DocConfig::default();
    let v = View::build(&t, &id(1), &cfg, &ViewExtras::default()).unwrap();
    let got: Vec<(ItemKind, bool, usize)> = v
        .items
        .iter()
        .map(|i| (i.kind, i.read_only, i.children.len()))
        .collect();
    assert_eq!(
        got,
        vec![
            (ItemKind::Bullet, false, 1),
            (ItemKind::Bullet, false, 2),
            (ItemKind::Code, false, 1),
            (ItemKind::Quote, false, 1),
            (ItemKind::Bullet, false, 1),
            (ItemKind::Placeholder, true, 0),
        ]
    );
    assert_eq!(v.items[0].checkbox, Checkbox::Open);
    assert!(v.items[1].children.iter().all(|c| c.ordered));
    let md = render_tree(&t, &cfg, &ViewExtras::default());
    assert_eq!(
        body(&md),
        "- [ ] ^wf-000000000002\n\t- [ ] todo child ^wf-000000000003\n- ^wf-000000000004\n\t1. [ ] first ^wf-000000000005\n\t2. [ ] second ^wf-000000000006\n- ``` ^wf-000000000007\n\n\t```\n\t- [ ] code child ^wf-000000000008\n- > ^wf-000000000009\n\t- [ ] quote child ^wf-00000000000a\n- ^wf-00000000000b\n\t- [ ] empty child ^wf-00000000000c\n- ⧉ (mirror of a deleted item) ^wf-00000000000d-ro\n"
    );
    let d = parse(&md).unwrap();
    let parsed: Vec<(ItemKind, &str, usize)> = d
        .items
        .iter()
        .map(|i| (i.kind, i.name_md.as_str(), i.children.len()))
        .collect();
    assert_eq!(
        parsed,
        vec![
            (ItemKind::Bullet, "", 1),
            (ItemKind::Bullet, "", 2),
            (ItemKind::Code, "", 1),
            (ItemKind::Quote, "", 1),
            (ItemKind::Bullet, "", 1),
            (ItemKind::Placeholder, "", 0),
        ]
    );
}

#[test]
fn view_checkbox_policies_and_sticky() {
    let t = tree(&[
        (2, 1, "parent", None, "bullets", false),
        (3, 2, "leaf", None, "bullets", false),
        (4, 1, "todo parent", None, "todo", false),
        (5, 4, "leaf 2", None, "bullets", false),
        (6, 1, "q", None, "quote-block", false),
    ]);
    let boxes = |policy: CheckboxPolicy, sticky: &[(u64, Checkbox)]| -> Vec<Checkbox> {
        let cfg = DocConfig {
            checkboxes: policy,
            ..DocConfig::default()
        };
        let extras = ViewExtras {
            sticky: Sticky {
                checkbox: sticky.iter().map(|&(n, c)| (id(n), c)).collect(),
                full_placement: BTreeMap::new(),
            },
            ..ViewExtras::default()
        };
        let v = View::build(&t, &id(1), &cfg, &extras).unwrap();
        v.walk().into_iter().map(|(_, i)| i.checkbox).collect()
    };
    use Checkbox::{None as N, Open as O};
    // Order: parent, leaf, todo parent, leaf 2, q.
    assert_eq!(
        boxes(CheckboxPolicy::TodosAndLeaves, &[]),
        vec![N, O, O, O, N]
    );
    assert_eq!(boxes(CheckboxPolicy::TodosOnly, &[]), vec![N, N, O, N, N]);
    assert_eq!(boxes(CheckboxPolicy::All, &[]), vec![O, O, O, O, N]);
    assert_eq!(boxes(CheckboxPolicy::None, &[]), vec![N, N, O, N, N]);
    // Sticky wins over the policy, except for `todo` (always open) and kinds without boxes.
    assert_eq!(
        boxes(
            CheckboxPolicy::TodosAndLeaves,
            &[(2, O), (3, N), (4, N), (6, O)]
        ),
        vec![O, N, O, O, N]
    );
}

#[test]
fn view_headings_by_depth() {
    let t = tree(&[
        (2, 1, "h1", None, "todo", false),
        (3, 2, "h2", None, "bullets", false),
        (4, 3, "item", None, "bullets", false),
        (5, 1, "nums", None, "numbered", false),
        (6, 5, "not ordered: a heading", None, "bullets", false),
    ]);
    let cfg = DocConfig {
        headings: 2,
        ..DocConfig::default()
    };
    let v = View::build(&t, &id(1), &cfg, &ViewExtras::default()).unwrap();
    let h: Vec<(Option<u8>, Checkbox, bool)> = v
        .walk()
        .into_iter()
        .map(|(_, i)| (i.heading, i.checkbox, i.ordered))
        .collect();
    assert_eq!(
        h,
        vec![
            (Some(1), Checkbox::None, false),
            (Some(2), Checkbox::None, false),
            (None, Checkbox::Open, false),
            (Some(1), Checkbox::None, false),
            (Some(2), Checkbox::None, false),
        ]
    );
    let md = render_tree(&t, &cfg, &ViewExtras::default());
    assert_eq!(
        body(&md),
        "# h1 %%wf:000000000002%%\n\n## h2 %%wf:000000000003%%\n\n- [ ] item ^wf-000000000004\n\n# nums %%wf:000000000005%%\n\n## not ordered: a heading %%wf:000000000006%%\n"
    );
}

#[test]
fn view_read_only_tiers() {
    let t = tree(&[
        (2, 1, "broken <b>bold", None, "bullets", false),
        (3, 1, "two\nlines", None, "bullets", false),
        (4, 1, "fine", Some("<i>unclosed"), "bullets", false),
        (5, 1, "a\n```", None, "code-block", false),
        (6, 1, "<b>x</b> <foo>y</foo>", None, "bullets", false),
    ]);
    let v = View::build(&t, &id(1), &DocConfig::default(), &ViewExtras::default()).unwrap();
    let ro: Vec<bool> = v.items.iter().map(|i| i.read_only).collect();
    // A code line that would close a ``` fence is not tier 3: the fence is lengthened and
    // the code stays editable (§5.3).
    assert_eq!(ro, vec![true, true, true, false, false]);
    assert_eq!(v.items[0].name.tier, Tier::ReadOnly);
    assert_eq!(v.items[3].name.tier, Tier::Exact);
    assert_eq!(v.items[4].name.tier, Tier::Lossy);
    let md = render_tree(&t, &DocConfig::default(), &ViewExtras::default());
    assert_eq!(
        body(&md),
        "- [ ] 🔒 broken bold ^wf-000000000002-ro\n- [ ] 🔒 two ⏎ lines ^wf-000000000003-ro\n- [ ] 🔒 fine ^wf-000000000004-ro\n\tunclosed\n- ```` ^wf-000000000005\n\ta\n\t```\n\t````\n- [ ] **x** y ^wf-000000000006\n"
    );
    let d = parse(&md).unwrap();
    assert_eq!(d.items[0].name_md, "broken bold");
    assert_eq!(d.items[3].name_md, "a\n```");
}

#[test]
fn view_temp_ids_have_no_tag() {
    let mut t = tree(&[]);
    let mut n = Node::new(NodeId::temp(1), Some(id(1)), "new item");
    n.priority = 5;
    t.insert(n).unwrap();
    let v = View::build(&t, &id(1), &DocConfig::default(), &ViewExtras::default()).unwrap();
    assert_eq!(v.items[0].tag, None);
    let md = render(&v, &fm());
    assert_eq!(body(&md), "- [ ] new item\n");
}

#[test]
fn view_conflict_markers() {
    let t = tree(&[
        (2, 1, "node", Some("remote note"), "bullets", false),
        (3, 2, "child", None, "bullets", false),
        (4, 1, "done", None, "bullets", true),
    ]);
    let marker = |n: u64, field, md: &str| ConflictMarker {
        node: id(n),
        field,
        local_md: md.to_owned(),
    };
    let extras = ViewExtras {
        conflicts: vec![
            marker(2, ConflictField::Note, "local\nnote"),
            marker(2, ConflictField::Name, "local name"),
            marker(1, ConflictField::Note, "local root note"),
            marker(4, ConflictField::Name, "gone"),
        ],
        ..ViewExtras::default()
    };
    let v = View::build(&t, &id(1), &DocConfig::default(), &extras).unwrap();
    assert_eq!(v.root_conflicts.len(), 1);
    assert_eq!(
        v.dropped_conflicts,
        vec![marker(4, ConflictField::Name, "gone")]
    );
    let md = render_tree(&t, &DocConfig::default(), &extras);
    assert_eq!(
        body(&md),
        "- ⚠ local note %%wf-conflict:note%%\n\tlocal root note\n- node ^wf-000000000002\n\tremote note\n\t- ⚠ local name %%wf-conflict:name%%\n\t- ⚠ local note %%wf-conflict:note%%\n\t\tlocal\n\t\tnote\n\t- [ ] child ^wf-000000000003\n"
    );
    let d = parse(&md).unwrap();
    assert!(d.items[0].is_marker());
    assert_eq!(d.items[0].note_md, vec!["local root note"]);
    assert_eq!(d.items[1].children[1].note_md, vec!["local", "note"]);
}

// ---------------------------------------------------------------------------------------
// Rendering details and normalisation (§4.1, §4.4, §5.4)
// ---------------------------------------------------------------------------------------

#[test]
fn render_empty_and_tag_styles() {
    let t = tree(&[(2, 1, "item", None, "bullets", false)]);
    let only_root = tree(&[]);
    assert_eq!(
        render_tree(&only_root, &DocConfig::default(), &ViewExtras::default()),
        "---\nworkflowy: 000000000001\n---\n"
    );
    for (style, form) in [
        (TagStyle::Auto, "^wf-000000000002"),
        (TagStyle::BlockId, "^wf-000000000002"),
        (TagStyle::Comment, "%%wf:000000000002%%"),
        (TagStyle::Dataview, "[wid:: 000000000002]"),
        (TagStyle::DataviewHidden, "(wid:: 000000000002)"),
        (TagStyle::Html, "<!-- wf:000000000002 -->"),
    ] {
        let cfg = DocConfig {
            tags: style,
            ..DocConfig::default()
        };
        let md = render_tree(&t, &cfg, &ViewExtras::default());
        assert_eq!(body(&md), format!("- [ ] item {form}\n"));
        let cfg = DocConfig {
            headings: 1,
            tags: style,
            ..DocConfig::default()
        };
        let md = render_tree(&t, &cfg, &ViewExtras::default());
        let heading_form = if matches!(style, TagStyle::Auto | TagStyle::BlockId) {
            "%%wf:000000000002%%"
        } else {
            form
        };
        assert_eq!(body(&md), format!("# item {heading_form}\n"));
    }
}

#[test]
fn normalize_canonicalises_text_and_markers() {
    let text = "---\nworkflowy: 000000000001\nsyncflowy: {ids: comment}\n---\n\n\nPre  \n* __bold__ _it_ ^wf-000000000002\n    note with &amp;\n    * [X] child (wid:: 000000000003)\n1) one\n7. two\n# heading\n- ⚠ local  %%wf-conflict:name%%\n- 🔒 kept __as is__ ^wf-000000000004-ro\n";
    let d = parse(text).unwrap();
    let n = normalize(&d);
    assert_eq!(
        n,
        "---\nworkflowy: 000000000001\nsyncflowy: {ids: comment}\n---\nPre\n\n- **bold** *it* %%wf:000000000002%%\n\tnote with &\n\t- [x] child %%wf:000000000003%%\n1. one\n2. two\n\n# heading\n\n- ⚠ local %%wf-conflict:name%%\n- 🔒 kept __as is__ %%wf:000000000004-ro%%\n"
    );
    let again = parse(&n).unwrap();
    assert!(equivalent(&again, &d, &InlineConfig::default()));
    assert_eq!(normalize(&again), n);
}

#[test]
fn equivalence() {
    let cfg = InlineConfig::default();
    let a = doc("- [ ] **x** ^wf-000000000002\n\tn\n");
    assert!(equivalent(
        &a,
        &doc("* __x__ %%wf:000000000002%%\n    n\n"),
        &cfg
    ));
    assert!(!equivalent(
        &a,
        &doc("- [x] **x** ^wf-000000000002\n\tn\n"),
        &cfg
    ));
    assert!(!equivalent(
        &a,
        &doc("- [ ] **y** ^wf-000000000002\n\tn\n"),
        &cfg
    ));
    assert!(!equivalent(
        &a,
        &doc("- [ ] **x** ^wf-000000000003\n\tn\n"),
        &cfg
    ));
    assert!(!equivalent(
        &a,
        &doc("- [ ] **x** ^wf-000000000002-ro\n\tn\n"),
        &cfg
    ));
    assert!(!equivalent(
        &a,
        &doc("1. [ ] **x** ^wf-000000000002\n\tn\n"),
        &cfg
    ));
    assert!(!equivalent(
        &a,
        &doc("- [ ] **x** ^wf-000000000002\n\tm\n"),
        &cfg
    ));
    assert!(!equivalent(
        &a,
        &doc("- [ ] **x** ^wf-000000000002\n\tn\n\t- c\n"),
        &cfg
    ));
    assert!(!equivalent(
        &a,
        &doc("# **x** %%wf:000000000002%%\nn\n"),
        &cfg
    ));
    let mut b = a.clone();
    b.frontmatter.generation = Some(1);
    assert!(!equivalent(&a, &b, &cfg));
    let mut b = a.clone();
    b.root_note = vec!["x".to_owned()];
    assert!(!equivalent(&a, &b, &cfg));
}

#[test]
fn parse_line_numbers() {
    let d = doc("pre\n\n- a\n\tnote\n\t- b\n\n- c\n");
    let lines: Vec<(usize, usize)> = d
        .walk()
        .into_iter()
        .map(|(_, i)| (i.line, i.end_line))
        .collect();
    assert_eq!(lines, vec![(6, 7), (8, 8), (10, 10)]);
}
