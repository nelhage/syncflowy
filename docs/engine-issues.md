# Sync engine: known issues and recommended fixes

Bugs, mismatches with the design, and recommended improvements in the sync engine (`engine/`), gathered on
2026-10-15 while writing the user guide. Three explorers ran the release binary (built from `0ee40e0`; the engine
source is unchanged at `c25a0c3`, where the line numbers were taken) and recorded fact sheets, and a reviewer
fact-checked the guide. Each item was then checked against the source, and the ambiguous ones were run again.
**Everything was observed against the mock server (`syncflowy-mock`), never against production Workflowy.**
The user guide (`docs/user-guide/`, in a separate pull request) documents these behaviours as they are today: when
you fix an item, search it and `engine/README.md` for the quoted output.

## 1. Bugs

Most severe first: risks to data and silently wrong results, then misleading output, then cosmetic issues.

1. **The `(+N hidden)` count leaves out descendants hidden by `depth:`, although they are deleted.**
   - What happens: with `depth: 1`, deleting the line `- Parent` (children `Child` > `Grandchild`, hidden by
     depth) deleted all three nodes. The dry run said `would Delete { ..., hidden_descendants: 0, mirror: false }`
     and the sync said `1 deleted (+0 hidden)` and `op 0: delete ... (+0 hidden): done`. The brake counts such
     nodes correctly (a line with 60 depth-hidden children was refused with `refusing to delete 1 visible and
     60 hidden nodes out of 2 (use --force)`). The daemon's `deleted 1 item(s) in Workflowy (with 0 hidden)` is
     wrong the same way, while its pending-delete entry has the right count.
   - Where: the merge computes the right number for each delete (`engine/src/merge/engine.rs:1983-1990`, stored
     in `DeleteIntent::hidden_descendants`). The planner ignores it and counts again on the remote tree with
     `Tree::count_hidden_descendants` (`engine/src/plan.rs:876-883`), which only knows completion: "visible" there
     means "not completed" (`engine/src/tree.rs:947-982`). The report line (`engine/src/main.rs:152`), the op log
     and the daemon event (`engine/src/daemon.rs:888-896`) all use the planner's number.
   - Fix: in `Planner::deletes`, take `d.hidden_descendants` from the merge for ordinary deletes (keep the
     separate count for mirror shells). Add a test to `engine/tests/sync_scenarios.rs`: a `depth: 1` file, delete a
     line with two depth-hidden descendants, and assert `hidden_descendants: 2` in the dry-run op and
     `hidden_deleted == 2` in the summary.

2. **A line with two id tags can lose its identity, and its node is deleted.**
   - What happens: the line `1. [ ] first step %%wf:308cdc6b13ab%% ^wf-17e3cc0e95ee` (its last tag also on the next
     line) gave `line 19: 2 id tags on one line; the last one wins` and
     `id tag 17e3cc0e95ee appears on lines 19, 20; line 20 keeps it, the others are new items`. "first step" was
     created as a new node and the original `308cdc6b13ab` was deleted (to the Trash, with its history and any
     Workflowy-only attributes), although the line also carried that node's own, unclaimed tag.
   - Where: the parser keeps the earlier tags in `extra_tags` (`engine/src/mdblock/parse.rs:723-733`).
     `Engine::identify` falls back to them only when the last tag is invalid (`engine/src/merge/engine.rs:636-646`),
     not when the line loses the duplicate contest in `resolve_duplicate` (`:693`).
   - Fix: in `resolve_duplicate`, give each losing item its next `extra_tags` candidate that no other item claims,
     and repeat until stable; only then make it a new item. Test in `engine/tests/merge_fixtures.rs` (or
     `sync_scenarios.rs`): the case above keeps both ids and deletes nothing.

3. **A top-level `paused:` or `force:` in the frontmatter is silently ignored.**
   - What happens: a file with `paused: true` on its own frontmatter line (not inside `syncflowy:`) was not paused:
     `sync --dry-run` planned its edits as usual, so a real sync pushes what the user meant to hold back. There is
     no warning, while an unknown key inside `syncflowy:` gets one (`unknown syncflowy key "colour" ...`).
   - Where: every top-level line other than `workflowy:` and `syncflowy:` is kept verbatim as `other`
     (`engine/src/mdblock/frontmatter.rs:414-417`).
   - Fix: in that branch, when the key is a `syncflowy:` option name (`headings`, `ids`, `checkboxes`, `dates`,
     `depth`, `paused`, `force`, `gen`), add a `ParseWarning` such as
     `line 3: "paused" is a syncflowy option; write it as syncflowy: {paused: true}`. Consider honouring a top-level
     `paused: true`, since pausing is the safe direction, but never a top-level `force`. Test in
     `engine/tests/mdblock.rs`.

4. **Changing only `ids:` or `checkboxes:` in the frontmatter has no effect.**
   - What happens: after changing `syncflowy: {gen: 4}` to `syncflowy: {ids: comment, gen: 4}`, `sync` reported
     `file unchanged (gen 4)`, also with `sync --force`. The new tag form appears only when the file is rewritten
     for another reason, or with `force: true` in the frontmatter. `checkboxes: all` never added boxes to existing
     lines.
   - Where: guard 4 in `finish` keeps the file when the new render is `mdblock::equivalent` to it
     (`engine/src/sync.rs:1029-1037`); `equivalent` (`engine/src/mdblock.rs:637`) ignores tag forms and
     checkboxes. The check reads the frontmatter's `force`, not the command line's. `sticky_from`
     (`engine/src/sync.rs:430-442`) then copies every existing box into the new render.
   - Fix: treat a config change as a reason to write. In `finish`, do not keep the file when `fm.config` differs
     from the config of the last write (`Checkpoint::config`, `engine/src/state.rs:300`), and build `sticky` empty
     when `checkboxes` changed. Test in `engine/tests/sync.rs`: change `ids:` alone and assert `file rewritten` with
     `%%wf:` tags; change to `checkboxes: all` and assert that interior items get `[ ]`.

5. **`render --headings N` accepts N above 6 and writes a file that never syncs.**
   - What happens: `render ROOT f.md --headings 7` wrote `syncflowy: {headings: 7, gen: 1}`, and every later `sync`
     reports `skipped: the file does not parse: bad syncflowy config headings: "7"` (exit 1).
   - Where: the flag is any `u8` (`engine/src/main.rs:67-69`, passed on at `:228-231`); the frontmatter parser
     accepts 0 to 6 only (`engine/src/mdblock/frontmatter.rs:295-297`).
   - Fix: `#[arg(long, default_value_t = 0, value_parser = clap::value_parser!(u8).range(0..=6))]`, plus the same
     check in `render_once` for library callers, with one shared constant for the maximum. Test in
     `engine/tests/sync_cli.rs`: `--headings 7` exits 2 and writes no file.

6. **"lost formatting Markdown could not show" is reported when nothing was lost.**
   - What happens: unfencing a two-line code block into `- A1 unfenced` reported
     `the name of "A1 unfenced" (d2803bab6c39) lost formatting Markdown could not show`. A forced edit of the
     read-only two-line name gave the same message (`the name of "Two-line name forced" ...`). Neither value had any
     formatting. A false alarm teaches users to ignore the real one.
   - Where: `Engine::detect` records the local item's field kind (`name_kind: kl`,
     `engine/src/merge/engine.rs:1009-1017`) and `merge_name` passes it to `write_value` (`:1343`).
     `write_value` (`engine/src/mdinline.rs:389-415`) round-trips the base value through that new kind. A multi-line
     value never round-trips as a one-line name, so `lost` is set as soon as the three-way merge fails.
   - Fix: when the base kind (`kb`) differs from `kl`, decide `lost` by whether the base value round-trips in `kb`
     (in which case the user saw all of it). For a forced edit of a read-only value, skip the `LossyWrite` event:
     the user saw the `🔒` and forced the edit. Test in `engine/tests/sync_scenarios.rs`: unfence a two-line code
     block and assert there is no `LossyWrite` event.

7. **A cross-file move produces a spurious `verification: Missing` warning.**
   - What happens (re-run for this list): cutting "A two" from `a.md` into `b.md`, renaming another line in `a.md`,
     and running `sync a.md b.md` moved the node correctly but printed
     `verification: Missing(NodeId("35bf992d-c9e9-4616-a12e-7696a6cecc1b"))` for `a.md`, plus the same as a `WARN`
     log line. The daemon logs it twice (`verification: ...` and `verification after the push: ...`). It does not
     appear when the source file pushed nothing else, because only files that applied something are verified.
   - Where: a node whose tag is claimed by another file of the run stays in the source file's merged tree at its
     remote position (`engine/src/merge.rs:151-154`; `apply_deletes`, `engine/src/merge/engine.rs:1883-1895`).
     `sync_all` then verifies that tree against the shared re-export (`engine/src/sync.rs:1749-1756`,
     `engine/src/apply.rs:1246`), where the node is now under the other root.
   - Fix: pass the nodes that another file's plan moved in this run to `verify` and skip `Missing` for them, or
     remove them from the source file's merged tree once the destination's plan contains the move. Test in
     `engine/tests/sync_multi.rs`: the scenario above has no `discrepancies`.

8. **Error messages print their cause twice.**
   - What happens: `error: API: invalid credentials: invalid credentials`,
     `error: parse: bad workflowy: root "not-an-id": bad workflowy: root "not-an-id"`,
     `error: API: transport error: io: Connection refused (os error 111): transport error: io: Connection refused (os error 111)`,
     `error: vault/big.md: entity not found: entity not found`, and from the daemon
     `error: no-such-dir: the vault is not a directory: the vault is not a directory`.
   - Where: the `Error` variants put the inner error in their message (`"API: {0}"`, `"{path}: {source}"`) and also
     expose it as the error's source (`#[from]`, and a field named `source`, which `thiserror` treats as
     `#[source]`) (`engine/src/error.rs:26-56`). `main` prints with `{e:#}` (`engine/src/main.rs:319`), which
     appends the source chain again.
   - Fix: print `{e}` in `main`; every message already includes its cause and `main` adds no `anyhow` context.
     (Or keep `{e:#}` and drop `{0}`/`{source}` from the messages, but per-file errors are printed with `{e}` at
     `engine/src/main.rs:250`.) Test in `engine/tests/sync_cli.rs`: a wrong key gives exactly
     `error: API: invalid credentials`.

9. **`status FILE` fails on a missing file, and misnames a root it has no checkpoint for.**
   - What happens: `status vault/big.md` on a moved file gave `error: vault/big.md: entity not found: entity not
     found` (exit 2), while plain `status` lists the same root with `the file cannot be read`. For a file whose
     root has no checkpoint, it says `error: sync root "cd613e30-..." not found`, which reads as if the root were
     missing from Workflowy.
   - Where: `engine/src/sync.rs:1842-1853`.
   - Fix: for a missing file, print `<file>: missing` and go on with the other files (exit 1). For an unknown root,
     say `<file>: no checkpoint for root ... in this state directory`. Test in `engine/tests/sync_cli.rs`.

10. **`status` creates the state directory, although it only reads.**
    - What happens: `status` with the default state directory left `~/.local/state/syncflowy/roots` behind.
    - Where: `StateStore::open` creates `<dir>/roots` (`engine/src/state.rs:657-664`); `status` calls it
      (`engine/src/sync.rs:1831`).
    - Fix: add a read-only `StateStore::open_existing` that returns `None` when the directory is missing, and use it
      in `status`. Test in `engine/tests/sync_cli.rs`: `status` with a fresh path leaves it absent.

11. **An API key variable that is set but empty counts as a key.**
    - What happens: `WORKFLOWY_API_KEY= syncflowy sync f.md` makes a request and fails with
      `error: API: invalid credentials: invalid credentials` instead of
      `error: no API key: pass --api-key or set WORKFLOWY_API_KEY`.
    - Where: `engine/src/main.rs:124-128`.
    - Fix: `.filter(|k| !k.trim().is_empty())` before `ok_or_else`. Test next to `missing_api_key_is_an_error` in
      `engine/tests/sync_cli.rs`.

12. **Report lines can contain raw line breaks and temp ids.**
    - What happens: `line 5: "Two-line` followed by `name" (e066de26e655) is read-only (or a mirror placeholder);
      the edit was not sent` on the next line; `temp-1 was pushed but is hidden: ...`; and
      `90c78092b4d4 was deleted in Workflowy but edited in Markdown: re-created as a new node (temp-2)`, which names
      neither the node nor its new tag.
    - Where: `node_label` (`engine/src/merge.rs:697-707`) quotes the raw plain text and falls back to the bare id
      for nodes that are not in the remote tree (new nodes, nodes deleted remotely); `ReportEvent::describe`
      (`:550-570`).
    - Fix: replace line breaks with ` ⏎ `, as the renderer does. Describe events after the apply with the `created`
      map, so that a temp id becomes `"a child I added" (2cfa93ea5c4e, new)`; name a remotely deleted node by its base
      name, and say `the original is in Workflowy's Trash`. Extend the unit test
      `describe_names_nodes_by_plain_text` (`engine/src/merge.rs:864`).

13. **Brake outcomes are printed twice.**
    - What happens: `vault/big.md: refused: the file is empty but 12 nodes were visible (use --force)` followed by
      `  warning: brake: the file is empty but 12 nodes were visible (use --force)`.
    - Where: `engine/src/sync.rs:822` adds the brake as a warning as well as returning `SyncOutcome::Braked`, which
      `main` prints as `refused: ...` (`engine/src/main.rs:181`, `:193-195`).
    - Fix: drop the warning. The daemon logs brakes from the outcome (`engine/src/daemon.rs:877-882`), not from the
      warnings. Test in `engine/tests/sync_cli.rs`: one line mentions the brake.

14. **Rust `Debug` output appears in user-facing text, and every conflict is reported twice.**
    - What happens: a conflict prints `conflict on the name of "Carrots (Nantes)" (3d437204e52d): Workflowy's
      version kept, yours is in the file` and then `conflict on the Name of cd447e35-b8b6-48fe-842e-3d437204e52d:
      resolve the marker in the file`. Dry runs print `would Create { temp: NodeId("temp-1"), parent:
      NodeId("..."), name: Direct("Carrots"), note: None, layout: "bullets" }`; verification prints
      `verification: Missing(NodeId("..."))`.
    - Where: `engine/src/main.rs:199-204` (`{:?}` of `ConflictField`), `:163` (dry run), `:206` (verification).
    - Fix: drop the second conflict line (the merge event already says it, by name), or print the field in lower
      case with the node's label. Print dry-run ops with the same wording as the applier's `op N: ...` lines, and
      give `OutlineDiff` a `Display`. Test in `engine/tests/sync_cli.rs`: no `NodeId(` in stdout.

15. **The daemon's Roots table says `not synced yet` during a run-wide failure.**
    - What happens: with the API unreachable, or with a bad key, every row's State reads `not synced yet`; the cause
      is only under Problems as `All files: API: transport error: ...`.
    - Where: run-wide errors are stored under the empty path (`engine/src/daemon.rs:974`), but each row looks up its
      own file's path (`:1034`).
    - Fix: when a row has no error of its own, use the run-wide one (`self.errors.get(Path::new(""))`) as
      `RootState::Failed`. Test in `engine/tests/daemon.rs` with an unreachable URL.

16. **An unparseable `gen` is reported as missing.**
    - What happens: `syncflowy: {gen: three}` gave
      `warning: the file's gen (missing) is unknown: safe mode (no deletes, moves or reorders)`.
    - Where: `"gen" => st.generation = v.parse::<u64>().ok()` (`engine/src/mdblock/frontmatter.rs:317`) discards
      the text; the warning prints `missing` for `None` (`engine/src/sync.rs:689-694`).
    - Fix: keep the raw value of an unparseable `gen` and print `the file's gen ("three") is not a number`. Test in
      `engine/tests/mdblock.rs`.

17. **Developer wording in user-facing text.**
    - What happens: every `--help` describes `--api-url` as `API base URL (the mock server for tests)`, and the
      status note says `Removed from a file but not yet deleted in Workflowy (§9.1 guard 2).`
    - Where: `engine/src/main.rs:39`; `engine/src/daemon/status.rs:290-291`.
    - Fix: `/// API base URL: https://workflowy.com, or https://beta.workflowy.com for mirrors.`; drop
      `(§9.1 guard 2)` and update `engine/tests/snapshots/daemon__status_note_content.snap`.

## 2. Code disagrees with the design or its own messages

1. **Safe mode also drops ticks, but the warning says "no deletes, moves or reorders".**
   - What happens: a sync with `gen: 999` undid a tick (`- [x] Item 13` came back as `- [ ] Item 13`), while the
     warning read `the file's gen (999) is unknown: safe mode (no deletes, moves or reorders)`.
   - Design vs code: design §9.1 guard 1 lists "no deletes, no moves and no reorders". The code also drops ticks
     (`Engine::tick`, `engine/src/merge/engine.rs:1837-1841`) and creates a line typed as `[x]` unticked
     (`:1643-1646`), as its module docs say (`engine/src/merge.rs:199-203`). The warning is at
     `engine/src/sync.rs:690`.
   - Fix: the code's choice is the safer one, so change the message to
     `safe mode (no deletes, moves, reorders or ticks)` and add ticks to design §9.1 guard 1 and `engine/README.md`.
     Keep the words "safe mode": the daemon files warnings by that substring (`engine/src/daemon.rs:900-903`).

2. **Guard 5 holds only within one `sync` command, and the CLI syncs conflict copies.**
   - What happens: `sync "vault/renamed-big (conflict copy).md"` on its own was accepted (`file rewritten (gen 5)`),
     although `vault/renamed-big.md` names the same root. The two were refused only when named in one command.
   - Design vs code: §9.1 guard 5 says two files claiming one root stop that root, and files whose names look like
     conflict copies are never synced. `sync_all` compares only the files of the run
     (`engine/src/sync.rs:1523-1538`), and the name check `is_conflict_copy` is used only by the daemon
     (`engine/src/daemon.rs:386`). The checkpoint already records the file last synced (`engine/src/state.rs:301-304`,
     set at `engine/src/sync.rs:1107`).
   - Fix: in `sync_all`, skip a file that `daemon::is_conflict_copy` matches (`skipped: looks like a sync conflict
     copy`). When the checkpoint's `file` differs from this file's canonical path, and the old path still exists
     and names the same root, pause with `DuplicateRoot([old, new])`. A real rename (old path gone, or naming
     another root) is accepted as today. Tests in `engine/tests/sync_cli.rs`.

3. **Hidden descendants are counted but not named.**
   - What happens: the report says `1 deleted (+1 hidden)`; nothing says which nodes went.
   - Design vs code: §4.5 says "the sync report should name them", and `DeleteIntent`'s own docs say "name them
     in the report" (`engine/src/merge.rs:442-443`). Only the count is kept.
   - Fix: collect the plain-text names of the hidden descendants in `apply_deletes`
     (`engine/src/merge/engine.rs:1983-2003`), and print them under the report line and in the dry run (where they
     matter most): `deleting "Old ideas" also deletes 1 hidden item: "Hedge maze"`, capped at about 10 names with
     `and N more`. Test in `engine/tests/sync_scenarios.rs`.

4. **Lines typed under a placeholder or stub are lifted, with a message that says the opposite.**
   - What happens: a child typed under a `⧉ (empty or mirror)` placeholder was created under the placeholder's
     parent. The only report line was `line 6: "" (6cf376e39b51) is read-only (or a mirror placeholder); the edit
     was not sent`, and a stub on the beta host behaved the same.
   - Design vs code: design §15.1 (`docs/sync-design.md:796`) wants such children kept as a conflict marker and
     reported; the implementation notes (`:862`) narrow this to "lifted to its parent and reported". The code lifts
     them, but reports through `refuse`, which emits one `ReadOnlyEditRefused` per placeholder
     (`create_parent`, `engine/src/merge/engine.rs:1576-1583`; `refuse`, `:431-438`).
   - Fix: add a `ReportEvent::Lifted` for each lifted create, described as
     `line 7: "a child typed under it" was typed under a read-only placeholder; created under "Parent" instead`, and
     keep `ReadOnlyEditRefused` for real edits of the placeholder. Update the test
     `items_typed_under_a_stub_are_lifted_and_reported` (`engine/tests/sync_mirrors.rs:338`), which today only
     checks for the refusal.

5. **The create nonce is in the name, where design §7.3 puts it in the note.**
   - Design vs code: §7.3 (`docs/sync-design.md:417`) and the §11 table ("a nonce note") put the nonce in the
     note. `apply.rs` puts it in the name and explains why: the API cannot write a `null` note back, so a nonce in
     the note would turn every note-less new node's note into `""` (`engine/src/apply.rs:63-69`).
   - Fix: the code is right; update §7.3, the §11 row and the implementation notes. No code change.

6. **Exports do not follow the 60 s minimum interval of §12.2.**
   - What happens: a `sync` that pushes makes two exports a moment apart (the request log shows
     `GET /api/v1/nodes-export` before and after the writes), and the CLI never waits between runs. The daemon
     spaces cycles by `--min-export-interval`, but a cycle that pushes also exports twice.
   - Design vs code: §12.2 recommends export "with a 60 s minimum interval in case the documented limit is ever
     enforced" (the API reference says 1 per minute, `docs/workflowy/upstream-api-reference.md:849`; it was seen
     unenforced once, `docs/workflowy/data-model.md:430`). The second export is the verification re-export
     (`engine/src/sync.rs:1737-1741`); there is a third after journal recovery (`:1631`). The daemon only spaces
     whole cycles (`engine/src/daemon.rs:688-692`). If the limit were enforced, the re-export would get 429s, and
     the client's retries (at most 4, backoff capped at 30 s, `engine/src/wfclient.rs:46-49`) give up within a
     minute, failing the run after its writes.
   - Fix: record the time of the last export in the state directory (shared by the CLI and the daemon) and wait in
     `export_tree` (`engine/src/sync.rs:394`) when it is under 60 s; or skip the verification export in that case
     and let the next cycle's export verify. Honour `Retry-After` on a 429. Test in `engine/tests/wfclient.rs` with
     a fake clock.

7. **Daemon messages tell the user to use `--force`, which the daemon does not have.**
   - What happens: the Roots row shows `error: no checkpoint for root 8d88348a-...; run \`syncflowy render\` first
     or use --force`, and a brake is logged as `... (use --force); set \`force: true\` in the frontmatter to override
     once`. In the daemon only `syncflowy: {force: true}` works. The note's own state for this case, `not
     initialised: run \`syncflowy render\`, or set \`force: true\` to adopt the file`
     (`engine/src/daemon/status.rs:191-194`), shows only before the first cycle, because a stored error wins
     (`engine/src/daemon.rs:1034` is matched before `:1045`).
   - Fix: keep the error's kind in `self.errors`, not only its text, and map `Error::NoCheckpoint` to
     `RootState::NoCheckpoint`. Make the `(use --force)` suffix of `Error::NoCheckpoint`
     (`engine/src/error.rs:61`) and of the brake messages (`engine/src/plan.rs:345-348`) a hint that the CLI and
     the daemon each word for themselves. Test in `engine/tests/daemon.rs`.

## 3. Recommended improvements

Behaviour that is defensible (sometimes what the design asks for) but surprises users.

1. **Moving a line between files depends on how the files are synced.**
   - What happens: a cut and paste from `moves.md` to `other.md`, synced as `sync vault/moves.md` and then
     `sync vault/other.md`, deleted the node and created a new one (`line 8: id tag 67edfe175330 does not belong to
     this file; treated as a new item`). The original went to the Trash with its completed children, colour and
     mirrors. Conversely, copying a tagged line into another file while it stays in the source moves the node, and
     the source line disappears at the next sync; within one file, a copied tag makes a new node.
   - Where: the CLI sends deletes at once; a tag from a root that is not in the run is a foreign tag
     (`engine/src/merge/engine.rs:619-646`).
   - Recommendation: before sending a delete, the CLI could read the files recorded in the other checkpoints of
     the state directory (`Checkpoint::file`). If the deleted node's tag appears in one, it would skip the delete and
     say `"Item E" (67edfe175330) is in other.md; sync both files in one command to move it`. For a copy, treat a tag
     that is still present in its own file in the same run as a duplicate (a new node), as within one file (§4.4).
     Trade-offs: extra file reads, and recorded paths can be stale (a miss only falls back to today's behaviour).
     Test in `engine/tests/sync_multi.rs`.

2. **Removing a tag together with a larger text edit deletes and re-creates the node.**
   - What happens: untagged `Quote me, slightly edited` (similarity about 0.48 to `Quote me`) and
     `Something completely different` (was `Todo me`) each deleted the old node and created a new one; the new one
     lost the `todo` layout. Design §1 counts destroying Workflowy-only features in a round trip as unacceptable.
   - Where: `REMATCH_MIN_RATIO = 0.6` and `REMATCH_MAX_DISTANCE = 3` (`engine/src/merge.rs:243-247`).
   - Recommendation: also pair an untagged line with a vanished tagged one when they are the only unmatched pair
     under the same parent and at the same index, whatever the similarity, and count a kept prefix (text appended
     to the old name) as similar. Report it as `untagged item matched to ...` so the user can see it. Trade-off: a
     real "delete a line, type a new one in its place" becomes a rename, which keeps the old node's history (the
     smaller loss). Test in `engine/tests/merge_fixtures.rs`.

3. **Ticking a node that a collaborator renamed drops your rename without a word.**
   - What happens: `- [x] Noted task, my rename`, while a collaborator renamed the node, completed it with their
     name; nothing reported the dropped text.
   - Where: `merge_name` skips the conflict when the line is ticked (`engine/src/merge/engine.rs:1347-1350`;
     module docs `engine/src/merge.rs:115-116`). Design §6.2 ("ticked | edited") keeps the remote edit, so the
     outcome follows the design, but the typed text vanishes with the line.
   - Recommendation: keep the outcome and report it as `MovedOutConflict` does:
     `conflict on the name of "Noted task, renamed by them", which you ticked: Workflowy's version kept; yours was
     "Noted task, my rename"`. Test in `engine/tests/sync_scenarios.rs`.

4. **`%%comments%%`, images and link titles typed in Obsidian do not survive a sync.**
   - What happens: `%%hidden comment%%` reaches Workflowy as visible text and comes back as
     `%\%hidden comment%\%`, which Obsidian no longer hides. `![img](https://example.com/i.png)` comes back as
     `\![img\](https://example.com/i.png)` and `[a](https://example.com "title")` as
     `[a\](https://example.com "title")`, with the URLs turned into Workflowy links. Wikilinks, embeds,
     `==highlight==` and `$math$` are written back unescaped and keep working.
   - Where: every `%%` is escaped because `%%wf:...%%` is a tag form (`engine/src/mdinline/render.rs:390`); the
     exceptions for the other Obsidian syntax are at `:355-389`; the policy is in `engine/src/mdinline.rs:33-38`.
   - Recommendation: escape `%%` only when what follows would read as a tag (`wf:` or `wf-conflict:`, allowing
     spaces), so a typed comment round-trips. Treat `![alt](url)` as literal text (no link inside) and write it
     back unescaped, like `![[embed]]`. Parse `[text](url "title")` as a link and drop the title with a lossy-write
     report. Trade-offs: `%%x%%` typed by a Workflowy user would then be hidden in Obsidian's reading view;
     Workflowy users see Obsidian comments as text either way (design §11). Add cases to the corpus in
     `engine/tests/mdinline_corpus.rs`.

5. **`checkboxes: none` behaves exactly like `todos`.**
   - What happens: both give a box to to-do nodes only; existing boxes also stay (see bug 4).
   - Where: `CheckboxPolicy::TodosOnly | CheckboxPolicy::None => false`, after to-dos always get `[ ]`
     (`engine/src/mdblock/view.rs:173-184`).
   - Recommendation: make `none` mean no boxes at all, to-dos included (ticking still works by typing `[x]`, which
     the parser accepts on any item), or drop `none` and accept it as an alias of `todos` with a warning.

6. **A node whose children are cut off by `depth:` looks like a leaf.**
   - What happens: "Big", with 60 children beyond the depth limit, rendered as `- [ ] Big`. Deleting that line
     deletes all 60.
   - Where: the box is decided from the view's children, which depth has already removed
     (`engine/src/mdblock/view.rs:150`).
   - Recommendation: treat a node with remote children as interior for the checkbox policy, and consider a visible
     hint (for example `- Big …`). Snapshot test in `engine/tests/mdblock_snapshots.rs`.

7. **`render --force` discards the old file's options and other frontmatter.**
   - What happens: the new frontmatter has only `workflowy:`, `headings` (from the flag) and `gen`; the old file's
     `ids:`, `checkboxes:` or `depth:` options and lines such as `tags:` are gone.
   - Where: `Frontmatter::new` in `render_once` (`engine/src/sync.rs:514-516`).
   - Recommendation: when the output file exists and names the same root, start from its frontmatter: keep its
     options (not `gen`, `force` or `paused`) and its other lines, and let `--headings` override only when given
     (make it an `Option<u8>`). Trade-off: a clean reset then needs the file deleted first; say so in `--help`.

8. **A heading typed deeper than `headings:` takes the list items after it as children.**
   - What happens: `### Skipped level typed by hand` typed above `- [ ] Beta child` in a `headings: 1` file gave
     `heading level 3 coerced to 2` and `heading at depth 2 but headings: 1; kept as a heading`; the heading was
     re-rendered as a list item and `Beta child` was moved under it in Workflowy.
   - Where: `engine/src/mdblock/parse.rs:424`.
   - Recommendation: the parse follows Markdown (a heading owns what follows it), so keep it, but say what it
     did: `line 17: heading at depth 2 but headings: 1; it becomes an item, and the 2 items after it become its
     children`.

9. **The daemon has no graceful shutdown and forgets its event log when it restarts.**
   - What happens: SIGTERM ends it at once (exit 143) without updating the status note; after a restart the
     note's event sections start again at `None.`. A cycle killed mid-apply is recovered from the journal by the
     next run (verified for a CLI `sync` killed with `kill -9` after 30 of 120 creates: no duplicates).
   - Where: no signal handling anywhere in `engine/src/` and no signal crate in `engine/Cargo.toml`; events are a
     `VecDeque` of at most `MAX_EVENTS = 50` in memory (`engine/src/daemon.rs:112-113`, `:712-718`).
   - Recommendation: on SIGTERM or SIGINT (for example with `signal-hook`), finish the current cycle, write
     `stopped at ...` to the status note and exit 0. Save the events to a file in the state directory and load them
     at start. Test in `engine/tests/daemon.rs` by signalling the binary.

10. **A code fence's info string becomes the first line of the code.**
    - What happens: a block opened with `` ```python `` became a code block named `python\nx = 1`, and the file now
      shows `python` as the first line inside the block.
    - Where: `finish_code` (`engine/src/mdblock/parse.rs:614-620`) does this on purpose, as the API's create
      pipeline does with pasted text.
    - Recommendation: Obsidian users routinely type a language, and Workflowy code blocks have none, so drop the
      info string and say so in the report. Trade-off: a pasted block no longer matches what the Workflowy UI
      would have stored.

11. **Mirror-aware rendering depends on whether the account has any mirror at all.**
    - What happens: on the beta host, genuinely empty bullets are ordinary items (`- ^wf-dc0a1ef2a4f0`) while the
      account has a mirror anywhere, and read-only placeholders (`- ⧉ (empty or mirror) ^wf-...-ro`) when it has
      none (from the code; the switch itself was not run). Creating the first mirror, or removing the last one,
      would rewrite those lines in every synced file.
    - Where: `Tree::has_mirror_metadata` (`engine/src/tree.rs:741-747`), used at `engine/src/mdblock/view.rs:217`.
    - Recommendation: also turn mirror-aware mode on when `--api-url` is the beta host, or remember per root that
      its exports came from a mirror-aware host. Trade-off: a remembered mode that is wrong (the same root later
      synced against production) would show real shells as ordinary empty items, so tie it to the host.

12. **Nothing detects a second engine syncing the same files.**
    - What happens: the per-root lock only covers one state directory (`engine/src/state.rs:623`,
      `engine/src/sync.rs:1561-1569`). Two engines with separate state directories on one vault would each read the
      other's `gen` as unknown (safe mode, with no hint why) or, worse, as one of their own generations with
      different text, and merge against the wrong base (from the code; not run).
    - Recommendation: keep a random engine id in the state directory and write it next to `gen`
      (`syncflowy: {gen: 412, engine: 3f2a}`); pause a file that carries another engine's id, with
      `another engine (3f2a) writes this file`. This does not cover two engines on separate vaults of one account,
      which stays a documented limitation.

## 4. Not confirmed

1. **Killing the daemon mid-cycle.** Journal recovery was verified for a killed CLI `sync` and is covered by the
   tests (`crash_recovery_through_the_journal`, `engine/tests/sync.rs`), but no daemon was killed during a cycle.
2. **The export rate limit.** Whether Workflowy enforces 1 export per minute is unknown: it was not enforced in
   one observation, and no 429 has ever been seen. This decides how urgent section 2 item 6 is.
3. **"Counted as hidden only if never shown."** The reviewer's reading of bug 1 does not hold: the report's count
   never includes depth-hidden nodes and the brake's count always does (the base is always the latest render).
4. **Conflict-copy names and Ctrl-C.** The patterns in `is_conflict_copy` (`engine/src/daemon.rs:386`) are marked
   UNVERIFIED in the code (Obsidian Sync's naming was never observed). Only SIGTERM (exit 143) was tested.
