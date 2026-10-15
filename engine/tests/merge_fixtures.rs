//! The merge fixture corpus: `tests/fixtures/merge/*.json`, one case per file (base
//! tree, remote tree, optional base doc, local doc, options) with the expected result
//! summary (`merge_support::summarize`). Every row of the §6.1 and §6.2 tables and every
//! §4.4 case has at least one fixture; each file's `description` cites its rule.
//!
//! Run with `UPDATE_MERGE_FIXTURES=1` to (re)write the `expected` member of every
//! fixture from the current implementation; review the diff before committing.

#[path = "merge_support.rs"]
mod merge_support;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use merge_support::{
    Case, DocSpec, NodeSpec, build_doc, build_tree, id, relabel, render_spec, summarize,
};
use serde::Deserialize;
use serde_json::Value;
use syncflowy::apply::Adoption;
use syncflowy::mdblock::{DocConfig, LocalDoc};
use syncflowy::merge::{CrossBase, CrossFile, MergeConfig, cross_bases};
use syncflowy::tree::{IdMap, NodeId};

/// A fixture. The remote tree is `remote` if given, else `base` plus `remote_extra`
/// top-level trees with `remote_edits` applied. The local doc is `local` if given, else
/// the base doc with `local_edits` applied. Edit ops (JSON objects):
///
/// * tree: `{"op":"set","id":…, <fields>}`, `{"op":"delete","id":…}`,
///   `{"op":"insert","parent":…,"index":…,"node":{…}}`,
///   `{"op":"move","id":…,"parent":…,"index":…}`;
/// * doc: `{"op":"set","tag":…, <fields>}`, `{"op":"delete","tag":…}`,
///   `{"op":"insert","parent":…|null,"index":…,"item":{…}}`,
///   `{"op":"move","tag":…,"parent":…|null,"index":…}`, `{"op":"untag","tag":…}`,
///   `{"op":"root_note","note":[…]}`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Fixture {
    description: String,
    #[serde(default = "default_root")]
    root: String,
    /// The root's subtree at base time.
    base: NodeSpec,
    /// Top-level nodes of the export (the root among them).
    #[serde(default)]
    remote: Option<Vec<NodeSpec>>,
    #[serde(default)]
    remote_extra: Vec<NodeSpec>,
    #[serde(default)]
    remote_edits: Vec<Value>,
    /// Defaults to the render of `base`.
    #[serde(default)]
    base_doc: Option<DocSpec>,
    #[serde(default)]
    local: Option<DocSpec>,
    #[serde(default)]
    local_edits: Vec<Value>,
    #[serde(default)]
    config: ConfigSpec,
    #[serde(default)]
    pending: Vec<String>,
    #[serde(default)]
    adoptions: Vec<AdoptionSpec>,
    #[serde(default)]
    prior_docs: Vec<DocSpec>,
    #[serde(default)]
    cross: CrossSpec,
    #[serde(default)]
    expected: Option<Value>,
}

fn default_root() -> String {
    "root".to_owned()
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct ConfigSpec {
    #[serde(default)]
    force: bool,
    #[serde(default)]
    safe_mode: bool,
    #[serde(default)]
    todo_from_checkbox: bool,
    #[serde(default)]
    max_depth: Option<u32>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AdoptionSpec {
    node: String,
    parent: String,
    name: String,
    #[serde(default)]
    note: Option<String>,
    #[serde(default = "default_layout")]
    layout: String,
}

fn default_layout() -> String {
    "bullets".to_owned()
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct CrossSpec {
    #[serde(default)]
    other_roots: Vec<String>,
    #[serde(default)]
    claimed_elsewhere: Vec<String>,
    #[serde(default)]
    still_in_own_file: Vec<String>,
    /// The base of the file a pasted line was cut from (`CrossFile::bases`).
    #[serde(default)]
    source: Option<SourceSpec>,
}

/// Another file's merge base: its root's subtree at base time, its base render
/// (defaults to the render of `base`) and older renders.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SourceSpec {
    base: NodeSpec,
    #[serde(default)]
    base_doc: Option<DocSpec>,
    #[serde(default)]
    prior_docs: Vec<DocSpec>,
}

/// `CrossFile::bases` from the fixture's source file, as `sync` computes it.
fn cross_bases_of(spec: &CrossSpec, doc: &DocConfig) -> BTreeMap<NodeId, CrossBase> {
    let Some(src) = &spec.source else {
        return BTreeMap::new();
    };
    let root = id(&src.base.id);
    let base = build_tree(std::slice::from_ref(&src.base));
    let base_doc = src
        .base_doc
        .clone()
        .unwrap_or_else(|| render_spec(&base, &root, doc, &BTreeSet::new()));
    let base_doc = build_doc(&root, &base_doc);
    let priors: Vec<LocalDoc> = src.prior_docs.iter().map(|d| build_doc(&root, d)).collect();
    let ids = IdMap::from_tree(&base);
    let wanted: BTreeSet<NodeId> = base.ids().cloned().collect();
    cross_bases(&base, &base_doc, &priors, &ids, &wanted)
}

/// Remove the object whose `key` is `id` anywhere in `list` (recursing into
/// `children`).
fn take(list: &mut Vec<Value>, key: &str, id: &str) -> Option<Value> {
    if let Some(pos) = list.iter().position(|v| v[key] == id) {
        return Some(list.remove(pos));
    }
    for v in list.iter_mut() {
        if let Some(Value::Array(kids)) = v.get_mut("children")
            && let Some(found) = take(kids, key, id)
        {
            return Some(found);
        }
    }
    None
}

fn find<'a>(list: &'a mut [Value], key: &str, id: &str) -> Option<&'a mut Value> {
    for v in list.iter_mut() {
        if v[key] == id {
            return Some(v);
        }
        if let Some(Value::Array(kids)) = v.get_mut("children")
            && let Some(found) = find(kids, key, id)
        {
            return Some(found);
        }
    }
    None
}

fn children_mut<'a>(list: &'a mut Vec<Value>, key: &str, parent: &Value) -> &'a mut Vec<Value> {
    match parent.as_str() {
        None => list,
        Some(p) => {
            let node = find(list, key, p).unwrap_or_else(|| panic!("no {key} {p}"));
            let obj = node.as_object_mut().expect("object");
            obj.entry("children")
                .or_insert_with(|| Value::Array(Vec::new()));
            obj.get_mut("children")
                .and_then(Value::as_array_mut)
                .expect("array")
        }
    }
}

/// Apply edit ops to a list of objects identified by `key` (module docs).
fn apply_edits(list: &mut Vec<Value>, key: &str, child_key: &str, edits: &[Value]) {
    for e in edits {
        let op = e["op"].as_str().expect("op");
        let target = e[key].as_str().map(str::to_owned);
        match op {
            "set" => {
                let t = target.expect("target");
                let node = find(list, key, &t).unwrap_or_else(|| panic!("no {key} {t}"));
                for (k, v) in e.as_object().expect("object") {
                    if k != "op" && k != key {
                        node[k.as_str()] = v.clone();
                    }
                }
                if let Some(new_tag) = e.get("new_tag") {
                    node[key] = new_tag.clone();
                    node.as_object_mut().expect("object").remove("new_tag");
                }
            }
            "untag" => {
                let t = target.expect("target");
                let node = find(list, key, &t).unwrap_or_else(|| panic!("no {key} {t}"));
                node.as_object_mut().expect("object").remove(key);
            }
            "delete" => {
                let t = target.expect("target");
                take(list, key, &t).unwrap_or_else(|| panic!("no {key} {t}"));
            }
            "insert" | "move" => {
                let item = if op == "insert" {
                    e[child_key].clone()
                } else {
                    let t = target.expect("target");
                    take(list, key, &t).unwrap_or_else(|| panic!("no {key} {t}"))
                };
                let kids = children_mut(list, key, &e["parent"]);
                let index = e["index"]
                    .as_u64()
                    .map_or(kids.len(), |i| (i as usize).min(kids.len()));
                kids.insert(index, item);
            }
            other => panic!("unknown op {other}"),
        }
    }
}

fn case_of(f: &Fixture) -> Case {
    let root = id(&f.root);
    let base = build_tree(std::slice::from_ref(&f.base));
    let remote_specs: Vec<NodeSpec> = match &f.remote {
        Some(r) => r.clone(),
        None => {
            let mut forest: Vec<Value> = std::iter::once(&f.base)
                .chain(f.remote_extra.iter())
                .map(|n| serde_json::to_value(n).expect("spec"))
                .collect();
            apply_edits(&mut forest, "id", "node", &f.remote_edits);
            serde_json::from_value(Value::Array(forest)).expect("remote spec")
        }
    };
    let remote = build_tree(&remote_specs);
    let mut cfg = MergeConfig {
        force: f.config.force,
        safe_mode: f.config.safe_mode,
        todo_from_checkbox: f.config.todo_from_checkbox,
        ..MergeConfig::default()
    };
    cfg.doc.max_depth = f.config.max_depth;
    let pending: BTreeSet<_> = f.pending.iter().map(|p| id(p)).collect();
    let base_spec = f
        .base_doc
        .clone()
        .unwrap_or_else(|| render_spec(&base, &root, &cfg.doc, &pending));
    let local_spec = match &f.local {
        Some(l) => l.clone(),
        None => {
            let mut doc = serde_json::to_value(&base_spec).expect("doc");
            let mut items: Vec<Value> = serde_json::from_value(doc["items"].take()).expect("items");
            let (note_edits, item_edits): (Vec<Value>, Vec<Value>) = f
                .local_edits
                .iter()
                .cloned()
                .partition(|e| e["op"] == "root_note");
            apply_edits(&mut items, "tag", "item", &item_edits);
            doc["items"] = Value::Array(items);
            for e in note_edits {
                doc["note"] = e["note"].clone();
            }
            serde_json::from_value(doc).expect("local spec")
        }
    };
    let base_doc = build_doc(&root, &base_spec);
    let local_doc = build_doc(&root, &local_spec);
    let mut case = Case::new(root.clone(), base, remote, base_doc, local_doc);
    case.cfg = cfg;
    case.pending = pending;
    case.adoptions = f
        .adoptions
        .iter()
        .map(|a| Adoption {
            node: id(&a.node),
            parent: id(&a.parent),
            name: a.name.clone(),
            note: a.note.clone(),
            layout: a.layout.clone(),
        })
        .collect();
    case.prior_docs = f.prior_docs.iter().map(|d| build_doc(&root, d)).collect();
    case.cross = CrossFile {
        other_roots: f.cross.other_roots.iter().map(|r| id(r)).collect(),
        claimed_elsewhere: f.cross.claimed_elsewhere.iter().map(|r| id(r)).collect(),
        still_in_own_file: f.cross.still_in_own_file.iter().map(|r| id(r)).collect(),
        bases: cross_bases_of(&f.cross, &case.cfg.doc),
    };
    case
}

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/merge")
}

fn actual_of(case: &Case) -> Value {
    match case.run() {
        Ok(r) => summarize(&r),
        Err(e) => relabel(serde_json::json!({ "error": e.to_string() })),
    }
}

#[test]
fn merge_fixture_corpus() {
    let update = std::env::var_os("UPDATE_MERGE_FIXTURES").is_some();
    let mut paths: Vec<PathBuf> = std::fs::read_dir(fixture_dir())
        .expect("fixture dir")
        .map(|e| e.expect("entry").path())
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
        .collect();
    paths.sort();
    assert!(paths.len() >= 40, "the corpus has {} fixtures", paths.len());
    let mut failures = Vec::new();
    for path in &paths {
        let text = std::fs::read_to_string(path).expect("read fixture");
        let f: Fixture =
            serde_json::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        assert!(!f.description.is_empty());
        let case = case_of(&f);
        let actual = actual_of(&case);
        // Determinism: the same inputs give the same output.
        assert_eq!(
            actual,
            actual_of(&case),
            "{}: not deterministic",
            path.display()
        );
        if update {
            let mut raw: Value = serde_json::from_str(&text).expect("json");
            raw["expected"] = actual;
            let out = serde_json::to_string_pretty(&raw).expect("serialize") + "\n";
            std::fs::write(path, out).expect("write fixture");
            continue;
        }
        match &f.expected {
            Some(exp) if *exp == actual => {}
            _ => failures.push(format!(
                "{}\n  expected: {}\n  actual:   {}",
                path.display(),
                f.expected
                    .as_ref()
                    .map(|e| serde_json::to_string_pretty(e).expect("json"))
                    .unwrap_or_else(|| "<missing>".into()),
                serde_json::to_string_pretty(&actual).expect("json")
            )),
        }
    }
    assert!(
        failures.is_empty(),
        "{} fixture(s) differ:\n{}",
        failures.len(),
        failures.join("\n\n")
    );
}
