//! Snapshot tests: the rich fixture tree (tests/fixtures/mdblock/rich_tree.json, an
//! export-shaped `{"nodes": [...]}` written by make_fixture.py) rendered under several
//! configurations. Every snapshot is also checked to be a render fixed point (§5.4
//! invariant 1) and to parse without warnings.
//!
//! Refresh with `INSTA_UPDATE=always cargo test --test mdblock_snapshots` and review the
//! diff of tests/snapshots/.

use std::collections::{BTreeMap, BTreeSet};

use syncflowy::mdblock::{
    Checkbox, CheckboxPolicy, ConflictMarker, DocConfig, Frontmatter, RootRef, Sticky, TagStyle,
    View, ViewExtras, normalize, parse, render,
};
use syncflowy::mdinline::ConflictField;
use syncflowy::tree::{ApiNode, NodeId, Tree};

fn fixture() -> Tree {
    let text = include_str!("fixtures/mdblock/rich_tree.json");
    let v: serde_json::Value = serde_json::from_str(text).expect("fixture json");
    let nodes: Vec<ApiNode> = serde_json::from_value(v["nodes"].clone()).expect("nodes");
    Tree::from_api(nodes).expect("tree")
}

fn id(n: u64) -> NodeId {
    NodeId::from_api(format!("00000000-0000-4000-8000-{n:012x}"))
}

fn frontmatter(cfg: DocConfig) -> Frontmatter {
    let mut fm = Frontmatter::new(RootRef::parse("000000000001").expect("root"));
    fm.config = cfg;
    fm.generation = Some(7);
    fm.other = vec!["tags: [workflowy, sync]".to_owned()];
    fm
}

fn render_checked(cfg: DocConfig, extras: &ViewExtras) -> String {
    let tree = fixture();
    let view = View::build(&tree, &id(1), &cfg, extras).expect("view");
    let fm = frontmatter(cfg);
    let md = render(&view, &fm);
    let doc = parse(&md).expect("parse");
    assert_eq!(normalize(&doc), md, "render is not a fixed point");
    assert!(doc.warnings.is_empty(), "warnings: {:#?}", doc.warnings);
    assert_eq!(doc.frontmatter, fm);
    md
}

#[test]
fn rich_default() {
    let md = render_checked(DocConfig::default(), &ViewExtras::default());
    insta::assert_snapshot!(md);
}

#[test]
fn rich_headings_two() {
    let cfg = DocConfig {
        headings: 2,
        ..DocConfig::default()
    };
    let md = render_checked(cfg, &ViewExtras::default());
    insta::assert_snapshot!(md);
}

#[test]
fn rich_conflicts_and_options() {
    let cfg = DocConfig {
        headings: 1,
        tags: TagStyle::Dataview,
        checkboxes: CheckboxPolicy::All,
        max_depth: Some(3),
        ..DocConfig::default()
    };
    let extras = ViewExtras {
        conflicts: vec![
            ConflictMarker {
                node: id(1),
                field: ConflictField::Note,
                local_md: "My local preamble\n\nwith a gap".to_owned(),
            },
            ConflictMarker {
                node: id(3),
                field: ConflictField::Name,
                local_md: "Draft the *spec* (my words)".to_owned(),
            },
            ConflictMarker {
                node: id(3),
                field: ConflictField::Note,
                local_md: "my note".to_owned(),
            },
            ConflictMarker {
                node: id(19),
                field: ConflictField::Name,
                local_md: "fn main() {\n    todo!()\n}".to_owned(),
            },
            // A marker on a hidden (completed) node is dropped.
            ConflictMarker {
                node: id(5),
                field: ConflictField::Name,
                local_md: "never shown".to_owned(),
            },
        ],
        hidden: BTreeSet::from([id(17)]),
        sticky: Sticky {
            checkbox: BTreeMap::from([(id(9), Checkbox::None), (id(15), Checkbox::None)]),
            full_placement: BTreeMap::new(),
        },
    };
    let tree = fixture();
    let view = View::build(&tree, &id(1), &cfg, &extras).expect("view");
    assert_eq!(view.dropped_conflicts.len(), 1);
    let md = render_checked(cfg, &extras);
    assert!(!md.contains("never shown"));
    insta::assert_snapshot!(md);
}
