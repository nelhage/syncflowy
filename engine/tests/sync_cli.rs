//! The `syncflowy` binary against the mock: `render`, `sync` (with `--force`), `status`,
//! the API key from `--api-key` or `WORKFLOWY_API_KEY`, and exit codes.

mod sync_support;

use std::process::{Command, Output};

use sync_support::*;

fn cli(env: &Env, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_syncflowy"))
        .arg("--api-url")
        .arg(env.mock.base_url())
        .arg("--state-dir")
        .arg(&env.cfg.state_dir)
        .args(args)
        .env("WORKFLOWY_API_KEY", env.mock.api_key())
        .env_remove("WORKFLOWY_API_URL")
        .env_remove("SYNCFLOWY_STATE_DIR")
        .output()
        .expect("run syncflowy")
}

fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

#[test]
fn render_sync_status() {
    let env = Env::new();
    let root = env.ui_create(None, "Project");
    let a = env.ui_create(Some(&root), "Alpha");
    for i in 0..6 {
        env.ui_create(Some(&root), &format!("item {i}"));
    }
    let file = env.path("project.md");
    let root_short = short(&root).to_string();
    let url = format!("https://workflowy.com/#/{root_short}");

    let o = cli(&env, &["render", &url, file.to_str().unwrap()]);
    assert!(o.status.success(), "{}{}", stdout(&o), stderr(&o));
    assert!(stdout(&o).contains("rendered (gen 1)"), "{}", stdout(&o));
    assert!(read(&file).contains(&format!("workflowy: {url}")));
    // A second render refuses.
    let o = cli(&env, &["render", &root_short, file.to_str().unwrap()]);
    assert_eq!(o.status.code(), Some(2), "{}", stderr(&o));
    assert!(stderr(&o).contains("exists"), "{}", stderr(&o));

    // Unchanged.
    let o = cli(&env, &["sync", file.to_str().unwrap()]);
    assert!(o.status.success(), "{}", stderr(&o));
    assert!(stdout(&o).contains("unchanged"), "{}", stdout(&o));

    // An edit, with the key given by --api-key instead of the environment.
    edit(&file, "Alpha", "Alpha, edited");
    let o = Command::new(env!("CARGO_BIN_EXE_syncflowy"))
        .args([
            "--api-url",
            env.mock.base_url(),
            "--api-key",
            env.mock.api_key(),
        ])
        .arg("--state-dir")
        .arg(&env.cfg.state_dir)
        .args(["sync", file.to_str().unwrap()])
        .env_remove("WORKFLOWY_API_KEY")
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", stderr(&o));
    assert!(stdout(&o).contains("1 updated"), "{}", stdout(&o));
    assert_eq!(env.export().get(&a).unwrap().name, "Alpha, edited");

    // status
    let o = cli(&env, &["status"]);
    assert!(o.status.success());
    let s = stdout(&o);
    assert!(s.contains(root.as_str()) && s.contains("gen 1"), "{s}");
    assert!(s.contains("as last written"), "{s}");

    // A truncated file is braked (exit 1) until --force.
    let text = read(&file);
    let fm_end = text.find("\n---\n").unwrap() + 5;
    write(
        &file,
        &format!("{}- Alpha, edited ^wf-{}\n", &text[..fm_end], short(&a)),
    );
    let o = cli(&env, &["sync", file.to_str().unwrap()]);
    assert_eq!(o.status.code(), Some(1), "{}", stdout(&o));
    assert!(stdout(&o).contains("refused"), "{}", stdout(&o));
    // Said once, as the outcome (not again as a warning).
    let brake_lines = stdout(&o)
        .lines()
        .filter(|l| l.contains("refusing to delete") || l.contains("the file is empty"))
        .count();
    assert_eq!(brake_lines, 1, "{}", stdout(&o));
    assert_eq!(env.export().children(&root).len(), 7);
    let o = cli(&env, &["sync", "--force", file.to_str().unwrap()]);
    assert!(o.status.success(), "{}{}", stdout(&o), stderr(&o));
    assert_eq!(env.export().children(&root).len(), 1);

    // A missing file: exit 1, nothing deleted.
    std::fs::remove_file(&file).unwrap();
    let o = cli(&env, &["sync", file.to_str().unwrap()]);
    assert_eq!(o.status.code(), Some(1));
    assert!(stdout(&o).contains("missing"), "{}", stdout(&o));
    assert_eq!(env.export().children(&root).len(), 1);
}

#[test]
fn reports_have_no_debug_output_and_name_a_conflict_once() {
    let env = Env::new();
    let root = env.ui_create(None, "Project");
    let a = env.ui_create(Some(&root), "Alpha");
    let file = env.path("project.md");
    env.render(&root, &file);
    env.ui(
        "edit",
        serde_json::json!({"id": a.as_str(), "text": "Alpha (remote)"}),
    );
    edit(&file, "Alpha", "Alpha (local)");
    let al = line_of(&read(&file), &a).to_owned();
    edit(&file, &format!("{al}\n"), &format!("{al}\n- Carrots\n"));

    let o = cli(&env, &["sync", "--dry-run", file.to_str().unwrap()]);
    assert!(o.status.success(), "{}{}", stdout(&o), stderr(&o));
    let out = stdout(&o);
    assert!(!out.contains("NodeId("), "{out}");
    assert!(out.contains("op 0: create "), "{out}");

    let o = cli(&env, &["sync", file.to_str().unwrap()]);
    assert!(o.status.success(), "{}{}", stdout(&o), stderr(&o));
    let out = stdout(&o);
    assert!(!out.contains("NodeId("), "{out}");
    let conflict_lines = out.lines().filter(|l| l.contains("conflict on")).count();
    assert_eq!(conflict_lines, 1, "{out}");
    assert!(
        out.contains("conflict on the name of \"Alpha (remote)\""),
        "{out}"
    );
}

#[test]
fn missing_api_key_is_an_error() {
    let env = Env::new();
    let o = Command::new(env!("CARGO_BIN_EXE_syncflowy"))
        .args(["--api-url", env.mock.base_url(), "sync", "x.md"])
        .env_remove("WORKFLOWY_API_KEY")
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(2));
    assert!(stderr(&o).contains("API key"), "{}", stderr(&o));
}

#[test]
fn status_of_missing_and_unknown_files() {
    let env = Env::new();
    let root = env.ui_create(None, "Project");
    env.ui_create(Some(&root), "Alpha");
    let file = env.path("project.md");
    env.render(&root, &file);
    let other = env.path("other.md");
    let other_root = "0f3c1b2a-1111-4222-8333-444455556666";
    write(
        &other,
        &format!("---\nworkflowy: {other_root}\n---\n- Beta\n"),
    );
    let gone = env.path("gone.md");

    let o = cli(
        &env,
        &[
            "status",
            gone.to_str().unwrap(),
            other.to_str().unwrap(),
            file.to_str().unwrap(),
        ],
    );
    let s = stdout(&o);
    assert_eq!(o.status.code(), Some(1), "{s}{}", stderr(&o));
    assert!(s.contains(&format!("{}: missing\n", gone.display())), "{s}");
    assert!(
        s.contains(&format!(
            "{}: no checkpoint for root {other_root} in this state directory\n",
            other.display()
        )),
        "{s}"
    );
    assert!(s.contains(&format!("{root}: gen 1")), "{s}");
    assert!(s.contains("as last written"), "{s}");
}

#[test]
fn status_creates_no_state_directory() {
    let env = Env::new();
    let fresh = env.dir.path().join("fresh-state");
    let file = env.dir.path().join("f.md");
    write(&file, "---\nworkflowy: abcdef123456\n---\n- x\n");
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_syncflowy"))
            .arg("--state-dir")
            .arg(&fresh)
            .args(args)
            .env_remove("WORKFLOWY_API_KEY")
            .env_remove("SYNCFLOWY_STATE_DIR")
            .output()
            .unwrap()
    };
    let o = run(&["status"]);
    assert!(o.status.success(), "{}", stderr(&o));
    assert!(stdout(&o).contains("no synced roots"), "{}", stdout(&o));
    let o = run(&["status", file.to_str().unwrap()]);
    assert_eq!(o.status.code(), Some(1), "{}", stderr(&o));
    assert!(
        stdout(&o).contains("no checkpoint for root"),
        "{}",
        stdout(&o)
    );
    assert!(!fresh.exists());
}

#[test]
fn a_second_file_for_a_root_is_refused_across_runs() {
    let env = Env::new();
    let root = env.ui_create(None, "Project");
    env.ui_create(Some(&root), "Alpha");
    let file = env.path("big.md");
    env.render(&root, &file);
    let before = env.snapshot();

    // A conflict copy is never synced, even on its own.
    let copy = env.path("big (conflicted copy 2026-10-13).md");
    std::fs::copy(&file, &copy).unwrap();
    edit(&copy, "Alpha", "Alpha, in the copy");
    let o = cli(&env, &["sync", copy.to_str().unwrap()]);
    assert_eq!(o.status.code(), Some(1), "{}{}", stdout(&o), stderr(&o));
    assert!(
        stdout(&o).contains("skipped: looks like a sync conflict copy"),
        "{}",
        stdout(&o)
    );

    // Another file naming the root while the synced one still exists: paused.
    let other = env.path("renamed-big.md");
    std::fs::rename(&copy, &other).unwrap();
    let o = cli(&env, &["sync", other.to_str().unwrap()]);
    assert_eq!(o.status.code(), Some(1), "{}{}", stdout(&o), stderr(&o));
    let s = stdout(&o);
    assert!(s.contains("paused: several files claim this root"), "{s}");
    assert!(s.contains("big.md"), "{s}");
    assert_eq!(env.snapshot(), before);

    // A real rename (the old path is gone) is synced.
    std::fs::remove_file(&file).unwrap();
    let o = cli(&env, &["sync", other.to_str().unwrap()]);
    assert!(o.status.success(), "{}{}", stdout(&o), stderr(&o));
    assert!(stdout(&o).contains("1 updated"), "{}", stdout(&o));
    // So is one whose old path now holds another root.
    let third = env.path("third.md");
    std::fs::rename(&other, &third).unwrap();
    let root2 = env.ui_create(None, "Other");
    env.ui_create(Some(&root2), "Beta");
    env.render(&root2, &other);
    edit(&third, "Alpha, in the copy", "Alpha again");
    let o = cli(&env, &["sync", third.to_str().unwrap()]);
    assert!(o.status.success(), "{}{}", stdout(&o), stderr(&o));
    assert!(stdout(&o).contains("1 updated"), "{}", stdout(&o));
}

#[test]
fn empty_api_key_is_missing() {
    let env = Env::new();
    for key in ["", "  "] {
        let o = Command::new(env!("CARGO_BIN_EXE_syncflowy"))
            .args(["--api-url", env.mock.base_url(), "sync", "x.md"])
            .env("WORKFLOWY_API_KEY", key)
            .output()
            .unwrap();
        assert_eq!(o.status.code(), Some(2));
        assert_eq!(
            stderr(&o),
            "error: no API key: pass --api-key or set WORKFLOWY_API_KEY\n"
        );
    }
}

#[test]
fn wrong_api_key_names_the_cause_once() {
    let env = Env::new();
    let root = env.ui_create(None, "Project");
    let file = env.path("project.md");
    env.render(&root, &file);
    let o = Command::new(env!("CARGO_BIN_EXE_syncflowy"))
        .args(["--api-url", env.mock.base_url(), "--api-key", "wrong"])
        .arg("--state-dir")
        .arg(&env.cfg.state_dir)
        .args(["sync", file.to_str().unwrap()])
        .env_remove("WORKFLOWY_API_KEY")
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(2));
    assert_eq!(stderr(&o), "error: API: invalid credentials\n");
}
