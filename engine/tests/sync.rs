//! `sync` end to end against the mock server: the initial render, the fast path, every
//! kind of local edit pushed, remote web-UI edits pulled, conflicts, the brakes, missing
//! files, stale files (render generations), the compare-and-swap write, and crash
//! recovery through the journal. Each test runs its own mock (ids are seeded, so the
//! snapshots are stable).

mod sync_support;

use std::panic::{AssertUnwindSafe, catch_unwind};

use serde_json::json;
use syncflowy::error::Error;
use syncflowy::mdblock::{self, DocConfig};
use syncflowy::merge::ReportEvent;
use syncflowy::plan::Brake;
use syncflowy::state::StateStore;
use syncflowy::sync::{self, PauseReason, SyncOutcome};
use syncflowy::tree::NodeId;
use syncflowy::wfclient::Api;

use sync_support::*;

/// The standard outline: Project / {Plan (note) / {Draft, Mock}, Errands / {Milk, Bob},
/// Later}.
struct Fixture {
    env: Env,
    root: NodeId,
    plan: NodeId,
    draft: NodeId,
    mock_item: NodeId,
    errands: NodeId,
    milk: NodeId,
    bob: NodeId,
    later: NodeId,
    file: std::path::PathBuf,
}

fn fixture() -> Fixture {
    let env = Env::new();
    let root = env.ui_create(None, "Project");
    let plan = env.ui_create_with(
        Some(&root),
        json!({"text": "Plan", "note_text": "first line\nsecond line"}),
    );
    let draft = env.ui_create(Some(&plan), "Draft the spec");
    let mock_item = env.ui_create(Some(&plan), "Write the mock");
    let errands = env.ui_create(Some(&root), "Errands");
    let milk = env.ui_create(Some(&errands), "Buy milk");
    let bob = env.ui_create(Some(&errands), "Call Bob");
    let later = env.ui_create(Some(&root), "Later");
    let file = env.path("project.md");
    let r = env.render(&root, &file);
    assert_eq!(r.outcome, SyncOutcome::Rendered { generation: 1 });
    Fixture {
        env,
        root,
        plan,
        draft,
        mock_item,
        errands,
        milk,
        bob,
        later,
        file,
    }
}

fn synced(r: &sync::SyncReport) -> (bool, u64) {
    match &r.outcome {
        SyncOutcome::Synced {
            wrote_file,
            generation,
            ..
        } => (*wrote_file, *generation),
        other => panic!("expected Synced, got {other:?}\n{r:#?}"),
    }
}

/// A second sync of the result is a no-op: the file is a fixed point.
fn assert_settled(f: &Fixture) {
    let before = f.env.snapshot();
    let text = read(&f.file);
    let r = f.env.sync(&f.file);
    assert_eq!(r.outcome, SyncOutcome::Unchanged, "{r:#?}\n{text}");
    assert_eq!(read(&f.file), text);
    assert_eq!(f.env.snapshot(), before);
}

// ---------------------------------------------------------------------------------------
// render
// ---------------------------------------------------------------------------------------

#[test]
fn initial_render_snapshot_and_checkpoint() {
    let env = Env::new();
    let root = env.ui_create_with(
        None,
        json!({"text": "Project", "note_text": "The root's note."}),
    );
    let plan = env.ui_create_with(
        Some(&root),
        json!({"text": "Plan", "note_text": "first line\nsecond line"}),
    );
    env.ui_create_with(Some(&plan), json!({"name": "<b>Bold</b> step"}));
    env.ui_create_with(Some(&plan), json!({"text": "A todo", "layoutMode": "todo"}));
    env.ui_create_with(
        Some(&root),
        json!({"name": "let x = 1;\nlet y = 2;", "layoutMode": "code-block"}),
    );
    env.ui_create_with(
        Some(&root),
        json!({"text": "A quote", "layoutMode": "quote-block"}),
    );
    let steps = env.ui_create_with(
        Some(&root),
        json!({"text": "Steps", "layoutMode": "numbered"}),
    );
    env.ui_create(Some(&steps), "one");
    env.ui_create(Some(&steps), "two");
    let finished = env.ui_create(Some(&root), "Finished");
    env.ui_create(Some(&finished), "hidden child");
    env.ui("complete", json!({"id": finished.as_str()}));
    let open = env.ui_create(Some(&root), "Open parent");
    let done_child = env.ui_create(Some(&open), "done child");
    env.ui("complete", json!({"id": done_child.as_str()}));
    env.ui_create(Some(&open), "open child");
    env.ui(
        "mirror",
        json!({"id": plan.as_str(), "parent_id": root.as_str()}),
    );
    env.ui_create_with(Some(&root), json!({"text": "", "layoutMode": "divider"}));

    let file = env.path("project.md");
    let r = env.render(&root, &file);
    assert_eq!(r.outcome, SyncOutcome::Rendered { generation: 1 });
    let text = read(&file);
    insta::assert_snapshot!("initial_render", text);
    assert!(!text.contains("hidden child") && !text.contains("done child"));

    // The checkpoint: generation 1, the exact text, the whole subtree (hidden nodes too).
    let store = StateStore::open(&env.cfg.state_dir).unwrap();
    let cp = store.load_checkpoint(&root).unwrap().expect("checkpoint");
    assert_eq!(cp.generation, 1);
    assert_eq!(cp.base_md, text);
    assert!(cp.base_wf.contains(&done_child));
    assert_eq!(cp.base_wf.len(), env.export().subtree(&root).unwrap().len());
    assert_eq!(
        cp.file.as_deref(),
        Some(std::fs::canonicalize(&file).unwrap().as_path())
    );
    let h = store.get_history(&root, 1).unwrap().expect("history 1");
    assert_eq!(h.md, text);
    assert!(!store.journal_path(&root).exists());

    // The file parses back to the same render (fixed point).
    let doc = mdblock::parse(&text).unwrap();
    assert_eq!(mdblock::normalize(&doc), text);

    // render refuses an existing file, and an existing checkpoint, without --force.
    let short = root.short().unwrap().to_string();
    let e =
        sync::render_once(&env.client, &short, &file, &DocConfig::default(), &env.cfg).unwrap_err();
    assert!(matches!(e, Error::FileExists(_)), "{e}");
    let other = env.path("other.md");
    let e = sync::render_once(&env.client, &short, &other, &DocConfig::default(), &env.cfg)
        .unwrap_err();
    assert!(matches!(e, Error::CheckpointExists(_)), "{e}");
    assert!(!other.exists());
    let mut forced = env.cfg.clone();
    forced.force = true;
    let r = sync::render_once(&env.client, &short, &file, &DocConfig::default(), &forced).unwrap();
    assert_eq!(r.outcome, SyncOutcome::Rendered { generation: 2 });
}

#[test]
fn render_with_headings_round_trips() {
    let f = fixture();
    let file = f.env.path("headings.md");
    std::fs::remove_file(&f.file).unwrap();
    let mut cfg = f.env.cfg.clone();
    cfg.force = true;
    let doc = DocConfig {
        headings: 1,
        ..DocConfig::default()
    };
    let short = f.root.short().unwrap().to_string();
    sync::render_once(&f.env.client, &short, &file, &doc, &cfg).unwrap();
    let text = read(&file);
    assert!(text.contains("# Plan %%wf:"), "{text}");
    let before = f.env.snapshot();
    assert_eq!(f.env.sync(&file).outcome, SyncOutcome::Unchanged);
    // A rename under a heading is pushed.
    edit(&file, "Buy milk", "Buy oat milk");
    f.env.sync(&file);
    assert_eq!(f.env.export().get(&f.milk).unwrap().name, "Buy oat milk");
    assert_ne!(f.env.snapshot(), before);
}

// ---------------------------------------------------------------------------------------
// The fast path
// ---------------------------------------------------------------------------------------

#[test]
fn unchanged_file_sends_nothing() {
    let f = fixture();
    let before = f.env.snapshot();
    let text = read(&f.file);
    let api = TestApi::new(&f.env.client);
    let r = sync::sync_once(&api, &f.file, &f.env.cfg).unwrap();
    assert_eq!(r.outcome, SyncOutcome::Unchanged);
    assert_eq!(api.mutations(), 0, "{:?}", api.calls());
    assert_eq!(api.calls(), vec!["export".to_owned()]);
    assert_eq!(read(&f.file), text);
    assert_eq!(f.env.snapshot(), before);
}

#[test]
fn remote_change_to_a_hidden_node_does_not_rewrite_the_file() {
    let f = fixture();
    let done = f.env.ui_create(Some(&f.later), "done");
    f.env.ui("complete", json!({"id": done.as_str()}));
    // `done` appeared and disappeared: a remote change, but not in the file's view.
    let text = read(&f.file);
    let r = f.env.sync(&f.file);
    assert_eq!(synced(&r), (false, 1), "{r:#?}");
    assert_eq!(read(&f.file), text);
    f.env.ui(
        "edit",
        json!({"id": done.as_str(), "text": "done, renamed"}),
    );
    let r = f.env.sync(&f.file);
    assert_eq!(synced(&r), (false, 1));
    assert_settled(&f);
}

// ---------------------------------------------------------------------------------------
// Local edits are pushed
// ---------------------------------------------------------------------------------------

#[test]
fn local_new_item_is_created_and_tagged() {
    let f = fixture();
    let text = read(&f.file);
    let draft_line = line_of(&text, &f.draft).to_owned();
    edit(
        &f.file,
        &format!("{draft_line}\n"),
        &format!("{draft_line}\n\t- A new step\n\t\twith a note\n"),
    );
    let r = f.env.sync(&f.file);
    assert_eq!(synced(&r), (true, 2), "{r:#?}");
    let tree = f.env.export();
    assert_eq!(
        child_names(&tree, &f.plan),
        ["Draft the spec", "A new step", "Write the mock"]
    );
    let new = by_name(&tree, "A new step");
    assert_eq!(new.note.as_deref(), Some("with a note"));
    let text = read(&f.file);
    assert!(
        line_of(&text, &new.id).contains("A new step ^wf-"),
        "{text}"
    );
    assert!(text.contains("syncflowy: {gen: 2}"), "{text}");
    assert_settled(&f);
}

#[test]
fn local_rename_and_note_edit_are_pushed() {
    let f = fixture();
    edit(&f.file, "Draft the spec", "Draft the **whole** spec");
    edit(&f.file, "second line", "second line, edited");
    let r = f.env.sync(&f.file);
    // The text is already canonical: nothing new to show, the file is not rewritten.
    assert_eq!(synced(&r), (false, 1), "{r:#?}");
    let tree = f.env.export();
    assert_eq!(
        tree.get(&f.draft).unwrap().name,
        "Draft the <b>whole</b> spec"
    );
    assert_eq!(
        tree.get(&f.plan).unwrap().note.as_deref(),
        Some("first line\nsecond line, edited")
    );
    assert_settled(&f);
}

#[test]
fn local_normalisation_is_deferred() {
    let f = fixture();
    edit(&f.file, "Draft the spec", "Draft the __whole__ spec");
    let text = read(&f.file);
    let r = f.env.sync(&f.file);
    assert_eq!(synced(&r), (false, 1), "{r:#?}");
    // Pushed as bold, but the file keeps the user's spelling until it is rewritten for
    // another reason (§9.1 guard 4).
    assert_eq!(
        f.env.export().get(&f.draft).unwrap().name,
        "Draft the <b>whole</b> spec"
    );
    assert_eq!(read(&f.file), text);
    assert_settled(&f);
    f.env
        .ui("edit", json!({"id": f.later.as_str(), "text": "Later on"}));
    f.env.sync(&f.file);
    assert!(read(&f.file).contains("Draft the **whole** spec"));
}

#[test]
fn local_move_between_parents_is_pushed() {
    let f = fixture();
    let text = read(&f.file);
    let milk = line_of(&text, &f.milk).to_owned();
    let draft = line_of(&text, &f.draft).to_owned();
    let mut t = text.replacen(&format!("{milk}\n"), "", 1);
    t = t.replacen(&format!("{draft}\n"), &format!("{draft}\n{milk}\n"), 1);
    write(&f.file, &t);
    let r = f.env.sync(&f.file);
    synced(&r);
    let tree = f.env.export();
    assert_eq!(tree.parent(&f.milk), Some(&f.plan));
    assert_eq!(
        child_names(&tree, &f.plan),
        ["Draft the spec", "Buy milk", "Write the mock"]
    );
    assert_eq!(child_names(&tree, &f.errands), ["Call Bob"]);
    assert_settled(&f);
}

#[test]
fn local_reorder_is_pushed() {
    let f = fixture();
    let text = read(&f.file);
    let milk = line_of(&text, &f.milk).to_owned();
    let bob = line_of(&text, &f.bob).to_owned();
    edit(
        &f.file,
        &format!("{milk}\n{bob}\n"),
        &format!("{bob}\n{milk}\n"),
    );
    let r = f.env.sync(&f.file);
    synced(&r);
    assert_eq!(
        child_names(&f.env.export(), &f.errands),
        ["Call Bob", "Buy milk"]
    );
    assert_settled(&f);
}

#[test]
fn local_delete_is_pushed() {
    let f = fixture();
    let text = read(&f.file);
    let bob = line_of(&text, &f.bob).to_owned();
    edit(&f.file, &format!("{bob}\n"), "");
    let r = f.env.sync(&f.file);
    synced(&r);
    let tree = f.env.export();
    assert!(!tree.contains(&f.bob));
    assert_eq!(child_names(&tree, &f.errands), ["Buy milk"]);
    // Deleted to the Trash (restorable).
    let trash = f.env.mock.get("/_mock/ui/trash").1;
    assert!(trash.to_string().contains(f.bob.as_str()), "{trash}");
    assert_settled(&f);
}

#[test]
fn local_tick_completes_and_hides() {
    let f = fixture();
    edit(&f.file, "- [ ] Buy milk", "- [x] Buy milk");
    let r = f.env.sync(&f.file);
    assert_eq!(synced(&r), (true, 2));
    let tree = f.env.export();
    assert!(tree.get(&f.milk).unwrap().completed);
    assert!(!read(&f.file).contains("Buy milk"));
    assert_settled(&f);
}

#[test]
fn new_item_typed_ticked_is_created_completed() {
    let f = fixture();
    let text = read(&f.file);
    let later = line_of(&text, &f.later).to_owned();
    edit(
        &f.file,
        &format!("{later}\n"),
        &format!("{later}\n\t- [x] Already done\n"),
    );
    let r = f.env.sync(&f.file);
    synced(&r);
    let tree = f.env.export();
    let n = by_name(&tree, "Already done");
    assert_eq!(tree.parent(&n.id), Some(&f.later));
    assert!(n.completed);
    assert!(!read(&f.file).contains("Already done"));
    assert_settled(&f);
}

// ---------------------------------------------------------------------------------------
// Remote edits arrive
// ---------------------------------------------------------------------------------------

#[test]
fn remote_ui_edits_arrive_in_the_file() {
    let f = fixture();
    let env = &f.env;
    // edit
    env.ui(
        "edit",
        json!({"id": f.milk.as_str(), "text": "Buy oat milk"}),
    );
    // create
    let eggs = env.ui_create_with(
        Some(&f.errands),
        json!({"text": "Buy eggs", "after_id": f.milk.as_str()}),
    );
    // move
    env.ui(
        "move",
        json!({"id": f.mock_item.as_str(), "parent_id": f.later.as_str()}),
    );
    // complete
    env.ui("complete", json!({"id": f.draft.as_str()}));
    let r = env.sync(&f.file);
    assert_eq!(synced(&r), (true, 2), "{r:#?}");
    let text = read(&f.file);
    assert!(line_of(&text, &f.milk).contains("Buy oat milk"), "{text}");
    assert!(line_of(&text, &eggs).contains("Buy eggs"));
    assert!(text.find("Buy oat milk").unwrap() < text.find("Buy eggs").unwrap());
    assert!(text.find("Later").unwrap() < text.find("Write the mock").unwrap());
    assert!(!text.contains("Draft the spec"));
    assert!(
        r.events.contains(&ReportEvent::Disappeared {
            node: f.draft.clone()
        }),
        "{:?}",
        r.events
    );
    assert_settled(&f);

    // uncomplete, delete, restore
    env.ui("uncomplete", json!({"id": f.draft.as_str()}));
    let del = env.ui("delete", json!({"id": f.bob.as_str()}));
    let r = env.sync(&f.file);
    assert_eq!(synced(&r), (true, 3));
    let text = read(&f.file);
    assert!(line_of(&text, &f.draft).contains("Draft the spec"));
    assert!(!text.contains("Call Bob"));
    assert_settled(&f);
    let _ = del;
    env.ui("restore", json!({"id": f.bob.as_str()}));
    let r = env.sync(&f.file);
    assert_eq!(synced(&r), (true, 4));
    assert!(line_of(&read(&f.file), &f.bob).contains("Call Bob"));
    assert_settled(&f);
}

#[test]
fn concurrent_edits_of_different_nodes_merge() {
    let f = fixture();
    f.env.ui(
        "edit",
        json!({"id": f.milk.as_str(), "text": "Buy oat milk"}),
    );
    edit(&f.file, "Call Bob", "Call Bob back");
    let r = f.env.sync(&f.file);
    assert_eq!(synced(&r), (true, 2));
    let tree = f.env.export();
    assert_eq!(tree.get(&f.bob).unwrap().name, "Call Bob back");
    let text = read(&f.file);
    assert!(text.contains("Buy oat milk") && text.contains("Call Bob back"));
    assert_settled(&f);
}

#[test]
fn conflicting_edit_keeps_remote_and_marks_the_file() {
    let f = fixture();
    f.env.ui(
        "edit",
        json!({"id": f.milk.as_str(), "text": "Buy oat milk"}),
    );
    edit(&f.file, "Buy milk", "Buy soy milk");
    let r = f.env.sync(&f.file);
    assert_eq!(synced(&r), (true, 2));
    assert_eq!(f.env.export().get(&f.milk).unwrap().name, "Buy oat milk");
    let text = read(&f.file);
    assert!(line_of(&text, &f.milk).contains("Buy oat milk"), "{text}");
    assert!(
        text.contains("- ⚠ Buy soy milk %%wf-conflict:name%%"),
        "{text}"
    );
    assert_eq!(r.conflicts.len(), 1);
    assert!(
        r.events
            .iter()
            .any(|e| matches!(e, ReportEvent::Conflict { node, .. } if *node == f.milk))
    );
    assert_settled(&f);
    // Deleting the marker resolves the conflict; nothing is pushed.
    let before = f.env.snapshot();
    edit(&f.file, "\t\t- ⚠ Buy soy milk %%wf-conflict:name%%\n", "");
    f.env.sync(&f.file);
    assert_eq!(f.env.snapshot(), before);
    assert!(!read(&f.file).contains("⚠"));
}

// ---------------------------------------------------------------------------------------
// Brakes, missing files, controls
// ---------------------------------------------------------------------------------------

fn big_fixture() -> (Env, NodeId, std::path::PathBuf) {
    let env = Env::new();
    let root = env.ui_create(None, "Big");
    for i in 0..12 {
        let p = env.ui_create(Some(&root), &format!("item {i}"));
        env.ui_create(Some(&p), &format!("child {i}"));
    }
    let file = env.path("big.md");
    env.render(&root, &file);
    (env, root, file)
}

#[test]
fn brakes_refuse_a_truncated_file_and_force_allows_it() {
    let (env, root, file) = big_fixture();
    let text = read(&file);
    // An editor crash kept the frontmatter and the first two items.
    let truncated: String = text.lines().take(8).map(|l| format!("{l}\n")).collect();
    write(&file, &truncated);
    let before = env.snapshot();
    let api = TestApi::new(&env.client);
    let r = sync::sync_once(&api, &file, &env.cfg).unwrap();
    assert!(
        matches!(r.outcome, SyncOutcome::Braked(Brake::TooManyDeletes { .. })),
        "{r:#?}"
    );
    assert_eq!(api.mutations(), 0);
    assert_eq!(env.snapshot(), before);
    assert_eq!(read(&file), truncated, "a braked cycle writes nothing");

    // --force lets it through.
    let mut forced = env.cfg.clone();
    forced.force = true;
    let r = sync::sync_once(&env.client, &file, &forced).unwrap();
    synced(&r);
    assert_eq!(env.export().children(&root).len(), 2);
}

#[test]
fn frontmatter_force_is_used_once_and_cleared() {
    let (env, root, file) = big_fixture();
    let text = read(&file);
    let mut truncated: String = text.lines().take(8).map(|l| format!("{l}\n")).collect();
    truncated = truncated.replace("syncflowy: {gen: 1}", "syncflowy: {gen: 1, force: true}");
    write(&file, &truncated);
    let r = env.sync(&file);
    assert_eq!(synced(&r), (true, 2));
    assert_eq!(env.export().children(&root).len(), 2);
    let text = read(&file);
    assert!(text.contains("syncflowy: {gen: 2}\n"), "{text}");
    assert!(!text.contains("force"));
}

#[test]
fn emptied_file_is_braked() {
    let (env, _root, file) = big_fixture();
    let text = read(&file);
    let fm_end = text.find("\n---\n").unwrap() + 5;
    write(&file, &text[..fm_end]);
    let r = env.sync(&file);
    assert!(matches!(r.outcome, SyncOutcome::Braked(_)), "{r:#?}");
    // A file with no frontmatter at all is skipped, never "everything deleted".
    write(&file, "");
    let r = env.sync(&file);
    assert!(matches!(r.outcome, SyncOutcome::Skipped { .. }), "{r:#?}");
}

#[test]
fn emptied_file_is_braked_despite_a_concurrent_remote_edit() {
    // A small file (under the fraction brake's minimum) truncated to its frontmatter
    // while a collaborator edits one item, or while it holds a mirror placeholder. The
    // merge keeps that item (delete vs edit, §6.2; placeholder deletes need --force,
    // §10), so it deletes fewer nodes than the base showed, but the file is still empty
    // (§8.3).
    for concurrent in ["edit", "placeholder"] {
        let env = Env::new();
        let root = env.ui_create(None, "Small");
        let alpha = env.ui_create(Some(&root), "Alpha");
        env.ui_create(Some(&root), "Beta");
        env.ui_create(Some(&root), "Gamma");
        if concurrent == "placeholder" {
            env.ui_create(Some(&root), "");
        }
        let file = env.path("small.md");
        env.render(&root, &file);
        if concurrent == "edit" {
            env.ui(
                "edit",
                json!({"id": alpha.as_str(), "text": "Alpha (edited)"}),
            );
        }
        let text = read(&file);
        let fm_end = text.find("\n---\n").unwrap() + 5;
        write(&file, &text[..fm_end]);
        let before = env.snapshot();
        let api = TestApi::new(&env.client);
        let r = sync::sync_once(&api, &file, &env.cfg).unwrap();
        assert!(
            matches!(r.outcome, SyncOutcome::Braked(Brake::EmptyFile { .. })),
            "{concurrent}: {r:#?}"
        );
        assert_eq!(api.mutations(), 0, "{concurrent}");
        assert_eq!(env.snapshot(), before, "{concurrent}");
        assert_eq!(
            read(&file),
            &text[..fm_end],
            "a braked cycle writes nothing"
        );
    }
}

#[test]
fn missing_file_pauses_without_touching_workflowy() {
    let f = fixture();
    std::fs::remove_file(&f.file).unwrap();
    let before = f.env.snapshot();
    let api = TestApi::new(&f.env.client);
    let r = sync::sync_once(&api, &f.file, &f.env.cfg).unwrap();
    assert_eq!(r.outcome, SyncOutcome::Paused(PauseReason::FileMissing));
    assert!(
        api.calls().is_empty(),
        "no API call at all: {:?}",
        api.calls()
    );
    assert_eq!(f.env.snapshot(), before);
    assert!(!f.file.exists(), "a missing file is never re-rendered");
}

#[test]
fn paused_file_is_not_synced() {
    let f = fixture();
    edit(
        &f.file,
        "syncflowy: {gen: 1}",
        "syncflowy: {gen: 1, paused: true}",
    );
    edit(&f.file, "Buy milk", "Buy soy milk");
    let api = TestApi::new(&f.env.client);
    let r = sync::sync_once(&api, &f.file, &f.env.cfg).unwrap();
    assert_eq!(r.outcome, SyncOutcome::Paused(PauseReason::Frontmatter));
    assert!(api.calls().is_empty());
}

#[test]
fn no_checkpoint_errors_and_force_bootstraps() {
    let f = fixture();
    std::fs::remove_dir_all(&f.env.cfg.state_dir).unwrap();
    edit(&f.file, "Buy milk", "Buy soy milk");
    let before = f.env.snapshot();
    let api = TestApi::new(&f.env.client);
    let e = sync::sync_once(&api, &f.file, &f.env.cfg).unwrap_err();
    assert!(matches!(e, Error::NoCheckpoint(_)), "{e}");
    assert_eq!(api.mutations(), 0);
    assert_eq!(f.env.snapshot(), before);

    let mut forced = f.env.cfg.clone();
    forced.force = true;
    let r = sync::sync_once(&f.env.client, &f.file, &forced).unwrap();
    assert!(
        r.warnings.iter().any(|w| w.contains("no checkpoint")),
        "{r:#?}"
    );
    assert_eq!(f.env.export().get(&f.milk).unwrap().name, "Buy soy milk");
    assert_settled(&f);
}

// ---------------------------------------------------------------------------------------
// Render generations (§9.1 guard 1)
// ---------------------------------------------------------------------------------------

#[test]
fn stale_file_is_rebased_not_mass_deleted() {
    let f = fixture();
    let gen1 = read(&f.file);
    // Remote work since generation 1: three new items.
    for name in ["New A", "New B", "New C"] {
        f.env.ui_create(Some(&f.later), name);
    }
    let r = f.env.sync(&f.file);
    assert_eq!(synced(&r), (true, 2));
    assert!(read(&f.file).contains("New C"));
    // A device that was offline uploads its gen-1 file with one edit.
    write(&f.file, &gen1.replace("Call Bob", "Call Bob today"));
    let before = f.env.export();
    let r = f.env.sync(&f.file);
    assert_eq!(synced(&r), (true, 3), "{r:#?}");
    let tree = f.env.export();
    assert_eq!(tree.get(&f.bob).unwrap().name, "Call Bob today");
    assert_eq!(
        child_names(&tree, &f.later),
        ["New A", "New B", "New C"],
        "the items added since gen 1 are not deleted"
    );
    assert_eq!(tree.len(), before.len());
    let text = read(&f.file);
    assert!(text.contains("New A") && text.contains("Call Bob today"));
    assert_settled(&f);
}

#[test]
fn file_without_gen_uses_safe_mode() {
    let f = fixture();
    let text = read(&f.file);
    let bob = line_of(&text, &f.bob).to_owned();
    let t = text
        .replace("syncflowy: {gen: 1}\n", "")
        .replace(&format!("{bob}\n"), "")
        .replace("Buy milk", "Buy soy milk");
    write(&f.file, &t);
    let r = f.env.sync(&f.file);
    assert!(r.warnings.iter().any(|w| w.contains("safe mode")), "{r:#?}");
    let tree = f.env.export();
    assert!(tree.contains(&f.bob), "safe mode never deletes");
    assert_eq!(tree.get(&f.milk).unwrap().name, "Buy soy milk");
    let text = read(&f.file);
    assert!(
        text.contains("Call Bob") && text.contains("gen: 2"),
        "{text}"
    );
    assert_settled(&f);
}

// ---------------------------------------------------------------------------------------
// Compare-and-swap (§9)
// ---------------------------------------------------------------------------------------

#[test]
fn file_changed_during_the_sync_is_not_overwritten() {
    let f = fixture();
    // A remote change, so the file needs a rewrite.
    f.env
        .ui("edit", json!({"id": f.later.as_str(), "text": "Later on"}));
    edit(&f.file, "Buy milk", "Buy soy milk");
    let file = f.file.clone();
    // While the cycle runs (at its first export), the user edits another line.
    let api = TestApi::with_export_hook(&f.env.client, |n| {
        if n == 0 {
            edit(&file, "Call Bob", "Call Bob back");
        }
    });
    let r = sync::sync_once(&api, &f.file, &f.env.cfg).unwrap();
    assert_eq!(synced(&r), (true, 2), "{r:#?}");
    let tree = f.env.export();
    assert_eq!(tree.get(&f.milk).unwrap().name, "Buy soy milk");
    assert_eq!(tree.get(&f.bob).unwrap().name, "Call Bob back");
    let text = read(&f.file);
    assert!(
        text.contains("Call Bob back"),
        "the concurrent edit survives:\n{text}"
    );
    assert!(text.contains("Later on") && text.contains("Buy soy milk"));
    assert_settled(&f);
}

#[test]
fn file_changed_during_the_sync_keeps_new_items_single() {
    let f = fixture();
    let text = read(&f.file);
    let later = line_of(&text, &f.later).to_owned();
    edit(
        &f.file,
        &format!("{later}\n"),
        &format!("{later}\n\t- Brand new\n"),
    );
    let file = f.file.clone();
    // The concurrent edit lands after the apply (at the re-export).
    let api = TestApi::with_export_hook(&f.env.client, |n| {
        if n == 1 {
            edit(&file, "Buy milk", "Buy soy milk");
        }
    });
    let r = sync::sync_once(&api, &f.file, &f.env.cfg).unwrap();
    synced(&r);
    let tree = f.env.export();
    assert_eq!(child_names(&tree, &f.later), ["Brand new"], "created once");
    assert_eq!(tree.get(&f.milk).unwrap().name, "Buy soy milk");
    let text = read(&f.file);
    assert!(
        text.contains("Brand new ^wf-") && text.contains("Buy soy milk"),
        "{text}"
    );
    assert_settled(&f);
}

#[test]
fn file_that_keeps_changing_is_skipped() {
    let f = fixture();
    f.env
        .ui("edit", json!({"id": f.later.as_str(), "text": "Later on"}));
    let file = f.file.clone();
    let mut k = 0;
    let api = TestApi::with_export_hook(&f.env.client, |_| {
        k += 1;
        edit(
            &file,
            &format!("Call Bob{}", "!".repeat(k - 1)),
            &format!("Call Bob{}", "!".repeat(k)),
        );
    });
    let mut cfg = f.env.cfg.clone();
    cfg.cas_retries = 2;
    let r = sync::sync_once(&api, &f.file, &cfg).unwrap();
    assert!(matches!(r.outcome, SyncOutcome::Skipped { .. }), "{r:#?}");
    // The user's text is intact; the next run finishes.
    assert!(read(&f.file).contains("Call Bob!"));
    let r = f.env.sync(&f.file);
    synced(&r);
    assert!(read(&f.file).contains("Later on"));
    assert_settled(&f);
}

// ---------------------------------------------------------------------------------------
// Crash recovery (§7.3)
// ---------------------------------------------------------------------------------------

/// Crash (panic) in the middle of an apply, before or after mutation `at` (0-based).
struct CrashApi<'a> {
    inner: &'a syncflowy::wfclient::Client,
    n: std::sync::atomic::AtomicUsize,
    at: usize,
    after: bool,
}

type ApiResult<T> = Result<T, syncflowy::wfclient::ApiError>;

impl CrashApi<'_> {
    fn mutate<T>(&self, f: impl FnOnce() -> ApiResult<T>) -> ApiResult<T> {
        let n = self.n.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if !self.after && n == self.at {
            panic!("injected crash before mutation {n}");
        }
        let r = f();
        if self.after && n == self.at {
            panic!("injected crash after mutation {n}");
        }
        r
    }
}

impl Api for CrashApi<'_> {
    fn export(&self) -> ApiResult<Vec<syncflowy::tree::Node>> {
        self.inner.export()
    }
    fn get(&self, id: &NodeId) -> ApiResult<syncflowy::tree::Node> {
        self.inner.get(id)
    }
    fn get_short(&self, id: &syncflowy::tree::ShortId) -> ApiResult<syncflowy::tree::Node> {
        self.inner.get_short(id)
    }
    fn list(&self, parent: Option<&NodeId>) -> ApiResult<Vec<syncflowy::tree::Node>> {
        self.inner.list(parent)
    }
    fn create(&self, req: &syncflowy::wfclient::CreateRequest) -> ApiResult<NodeId> {
        self.mutate(|| self.inner.create(req))
    }
    fn update(&self, id: &NodeId, req: &syncflowy::wfclient::UpdateRequest) -> ApiResult<()> {
        self.mutate(|| self.inner.update(id, req))
    }
    fn move_node(
        &self,
        id: &NodeId,
        parent: &NodeId,
        position: syncflowy::tree::Position,
    ) -> ApiResult<()> {
        self.mutate(|| self.inner.move_node(id, parent, position))
    }
    fn complete(&self, id: &NodeId) -> ApiResult<()> {
        self.mutate(|| self.inner.complete(id))
    }
    fn uncomplete(&self, id: &NodeId) -> ApiResult<()> {
        self.mutate(|| self.inner.uncomplete(id))
    }
    fn delete(&self, id: &NodeId) -> ApiResult<()> {
        self.mutate(|| self.inner.delete(id))
    }
    fn request_count(&self) -> u64 {
        self.inner.request_count()
    }
}

fn crash_during_sync(f: &Fixture, at: usize, after: bool) {
    let api = CrashApi {
        inner: &f.env.client,
        n: std::sync::atomic::AtomicUsize::new(0),
        at,
        after,
    };
    let r = catch_unwind(AssertUnwindSafe(|| {
        sync::sync_once(&api, &f.file, &f.env.cfg)
    }));
    assert!(r.is_err(), "the injected crash happened");
}

#[test]
fn crash_recovery_through_the_journal() {
    for (at, after) in [(0, true), (1, false), (1, true), (2, true)] {
        let f = fixture();
        let text = read(&f.file);
        let later = line_of(&text, &f.later).to_owned();
        // Two new items (the second needs the two-step create) and a rename.
        edit(
            &f.file,
            &format!("{later}\n"),
            &format!("{later}\n\t- First new\n\t- *Second* new\n"),
        );
        edit(&f.file, "Call Bob", "Call Bob back");
        let edited = read(&f.file);
        crash_during_sync(&f, at, after);
        assert_eq!(
            read(&f.file),
            edited,
            "a crashed cycle never wrote the file"
        );
        let store = StateStore::open(&f.env.cfg.state_dir).unwrap();
        assert!(
            store.journal_path(&f.root).exists(),
            "the journal survives the crash"
        );

        let r = f.env.sync(&f.file);
        synced(&r);
        assert!(r.recovered.is_some(), "{r:#?}");
        let tree = f.env.export();
        assert_eq!(
            child_names(&tree, &f.later),
            ["First new", "<i>Second</i> new"],
            "crash at {at} (after: {after}): no duplicate, no nonce left"
        );
        assert_eq!(tree.get(&f.bob).unwrap().name, "Call Bob back");
        assert!(
            !store.journal_path(&f.root).exists(),
            "the journal is committed"
        );
        let text = read(&f.file);
        assert!(
            text.contains("First new ^wf-") && text.contains("*Second* new ^wf-"),
            "{text}"
        );
        assert_settled(&f);
    }
}

/// The creates that keep a tag: a resurrection (Later deleted in Workflowy, edited
/// here), a copied line that lost its duplicated tag (Bob's, pasted above it, so a
/// reorder follows the creates), plus an untagged new line.
fn edit_tagged_creates(f: &Fixture) {
    f.env.ui("delete", json!({"id": f.later.as_str()}));
    let text = read(&f.file);
    let bob = line_of(&text, &f.bob).to_owned();
    let copy = bob.replace("Call Bob", "Call Bob again");
    edit(
        &f.file,
        &format!("{bob}\n"),
        &format!("{copy}\n{bob}\n\t- Untagged new\n"),
    );
    edit(&f.file, "Later", "Later, edited");
}

/// Each of the creates of [`edit_tagged_creates`] exists exactly once.
fn assert_tagged_creates_single(f: &Fixture, what: &str) {
    let tree = f.env.export();
    assert_eq!(
        child_names(&tree, &f.errands),
        ["Buy milk", "Call Bob again", "Call Bob", "Untagged new"],
        "{what}"
    );
    assert_eq!(
        child_names(&tree, &f.root),
        ["Plan", "Errands", "Later, edited"],
        "{what}"
    );
    let text = read(&f.file);
    assert_eq!(text.matches("Later, edited").count(), 1, "{what}:\n{text}");
    assert_eq!(text.matches("Call Bob again").count(), 1, "{what}:\n{text}");
}

#[test]
fn cas_rerun_does_not_recreate_resurrections_or_copied_lines() {
    let f = fixture();
    edit_tagged_creates(&f);
    let file = f.file.clone();
    // After the apply (at the re-export), the user edits another line: the file
    // changed, so the merge runs again with this cycle's creates as adoptions (§9).
    let api = TestApi::with_export_hook(&f.env.client, |n| {
        if n == 1 {
            edit(&file, "Buy milk", "Buy soy milk");
        }
    });
    let r = sync::sync_once(&api, &f.file, &f.env.cfg).unwrap();
    synced(&r);
    assert_eq!(
        f.env.export().get(&f.milk).unwrap().name,
        "Buy soy milk",
        "{r:#?}"
    );
    let tree = f.env.export();
    assert_eq!(
        child_names(&tree, &f.errands),
        ["Buy soy milk", "Call Bob again", "Call Bob", "Untagged new"],
        "{r:#?}"
    );
    assert_eq!(
        child_names(&tree, &f.root),
        ["Plan", "Errands", "Later, edited"],
        "{r:#?}"
    );
    assert_settled(&f);
}

#[test]
fn crash_recovery_does_not_recreate_resurrections_or_copied_lines() {
    // Three creates (one request each), then the reorder: crash after each create
    // (after the last one, before the reorder), and before the second.
    for (at, after) in [(0, true), (1, false), (1, true), (2, true), (3, true)] {
        let f = fixture();
        edit_tagged_creates(&f);
        crash_during_sync(&f, at, after);
        let r = f.env.sync(&f.file);
        synced(&r);
        assert!(r.recovered.is_some(), "{r:#?}");
        assert_tagged_creates_single(&f, &format!("crash at {at} (after: {after})"));
        assert_settled(&f);

/// A dry run writes nothing anywhere, not even recovery's writes: with a journal left by
/// a crash in the middle of an apply (two-step creates included), it sends no request
/// that mutates, leaves the journal byte-for-byte, warns that a journal is pending, and
/// the next real run still recovers it.
#[test]
fn dry_run_does_not_recover_the_journal() {
    for (at, after) in [(0, true), (1, false), (1, true), (2, true)] {
        let f = fixture();
        let text = read(&f.file);
        let later = line_of(&text, &f.later).to_owned();
        edit(
            &f.file,
            &format!("{later}\n"),
            &format!("{later}\n\t- First new\n\t- *Second* new\n"),
        );
        edit(&f.file, "Call Bob", "Call Bob back");
        let edited = read(&f.file);
        crash_during_sync(&f, at, after);
        let store = StateStore::open(&f.env.cfg.state_dir).unwrap();
        let journal = store.journal_path(&f.root);
        let journal_text = std::fs::read(&journal).expect("the journal survives the crash");
        let before = f.env.snapshot();

        let api = TestApi::new(&f.env.client);
        let mut cfg = f.env.cfg.clone();
        cfg.dry_run = true;
        let r = sync::sync_once(&api, &f.file, &cfg).unwrap();
        assert!(matches!(r.outcome, SyncOutcome::DryRun { .. }), "{r:#?}");
        assert_eq!(api.mutations(), 0, "crash at {at}: {:?}", api.calls());
        assert_eq!(f.env.snapshot(), before);
        assert_eq!(std::fs::read(&journal).unwrap(), journal_text);
        assert_eq!(read(&f.file), edited);
        assert!(r.recovered.is_none(), "{r:#?}");
        assert!(
            r.warnings.iter().any(|w| w.contains("journal")),
            "{:?}",
            r.warnings
        );

        let r = f.env.sync(&f.file);
        synced(&r);
        assert!(r.recovered.is_some(), "{r:#?}");
        assert_eq!(
            child_names(&f.env.export(), &f.later),
            ["First new", "<i>Second</i> new"],
            "crash at {at} (after: {after})"
        );
    }
}

// ---------------------------------------------------------------------------------------
// Symbolic links and file modes
// ---------------------------------------------------------------------------------------

/// A synced file that is a symbolic link stays one: the sync (compare-and-swap write)
/// and a forced render rewrite the file it names, which keeps its mode, instead of
/// replacing the link with a regular file that silently diverges from the real one.
#[cfg(unix)]
#[test]
fn symlinked_file_stays_a_link_and_keeps_its_mode() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let mode = |p: &std::path::Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o7777;
    let f = fixture();
    let elsewhere = f.env.dir.path().join("elsewhere");
    std::fs::create_dir_all(&elsewhere).unwrap();
    let real = elsewhere.join("project.md");
    std::fs::rename(&f.file, &real).unwrap();
    symlink(&real, &f.file).unwrap();
    std::fs::set_permissions(&real, std::fs::Permissions::from_mode(0o600)).unwrap();
    let is_link = || {
        std::fs::symlink_metadata(&f.file)
            .unwrap()
            .file_type()
            .is_symlink()
    };
    let no_temps = |dir: &std::path::Path| {
        std::fs::read_dir(dir)
            .unwrap()
            .flatten()
            .all(|e| !e.file_name().to_string_lossy().contains(".tmp-"))
    };

    // A remote edit re-renders the file; a local edit through the link is pushed.
    f.env.ui(
        "edit",
        json!({"id": f.milk.as_str(), "text": "Buy oat milk"}),
    );
    edit(&f.file, "Call Bob", "Call Bob back");
    let r = f.env.sync(&f.file);
    assert_eq!(synced(&r), (true, 2), "{r:#?}");
    assert!(is_link(), "the link was replaced by a regular file");
    let text = read(&real);
    assert!(line_of(&text, &f.milk).contains("Buy oat milk"), "{text}");
    assert!(text.contains("gen: 2"), "{text}");
    assert_eq!(f.env.export().get(&f.bob).unwrap().name, "Call Bob back");
    assert_eq!(mode(&real), 0o600);
    assert!(no_temps(&elsewhere) && no_temps(f.file.parent().unwrap()));
    assert_settled(&f);

    // A forced render over the link writes through it too.
    let mut forced = f.env.cfg.clone();
    forced.force = true;
    let short = f.root.short().unwrap().to_string();
    let r = sync::render_once(
        &f.env.client,
        &short,
        &f.file,
        &DocConfig::default(),
        &forced,
    )
    .unwrap();
    assert!(matches!(r.outcome, SyncOutcome::Rendered { .. }), "{r:#?}");
    assert!(is_link());
    assert!(read(&real).contains("Buy oat milk"));
    assert_eq!(mode(&real), 0o600);
    assert!(no_temps(&elsewhere) && no_temps(f.file.parent().unwrap()));
}

// ---------------------------------------------------------------------------------------
// status and dry runs
// ---------------------------------------------------------------------------------------

#[test]
fn status_reports_local_state() {
    let f = fixture();
    let s = sync::status(&f.env.cfg, &[]).unwrap();
    assert_eq!(s.len(), 1);
    assert_eq!(s[0].root, f.root);
    assert_eq!(s[0].generation, 1);
    assert_eq!(s[0].locally_modified, Some(false));
    assert!(!s[0].journal_pending);
    edit(&f.file, "Buy milk", "Buy soy milk");
    let s = sync::status(&f.env.cfg, std::slice::from_ref(&f.file)).unwrap();
    assert_eq!(s[0].locally_modified, Some(true));
}

#[test]
fn dry_run_writes_nothing() {
    let f = fixture();
    edit(&f.file, "Buy milk", "Buy soy milk");
    let text = read(&f.file);
    let before = f.env.snapshot();
    let mut cfg = f.env.cfg.clone();
    cfg.dry_run = true;
    let r = sync::sync_once(&f.env.client, &f.file, &cfg).unwrap();
    let SyncOutcome::DryRun { plan } = r.outcome else {
        panic!("{r:#?}")
    };
    assert_eq!(plan.ops.len(), 1);
    assert_eq!(f.env.snapshot(), before);
    assert_eq!(read(&f.file), text);
}
