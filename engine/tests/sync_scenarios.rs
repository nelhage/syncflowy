//! Harder `sync` scenarios against the mock: nested creates with names that need the
//! two-step create, resurrection, lost tags, root notes, code blocks, remote moves out of
//! the root, deferred deletes (daemon mode), and the rerun after a CAS conflict.

mod sync_support;

use std::time::Duration;

use serde_json::json;
use syncflowy::mdblock::DocConfig;
use syncflowy::merge::ReportEvent;
use syncflowy::plan::Op;
use syncflowy::state::StateStore;
use syncflowy::sync::{self, SyncOutcome};
use syncflowy::tree::NodeId;

use sync_support::*;

fn setup() -> (Env, NodeId, NodeId, NodeId, std::path::PathBuf) {
    let env = Env::new();
    let root = env.ui_create(None, "Root");
    let a = env.ui_create(Some(&root), "Alpha");
    let b = env.ui_create(Some(&root), "Beta");
    let file = env.path("root.md");
    env.render(&root, &file);
    (env, root, a, b, file)
}

fn settle(env: &Env, file: &std::path::Path) {
    let before = env.snapshot();
    let text = read(file);
    let r = env.sync(file);
    assert_eq!(r.outcome, SyncOutcome::Unchanged, "{r:#?}\n{text}");
    assert_eq!(env.snapshot(), before);
    assert_eq!(read(file), text);
}

fn no_nonces(env: &Env) {
    let tree = env.export();
    for n in tree.iter() {
        assert!(!n.name.contains("syncflowy:"), "nonce left: {n:?}");
    }
}

#[test]
fn nested_creates_with_special_names() {
    let (env, root, a, _b, file) = setup();
    let text = read(&file);
    let al = line_of(&text, &a).to_owned();
    edit(
        &file,
        &format!("{al}\n"),
        &format!(
            "{al}\n\t- Parent & *child* [link](https://example.com)\n\t\t- #tag  item `code`\n\t\t\t- deep ~~strike~~\n- New top with note\n\tnote line 1\n\tnote line 2\n"
        ),
    );
    let r = env.sync(&file);
    assert!(matches!(r.outcome, SyncOutcome::Synced { .. }), "{r:#?}");
    assert!(r.discrepancies.is_empty(), "{:?}", r.discrepancies);
    let tree = env.export();
    let p = tree.children(&a)[0].clone();
    assert_eq!(
        tree.get(&p).unwrap().name,
        "Parent &amp; <i>child</i> <a href=\"https://example.com\">link</a>"
    );
    let c = tree.children(&p)[0].clone();
    assert_eq!(tree.children(&c).len(), 1);
    let top = by_name(&tree, "New top with note");
    assert_eq!(tree.parent(&top.id), Some(&root));
    assert_eq!(top.note.as_deref(), Some("note line 1\nnote line 2"));
    no_nonces(&env);
    settle(&env, &file);
}

#[test]
fn remote_delete_of_a_locally_edited_node_resurrects_it() {
    let (env, root, a, _b, file) = setup();
    env.ui("delete", json!({"id": a.as_str()}));
    edit(&file, "Alpha", "Alpha, edited");
    let r = env.sync(&file);
    assert!(
        r.events
            .iter()
            .any(|e| matches!(e, ReportEvent::Resurrected { old, .. } if *old == a)),
        "{r:#?}"
    );
    let tree = env.export();
    assert_eq!(child_names(&tree, &root), ["Alpha, edited", "Beta"]);
    assert!(!tree.contains(&a));
    // Named by the base name and the new node's own tag, never by a temp id.
    let new = by_name(&tree, "Alpha, edited").id.clone();
    let line = format!(
        "\"Alpha\" ({}) was deleted in Workflowy but edited in Markdown: re-created as \
         \"Alpha, edited\" ({}, new) (the original is in Workflowy's Trash)",
        short(&a),
        short(&new)
    );
    assert!(r.messages.contains(&line), "{r:#?}");
    let text = read(&file);
    assert!(!text.contains(&short(&a).to_string()), "{text}");
    settle(&env, &file);
}

#[test]
fn lost_tag_is_rematched() {
    let (env, root, a, _b, file) = setup();
    let tag = format!(" ^wf-{}", short(&a));
    edit(&file, &tag, "");
    edit(&file, "Alpha", "Alpha!");
    let r = env.sync(&file);
    assert!(
        r.events
            .iter()
            .any(|e| matches!(e, ReportEvent::Rematched { node, .. } if *node == a)),
        "{r:#?}"
    );
    let tree = env.export();
    assert_eq!(child_names(&tree, &root), ["Alpha!", "Beta"]);
    assert!(read(&file).contains(&tag));
    settle(&env, &file);
}

#[test]
fn root_note_and_code_block_edits() {
    let (env, root, a, _b, file) = setup();
    let text = read(&file);
    edit(
        &file,
        "---\n- [ ] Alpha",
        "---\nA root note.\n\n- [ ] Alpha",
    );
    let al = line_of(&text, &a).to_owned();
    edit(
        &file,
        &format!("{al}\n"),
        &format!("{al}\n\t- ```\n\t\tfn main() {{}}\n\t\t```\n\t- > a quote\n"),
    );
    let r = env.sync(&file);
    assert!(matches!(r.outcome, SyncOutcome::Synced { .. }), "{r:#?}");
    let tree = env.export();
    assert_eq!(
        tree.get(&root).unwrap().note.as_deref(),
        Some("A root note.")
    );
    let kids: Vec<_> = tree
        .children(&a)
        .iter()
        .map(|c| tree.get(c).unwrap().clone())
        .collect();
    assert_eq!(kids[0].layout, "code-block");
    assert_eq!(kids[0].name, "fn main() {}");
    assert_eq!(kids[1].layout, "quote-block");
    assert_eq!(kids[1].name, "a quote");
    settle(&env, &file);
    // Edit the code.
    edit(&file, "fn main() {}", "fn main() { run(); }");
    env.sync(&file);
    assert_eq!(
        env.export().get(&kids[0].id).unwrap().name,
        "fn main() { run(); }"
    );
    settle(&env, &file);
}

#[test]
fn unfencing_or_forcing_a_multi_line_value_loses_nothing() {
    let env = Env::new();
    let root = env.ui_create(None, "Root");
    let code = env.ui_create_with(
        Some(&root),
        json!({"text": "line one\nline two", "layoutMode": "code-block"}),
    );
    let two = env.ui_create(Some(&root), "Two-line\nname");
    let file = env.path("root.md");
    env.render(&root, &file);
    let lossy = |r: &sync::SyncReport| {
        r.events
            .iter()
            .any(|e| matches!(e, ReportEvent::LossyWrite { .. }))
    };

    // The user saw both lines of the code block when they unfenced it.
    let cs = short(&code);
    edit(
        &file,
        &format!("- ``` ^wf-{cs}\n\tline one\n\tline two\n\t```\n"),
        &format!("- A1 unfenced ^wf-{cs}\n"),
    );
    let r = env.sync(&file);
    assert!(matches!(r.outcome, SyncOutcome::Synced { .. }), "{r:#?}");
    assert!(!lossy(&r), "{r:#?}");
    assert_eq!(env.export().get(&code).unwrap().name, "A1 unfenced");

    // A forced edit of a read-only name: the user saw its 🔒.
    let mut cfg = env.cfg.clone();
    cfg.force = true;
    edit(&file, "🔒 Two-line ⏎ name", "Two-line name forced");
    let r = sync::sync_once(&env.client, &file, &cfg).unwrap();
    assert!(matches!(r.outcome, SyncOutcome::Synced { .. }), "{r:#?}");
    assert!(!lossy(&r), "{r:#?}");
    assert_eq!(env.export().get(&two).unwrap().name, "Two-line name forced");
    settle(&env, &file);
}

#[test]
fn remote_move_out_of_the_root_leaves_the_file() {
    let (env, _root, a, _b, file) = setup();
    let elsewhere = env.ui_create(None, "Elsewhere");
    env.ui(
        "move",
        json!({"id": a.as_str(), "parent_id": elsewhere.as_str()}),
    );
    edit(&file, "Alpha", "Alpha edited here");
    let r = env.sync(&file);
    assert!(
        r.events
            .iter()
            .any(|e| matches!(e, ReportEvent::MovedOut { node, .. } if *node == a)),
        "{r:#?}"
    );
    assert!(!read(&file).contains("Alpha"));
    let tree = env.export();
    assert_eq!(tree.parent(&a), Some(&elsewhere));
    // §6.1/§6.2: the node exists, so the text typed on its line follows it.
    assert_eq!(tree.get(&a).unwrap().name, "Alpha edited here");
    settle(&env, &file);
}

#[test]
fn tick_of_a_node_moved_out_of_the_root_completes_it() {
    let (env, _root, a, b, file) = setup();
    let elsewhere = env.ui_create(None, "Elsewhere");
    let child = env.ui_create(Some(&b), "Beta child");
    env.sync(&file);
    env.ui(
        "move",
        json!({"id": b.as_str(), "parent_id": elsewhere.as_str()}),
    );
    edit(&file, "- [ ] Beta child", "- [x] Beta child");
    edit(&file, "Alpha", "Alpha too");
    let r = env.sync(&file);
    assert!(matches!(r.outcome, SyncOutcome::Synced { .. }), "{r:#?}");
    let tree = env.export();
    assert_eq!(tree.parent(&child), Some(&b));
    assert!(tree.get(&child).unwrap().completed, "{r:#?}");
    assert!(!tree.get(&b).unwrap().completed);
    assert_eq!(tree.get(&a).unwrap().name, "Alpha too");
    assert!(!read(&file).contains("Beta"));
    settle(&env, &file);
}

#[test]
fn conflict_on_a_node_moved_out_of_the_root_is_reported() {
    let (env, _root, a, _b, file) = setup();
    let elsewhere = env.ui_create(None, "Elsewhere");
    env.ui(
        "move",
        json!({"id": a.as_str(), "parent_id": elsewhere.as_str()}),
    );
    env.ui("edit", json!({"id": a.as_str(), "text": "Alpha (remote)"}));
    edit(&file, "Alpha", "Alpha (local)");
    let r = env.sync(&file);
    // Remote wins in Workflowy; the line left the file, so the report keeps the text.
    assert_eq!(env.export().get(&a).unwrap().name, "Alpha (remote)");
    assert!(
        r.events.iter().any(|e| matches!(
            e,
            ReportEvent::MovedOutConflict { node, local_md, .. }
                if *node == a && local_md == "Alpha (local)"
        )),
        "{r:#?}"
    );
    assert!(
        r.messages.iter().any(|m| m.contains("Alpha (local)")),
        "{r:#?}"
    );
    settle(&env, &file);
}

#[test]
fn ticking_a_node_renamed_remotely_reports_the_dropped_rename() {
    let (env, _root, a, _b, file) = setup();
    env.ui(
        "edit",
        json!({"id": a.as_str(), "text": "Alpha, renamed by them"}),
    );
    edit(&file, "- [ ] Alpha", "- [x] Alpha, my rename");
    let r = env.sync(&file);
    // §6.2: the completion is pushed and the remote name kept.
    let tree = env.export();
    let node = tree.get(&a).unwrap();
    assert!(node.completed);
    assert_eq!(node.name, "Alpha, renamed by them");
    assert!(
        r.events.iter().any(|e| matches!(
            e,
            ReportEvent::TickedConflict { node, local_md, .. }
                if *node == a && local_md == "Alpha, my rename"
        )),
        "{r:#?}"
    );
    assert!(
        r.messages
            .iter()
            .any(|m| m.contains("which you ticked") && m.contains("\"Alpha, my rename\"")),
        "{r:#?}"
    );
    settle(&env, &file);
}

#[test]
fn deleting_a_parent_takes_its_hidden_children() {
    let (env, root, a, _b, file) = setup();
    let done = env.ui_create(Some(&a), "done child");
    env.ui("complete", json!({"id": done.as_str()}));
    env.sync(&file);
    let text = read(&file);
    let al = line_of(&text, &a).to_owned();
    edit(&file, &format!("{al}\n"), "");
    let r = env.sync(&file);
    let SyncOutcome::Synced { pushed, .. } = r.outcome else {
        panic!("{r:#?}")
    };
    assert_eq!((pushed.deletes, pushed.hidden_deleted), (1, 1));
    let named = format!(
        "deleting \"Alpha\" ({}) also deletes 1 hidden item: \"done child\"",
        short(&a)
    );
    assert!(r.messages.contains(&named), "{r:#?}");
    let tree = env.export();
    assert!(!tree.contains(&a) && !tree.contains(&done));
    assert_eq!(child_names(&tree, &root), ["Beta"]);
    settle(&env, &file);
}

#[test]
fn deleting_a_line_counts_descendants_hidden_by_depth() {
    let env = Env::new();
    let root = env.ui_create(None, "Root");
    let parent = env.ui_create(Some(&root), "Parent");
    let child = env.ui_create(Some(&parent), "Child");
    let grandchild = env.ui_create(Some(&child), "Grandchild");
    env.ui_create(Some(&root), "Other");
    let file = env.path("root.md");
    let doc = DocConfig {
        max_depth: Some(1),
        ..DocConfig::default()
    };
    env.render_with(&root, &file, &doc);
    let text = read(&file);
    assert!(!text.contains("Child"), "{text}");
    let pl = line_of(&text, &parent).to_owned();
    edit(&file, &format!("{pl}\n"), "");

    let mut cfg = env.cfg.clone();
    cfg.dry_run = true;
    let r = sync::sync_once(&env.client, &file, &cfg).unwrap();
    let SyncOutcome::DryRun { plan } = &r.outcome else {
        panic!("{r:#?}")
    };
    assert!(
        plan.ops.iter().any(|op| matches!(
            op,
            Op::Delete { node, hidden_descendants: 2, mirror: false } if *node == parent
        )),
        "{plan:#?}"
    );

    let r = env.sync(&file);
    let SyncOutcome::Synced { pushed, .. } = r.outcome else {
        panic!("{r:#?}")
    };
    assert_eq!((pushed.deletes, pushed.hidden_deleted), (1, 2));
    let tree = env.export();
    assert!(!tree.contains(&parent) && !tree.contains(&child) && !tree.contains(&grandchild));
    settle(&env, &file);
}

#[test]
fn a_dry_run_names_the_hidden_items_a_delete_takes() {
    let (env, _root, a, _b, file) = setup();
    for i in 0..12 {
        let h = env.ui_create(Some(&a), &format!("h{i}"));
        env.ui("complete", json!({"id": h.as_str()}));
    }
    env.sync(&file);
    let text = read(&file);
    let al = line_of(&text, &a).to_owned();
    edit(&file, &format!("{al}\n"), "");
    let mut cfg = env.cfg.clone();
    cfg.force = true; // 13 deletes out of 2 visible items engage the brake
    cfg.dry_run = true;
    let r = sync::sync_once(&env.client, &file, &cfg).unwrap();
    assert!(matches!(r.outcome, SyncOutcome::DryRun { .. }), "{r:#?}");
    let names: Vec<String> = (0..10).map(|i| format!("\"h{i}\"")).collect();
    let named = format!(
        "deleting \"Alpha\" ({}) also deletes 12 hidden items: {}, and 2 more",
        short(&a),
        names.join(", ")
    );
    assert!(r.messages.contains(&named), "{r:#?}");
}

#[test]
fn deferred_deletes_wait_for_the_grace_period() {
    let (env, root, a, _b, file) = setup();
    let mut cfg = env.cfg.clone();
    cfg.delete_grace = Some(Duration::from_secs(0));
    let text = read(&file);
    let al = line_of(&text, &a).to_owned();
    edit(&file, &format!("{al}\n"), "");
    // First cycle: pending, hidden from the file, not sent.
    let r = sync::sync_once(&env.client, &file, &cfg).unwrap();
    assert!(matches!(r.outcome, SyncOutcome::Synced { .. }), "{r:#?}");
    assert!(env.export().contains(&a));
    assert!(!read(&file).contains("Alpha"));
    let st = sync::status(&cfg).unwrap();
    assert_eq!(st[0].pending_deletes.len(), 1);
    // Next cycle with the file unchanged: due, sent.
    sync::sync_once(&env.client, &file, &cfg).unwrap();
    assert!(!env.export().contains(&a));
    assert_eq!(child_names(&env.export(), &root), ["Beta"]);
    let st = sync::status(&cfg).unwrap();
    assert!(st[0].pending_deletes.is_empty());
    let r = sync::sync_once(&env.client, &file, &cfg).unwrap();
    assert_eq!(r.outcome, SyncOutcome::Unchanged);
}

#[test]
fn deferred_delete_is_cancelled_when_the_line_comes_back() {
    let (env, _root, a, _b, file) = setup();
    let mut cfg = env.cfg.clone();
    cfg.delete_grace = Some(Duration::from_secs(3600));
    let original = read(&file);
    let al = line_of(&original, &a).to_owned();
    edit(&file, &format!("{al}\n"), "");
    sync::sync_once(&env.client, &file, &cfg).unwrap();
    assert!(env.export().contains(&a));
    // The line comes back (an undo on a client); the gen it names is now old.
    write(&file, &original);
    let r = sync::sync_once(&env.client, &file, &cfg).unwrap();
    assert!(matches!(r.outcome, SyncOutcome::Synced { .. }), "{r:#?}");
    assert!(read(&file).contains("Alpha"));
    assert!(sync::status(&cfg).unwrap()[0].pending_deletes.is_empty());
    assert!(env.export().contains(&a));
}

#[test]
fn cas_rerun_after_a_push_does_not_conflict_with_itself() {
    let (env, _root, a, b, file) = setup();
    env.ui("edit", json!({"id": b.as_str(), "text": "Beta (remote)"}));
    edit(&file, "Alpha", "Alpha 1");
    let f2 = file.clone();
    // After our push of "Alpha 1" (at the re-export), the user types more on that line.
    let api = TestApi::with_export_hook(&env.client, |n| {
        if n == 1 {
            edit(&f2, "Alpha 1", "Alpha 12");
        }
    });
    let r = sync::sync_once(&api, &file, &env.cfg).unwrap();
    assert!(matches!(r.outcome, SyncOutcome::Synced { .. }), "{r:#?}");
    assert!(r.conflicts.is_empty(), "{r:#?}");
    let tree = env.export();
    assert_eq!(tree.get(&a).unwrap().name, "Alpha 12");
    let text = read(&file);
    assert!(!text.contains('⚠'), "{text}");
    assert!(text.contains("Alpha 12") && text.contains("Beta (remote)"));
    settle(&env, &file);
}

// ---------------------------------------------------------------------------------------
// Ops skipped by the applier (a collaborator raced the pre-write check, §7.1, §9)
// ---------------------------------------------------------------------------------------

#[test]
fn local_edit_skipped_by_the_pre_write_check_becomes_a_conflict() {
    let (env, _root, a, _b, file) = setup();
    edit(&file, "Alpha", "Alpha LOCAL");
    // A collaborator renames Alpha after our export, just before the pre-write GET.
    let mut fired = false;
    let api = TestApi::with_call_hook(&env.client, |kind| {
        if kind == "get" && !fired {
            fired = true;
            env.ui("edit", json!({"id": a.as_str(), "text": "Alpha REMOTE"}));
        }
    });
    let r = sync::sync_once(&api, &file, &env.cfg).unwrap();
    drop(api);
    assert!(fired);
    assert!(matches!(r.outcome, SyncOutcome::Synced { .. }), "{r:#?}");
    // §6.1: remote wins in Workflowy, the local version survives in the Markdown.
    assert_eq!(env.export().get(&a).unwrap().name, "Alpha REMOTE");
    let text = read(&file);
    assert!(line_of(&text, &a).contains("Alpha REMOTE"), "{text}");
    assert!(text.contains("⚠ Alpha LOCAL"), "{text}");
    assert_eq!(r.conflicts.len(), 1, "{r:#?}");
    settle(&env, &file);
}

#[test]
fn local_note_edit_skipped_by_the_pre_write_check_becomes_a_conflict() {
    let env = Env::new();
    let root = env.ui_create(None, "Root");
    let a = env.ui_create_with(
        Some(&root),
        json!({"text": "Alpha", "note_text": "old note"}),
    );
    let file = env.path("root.md");
    env.render(&root, &file);
    edit(&file, "old note", "local note");
    let mut fired = false;
    let api = TestApi::with_call_hook(&env.client, |kind| {
        if kind == "get" && !fired {
            fired = true;
            env.ui(
                "edit",
                json!({"id": a.as_str(), "note_text": "remote note"}),
            );
        }
    });
    let r = sync::sync_once(&api, &file, &env.cfg).unwrap();
    drop(api);
    assert!(fired);
    assert!(matches!(r.outcome, SyncOutcome::Synced { .. }), "{r:#?}");
    assert_eq!(
        env.export().get(&a).unwrap().note.as_deref(),
        Some("remote note")
    );
    let text = read(&file);
    assert!(text.contains("remote note"), "{text}");
    assert!(text.contains("local note"), "{text}");
    assert_eq!(r.conflicts.len(), 1, "{r:#?}");
    settle(&env, &file);
}

#[test]
fn new_item_whose_parent_vanished_before_the_create_is_resurrected() {
    let (env, root, a, _b, file) = setup();
    let text = read(&file);
    let al = line_of(&text, &a).to_owned();
    edit(
        &file,
        &format!("{al}\n"),
        &format!("{al}\n\t- My important new child\n"),
    );
    // A collaborator deletes Alpha after our export, just before the create.
    let mut fired = false;
    let api = TestApi::with_call_hook(&env.client, |kind| {
        if kind == "create" && !fired {
            fired = true;
            env.ui("delete", json!({"id": a.as_str()}));
        }
    });
    let r = sync::sync_once(&api, &file, &env.cfg).unwrap();
    drop(api);
    assert!(fired);
    assert!(matches!(r.outcome, SyncOutcome::Synced { .. }), "{r:#?}");
    // §6.2 delete vs. added descendants: the node is resurrected with the new child.
    let tree = env.export();
    assert!(!tree.contains(&a));
    let alpha = by_name(&tree, "Alpha").id.clone();
    assert_eq!(tree.parent(&alpha), Some(&root));
    assert_eq!(child_names(&tree, &alpha), ["My important new child"]);
    let text = read(&file);
    assert!(text.contains("My important new child"), "{text}");
    assert!(line_of(&text, &alpha).contains("Alpha"), "{text}");
    no_nonces(&env);
    settle(&env, &file);
}

#[test]
fn skipped_local_content_is_not_committed_when_no_retry_is_left() {
    let (env, _root, a, _b, file) = setup();
    edit(&file, "Alpha", "Alpha LOCAL");
    let edited = read(&file);
    let cp_before = StateStore::open(&env.cfg.state_dir)
        .unwrap()
        .load_checkpoint(&root_of(&env, &file))
        .unwrap()
        .unwrap();
    let mut fired = false;
    let api = TestApi::with_call_hook(&env.client, |kind| {
        if kind == "get" && !fired {
            fired = true;
            env.ui("edit", json!({"id": a.as_str(), "text": "Alpha REMOTE"}));
        }
    });
    let mut cfg = env.cfg.clone();
    cfg.cas_retries = 0;
    let r = sync::sync_once(&api, &file, &cfg).unwrap();
    drop(api);
    assert!(fired);
    // Nothing that drops the local text is committed: the file and checkpoint stay.
    assert!(matches!(r.outcome, SyncOutcome::Skipped { .. }), "{r:#?}");
    assert_eq!(read(&file), edited);
    let cp_after = StateStore::open(&env.cfg.state_dir)
        .unwrap()
        .load_checkpoint(&cp_before.root)
        .unwrap()
        .unwrap();
    assert_eq!(cp_after.generation, cp_before.generation);
    assert_eq!(cp_after.base_md, cp_before.base_md);
    // The next, quiet cycle keeps the local text as a conflict.
    let r = env.sync(&file);
    assert!(matches!(r.outcome, SyncOutcome::Synced { .. }), "{r:#?}");
    let text = read(&file);
    assert!(text.contains("⚠ Alpha LOCAL"), "{text}");
    assert!(line_of(&text, &a).contains("Alpha REMOTE"), "{text}");
    settle(&env, &file);
}

fn root_of(env: &Env, file: &std::path::Path) -> NodeId {
    let st = sync::status_of_files(&env.cfg, &[file.to_path_buf()]).unwrap();
    st[0].status.as_ref().unwrap().root.clone()
}
