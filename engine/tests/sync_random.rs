//! Randomised end-to-end cycles against the mock: random Markdown edits (rename, new
//! item, delete, tick, reorder, note) and random web-UI edits (rename, create, complete,
//! delete, move) between syncs. After every sync: no error, no verification
//! discrepancy, no nonce left in Workflowy, the local edits that did not race a remote
//! edit arrived, and a second sync is a no-op (the file is settled).

mod sync_support;

use serde_json::json;
use syncflowy::sync::SyncOutcome;
use syncflowy::tree::{NodeId, Tree};

use sync_support::*;

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
    fn chance(&mut self, pct: u64) -> bool {
        self.next() % 100 < pct
    }
}

const WORDS: &[&str] = &[
    "alpha", "beta", "gamma", "delta", "tasks", "notes", "R&D", "x*y", "a<b", "#tag", "50%",
    "café", "go-live", "[draft]", "`cmd`",
];

fn word(rng: &mut Rng) -> String {
    let n = 1 + rng.below(3);
    (0..n)
        .map(|_| WORDS[rng.below(WORDS.len())])
        .collect::<Vec<_>>()
        .join(" ")
}

/// An item line of the file: index, indentation depth, tag (short id).
fn items(text: &str) -> Vec<(usize, usize, Option<String>)> {
    text.lines()
        .enumerate()
        .filter_map(|(i, l)| {
            let depth = l.len() - l.trim_start_matches('\t').len();
            let rest = &l[depth..];
            if !rest.starts_with("- ") {
                return None;
            }
            let tag = l.rfind("^wf-").map(|k| l[k + 4..k + 16].to_owned());
            Some((i, depth, tag))
        })
        .collect()
}

/// True if line `i` (an item) has no deeper line right after it (a leaf, no note).
fn is_leaf(lines: &[&str], i: usize, depth: usize) -> bool {
    match lines.get(i + 1) {
        None => true,
        Some(n) => n.len() - n.trim_start_matches('\t').len() <= depth || n.trim().is_empty(),
    }
}

/// One random local edit: the new text and, for a rename, (short id, new text).
fn local_edit(rng: &mut Rng, text: &str) -> (String, Option<(String, String)>) {
    let lines: Vec<&str> = text.lines().collect();
    let its: Vec<_> = items(text)
        .into_iter()
        .filter(|(i, _, t)| t.is_some() && !lines[*i].contains("⚠") && !lines[*i].contains("-ro"))
        .collect();
    let mut out: Vec<String> = lines.iter().map(|l| l.to_string()).collect();
    let kind = rng.below(7);
    if its.is_empty() {
        out.push(format!("- {}", word(rng)));
        return (join(&out), None);
    }
    let (i, depth, tag) = its[rng.below(its.len())].clone();
    let tag = tag.unwrap();
    let indent = "\t".repeat(depth);
    let desc;
    let mut renamed = None;
    match kind {
        0 => {
            // rename: replace the text before the tag
            let new = word(rng);
            let l = &lines[i];
            let k = l.rfind(" ^wf-").unwrap();
            let prefix = if l[depth..].starts_with("- [ ] ") {
                "- [ ] "
            } else {
                "- "
            };
            out[i] = format!("{indent}{prefix}{new}{}", &l[k..]);
            desc = format!("rename {tag} to {new:?}");
            renamed = Some((tag, new));
        }
        1 => {
            // new item after this one's subtree... simply as its first child
            let new = word(rng);
            out.insert(i + 1, format!("{indent}\t- {new}"));
            desc = format!("new child of {tag}: {new:?}");
        }
        2 if is_leaf(&lines, i, depth) => {
            out.remove(i);
            desc = format!("delete {tag}");
        }
        3 if is_leaf(&lines, i, depth) => {
            let l = &lines[i];
            out[i] = if l[depth..].starts_with("- [ ] ") {
                l.replacen("- [ ] ", "- [x] ", 1)
            } else {
                l.replacen("- ", "- [x] ", 1)
            };
            desc = format!("tick {tag}");
        }
        4 if is_leaf(&lines, i, depth)
            && lines.get(i + 1).is_some_and(|n| {
                n.len() - n.trim_start_matches('\t').len() == depth
                    && n[depth..].starts_with("- ")
                    && is_leaf(&lines, i + 1, depth)
            }) =>
        {
            out.swap(i, i + 1);
            desc = format!("swap {tag} with the next sibling");
        }
        5 if is_leaf(&lines, i, depth) => {
            out.insert(i + 1, format!("{indent}\t{}", word(rng)));
            desc = format!("note on {tag}");
        }
        6 if is_leaf(&lines, i, depth) && its.len() > 1 => {
            // move: cut the leaf and paste it as the first child of another item
            let (j, jdepth, jtag) = its[rng.below(its.len())].clone();
            if j == i {
                desc = "no-op move".to_owned();
            } else {
                let line = lines[i][depth..].to_owned();
                out[i] = String::new();
                out.insert(j + 1, format!("{}\t{line}", "\t".repeat(jdepth)));
                out.retain(|l| !l.is_empty());
                desc = format!("move {tag} under {}", jtag.unwrap());
            }
        }
        _ => {
            out.push(format!("- {}", word(rng)));
            desc = "new top-level item".to_owned();
        }
    }
    eprintln!("  local: {desc}");
    (join(&out), renamed)
}

fn join(lines: &[String]) -> String {
    let mut s = lines.join("\n");
    s.push('\n');
    s
}

/// One random remote edit through the UI routes; returns the node touched.
fn remote_edit(rng: &mut Rng, env: &Env, root: &NodeId) -> Option<NodeId> {
    let tree = env.export();
    let nodes: Vec<NodeId> = tree
        .descendants(root)
        .into_iter()
        .filter(|n| tree.is_visible(n, root))
        .collect();
    if nodes.is_empty() {
        env.ui_create(Some(root), &word(rng));
        return None;
    }
    let n = nodes[rng.below(nodes.len())].clone();
    match rng.below(5) {
        0 => {
            env.ui("edit", json!({"id": n.as_str(), "text": word(rng)}));
        }
        1 => {
            env.ui_create(Some(&n), &word(rng));
        }
        2 => {
            env.ui("complete", json!({"id": n.as_str()}));
        }
        3 => {
            env.ui("delete", json!({"id": n.as_str()}));
        }
        _ => {
            let dest = if rng.chance(50) {
                root.clone()
            } else {
                nodes[rng.below(nodes.len())].clone()
            };
            if dest == n || tree.is_descendant(&dest, &n) {
                return None;
            }
            env.ui(
                "move",
                json!({"id": n.as_str(), "parent_id": dest.as_str()}),
            );
        }
    }
    Some(n)
}

fn check_no_nonces(tree: &Tree) {
    for n in tree.iter() {
        assert!(!n.name.contains("syncflowy:"), "nonce left: {n:?}");
    }
}

fn run_seed(seed: u64, rounds: usize) {
    let env = Env::new();
    let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    let root = env.ui_create(None, "Root");
    for _ in 0..3 {
        let p = env.ui_create(Some(&root), &word(&mut rng));
        for _ in 0..rng.below(3) {
            env.ui_create(Some(&p), &word(&mut rng));
        }
    }
    let file = env.path("random.md");
    env.render(&root, &file);
    for round in 0..rounds {
        let before = read(&file);
        let mut renames = std::collections::BTreeMap::new();
        let mut text = before.clone();
        for _ in 0..1 + rng.below(3) {
            let (t, renamed) = local_edit(&mut rng, &text);
            text = t;
            renames.extend(renamed); // the last rename of a node wins
        }
        write(&file, &text);
        let mut touched = Vec::new();
        for _ in 0..rng.below(3) {
            touched.extend(remote_edit(&mut rng, &env, &root));
        }
        let mut r = env.sync(&file);
        if let SyncOutcome::Braked(b) = &r.outcome {
            // A random edit may delete most of a small file: the brake is right; force.
            eprintln!("  braked ({b}); forcing");
            let mut forced = env.cfg.clone();
            forced.force = true;
            r = syncflowy::sync::sync_once(&env.client, &file, &forced).expect("forced sync");
        }
        let ctx = format!(
            "seed {seed} round {round}\nbefore:\n{before}\nedited:\n{text}\nafter:\n{}\n{r:#?}",
            read(&file)
        );
        assert!(
            matches!(
                r.outcome,
                SyncOutcome::Synced { .. } | SyncOutcome::Unchanged
            ),
            "{ctx}"
        );
        assert!(r.discrepancies.is_empty(), "{ctx}");
        let tree = env.export();
        check_no_nonces(&tree);
        // Renames of nodes nobody touched remotely arrived (as their plain words).
        for (short, new) in &renames {
            let Some(node) = tree
                .iter()
                .find(|n| n.id.as_str().ends_with(short.as_str()))
            else {
                continue;
            };
            // Retyping an older value is the documented guard-3 false positive.
            let stale = r.events.iter().any(|e| {
                matches!(e, syncflowy::merge::ReportEvent::StaleEcho { node: n, .. } if *n == node.id)
            });
            if stale || touched.contains(&node.id) || !tree.is_visible(&node.id, &root) {
                continue;
            }
            // The typed Markdown, as Workflowy should now hold it.
            let expect = syncflowy::mdinline::md_to_wf(
                new,
                syncflowy::mdinline::FieldKind::Name,
                &syncflowy::mdinline::InlineConfig::default(),
            );
            assert_eq!(node.name, expect, "rename of {short}\n{ctx}");
        }
        // Settled.
        let snap = env.snapshot();
        let settled = read(&file);
        let r2 = env.sync(&file);
        assert_eq!(
            r2.outcome,
            SyncOutcome::Unchanged,
            "not settled\n{ctx}\n{r2:#?}"
        );
        assert_eq!(env.snapshot(), snap, "{ctx}");
        assert_eq!(read(&file), settled);
    }
}

#[test]
fn random_cycles() {
    let seeds: u64 = std::env::var("SYNC_RANDOM_SEEDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(6);
    for seed in 1..=seeds {
        run_seed(seed, 12);
    }
}
