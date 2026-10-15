//! The mdinline corpus (`tests/fixtures/mdinline/corpus.jsonl`): realistic values from
//! the UI findings (UI:E1–E8, F1/F2, C2), the API's create pipeline (T§5–7, DM§6.2) and
//! everyday typing, with their reviewed Markdown rendering and tier.
//!
//! Checks, for every entry: the stored rendering and tier; block safety (EC 4); the round
//! trip for tiers 1/2 (EC 1); values the UI produces are tier 1 or 2 (§5.3); and that a
//! user re-saving the rendered Markdown unchanged writes the value back (§5.2, §5.4).
//!
//! `MDINLINE_BLESS=1` rewrites the stored rendering and tier from the current code (for
//! review in the diff). `make_corpus.py` in the same directory regenerates the inputs.
//!
//! [`typed_obsidian_syntax`] checks Markdown typed in Obsidian the other way round: what
//! it stores in Workflowy and how it comes back.

use std::path::PathBuf;

use serde_json::{Value, json};
use syncflowy::mdinline::{
    FieldKind, InlineConfig, Tier, drops_link_title, line_is_block_safe, md_to_wf, wf_to_md,
    write_value,
};

fn corpus_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mdinline/corpus.jsonl")
}

fn kind_of(s: &str) -> FieldKind {
    match s {
        "name" => FieldKind::Name,
        "note" => FieldKind::Note,
        "code" => FieldKind::Code,
        other => panic!("unknown kind {other}"),
    }
}

fn tier_name(t: Tier) -> &'static str {
    match t {
        Tier::Exact => "Exact",
        Tier::Passthrough => "Passthrough",
        Tier::Lossy => "Lossy",
        Tier::ReadOnly => "ReadOnly",
    }
}

#[test]
fn corpus() {
    let cfg = InlineConfig::default();
    let bless = std::env::var_os("MDINLINE_BLESS").is_some();
    let text = std::fs::read_to_string(corpus_path()).expect("corpus.jsonl");
    let mut rows: Vec<Value> = text
        .lines()
        .map(|l| serde_json::from_str(l).expect("corpus line"))
        .collect();
    assert!(rows.len() >= 100, "corpus too small");
    let mut failures = Vec::new();
    for row in &mut rows {
        let kind = kind_of(row["kind"].as_str().unwrap());
        let wf = row["wf"].as_str().unwrap().to_owned();
        let src = row["src"].as_str().unwrap().to_owned();
        let got = wf_to_md(&wf, kind, &cfg);
        if bless {
            row["md"] = json!(got.text);
            row["tier"] = json!(tier_name(got.tier));
        }
        let mut fail = |msg: String| failures.push(format!("[{src}] {wf:?}: {msg}"));
        if row["md"].as_str() != Some(got.text.as_str()) {
            fail(format!("rendered {:?}, stored {}", got.text, row["md"]));
        }
        if row["tier"].as_str() != Some(tier_name(got.tier)) {
            fail(format!("tier {:?}, stored {}", got.tier, row["tier"]));
        }
        for line in got.text.split('\n') {
            if !line_is_block_safe(line) {
                fail(format!("unsafe line {line:?}"));
            }
        }
        if kind == FieldKind::Name && got.text.contains('\n') {
            fail("multi-line name".to_owned());
        }
        let exact = matches!(got.tier, Tier::Exact | Tier::Passthrough);
        if row["expect"] == "ui" && !exact {
            fail(format!("UI value is {:?}", got.tier));
        }
        if exact && md_to_wf(&got.text, kind, &cfg) != wf {
            fail(format!(
                "round trip gives {:?}",
                md_to_wf(&got.text, kind, &cfg)
            ));
        }
        // Saving the rendering unchanged never changes a representable value (§5.2).
        if got.tier != Tier::ReadOnly {
            let w = write_value(&wf, &got.text, kind, &cfg);
            if exact && (w.wf != wf || w.lost) {
                fail(format!(
                    "unchanged save writes {:?} (lost {})",
                    w.wf, w.lost
                ));
            }
        }
    }
    if bless {
        let out: String = rows
            .iter()
            .map(|r| serde_json::to_string(r).unwrap() + "\n")
            .collect();
        std::fs::write(corpus_path(), out).expect("write corpus");
    }
    assert!(
        failures.is_empty(),
        "{} failures:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// Obsidian syntax typed in Markdown (§4.2): the WF value it is stored as, the Markdown
/// it comes back as after a sync, and whether the write drops something (a link title,
/// reported as a lossy write).
#[test]
fn typed_obsidian_syntax() {
    let cfg = InlineConfig::default();
    for (md, wf, back, lossy) in [
        // A comment is stored as text and comes back as typed, so Obsidian still hides it.
        (
            "draft %%hidden comment%%",
            "draft %%hidden comment%%",
            "draft %%hidden comment%%",
            false,
        ),
        // ... unless it would read as a tag.
        (
            "%%wf:8e1f00aa2b3c%%",
            "%%wf:8e1f00aa2b3c%%",
            "%\\%wf:8e1f00aa2b3c%%",
            false,
        ),
        // An image is literal text: no link inside, written back unescaped.
        (
            "![img](https://example.com/i.png)",
            "![img](https://example.com/i.png)",
            "![img](https://example.com/i.png)",
            false,
        ),
        (
            "![[embed.png]] and ![a](b.png)",
            "![[embed.png]] and ![a](b.png)",
            "![[embed.png]] and ![a](b.png)",
            false,
        ),
        // A link title has no WF form: the link is kept, the title dropped.
        (
            "[a](https://example.com \"title\")",
            r#"<a href="https://example.com">a</a>"#,
            "[a](https://example.com)",
            true,
        ),
        (
            "see [a](u 'title') and [b](v)",
            r#"see <a href="u">a</a> and <a href="v">b</a>"#,
            "see [a](u) and [b](v)",
            true,
        ),
    ] {
        for kind in [FieldKind::Name, FieldKind::Note] {
            assert_eq!(md_to_wf(md, kind, &cfg), wf, "{md}");
            let got = wf_to_md(wf, kind, &cfg);
            assert_eq!((got.text.as_str(), got.tier), (back, Tier::Exact), "{md}");
            assert_eq!(drops_link_title(md, kind, &cfg), lossy, "{md}");
        }
    }
}
