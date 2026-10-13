//! Differential tests of the §7.2 create-name whitelist ([`plan::create_name_is_safe`])
//! against the mock's create-path name pipeline (`src/syncflowy/mock/text.py`,
//! data-model.md §6.2), which is the test oracle for the real API.
//!
//! * Generated names (proptest strategies, sampled deterministically and sent to the
//!   oracle in one batch): every name the whitelist accepts must come back from the
//!   pipeline unchanged, as a single plain, incomplete block.
//! * Hand-written names: the whitelist rejects every documented risky case (edge
//!   whitespace, line breaks, markdown prefixes, entities, inline markdown, dates,
//!   completion leaks), and the oracle confirms that each of them is really changed.
//! * End to end: whitelisted names sent through the mock server's create endpoint are
//!   stored verbatim, with no extra child nodes.
//!
//! The oracle is `tests/fixtures/plan/create_oracle.py`, run with
//! `uv run python -I` from the repository root.

use std::io::Write;
use std::process::{Command, Stdio};

use proptest::strategy::{Strategy, ValueTree};
use proptest::test_runner::TestRunner;
use serde_json::{Value, json};
use syncflowy::plan::create_name_is_safe;
use syncflowy::testsupport::{MockServer, repo_root};

/// One block of the pipeline's output: (name, layout from a prefix, completed).
type Block = (String, Option<String>, bool);

/// Run the Python oracle on `names`.
fn oracle(names: &[String]) -> Vec<Vec<Block>> {
    let mut child = Command::new("uv")
        .args([
            "run",
            "--quiet",
            "python",
            "-I",
            "engine/tests/fixtures/plan/create_oracle.py",
        ])
        .current_dir(repo_root())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn `uv run python`");
    let input = serde_json::to_vec(names).unwrap();
    child.stdin.take().unwrap().write_all(&input).unwrap();
    let out = child.wait_with_output().expect("oracle runs");
    assert!(
        out.status.success(),
        "oracle failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: Vec<Vec<(String, Option<String>, bool)>> =
        serde_json::from_slice(&out.stdout).expect("oracle output is JSON");
    assert_eq!(v.len(), names.len());
    v
}

fn is_fixed_point(name: &str, blocks: &[Block]) -> bool {
    blocks == [(name.to_owned(), None, false)]
}

/// Characters that make up the generated names: mostly harmless text, plus every
/// character class the pipeline treats specially.
const SAFE_ALPHABET: &str = r"[a-zA-Z0-9 .,:;!?'()/%$@^=+|{}éü€中✓\u{a0}\u{2003}\u{feff}\u{200b}-]";
const RISKY_TOKENS: &[&str] = &[
    "&",
    "&amp;",
    "&#42;",
    "&lt;b&gt;",
    "&#10;",
    "*",
    "**",
    "[",
    "]",
    "`",
    "```",
    "~~",
    "#",
    "# ",
    "## ",
    "#### ",
    "<",
    ">",
    "> ",
    "\\",
    "\\*",
    "_",
    "\"",
    "\n",
    "\r",
    "\t",
    "\u{b}",
    "\u{c}",
    "\u{1c}",
    "\u{85}",
    "\u{2028}",
    "\u{2029}",
    "\u{0}",
    "\u{7f}",
    " ",
    "- ",
    "- [x] ",
    "- [ ] ",
    "+ ",
    "1. ",
    "http",
    "[2026-01-15]",
    "[2026-1-5 9:5]",
    "[x](y)",
    "\u{a0}",
    "\u{3000}",
];

fn sample<S: Strategy>(strategy: S, n: usize, runner: &mut TestRunner) -> Vec<S::Value> {
    (0..n)
        .map(|_| strategy.new_tree(runner).unwrap().current())
        .collect()
}

/// A deterministic sample of names: plain text (mostly accepted), plain text with risky
/// tokens spliced in (near misses), and arbitrary Unicode strings.
fn generated_names() -> Vec<String> {
    let mut runner = TestRunner::deterministic();
    let plain = || proptest::string::string_regex(&format!("{SAFE_ALPHABET}{{1,24}}")).unwrap();
    let mut names = sample(plain(), 3000, &mut runner);
    let near_miss = (
        plain(),
        proptest::sample::select(RISKY_TOKENS),
        0usize..100,
        proptest::sample::select(RISKY_TOKENS),
        0usize..100,
        proptest::bool::ANY,
    )
        .prop_map(|(base, t1, p1, t2, p2, two)| {
            let mut chars: Vec<String> = base.chars().map(String::from).collect();
            let at = p1 % (chars.len() + 1);
            chars.insert(at, t1.to_owned());
            if two {
                let at = p2 % (chars.len() + 1);
                chars.insert(at, t2.to_owned());
            }
            chars.concat()
        });
    names.extend(sample(near_miss, 3000, &mut runner));
    names.extend(sample(
        proptest::arbitrary::any::<String>(),
        1000,
        &mut runner,
    ));
    names
}

#[test]
fn whitelisted_names_are_fixed_points_of_the_create_pipeline() {
    let names = generated_names();
    let results = oracle(&names);
    let mut accepted = 0;
    let mut fixed = 0;
    let mut wrong = Vec::new();
    for (name, blocks) in names.iter().zip(&results) {
        let is_fixed = is_fixed_point(name, blocks);
        fixed += is_fixed as usize;
        if create_name_is_safe(name) {
            accepted += 1;
            if !is_fixed {
                wrong.push((name.clone(), blocks.clone()));
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "whitelisted names changed by the create pipeline: {wrong:#?}"
    );
    // The sample must exercise both sides of the whitelist.
    assert!(accepted >= 1000, "only {accepted} names accepted");
    assert!(
        names.len() - fixed >= 1000,
        "only {} names changed by the pipeline",
        names.len() - fixed
    );
    eprintln!(
        "{} names: {accepted} whitelisted, {fixed} fixed points (the whitelist is conservative)",
        names.len()
    );
}

/// Risky names the pipeline really changes: (name, why).
const CHANGED: &[(&str, &str)] = &[
    (" lead", "leading space stripped"),
    ("trail ", "trailing space stripped"),
    ("\u{a0}nbsp", "NBSP stripped by str.strip"),
    ("nbsp\u{a0}", "NBSP stripped by str.strip"),
    ("\u{3000}wide", "ideographic space stripped"),
    ("a\nb", "soft-wrap join"),
    ("A\nB", "split into a child"),
    ("a\r\nB", "CR normalised, then split"),
    ("x\u{2028}y", "line separator splits"),
    ("x\u{2029}y", "paragraph separator splits"),
    ("x\u{85}y", "NEL splits"),
    ("x\u{b}y", "vertical tab splits"),
    ("x\u{c}y", "form feed splits"),
    ("x\u{0}y", "NUL stripped"),
    ("x\u{1f}y", "control stripped"),
    ("- item", "bullet prefix eaten"),
    ("- [ ] todo", "todo prefix"),
    ("- [x] done", "completion leak"),
    ("first\n- [x] done", "completion leak in a later block"),
    ("# h", "h1 prefix"),
    ("## h", "h2 prefix"),
    ("### h", "h3 prefix"),
    ("#### h", "h4 becomes p and bold"),
    ("a &amp; b", "entity decoded"),
    ("&#42;x&#42;", "entity then italic"),
    ("&lt;b&gt;x", "entity becomes markup"),
    ("a&#10;b", "entity newline joins"),
    ("**b**", "bold"),
    ("*i*", "italic"),
    ("**x", "empty italic"),
    ("`c`", "code"),
    ("~~s~~", "strike"),
    ("[l](u)", "link"),
    ("![img](u)", "link inside an image"),
    ("[2026-01-15]", "date"),
    ("on [2026-1-5 9:5]", "date with time"),
    ("a\\*b", "escape consumed"),
    ("```code```", "one-line code fence"),
];

/// Risky-looking names the pipeline leaves alone today; the whitelist still rejects
/// them (conservative, §7.2).
const UNCHANGED_BUT_REJECTED: &[&str] = &[
    "+ plus",
    "> quote",
    "1. first",
    "-dash",
    "snake_case",
    "say \"hi\"",
    "see http://x",
    "<b>x</b>",
    "tab\tx",
    "x\\y",
    "#tag",
    "a ~ b",
];

#[test]
fn whitelist_rejects_the_documented_risky_cases() {
    let names: Vec<String> = CHANGED
        .iter()
        .map(|(n, _)| n.to_string())
        .chain(UNCHANGED_BUT_REJECTED.iter().map(|n| n.to_string()))
        .collect();
    let results = oracle(&names);
    for ((name, why), blocks) in CHANGED.iter().zip(&results) {
        assert!(
            !create_name_is_safe(name),
            "whitelist accepts {name:?} ({why})"
        );
        assert!(
            !is_fixed_point(name, blocks),
            "{name:?} ({why}) is not changed by the pipeline: {blocks:?}"
        );
    }
    for (name, blocks) in UNCHANGED_BUT_REJECTED.iter().zip(&results[CHANGED.len()..]) {
        assert!(!create_name_is_safe(name), "whitelist accepts {name:?}");
        assert!(
            is_fixed_point(name, blocks),
            "{name:?} is changed by the pipeline: move it to CHANGED: {blocks:?}"
        );
    }
}

#[test]
fn mock_create_endpoint_stores_whitelisted_names_verbatim() {
    let mock = MockServer::start();
    let (status, body) = mock.post(
        "/api/v1/nodes",
        &json!({"parent_id": "None", "name": "parent", "position": "bottom"}),
    );
    assert_eq!(status, 200, "{body}");
    let parent = body["item_id"].as_str().unwrap().to_owned();
    let create = |name: &str| -> (Value, Vec<Value>) {
        let (status, body) = mock.post(
            "/api/v1/nodes",
            &json!({
                "parent_id": parent,
                "name": name,
                "layoutMode": "bullets",
                "position": "bottom",
            }),
        );
        assert_eq!(status, 200, "{body}");
        let id = body["item_id"].as_str().unwrap().to_owned();
        let (status, node) = mock.get(&format!("/api/v1/nodes/{id}"));
        assert_eq!(status, 200, "{node}");
        let (status, kids) = mock.get(&format!("/api/v1/nodes?parent_id={id}"));
        assert_eq!(status, 200, "{kids}");
        (
            node["node"].clone(),
            kids["nodes"].as_array().unwrap().clone(),
        )
    };
    let safe = [
        "Plain",
        "Buy milk, eggs and bread.",
        "Résumé – naïve café ✓ 中文",
        "What? Yes! (maybe) 100% of 3/4",
        "x - y + z = 1. done",
        "zero\u{200b}width and\u{feff}bom",
    ];
    for name in safe {
        assert!(create_name_is_safe(name), "{name:?}");
        let (node, kids) = create(name);
        assert_eq!(node["name"], name, "{node}");
        assert_eq!(node["completed"], false, "{node}");
        assert_eq!(node["data"]["layoutMode"], "bullets", "{node}");
        assert!(kids.is_empty(), "{kids:?}");
    }
    let near_misses = [
        " Plain",
        "Plain ",
        "Buy milk &amp; eggs",
        "- [x] Buy milk",
        "# Plain",
        "*Plain*",
        "Plain\nMore",
        "on [2026-01-15]",
    ];
    for name in near_misses {
        assert!(!create_name_is_safe(name), "{name:?}");
        let (node, kids) = create(name);
        assert!(
            node["name"] != name || node["completed"] == true || !kids.is_empty(),
            "{name:?} stored unchanged: {node}"
        );
    }
}
