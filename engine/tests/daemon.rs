//! The daemon against the mock (§9.1, §14 step 6): discovery by root and renames,
//! duplicate roots, missing files, debouncing, deferred deletes (grace period, cancel,
//! cross-file move, held for a split arrival), guard 3 (stale echoes, dropped
//! resurrections), guard 6 (Tasks done dates) and the status note. Every test drives
//! `Daemon::step` with a fake clock and a temp vault; nothing sleeps.

mod sync_support;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde_json::json;
use syncflowy::daemon::{Daemon, DaemonConfig, FakeTime, STATUS_NOTE, StepReport, discover};
use syncflowy::sync::SyncOutcome;
use syncflowy::tree::NodeId;
use syncflowy::wfclient::Api;

use sync_support::*;

/// The fake clock's start (unix seconds): 2026-09-21 14:13 UTC.
const T0: i64 = 1_790_000_000;

/// A daemon over the test environment's vault, with a fake clock.
struct Rig {
    env: Env,
    time: Arc<FakeTime>,
    d: Daemon,
}

impl Rig {
    fn new(env: Env) -> Rig {
        Rig::with(env, |_| {})
    }

    fn with(env: Env, tweak: impl FnOnce(&mut DaemonConfig)) -> Rig {
        let vault = env.dir.path().join("vault");
        std::fs::create_dir_all(&vault).unwrap();
        let mut cfg = DaemonConfig::new(vault, env.cfg.clone());
        tweak(&mut cfg);
        let time = Arc::new(FakeTime::new(T0));
        let d = Daemon::new(cfg, time.clone()).expect("daemon");
        Rig { env, time, d }
    }

    fn vault(&self) -> PathBuf {
        self.env.dir.path().join("vault")
    }

    /// One step with the real client; `changed` are the paths of file events.
    fn step(&mut self, changed: &[&Path]) -> StepReport {
        step_with(&mut self.d, &self.env.client, changed)
    }

    fn advance(&self, secs: u64) {
        self.time.advance(Duration::from_secs(secs));
    }

    /// Advance in poll-sized steps (one cycle each) for `secs` seconds.
    fn run_for(&mut self, secs: u64) {
        let mut left = secs;
        while left > 0 {
            let s = left.min(60);
            self.advance(s);
            self.step(&[]);
            left -= s;
        }
    }

    fn note(&self) -> String {
        read(&self.vault().join(STATUS_NOTE))
    }

    fn outcome(report: &StepReport, file: &Path) -> SyncOutcome {
        let r = report
            .results
            .iter()
            .find(|(f, _)| f == file)
            .unwrap_or_else(|| panic!("{} not synced: {report:#?}", file.display()));
        r.1.as_ref().expect("no error").outcome.clone()
    }
}

/// One step of `d` through `api`.
fn step_with(d: &mut Daemon, api: &dyn Api, changed: &[&Path]) -> StepReport {
    let changed: Vec<PathBuf> = changed.iter().map(|p| p.to_path_buf()).collect();
    d.step(api, &changed).expect("step")
}

/// Root "Root" with children Alpha and Beta, rendered to `<vault>/root.md`.
fn one_file() -> (Env, NodeId, NodeId, NodeId, PathBuf) {
    let env = Env::new();
    let root = env.ui_create(None, "Root");
    let a = env.ui_create(Some(&root), "Alpha");
    let b = env.ui_create(Some(&root), "Beta");
    let file = env.path("root.md");
    env.render(&root, &file);
    (env, root, a, b, file)
}

fn is_synced(o: &SyncOutcome) -> bool {
    matches!(o, SyncOutcome::Synced { .. })
}

fn remove_line(file: &Path, id: &NodeId) -> String {
    let text = read(file);
    let line = line_of(&text, id).to_owned();
    edit(file, &format!("{line}\n"), "");
    line
}

// ---------------------------------------------------------------------------------------
// Discovery (guard 5)
// ---------------------------------------------------------------------------------------

#[test]
fn discovery_finds_files_by_root_and_follows_renames() {
    let env = Env::new();
    let ra = env.ui_create(None, "Root A");
    let alpha = env.ui_create(Some(&ra), "Alpha");
    let rb = env.ui_create(None, "Root B");
    env.ui_create(Some(&rb), "Beta");
    let fa = env.path("a.md");
    let fb = env.path("sub/b.md");
    env.render(&ra, &fa);
    env.render(&rb, &fb);
    let vault = env.dir.path().join("vault");
    write(&env.path("plain.md"), "# Just a note\n");
    write(&env.path("other.md"), "---\ntitle: x\n---\nbody\n");
    std::fs::create_dir_all(vault.join(".obsidian")).unwrap();
    write(&vault.join(".obsidian/a.md"), &read(&fa));
    let copy = env.path("a (conflicted copy 2026-10-13).md");
    write(&copy, &read(&fa));
    let broken = env.path("broken.md");
    write(&broken, "---\nworkflowy: not an id!\n---\n- x\n");

    let disc = discover(&vault);
    let files: Vec<&Path> = disc.files.iter().map(|f| f.path.as_path()).collect();
    assert_eq!(files, [fa.as_path(), fb.as_path()]);
    assert!(disc.duplicates.is_empty(), "{disc:#?}");
    let skipped: Vec<&Path> = disc.skipped.iter().map(|s| s.0.as_path()).collect();
    assert_eq!(skipped, [copy.as_path(), broken.as_path()]);
    assert!(disc.skipped[0].1.contains("conflict copy"), "{disc:#?}");

    let mut rig = Rig::new(env);
    let r = rig.step(&[]);
    assert_eq!(r.synced.as_deref(), Some(&[fa.clone(), fb.clone()][..]));
    assert_eq!(Rig::outcome(&r, &fa), SyncOutcome::Unchanged);
    assert!(rig.note().contains("[[a (conflicted copy 2026-10-13)]]"));

    // Rename on a client, and edit: the file is still found by its root.
    let renamed = rig.env.path("renamed.md");
    std::fs::rename(&fa, &renamed).unwrap();
    edit(&renamed, "] Alpha ", "] Alpha edited ");
    let r = rig.step(&[&fa, &renamed]);
    assert!(r.synced.is_none(), "{r:#?}");
    rig.advance(60);
    let r = rig.step(&[]);
    assert!(is_synced(&Rig::outcome(&r, &renamed)), "{r:#?}");
    assert_eq!(rig.env.export().get(&alpha).unwrap().name, "Alpha edited");
    let note = rig.note();
    assert!(note.contains("[[renamed]]"), "{note}");
    assert!(!note.contains("[[a]]"), "{note}");
}

#[test]
fn a_root_claimed_by_two_files_is_paused() {
    let (env, root, a, _b, file) = one_file();
    let copy = env.path("copy.md");
    write(&copy, &read(&file));
    let mut rig = Rig::new(env);
    rig.env
        .ui("edit", json!({"id": a.as_str(), "text": "Alpha (remote)"}));
    edit(&copy, "] Beta ", "] Beta local ");
    let before = rig.env.snapshot();
    let (file_text, copy_text) = (read(&file), read(&copy));
    let api = TestApi::new(&rig.env.client);
    for _ in 0..3 {
        step_with(&mut rig.d, &api, &[&copy]);
        rig.advance(60);
    }
    assert_eq!(api.mutations(), 0);
    assert_eq!(api.exports.load(std::sync::atomic::Ordering::SeqCst), 0);
    assert_eq!(rig.env.snapshot(), before);
    assert_eq!(read(&file), file_text);
    assert_eq!(read(&copy), copy_text);
    let note = rig.note();
    let short = short(&root).to_string();
    assert!(
        note.contains(&format!("[[copy]], [[root]] all name root `{short}`")),
        "{note}"
    );
    assert!(
        note.contains("paused: several files name this root"),
        "{note}"
    );

    drop(api);

    // Removing the copy resumes the root.
    std::fs::remove_file(&copy).unwrap();
    rig.step(&[&copy]);
    rig.advance(60);
    rig.step(&[]);
    assert!(read(&file).contains("Alpha (remote)"));
    assert!(
        !rig.note().contains("paused: several files"),
        "{}",
        rig.note()
    );
}

#[test]
fn a_missing_file_pauses_its_root_and_deletes_nothing() {
    let (env, root, _a, _b, file) = one_file();
    let mut rig = Rig::new(env);
    let r = rig.step(&[]);
    assert_eq!(Rig::outcome(&r, &file), SyncOutcome::Unchanged);
    std::fs::remove_file(&file).unwrap();
    let before = rig.env.snapshot();
    let api = TestApi::new(&rig.env.client);
    step_with(&mut rig.d, &api, &[&file]);
    for _ in 0..15 {
        rig.advance(60);
        step_with(&mut rig.d, &api, &[]);
    }
    assert_eq!(api.mutations(), 0);
    assert_eq!(rig.env.snapshot(), before);
    assert!(!file.exists(), "the file must not be re-rendered");
    let note = rig.note();
    let short = short(&root).to_string();
    assert!(
        note.contains(&format!(
            "| (none) | Root `{short}` | 2026-09-21 14:13 UTC | 1 | paused: no file names this root; nothing is deleted |"
        )),
        "{note}"
    );
}

// ---------------------------------------------------------------------------------------
// Debounce (guard 4)
// ---------------------------------------------------------------------------------------

#[test]
fn edits_are_pushed_only_after_the_quiet_period() {
    let (env, _root, a, _b, file) = one_file();
    let mut rig = Rig::with(env, |c| {
        c.debounce = Duration::from_secs(45);
        c.min_export_interval = Duration::from_secs(10);
        c.remote_poll = Duration::from_secs(3600);
    });
    rig.step(&[]);
    let start = rig.time.clone();
    let name = |rig: &Rig| rig.env.export().get(&a).unwrap().name.clone();

    rig.advance(5);
    edit(&file, "] Alpha ", "] Alpha 1 ");
    assert!(rig.step(&[&file]).synced.is_none());
    rig.advance(30); // t = 35: still typing
    edit(&file, "] Alpha 1 ", "] Alpha 12 ");
    assert!(rig.step(&[&file]).synced.is_none());
    use syncflowy::daemon::Time;
    assert_eq!(
        rig.d.next_wakeup(),
        start.now() + Duration::from_secs(45),
        "ready 45 s after the last change"
    );
    rig.advance(40); // t = 75: 40 s since the last change
    assert!(rig.step(&[]).synced.is_none());
    assert_eq!(name(&rig), "Alpha");
    rig.advance(5); // t = 80
    let r = rig.step(&[]);
    assert!(is_synced(&Rig::outcome(&r, &file)), "{r:#?}");
    assert_eq!(name(&rig), "Alpha 12");

    // A file event with unchanged content does not restart anything; the next cycle
    // waits for the poll.
    rig.advance(20);
    assert!(rig.step(&[&file]).synced.is_none());
}

#[test]
fn an_edit_made_during_a_cycle_gets_its_full_quiet_period() {
    // A cycle can take tens of seconds (an export, rate-limited writes). An edit saved
    // while it runs must be debounced from when it was seen, not from the cycle's start
    // (guard 4).
    let (env, _root, a, _b, file) = one_file();
    let mut rig = Rig::with(env, |c| {
        c.debounce = Duration::from_secs(45);
        c.min_export_interval = Duration::from_secs(60);
        c.remote_poll = Duration::from_secs(60);
    });
    let time = rig.time.clone();
    let edited = file.clone();
    let api = TestApi::with_export_hook(&rig.env.client, move |n| {
        if n == 1 {
            // The second cycle's export takes 40 s; the user saves meanwhile.
            time.advance(Duration::from_secs(40));
            edit(&edited, "] Alpha ", "] Alpha typ ");
        }
    });
    step_with(&mut rig.d, &api, &[]); // t = 0
    rig.time.advance(Duration::from_secs(60));
    step_with(&mut rig.d, &api, &[]); // t = 60; the cycle ends at t = 100
    assert!(read(&file).contains("] Alpha typ "));
    let name = |rig: &Rig| rig.env.export().get(&a).unwrap().name.clone();
    use syncflowy::daemon::Time;
    let edited_at = rig.time.now();
    assert_eq!(
        rig.d.next_wakeup(),
        edited_at + Duration::from_secs(20),
        "the poll at t = 120 comes first"
    );
    rig.time.advance(Duration::from_secs(20));
    let r = step_with(&mut rig.d, &api, &[]); // t = 120: 20 s after the edit
    assert!(r.synced.is_some(), "the poll cycle runs: {r:#?}");
    assert!(
        r.results.iter().all(|(f, _)| f != &file),
        "the file is still settling: {r:#?}"
    );
    assert_eq!(name(&rig), "Alpha");
    rig.time.advance(Duration::from_secs(60));
    let r = step_with(&mut rig.d, &api, &[]); // t = 180
    assert!(is_synced(&Rig::outcome(&r, &file)), "{r:#?}");
    assert_eq!(name(&rig), "Alpha typ");
}

#[test]
fn remote_changes_are_polled_without_file_events() {
    let (env, _root, a, _b, file) = one_file();
    let mut rig = Rig::new(env);
    rig.step(&[]);
    rig.env
        .ui("edit", json!({"id": a.as_str(), "text": "Alpha (remote)"}));
    rig.advance(30);
    assert!(rig.step(&[]).synced.is_none());
    assert!(!read(&file).contains("Alpha (remote)"));
    rig.advance(30);
    let r = rig.step(&[]);
    assert!(is_synced(&Rig::outcome(&r, &file)), "{r:#?}");
    assert!(read(&file).contains("Alpha (remote)"));
}

#[test]
fn paused_and_forced_files() {
    let (env, _root, a, _b, file) = one_file();
    let mut rig = Rig::new(env);
    rig.step(&[]);
    edit(
        &file,
        "syncflowy: {gen: 1}",
        "syncflowy: {gen: 1, paused: true}",
    );
    edit(&file, "] Alpha ", "] Alpha local ");
    rig.step(&[&file]);
    rig.run_for(180);
    assert_eq!(rig.env.export().get(&a).unwrap().name, "Alpha");
    assert!(rig.note().contains("paused (`paused: true`)"));

    // force: true is a one-shot control: synced, then cleared in the file.
    edit(
        &file,
        "syncflowy: {gen: 1, paused: true}",
        "syncflowy: {gen: 1, force: true}",
    );
    rig.step(&[&file]);
    rig.run_for(60);
    assert_eq!(rig.env.export().get(&a).unwrap().name, "Alpha local");
    let text = read(&file);
    assert!(!text.contains("force"), "{text}");
}

// ---------------------------------------------------------------------------------------
// Deferred deletes (guard 2)
// ---------------------------------------------------------------------------------------

#[test]
fn a_deferred_delete_fires_after_the_grace_period() {
    let (env, root, a, _b, file) = one_file();
    let mut rig = Rig::new(env);
    rig.step(&[]);
    rig.advance(1);
    remove_line(&file, &a);
    rig.step(&[&file]);
    rig.advance(59); // t = 60: ready, and the poll
    let r = rig.step(&[]);
    assert!(is_synced(&Rig::outcome(&r, &file)), "{r:#?}");
    assert!(rig.env.export().contains(&a), "deferred, not sent");
    assert!(
        !read(&file).contains("Alpha"),
        "pending deletes are not rendered back"
    );
    let note = rig.note();
    assert!(
        note.contains(
            "- [[root]]: \"Alpha\", removed 2026-09-21 14:14 UTC, deleted at the first cycle after 2026-09-21 14:24 UTC with the file unchanged"
        ),
        "{note}"
    );
    rig.run_for(540); // t = 600
    assert!(rig.env.export().contains(&a));
    rig.run_for(60); // t = 660: the grace period is over
    assert!(!rig.env.export().contains(&a));
    assert_eq!(child_names(&rig.env.export(), &root), ["Beta"]);
    let note = rig.note();
    assert!(note.contains("## Pending deletes\n\nNone."), "{note}");
    assert!(note.contains("deleted 1 item(s) in Workflowy"), "{note}");
}

#[test]
fn a_deferred_delete_is_cancelled_when_the_line_comes_back() {
    let (env, _root, a, _b, file) = one_file();
    let mut rig = Rig::new(env);
    rig.step(&[]);
    let original = read(&file);
    remove_line(&file, &a);
    rig.step(&[&file]);
    rig.run_for(60);
    assert!(rig.note().contains("\"Alpha\", removed"));
    // An undo on a client brings the line back.
    write(&file, &original);
    rig.step(&[&file]);
    rig.run_for(900);
    assert!(rig.env.export().contains(&a));
    assert!(read(&file).contains("Alpha"));
    let note = rig.note();
    assert!(note.contains("## Pending deletes\n\nNone."), "{note}");
    assert!(note.contains("the pending delete of \"Alpha\""), "{note}");
}

/// Two roots in two files: `Root A` (Alpha, Beta) in a.md and `Root B` (Gamma) in b.md.
fn two_files() -> (Env, NodeId, NodeId, NodeId, PathBuf, PathBuf) {
    let env = Env::new();
    let ra = env.ui_create(None, "Root A");
    let alpha = env.ui_create(Some(&ra), "Alpha");
    env.ui_create(Some(&ra), "Beta");
    let rb = env.ui_create(None, "Root B");
    env.ui_create(Some(&rb), "Gamma");
    let fa = env.path("a.md");
    let fb = env.path("b.md");
    env.render(&ra, &fa);
    env.render(&rb, &fb);
    (env, ra, rb, alpha, fa, fb)
}

fn paste_after_gamma(fb: &Path, line: &str) {
    let text = read(fb);
    let gamma = text
        .lines()
        .find(|l| l.contains("Gamma"))
        .unwrap()
        .to_owned();
    edit(fb, &format!("{gamma}\n"), &format!("{gamma}\n{line}\n"));
}

#[test]
fn a_cut_and_paste_into_another_file_becomes_a_move() {
    let (env, ra, rb, alpha, fa, fb) = two_files();
    let mut rig = Rig::new(env);
    rig.step(&[]);
    let line = remove_line(&fa, &alpha);
    rig.step(&[&fa]);
    rig.run_for(60);
    assert!(rig.env.export().contains(&alpha));
    // The paste arrives later (split arrival).
    rig.advance(120);
    paste_after_gamma(&fb, &line);
    rig.step(&[&fb]);
    rig.run_for(60);
    let tree = rig.env.export();
    assert_eq!(tree.parent(&alpha), Some(&rb), "moved, not deleted");
    assert_eq!(child_names(&tree, &rb), ["Gamma", "Alpha"]);
    assert_eq!(child_names(&tree, &ra), ["Beta"]);
    rig.run_for(900);
    let tree = rig.env.export();
    assert_eq!(tree.parent(&alpha), Some(&rb));
    assert!(read(&fb).contains("Alpha"));
    assert!(!read(&fa).contains("Alpha"));
    assert!(rig.note().contains("## Pending deletes\n\nNone."));
}

#[test]
fn due_deletes_are_held_while_another_file_settles() {
    let (env, _ra, rb, alpha, fa, fb) = two_files();
    let mut rig = Rig::new(env);
    rig.step(&[]);
    rig.advance(1);
    let line = remove_line(&fa, &alpha);
    rig.step(&[&fa]);
    rig.advance(59);
    rig.step(&[]); // t = 60: pending, due at 660
    rig.run_for(570); // t = 630
    // The paste lands 30 s before the next cycle; at t = 660 the delete is due but b.md
    // is still settling.
    paste_after_gamma(&fb, &line);
    rig.step(&[&fb]);
    rig.advance(30);
    let r = rig.step(&[]); // t = 660
    assert!(r.synced.is_some());
    assert!(rig.env.export().contains(&alpha), "held");
    assert!(
        rig.note()
            .contains("Held: another synced file has edits waiting")
    );
    rig.run_for(60); // t = 720: b.md is ready
    let tree = rig.env.export();
    assert_eq!(tree.parent(&alpha), Some(&rb));
    rig.run_for(900);
    assert_eq!(rig.env.export().parent(&alpha), Some(&rb));
}

#[test]
fn a_braked_file_is_retried_but_does_not_hold_other_deletes() {
    let (env, _ra, rb, alpha, fa, fb) = two_files();
    let mut rig = Rig::new(env);
    rig.step(&[]);
    rig.advance(1);
    remove_line(&fa, &alpha);
    // Emptying b.md engages the empty-file brake (§8.3).
    let text = read(&fb);
    let gamma = text
        .lines()
        .find(|l| l.contains("Gamma"))
        .unwrap()
        .to_owned();
    edit(&fb, &format!("{gamma}\n"), "");
    rig.step(&[&fa, &fb]);
    rig.advance(59);
    let r = rig.step(&[]); // t = 60
    assert!(
        matches!(Rig::outcome(&r, &fb), SyncOutcome::Braked(_)),
        "{r:#?}"
    );
    // Retried at every cycle (no new debounce), so it never counts as settling.
    rig.advance(60);
    let r = rig.step(&[&fb]);
    assert!(matches!(Rig::outcome(&r, &fb), SyncOutcome::Braked(_)));
    rig.run_for(600); // t = 720
    assert!(
        !rig.env.export().contains(&alpha),
        "not held by the braked file"
    );
    assert_eq!(child_names(&rig.env.export(), &rb), ["Gamma"]);
    let note = rig.note();
    assert_eq!(note.matches("refused by a brake").count(), 2, "{note}");
    assert!(note.contains("| [[b]] | Root B"), "{note}");
    // The daemon has no `--force`: its messages name the frontmatter key.
    assert!(!note.contains("--force"), "{note}");
}

// ---------------------------------------------------------------------------------------
// Guard 3 and guard 6
// ---------------------------------------------------------------------------------------

#[test]
fn a_stale_echo_is_not_pushed() {
    let (env, _root, a, _b, file) = one_file();
    env.ui("edit", json!({"id": a.as_str(), "text": "Alpha (remote)"}));
    let mut rig = Rig::new(env);
    rig.step(&[]); // renders gen 2 with the remote value
    let text = read(&file);
    assert!(
        text.contains("{gen: 2}") && text.contains("Alpha (remote)"),
        "{text}"
    );
    // A text merge on a client puts gen 1's value back into the gen-2 file.
    edit(&file, "] Alpha (remote) ", "] Alpha ");
    rig.advance(5);
    rig.step(&[&file]);
    rig.run_for(60);
    assert_eq!(rig.env.export().get(&a).unwrap().name, "Alpha (remote)");
    assert!(read(&file).contains("] Alpha (remote) "), "re-rendered");
    let note = rig.note();
    assert!(
        note.contains("equals an older render; ignored as a stale echo"),
        "{note}"
    );
}

#[test]
fn a_resurrection_by_a_merge_is_dropped() {
    let (env, root, a, _b, file) = one_file();
    let gen1 = read(&file);
    let alpha_line = line_of(&gen1, &a).to_owned();
    env.ui("complete", json!({"id": a.as_str()}));
    let mut rig = Rig::new(env);
    rig.step(&[]); // gen 2: Alpha left the file
    let text = read(&file);
    assert!(!text.contains("Alpha"), "{text}");
    // A bad merge re-inserts the line under the gen-2 frontmatter.
    let beta = text
        .lines()
        .find(|l| l.contains("Beta"))
        .unwrap()
        .to_owned();
    edit(
        &file,
        &format!("{beta}\n"),
        &format!("{beta}\n\t{alpha_line}\n"),
    );
    rig.advance(5);
    rig.step(&[&file]);
    rig.run_for(60);
    let tree = rig.env.export();
    let alpha = tree.get(&a).unwrap();
    assert!(alpha.completed, "still completed");
    assert_eq!(tree.parent(&a), Some(&root), "not moved");
    assert!(!read(&file).contains("Alpha"));
    let note = rig.note();
    assert!(
        note.contains("is hidden in Workflowy and was not in the base; dropped"),
        "{note}"
    );
}

#[test]
fn a_tasks_done_date_is_stripped_before_completion() {
    let (env, _root, a, b, file) = one_file();
    let mut rig = Rig::new(env);
    rig.step(&[]);
    // The Tasks plugin appends the done date when the box is ticked, before the tag or
    // after it.
    edit(&file, "- [ ] Alpha ^", "- [x] Alpha ✅ 2026-10-13 ^");
    let text = read(&file);
    let bl = line_of(&text, &b).to_owned();
    edit(&file, &bl, &format!("- [x] {} ✅ 2026-10-13", &bl[6..]));
    rig.step(&[&file]);
    rig.run_for(60);
    let tree = rig.env.export();
    for (id, name) in [(&a, "Alpha"), (&b, "Beta")] {
        let n = tree.get(id).unwrap();
        assert!(n.completed, "{n:?}");
        assert_eq!(n.name, name);
    }
}

// ---------------------------------------------------------------------------------------
// The status note
// ---------------------------------------------------------------------------------------

#[test]
fn status_note_content() {
    let env = Env::new();
    let ra = env.ui_create(None, "Projects");
    let alpha = env.ui_create(Some(&ra), "Alpha");
    let beta = env.ui_create(Some(&ra), "Beta");
    let rb = env.ui_create(None, "Inbox");
    env.ui_create(Some(&rb), "Gamma");
    let rc = env.ui_create(None, "Archive");
    env.ui_create(Some(&rc), "Old");
    let rd = env.ui_create(None, "Shared");
    env.ui_create(Some(&rd), "Delta");
    let fa = env.path("Projects.md");
    let fb = env.path("notes/Inbox.md");
    let fc = env.path("Archive.md");
    let fd = env.path("Shared.md");
    env.render(&ra, &fa);
    env.render(&rb, &fb);
    env.render(&rc, &fc);
    env.render(&rd, &fd);
    write(&env.path("Shared copy.md"), &read(&fd));
    write(&env.path("Projects (Conflict 2026-09-21).md"), &read(&fa));
    let mut rig = Rig::new(env);
    rig.step(&[]);

    // A conflict on Beta, a pending delete of Alpha, a missing file, safe mode.
    rig.env.ui(
        "edit",
        json!({"id": beta.as_str(), "text": "Beta (theirs)"}),
    );
    edit(&fa, "] Beta ", "] Beta (mine) ");
    remove_line(&fa, &alpha);
    std::fs::remove_file(&fc).unwrap();
    // A client wrote Inbox without its generation: safe mode.
    edit(&fb, "syncflowy: {gen: 1}\n", "");
    rig.advance(10);
    rig.step(&[&fa, &fb, &fc]);
    rig.run_for(60);
    let note = rig.note();
    insta::assert_snapshot!(note);
    assert!(rig.env.export().contains(&alpha));
}

/// The status note's Problems section.
fn problems(note: &str) -> String {
    let start = note.find("## Problems\n").expect("a Problems section");
    let rest = &note[start..];
    let end = rest[3..].find("\n## ").map_or(rest.len(), |i| i + 3);
    rest[..end].to_owned()
}

#[test]
fn a_file_error_leaves_the_status_note_with_the_file() {
    let (env, _root, _a, _b, _file) = one_file();
    let ghost = env.path("ghost.md");
    write(&ghost, "---\nworkflowy: 0123456789ab\n---\n- x\n");
    let mut rig = Rig::new(env);
    rig.step(&[]);
    rig.run_for(60);
    let p = problems(&rig.note());
    assert!(p.contains("- [[ghost]]: "), "{p}");

    // Renamed: the error now belongs to the new path only.
    let renamed = rig.env.path("renamed ghost.md");
    std::fs::rename(&ghost, &renamed).unwrap();
    rig.step(&[&ghost, &renamed]);
    rig.run_for(60);
    let p = problems(&rig.note());
    assert!(p.contains("- [[renamed ghost]]: "), "{p}");
    assert!(!p.contains("[[ghost]]"), "{p}");

    // Deleted: no error is left behind.
    std::fs::remove_file(&renamed).unwrap();
    rig.advance(1);
    rig.step(&[&renamed]);
    let p = problems(&rig.note());
    assert_eq!(p, "## Problems\n\nNone.\n", "{p}");
}

/// An `Api` that fails its exports on demand.
struct FlakyApi<'a> {
    inner: &'a syncflowy::wfclient::Client,
    /// 0: pass through, 1: 503, 2: 401.
    mode: std::cell::Cell<u8>,
}

impl FlakyApi<'_> {
    fn fail(&self) -> Option<syncflowy::wfclient::ApiError> {
        use syncflowy::wfclient::ApiError;
        match self.mode.get() {
            1 => Some(ApiError::Server {
                status: 503,
                body: "unavailable".into(),
            }),
            2 => Some(ApiError::Unauthorized),
            _ => None,
        }
    }
}

impl Api for FlakyApi<'_> {
    fn export(&self) -> Result<Vec<syncflowy::tree::Node>, syncflowy::wfclient::ApiError> {
        match self.fail() {
            Some(e) => Err(e),
            None => self.inner.export(),
        }
    }
    fn get(&self, id: &NodeId) -> Result<syncflowy::tree::Node, syncflowy::wfclient::ApiError> {
        self.inner.get(id)
    }
    fn get_short(
        &self,
        id: &syncflowy::tree::ShortId,
    ) -> Result<syncflowy::tree::Node, syncflowy::wfclient::ApiError> {
        self.inner.get_short(id)
    }
    fn list(
        &self,
        parent: Option<&NodeId>,
    ) -> Result<Vec<syncflowy::tree::Node>, syncflowy::wfclient::ApiError> {
        self.inner.list(parent)
    }
    fn create(
        &self,
        req: &syncflowy::wfclient::CreateRequest,
    ) -> Result<NodeId, syncflowy::wfclient::ApiError> {
        self.inner.create(req)
    }
    fn update(
        &self,
        id: &NodeId,
        req: &syncflowy::wfclient::UpdateRequest,
    ) -> Result<(), syncflowy::wfclient::ApiError> {
        self.inner.update(id, req)
    }
    fn move_node(
        &self,
        id: &NodeId,
        parent: &NodeId,
        position: syncflowy::tree::Position,
    ) -> Result<(), syncflowy::wfclient::ApiError> {
        self.inner.move_node(id, parent, position)
    }
    fn complete(&self, id: &NodeId) -> Result<(), syncflowy::wfclient::ApiError> {
        self.inner.complete(id)
    }
    fn uncomplete(&self, id: &NodeId) -> Result<(), syncflowy::wfclient::ApiError> {
        self.inner.uncomplete(id)
    }
    fn delete(&self, id: &NodeId) -> Result<(), syncflowy::wfclient::ApiError> {
        self.inner.delete(id)
    }
    fn request_count(&self) -> u64 {
        self.inner.request_count()
    }
}

#[test]
fn api_errors_are_reported_and_a_401_stops_the_daemon() {
    let (env, _root, a, _b, file) = one_file();
    let mut rig = Rig::new(env);
    let api = FlakyApi {
        inner: &rig.env.client,
        mode: std::cell::Cell::new(1),
    };
    step_with(&mut rig.d, &api, &[]);
    // A failed export is reported, never fatal; the next poll retries.
    api.mode.set(1);
    rig.env
        .ui("edit", json!({"id": a.as_str(), "text": "Alpha (remote)"}));
    rig.time.advance(Duration::from_secs(60));
    let r = step_with(&mut rig.d, &api, &[]);
    assert_eq!(r.results.len(), 1);
    assert!(r.results.iter().all(|(_, res)| res.is_err()), "{r:#?}");
    let note = rig.note();
    assert!(
        note.contains(
            "## API errors\n\n- 2026-09-21 14:14 UTC the cycle failed: API: server error 503"
        ),
        "{note}"
    );
    assert!(
        note.contains("- All files: API: server error 503"),
        "{note}"
    );
    api.mode.set(0);
    rig.time.advance(Duration::from_secs(60));
    step_with(&mut rig.d, &api, &[]);
    assert!(read(&file).contains("Alpha (remote)"));
    assert!(!rig.note().contains("- All files:"));
    // 401: fatal.
    api.mode.set(2);
    rig.time.advance(Duration::from_secs(60));
    let changed: Vec<PathBuf> = Vec::new();
    let err = rig.d.step(&api, &changed).expect_err("401 is fatal");
    assert!(syncflowy::daemon::is_fatal(&err), "{err}");
}

/// The status note's Roots section.
fn roots(note: &str) -> String {
    let start = note.find("## Roots\n").expect("a Roots section");
    let rest = &note[start..];
    let end = rest[3..].find("\n## ").map_or(rest.len(), |i| i + 3);
    rest[..end].to_owned()
}

#[test]
fn a_run_wide_failure_shows_in_every_root_row() {
    let (env, _root, _a, _b, _file) = one_file();
    let mut rig = Rig::new(env);
    let api = FlakyApi {
        inner: &rig.env.client,
        mode: std::cell::Cell::new(1),
    };
    step_with(&mut rig.d, &api, &[]);
    let r = roots(&rig.note());
    assert!(r.contains("| error: API: server error 503"), "{r}");
    assert!(!r.contains("not synced yet"), "{r}");
    // Once a cycle succeeds the row is fine again.
    api.mode.set(0);
    rig.time.advance(Duration::from_secs(60));
    step_with(&mut rig.d, &api, &[]);
    let r = roots(&rig.note());
    assert!(r.contains("| ok |"), "{r}");
}

#[test]
fn a_file_without_a_checkpoint_says_how_to_adopt_it() {
    let env = Env::new();
    let root = env.ui_create(None, "Root");
    env.ui_create(Some(&root), "Alpha");
    let file = env.path("root.md");
    let mut rig = Rig::new(env);
    write(&file, &format!("---\nworkflowy: {root}\n---\n- Alpha\n"));
    rig.step(&[]);
    rig.run_for(60);
    let note = rig.note();
    let r = roots(&note);
    assert!(
        r.contains(
            "| not initialised: run `syncflowy render`, or set `force: true` to adopt the file |"
        ),
        "{r}"
    );
    assert!(
        problems(&note).contains("set `force: true` in the frontmatter to adopt the file"),
        "{note}"
    );
    assert!(!note.contains("--force"), "{note}");

    // `force: true` adopts it.
    write(
        &file,
        &format!("---\nworkflowy: {root}\nsyncflowy: {{force: true}}\n---\n- Alpha\n"),
    );
    rig.advance(1);
    rig.step(&[&file]);
    rig.run_for(120);
    let r = roots(&rig.note());
    assert!(r.contains("| ok |"), "{r}");
}

// ---------------------------------------------------------------------------------------
// Stopping and restarting
// ---------------------------------------------------------------------------------------

#[test]
fn stop_is_noted_and_the_event_log_survives_a_restart() {
    let (env, ..) = one_file();
    let ghost = env.path("ghost.md");
    write(&ghost, "---\nworkflowy: 0123456789ab\n---\n- x\n");
    let mut rig = Rig::new(env);
    rig.step(&[]);
    rig.run_for(60);
    let note = rig.note();
    assert!(note.contains("[[ghost]]: sync root"), "{note}");
    assert!(!note.contains("Stopped at"), "{note}");
    rig.advance(30);
    rig.d.stop().unwrap();
    let note = rig.note();
    assert!(
        note.contains("Stopped at 2026-09-21 14:14 UTC: nothing is synced"),
        "{note}"
    );
    assert!(
        note.contains("- 2026-09-21 14:14 UTC the daemon stopped\n"),
        "{note}"
    );

    // A new daemon over the same state directory starts with the old events.
    let cfg = rig.d.config().clone();
    rig.d = Daemon::new(cfg, rig.time.clone()).unwrap();
    rig.advance(30);
    rig.step(&[]);
    let note = rig.note();
    assert!(!note.contains("Stopped at"), "{note}");
    assert!(note.contains("the daemon stopped"), "{note}");
    assert!(note.contains("[[ghost]]: sync root"), "{note}");
}

/// `syncflowy daemon` over `env`'s vault, polling every second, logging to `log`.
fn spawn_daemon(env: &Env, log: &Path) -> std::process::Child {
    std::process::Command::new(env!("CARGO_BIN_EXE_syncflowy"))
        .args(["--api-url", env.mock.base_url(), "--state-dir"])
        .arg(&env.cfg.state_dir)
        .args(["daemon", "--vault"])
        .arg(env.dir.path().join("vault"))
        .args(["--debounce", "0", "--poll", "1"])
        .args(["--min-export-interval", "0"])
        .env("WORKFLOWY_API_KEY", env.mock.api_key())
        .env_remove("WORKFLOWY_API_URL")
        .env_remove("SYNCFLOWY_STATE_DIR")
        .stdout(std::process::Stdio::null())
        .stderr(std::fs::File::create(log).unwrap())
        .spawn()
        .expect("spawn syncflowy daemon")
}

/// Wait up to 30 s for `cond` (this test runs the real binary, so it has to sleep).
fn wait_for(what: &str, log: &Path, mut cond: impl FnMut() -> bool) {
    let t = std::time::Instant::now();
    while !cond() {
        assert!(
            t.elapsed() < Duration::from_secs(30),
            "timed out waiting for {what}; daemon log:\n{}",
            std::fs::read_to_string(log).unwrap_or_default()
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Send `signal` (`TERM`, `INT`) to `child` and return its exit code.
fn signal_and_wait(child: &mut std::process::Child, signal: &str, log: &Path) -> Option<i32> {
    let ok = std::process::Command::new("kill")
        .args([&format!("-{signal}"), &child.id().to_string()])
        .status()
        .unwrap()
        .success();
    assert!(ok, "kill -{signal}");
    let mut status = None;
    wait_for("the daemon to exit", log, || {
        status = child.try_wait().unwrap();
        status.is_some()
    });
    status.unwrap().code()
}

#[test]
fn the_daemon_binary_stops_cleanly_on_sigterm_and_sigint() {
    let (env, ..) = one_file();
    let note_path = env.dir.path().join("vault").join(STATUS_NOTE);
    let log = env.dir.path().join("daemon.log");
    let note = || std::fs::read_to_string(&note_path).unwrap_or_default();

    let mut child = spawn_daemon(&env, &log);
    wait_for("a first cycle", &log, || note().contains("| ok |"));
    assert_eq!(signal_and_wait(&mut child, "TERM", &log), Some(0));
    let text = note();
    assert!(text.contains("\nStopped at "), "{text}");
    assert_eq!(text.matches("the daemon stopped").count(), 1, "{text}");
    assert!(env.cfg.state_dir.join("daemon-events.json").is_file());

    // Restarted: the note is rewritten and keeps the earlier stop in its events.
    let mut child = spawn_daemon(&env, &log);
    wait_for("the note to be rewritten", &log, || {
        let t = note();
        t.contains("| ok |") && !t.contains("Stopped at")
    });
    assert!(note().contains("the daemon stopped"));
    assert_eq!(signal_and_wait(&mut child, "INT", &log), Some(0));
    let text = note();
    assert!(text.contains("\nStopped at "), "{text}");
    assert_eq!(text.matches("the daemon stopped").count(), 2, "{text}");
}
