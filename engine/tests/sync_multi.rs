//! `sync_all` against the mock: several files in one run share one export and one id
//! map, so a tagged item cut from one file and pasted into another is a move (§6.4);
//! roots claimed twice pause (guard 5); nested roots are refused.

mod sync_support;

use syncflowy::error::Error;
use syncflowy::sync::{self, PauseReason, SyncOutcome, SyncReport};
use syncflowy::tree::NodeId;

use sync_support::*;

struct Two {
    env: Env,
    ra: NodeId,
    rb: NodeId,
    x: NodeId,
    x_child: NodeId,
    keep: NodeId,
    target: NodeId,
    a: std::path::PathBuf,
    b: std::path::PathBuf,
}

fn two_files() -> Two {
    let env = Env::new();
    let ra = env.ui_create(None, "Alpha");
    let keep = env.ui_create(Some(&ra), "Stays in alpha");
    let x = env.ui_create(Some(&ra), "Wanders off");
    let x_child = env.ui_create(Some(&x), "with its child");
    let rb = env.ui_create(None, "Beta");
    let target = env.ui_create(Some(&rb), "Destination");
    let a = env.path("alpha.md");
    let b = env.path("beta.md");
    env.render(&ra, &a);
    env.render(&rb, &b);
    Two {
        env,
        ra,
        rb,
        x,
        x_child,
        keep,
        target,
        a,
        b,
    }
}

fn reports(
    env: &Env,
    api: &dyn syncflowy::wfclient::Api,
    files: &[std::path::PathBuf],
) -> Vec<SyncReport> {
    sync::sync_all(api, files, &env.cfg)
        .expect("run")
        .into_iter()
        .map(|r| r.result.expect("file result"))
        .collect()
}

#[test]
fn tag_moved_between_files_is_a_move() {
    let t = two_files();
    let a_text = read(&t.a);
    let x_line = line_of(&a_text, &t.x).to_owned();
    let child_line = line_of(&a_text, &t.x_child).to_owned();
    // Cut from alpha.md...
    edit(&t.a, &format!("{x_line}\n{child_line}\n"), "");
    // ...and paste under "Destination" in beta.md (one level deeper).
    let b_text = read(&t.b);
    let dest = line_of(&b_text, &t.target).to_owned();
    edit(
        &t.b,
        &format!("{dest}\n"),
        &format!("{dest}\n\t{x_line}\n\t{child_line}\n"),
    );

    let api = TestApi::new(&t.env.client);
    let rs = reports(&t.env, &api, &[t.a.clone(), t.b.clone()]);
    let calls = api.calls();
    assert!(
        calls
            .iter()
            .all(|c| !c.starts_with("create") && !c.starts_with("delete")),
        "a move, not delete + create: {calls:?}"
    );
    assert_eq!(
        calls.iter().filter(|c| c.starts_with("move")).count(),
        1,
        "{calls:?}"
    );
    assert_eq!(
        calls.iter().filter(|c| *c == "export").count(),
        2,
        "one export for the run and one after the applies: {calls:?}"
    );
    let tree = t.env.export();
    assert_eq!(tree.parent(&t.x), Some(&t.target));
    assert_eq!(tree.parent(&t.x_child), Some(&t.x));
    assert_eq!(child_names(&tree, &t.ra), ["Stays in alpha"]);
    for r in &rs {
        assert!(matches!(r.outcome, SyncOutcome::Synced { .. }), "{r:#?}");
    }
    let a_text = read(&t.a);
    let b_text = read(&t.b);
    assert!(!a_text.contains("Wanders off"), "{a_text}");
    assert!(line_of(&b_text, &t.x).starts_with("\t- "), "{b_text}");
    assert!(
        line_of(&b_text, &t.x_child).starts_with("\t\t- "),
        "{b_text}"
    );
    assert!(line_of(&a_text, &t.keep).contains("Stays in alpha"));

    // Settled: a second run sends nothing.
    let before = t.env.snapshot();
    let rs = reports(&t.env, &t.env.client, &[t.a.clone(), t.b.clone()]);
    assert!(
        rs.iter().all(|r| r.outcome == SyncOutcome::Unchanged),
        "{rs:#?}"
    );
    assert_eq!(t.env.snapshot(), before);
}

#[test]
fn edits_in_both_files_in_one_run() {
    let t = two_files();
    edit(&t.a, "Stays in alpha", "Stays in alpha, renamed");
    let b_text = read(&t.b);
    let dest = line_of(&b_text, &t.target).to_owned();
    edit(
        &t.b,
        &format!("{dest}\n"),
        &format!("{dest}\n\t- New in beta\n"),
    );
    let rs = reports(&t.env, &t.env.client, &[t.a.clone(), t.b.clone()]);
    assert_eq!(rs.len(), 2);
    let tree = t.env.export();
    assert_eq!(tree.get(&t.keep).unwrap().name, "Stays in alpha, renamed");
    assert_eq!(child_names(&tree, &t.target), ["New in beta"]);
    assert!(read(&t.b).contains("New in beta ^wf-"));
}

#[test]
fn two_files_claiming_one_root_are_paused() {
    let t = two_files();
    let copy = t.env.path("alpha (copy).md");
    std::fs::copy(&t.a, &copy).unwrap();
    edit(&copy, "Stays in alpha", "Changed in the copy");
    let before = t.env.snapshot();
    let rs = reports(
        &t.env,
        &t.env.client,
        &[t.a.clone(), copy.clone(), t.b.clone()],
    );
    for r in &rs[..2] {
        assert!(
            matches!(&r.outcome, SyncOutcome::Paused(PauseReason::DuplicateRoot(p)) if p.len() == 2),
            "{r:#?}"
        );
    }
    assert_eq!(rs[2].outcome, SyncOutcome::Unchanged);
    assert_eq!(t.env.snapshot(), before);
}

#[test]
fn nested_roots_are_refused() {
    let t = two_files();
    let inner = t.env.path("inner.md");
    t.env.render(&t.x, &inner);
    edit(&inner, "with its child", "with its renamed child");
    let before = t.env.snapshot();
    let results = sync::sync_all(&t.env.client, &[t.a.clone(), inner.clone()], &t.env.cfg).unwrap();
    assert_eq!(
        results[0].result.as_ref().unwrap().outcome,
        SyncOutcome::Unchanged
    );
    match &results[1].result {
        Err(Error::NestedRoots { outer, inner }) => {
            assert_eq!(outer, &t.ra);
            assert_eq!(inner, &t.x);
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(t.env.snapshot(), before);
    let _ = t.rb;
}

#[test]
fn per_file_failures_do_not_stop_the_run() {
    let t = two_files();
    let missing = t.env.path("gone.md");
    edit(&t.b, "Destination", "Destination!");
    let results = sync::sync_all(
        &t.env.client,
        &[missing.clone(), t.a.clone(), t.b.clone()],
        &t.env.cfg,
    )
    .unwrap();
    assert_eq!(
        results[0].result.as_ref().unwrap().outcome,
        SyncOutcome::Paused(PauseReason::FileMissing)
    );
    assert_eq!(
        results[1].result.as_ref().unwrap().outcome,
        SyncOutcome::Unchanged
    );
    assert!(matches!(
        results[2].result.as_ref().unwrap().outcome,
        SyncOutcome::Synced { .. }
    ));
    assert_eq!(t.env.export().get(&t.target).unwrap().name, "Destination!");
}
