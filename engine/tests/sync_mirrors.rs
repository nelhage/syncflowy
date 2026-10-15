//! Mirrors as their origins (design §15.1) end to end against the mock in its
//! `beta_mirrors` mode (data-model §10.5): a node mirrored into the sync root is shown
//! and edited in full at its shell, a second placement of the same content is a stub,
//! writes through the shell reach the origin, deleting a shell removes just the mirror,
//! and an origin that still has mirrors cannot be deleted.

mod sync_support;

use serde_json::json;
use syncflowy::mdblock::{self, ItemKind};
use syncflowy::merge::ReportEvent;
use syncflowy::sync::SyncOutcome;
use syncflowy::tree::NodeId;
use syncflowy::wfclient::Api;

use sync_support::*;

/// Root "Project" / {"Plan" / {"Draft"}, "Later" / {S2 = mirror of Plan}, S = mirror of
/// "Inbox" }; "Reference" (outside the root) / {"Inbox" / {"item 1", "item 2"}}.
struct Fixture {
    env: Env,
    root: NodeId,
    plan: NodeId,
    draft: NodeId,
    later: NodeId,
    inbox: NodeId,
    item1: NodeId,
    item2: NodeId,
    shell: NodeId,
    stub: NodeId,
    file: std::path::PathBuf,
}

fn beta_env() -> Env {
    let env = Env::new();
    env.mock.config(json!({"beta_mirrors": true}));
    env
}

fn mirror(env: &Env, origin: &NodeId, parent: &NodeId) -> NodeId {
    let r = env.ui(
        "mirror",
        json!({"id": origin.as_str(), "parent_id": parent.as_str(), "position": "bottom"}),
    );
    NodeId::parse(r["id"].as_str().expect("id")).expect("uuid")
}

fn fixture() -> Fixture {
    let env = beta_env();
    let root = env.ui_create(None, "Project");
    let plan = env.ui_create(Some(&root), "Plan");
    let draft = env.ui_create(Some(&plan), "Draft");
    let later = env.ui_create(Some(&root), "Later");
    let reference = env.ui_create(None, "Reference");
    let inbox = env.ui_create_with(
        Some(&reference),
        json!({"text": "Inbox", "note_text": "triage daily"}),
    );
    let item1 = env.ui_create(Some(&inbox), "item 1");
    let item2 = env.ui_create(Some(&inbox), "item 2");
    let shell = mirror(&env, &inbox, &root);
    let stub = mirror(&env, &plan, &later);
    let file = env.path("project.md");
    let r = env.render(&root, &file);
    assert_eq!(r.outcome, SyncOutcome::Rendered { generation: 1 });
    Fixture {
        env,
        root,
        plan,
        draft,
        later,
        inbox,
        item1,
        item2,
        shell,
        stub,
        file,
    }
}

fn synced(r: &syncflowy::sync::SyncReport) -> (bool, u64) {
    match &r.outcome {
        SyncOutcome::Synced {
            wrote_file,
            generation,
            ..
        } => (*wrote_file, *generation),
        other => panic!("expected Synced, got {other:?}\n{r:#?}"),
    }
}

fn assert_settled(f: &Fixture) {
    let before = f.env.snapshot();
    let text = read(&f.file);
    let r = f.env.sync(&f.file);
    assert_eq!(r.outcome, SyncOutcome::Unchanged, "{r:#?}\n{text}");
    assert_eq!(read(&f.file), text);
    assert_eq!(f.env.snapshot(), before);
}

fn stub_line(f: &Fixture) -> String {
    format!(
        "- {} ^wf-{}-ro",
        mdblock::stub_text(&short(&f.plan), "Plan"),
        short(&f.stub)
    )
}

#[test]
fn render_shows_mirrored_content_in_full_and_a_second_placement_as_a_stub() {
    let f = fixture();
    let text = read(&f.file);
    let expected = format!(
        "---\nworkflowy: {root}\nsyncflowy: {{gen: 1}}\n---\n\
         - Plan ^wf-{plan}\n\
         \t- [ ] Draft ^wf-{draft}\n\
         - Later ^wf-{later}\n\
         \t{stub}\n\
         - Inbox ^wf-{shell}\n\
         \ttriage daily\n\
         \t- [ ] item 1 ^wf-{item1}\n\
         \t- [ ] item 2 ^wf-{item2}\n",
        root = short(&f.root),
        plan = short(&f.plan),
        draft = short(&f.draft),
        later = short(&f.later),
        stub = stub_line(&f),
        shell = short(&f.shell),
        item1 = short(&f.item1),
        item2 = short(&f.item2),
    );
    assert_eq!(text, expected);
    // The stub parses as a stub with one tag and no warning.
    let doc = mdblock::parse(&text).unwrap();
    let later = &doc.items[1];
    assert_eq!(later.children[0].kind, ItemKind::MirrorStub);
    assert!(later.children[0].extra_tags.is_empty());
    assert!(doc.warnings.is_empty(), "{:?}", doc.warnings);
    assert_settled(&f);
}

#[test]
fn edits_ticks_creates_and_moves_through_the_shell_reach_the_origin() {
    let f = fixture();
    edit(&f.file, "- Inbox ^wf-", "- Inbox today ^wf-");
    edit(
        &f.file,
        "\ttriage daily\n",
        "\ttriage daily\n\tand weekly\n",
    );
    edit(&f.file, "- [ ] item 1 ^wf-", "- [ ] item one ^wf-");
    edit(&f.file, "- [ ] item 2 ^wf-", "- [x] item 2 ^wf-");
    let text = read(&f.file);
    let text = format!("{text}\t- item 3\n");
    // Move Draft under the mirrored Inbox.
    let draft_line = format!("\t- [ ] Draft ^wf-{}\n", short(&f.draft));
    let text = text.replacen(&draft_line, "", 1) + &draft_line;
    write(&f.file, &text);

    let r = f.env.sync(&f.file);
    let (wrote, generation) = synced(&r);
    assert!(wrote);
    assert_eq!(generation, 2);
    assert!(r.discrepancies.is_empty(), "{:?}", r.discrepancies);

    let t = f.env.export();
    let inbox = t.get(&f.inbox).unwrap();
    assert_eq!(inbox.name, "Inbox today");
    assert_eq!(inbox.note.as_deref(), Some("triage daily\nand weekly"));
    assert_eq!(t.get(&f.item1).unwrap().name, "item one");
    assert!(t.get(&f.item2).unwrap().completed);
    assert_eq!(t.parent(&f.draft), Some(&f.inbox), "moved under the origin");
    let item3 = by_name(&t, "item 3");
    assert_eq!(
        item3.parent_id.as_ref(),
        Some(&f.inbox),
        "created under the origin"
    );
    assert_eq!(
        child_names(&t, &f.inbox),
        ["item one", "item 2", "item 3", "Draft"].map(String::from)
    );
    // The shell row itself is untouched (the API shows the origin through it).
    let dump = f.env.mock.dump();
    let row = dump["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["id"] == f.shell.as_str())
        .unwrap();
    assert_eq!(row["name"], "");

    let text = read(&f.file);
    assert!(text.contains(&format!("- Inbox today ^wf-{}", short(&f.shell))));
    assert!(text.contains("item one") && !text.contains("item 2"));
    assert!(line_of(&text, &f.draft).starts_with("\t- [ ] Draft"));
    assert!(line_of(&text, &item3.id).contains("item 3"));
    assert_settled(&f);
}

#[test]
fn remote_edits_of_the_origin_show_at_the_shell() {
    let f = fixture();
    f.env.ui(
        "edit",
        json!({"id": f.inbox.as_str(), "text": "Inbox (renamed)"}),
    );
    f.env.ui_create(Some(&f.inbox), "item 3");
    let r = f.env.sync(&f.file);
    let (wrote, _) = synced(&r);
    assert!(wrote);
    let text = read(&f.file);
    assert!(text.contains(&format!("- Inbox (renamed) ^wf-{}", short(&f.shell))));
    assert!(text.contains("- [ ] item 3 ^wf-"));
    assert_settled(&f);
}

#[test]
fn deleting_the_shell_line_removes_only_the_mirror() {
    let f = fixture();
    let text = read(&f.file);
    let cut = text
        .find(&format!("- Inbox ^wf-{}", short(&f.shell)))
        .unwrap();
    write(&f.file, &text[..cut]);
    let r = f.env.sync(&f.file);
    // The file already shows the result: nothing to rewrite.
    let (wrote, _) = synced(&r);
    assert!(!wrote);
    match &r.outcome {
        SyncOutcome::Synced { pushed, .. } => {
            assert_eq!(
                (pushed.deletes, pushed.hidden_deleted),
                (1, 0),
                "{pushed:?}"
            );
        }
        _ => unreachable!(),
    }
    let t = f.env.export();
    assert!(!t.contains(&f.shell));
    assert_eq!(t.get(&f.inbox).unwrap().name, "Inbox");
    assert_eq!(
        child_names(&t, &f.inbox),
        ["item 1", "item 2"].map(String::from)
    );
    assert!(
        t.get(&f.inbox).unwrap().mirror.is_none(),
        "no registry left"
    );
    assert!(!read(&f.file).contains("Inbox"));
    assert_settled(&f);
}

#[test]
fn deleting_an_origin_that_still_has_mirrors_is_refused() {
    let f = fixture();
    let text = read(&f.file);
    let plan = format!(
        "- Plan ^wf-{}\n\t- [ ] Draft ^wf-{}\n",
        short(&f.plan),
        short(&f.draft)
    );
    write(&f.file, &text.replacen(&plan, "", 1));
    let r = f.env.sync(&f.file);
    assert!(
        r.events.contains(&ReportEvent::MirrorDeleteRefused {
            node: f.plan.clone(),
            origin: f.plan.clone(),
        }),
        "{:?}",
        r.events
    );
    let (wrote, _) = synced(&r);
    assert!(wrote, "the line comes back");
    assert!(f.env.export().contains(&f.plan));
    assert!(read(&f.file).contains("- Plan ^wf-"), "{}", read(&f.file));
    assert_settled(&f);
    // --force does not override it (the API would refuse, DM§10.5).
    let text = read(&f.file);
    write(&f.file, &text.replacen(&plan, "", 1));
    let mut forced = f.env.cfg.clone();
    forced.force = true;
    let r = syncflowy::sync::sync_once(&f.env.client, &f.file, &forced).unwrap();
    assert!(f.env.export().contains(&f.plan));
    assert!(matches!(r.outcome, SyncOutcome::Synced { .. }), "{r:#?}");
}

#[test]
fn deleting_the_stub_removes_that_mirror() {
    let f = fixture();
    let text = read(&f.file);
    write(
        &f.file,
        &text.replacen(&format!("\t{}\n", stub_line(&f)), "", 1),
    );
    let r = f.env.sync(&f.file);
    synced(&r);
    let t = f.env.export();
    assert!(!t.contains(&f.stub));
    assert!(t.contains(&f.plan) && t.contains(&f.draft));
    assert_settled(&f);
}

#[test]
fn stub_text_edits_and_ticks_are_refused_and_rewritten() {
    let f = fixture();
    let text = read(&f.file);
    let line = stub_line(&f);
    let edited = format!("- [x] typed over ^wf-{}-ro", short(&f.stub));
    write(&f.file, &text.replacen(&line, &edited, 1));
    let r = f.env.sync(&f.file);
    let (wrote, _) = synced(&r);
    assert!(wrote);
    assert!(
        r.events
            .iter()
            .any(|e| matches!(e, ReportEvent::ReadOnlyEditRefused { node, .. } if node == &f.stub)),
        "{:?}",
        r.events
    );
    let body = |t: &str| {
        t.split_once("---\n---\n")
            .map(|x| x.1.to_owned())
            .unwrap_or_else(|| t.splitn(3, "---\n").nth(2).unwrap().to_owned())
    };
    assert_eq!(
        body(&read(&f.file)),
        body(&text),
        "rewritten in canonical form"
    );
    let t = f.env.export();
    assert_eq!(t.get(&f.plan).unwrap().name, "Plan");
    assert!(!t.get(&f.plan).unwrap().completed);
}

#[test]
fn items_typed_under_a_stub_are_lifted_and_reported() {
    let f = fixture();
    let text = read(&f.file);
    let line = stub_line(&f);
    write(
        &f.file,
        &text.replacen(
            &format!("\t{line}\n"),
            &format!("\t{line}\n\t\t- under the stub\n"),
            1,
        ),
    );
    let r = f.env.sync(&f.file);
    synced(&r);
    let t = f.env.export();
    let n = by_name(&t, "under the stub");
    assert_eq!(
        n.parent_id.as_ref(),
        Some(&f.later),
        "lifted to the stub's parent"
    );
    assert!(
        r.events
            .iter()
            .any(|e| matches!(e, ReportEvent::ReadOnlyEditRefused { node, .. } if node == &f.stub)),
        "{:?}",
        r.events
    );
    assert_settled(&f);
}

#[test]
fn the_full_placement_is_sticky_across_reorders() {
    let f = fixture();
    // Move "Later" (with the stub) above "Plan": the stub now comes first in pre-order,
    // but Plan keeps the full rendering.
    let text = read(&f.file);
    let later_block = format!("- Later ^wf-{}\n\t{}\n", short(&f.later), stub_line(&f));
    let plan_block = format!(
        "- Plan ^wf-{}\n\t- [ ] Draft ^wf-{}\n",
        short(&f.plan),
        short(&f.draft)
    );
    let moved = text.replacen(&later_block, "", 1).replacen(
        &plan_block,
        &format!("{later_block}{plan_block}"),
        1,
    );
    write(&f.file, &moved);
    let r = f.env.sync(&f.file);
    // The file already shows the result: Plan kept the full rendering, nothing to rewrite.
    let (wrote, _) = synced(&r);
    assert!(!wrote, "{r:#?}");
    let t = f.env.export();
    assert_eq!(
        child_names(&t, &f.root),
        ["Later", "Plan", "Inbox"].map(String::from)
    );
    let text = read(&f.file);
    assert!(
        text.contains(&format!("{later_block}{plan_block}")),
        "{text}"
    );
    assert_settled(&f);
}

#[test]
fn a_mirror_removed_in_workflowy_leaves_the_file() {
    let f = fixture();
    f.env.client.delete_mirror(&f.shell).unwrap();
    let r = f.env.sync(&f.file);
    let (wrote, _) = synced(&r);
    assert!(wrote);
    let text = read(&f.file);
    assert!(!text.contains("Inbox") && !text.contains("item 1"));
    assert!(f.env.export().contains(&f.item1));
    assert_settled(&f);
}

#[test]
fn a_mirror_added_in_workflowy_appears_as_its_content() {
    let f = fixture();
    let shell2 = mirror(&f.env, &f.inbox, &f.later);
    let r = f.env.sync(&f.file);
    synced(&r);
    let text = read(&f.file);
    // The first placement keeps the full rendering; the new one is a stub of the shell.
    assert!(text.contains(&format!("- Inbox ^wf-{}", short(&f.shell))));
    let expected = format!(
        "\t- {} ^wf-{}-ro",
        mdblock::stub_text(&short(&f.shell), "Inbox"),
        short(&shell2)
    );
    assert!(text.contains(&expected), "{text}");
    assert_settled(&f);
}

#[test]
fn an_orphan_shell_is_a_placeholder() {
    let f = fixture();
    // An orphan arises on the beta API when a shell's persisted child is itself an
    // origin and the shell is removed (DM§10.5): O2 moved under the shell, then
    // DELETE /mirror on the shell deletes O2 and orphans O2's mirror in the root.
    let o2 = f.env.ui_create(None, "Orphaned");
    let s3 = mirror(&f.env, &o2, &f.root);
    f.env
        .client
        .move_node(&o2, &f.shell, syncflowy::tree::Position::Bottom)
        .unwrap();
    f.env.client.delete_mirror(&f.shell).unwrap();
    assert!(!f.env.export().contains(&o2));
    let r = f.env.sync(&f.file);
    synced(&r);
    let text = read(&f.file);
    assert!(text.contains(&format!(
        "- {} ^wf-{}-ro",
        mdblock::PLACEHOLDER_TEXT,
        short(&s3)
    )));
    assert_settled(&f);
}

#[test]
fn two_files_showing_the_same_content_both_edit_it() {
    // A second root with its own mirror of Inbox: both files show the content; an edit in
    // one shows up in the other, and a delete of a child in one is a delete.
    let f = fixture();
    let other = f.env.ui_create(None, "Other");
    let shell_b = mirror(&f.env, &f.inbox, &other);
    let file_b = f.env.path("other.md");
    f.env.render(&other, &file_b);
    assert!(read(&file_b).contains(&format!("- Inbox ^wf-{}", short(&shell_b))));

    edit(&f.file, "- [ ] item 1 ^wf-", "- [ ] item 1 (from A) ^wf-");
    let text_b = read(&file_b);
    write(
        &file_b,
        &text_b.replacen(&format!("\t- [ ] item 2 ^wf-{}\n", short(&f.item2)), "", 1),
    );
    let results =
        syncflowy::sync::sync_all(&f.env.client, &[f.file.clone(), file_b.clone()], &f.env.cfg)
            .unwrap();
    for r in &results {
        let r = r.result.as_ref().unwrap();
        synced(r);
    }
    let t = f.env.export();
    assert_eq!(t.get(&f.item1).unwrap().name, "item 1 (from A)");
    assert!(!t.contains(&f.item2), "deleted through file B");
    assert!(read(&file_b).contains("item 1 (from A)"));
    assert!(!read(&f.file).contains("item 2"));
}
