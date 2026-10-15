# syncflowy engine

A two-way sync between [Workflowy](https://workflowy.com) subtrees and Markdown files, written in Rust. One
Workflowy node (the **sync root**) and its subtree is mirrored into one Markdown file; the file can live in an
Obsidian vault. The specification is [`../docs/sync-design.md`](../docs/sync-design.md) (cited as §x.y in the
code). Facts about the API are in [`../docs/workflowy/data-model.md`](../docs/workflowy/data-model.md) (DM§x) and
about the web UI in [`../docs/workflowy/ui-findings.md`](../docs/workflowy/ui-findings.md) (UI:xx).

The crate builds one binary, `syncflowy`, and a library of the same name.

## Build and test

```sh
cd engine
cargo build --release          # target/release/syncflowy
cargo test                     # unit, property, snapshot, differential and end-to-end tests
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

A static Linux binary (§12.1), which is what CI publishes as a build artifact, needs the musl target and a musl C
compiler for `ring` (`rustup target add x86_64-unknown-linux-musl`, `apt-get install musl-tools`):

```sh
CC_x86_64_unknown_linux_musl=musl-gcc cargo build --release --target x86_64-unknown-linux-musl
```

The end-to-end tests run against the **Python mock server** (`src/syncflowy/mock`). `testsupport::MockServer`
starts it with `uv run syncflowy-mock`, so `uv` and the Python environment of the repository root must work
(`uv sync`, or the Nix dev shell), and runs it in its beta mode (`--beta-mirrors`). The differential tests also
call the Python `wfhtml` and the mock's create pipeline. The tests never contact the real API. Use
`cargo test --test <file>` for an inner loop.

## Configuration

| setting | flag | environment | default |
|---|---|---|---|
| API key | `--api-key` | `WORKFLOWY_API_KEY` | none (required by `render`, `sync` and `daemon`) |
| API base URL | `--api-url` | `WORKFLOWY_API_URL` | `https://beta.workflowy.com` (the beta API; the production host is refused, see Safety) |
| state directory | `--state-dir` | `SYNCFLOWY_STATE_DIR` | `$XDG_STATE_HOME/syncflowy`, else `~/.local/state/syncflowy` |
| request log | `--request-log FILE` | | none; appends every API request as JSONL |
| verbosity | `-v` (info), `-vv` (debug) | `RUST_LOG` also works | warnings only |

The state directory holds the checkpoints and must be **outside the vault** (see [State directory](#state-directory)).

Exit codes: `0` success, `1` the run needs attention (a brake, a skipped or paused file, an error in one file of a
`sync`), `2` a fatal error (no API key, unusable state, unknown root, ...).

## Commands

The examples use a local mock (`uv run syncflowy-mock --db /tmp/x.sqlite3 --port 8765`, key `test-key`):

```sh
export WORKFLOWY_API_KEY=test-key
S="--api-url http://127.0.0.1:8765 --state-dir /tmp/st"
```

### `render <ROOT> <OUT.md> [--headings N] [--force]`

Renders the subtree under `ROOT` (a short id, a full UUID, or a `https://workflowy.com/#/<short>` URL) into a new
file and records the first checkpoint, which starts the sync. `--headings N` (0 to 6) renders depth 1..N as `#`
headings. It refuses to overwrite an existing file or checkpoint unless `--force` is given. With `--force`, an
existing file that names the same root keeps its `syncflowy:` options (not `gen`, `force` or `paused`) and its
other frontmatter lines, and `--headings` changes only the heading depth; delete the file first for a clean reset.

```console
$ syncflowy $S render 5e23116b-0a18-4c58-82b5-cbe4cb0343ff vault/plan.md
vault/plan.md: rendered (gen 1)
```

### `sync <FILE.md>... [--force] [--dry-run]`

Runs one cycle over the given files, which share one export (several files can move nodes between them, §6.4).
Per file it merges, pushes your edits, and rewrites the file from the fresh remote state. The report says what was
pushed (`synced: 1 created, 0 updated, 0 moved, 0 reordered, 1 completed, 0 deleted (+0 hidden); file rewritten (gen 3)`),
or `unchanged`, `paused`, `refused: ...` (a brake) or `skipped: ...`.

* `--force` overrides the mass-change brakes and the orphan-placeholder guards (see Safety). It also bootstraps a
  file that has no checkpoint (see Limitations).
* `--dry-run` plans and prints the operations, and writes nothing. It does not recover a leftover journal; it
  warns that one is pending. A delete that takes hidden items (completed, or below `depth:`) names them, as the
  report of a real sync does.
* **Moving a line between files**: cut the tagged line from one file, paste it into the other, and sync both
  files in one command. The node moves with its id, children and Workflowy-only attributes. If you sync only the
  file you cut from, the engine finds the tag in the other file (it reads the files recorded in the state
  directory's other checkpoints) and holds the delete back with
  `warning: "Item E" (67edfe175330) is in other.md; sync both files in one command to move it`. The line stays
  out of the file and the delete stays pending until a run syncs both files (a move) or no longer finds the tag
  (the delete is sent). If the other file is paused, the run leaves it out, so the warning says
  `..., which is paused; unpause it and sync both files in one command to move it` and the delete stays held
  until then. A held delete that is finally sent counts and names the cut line's children as hidden items,
  including those that were visible when it was cut. Syncing only the file you pasted into makes the line a new node, since its tag belongs to
  a file outside the run, and the original is then deleted. A tagged line *copied* into another file while it
  stays in its own file becomes a new node there, as a copied line does within one file.

```console
$ syncflowy $S sync --dry-run vault/plan.md
vault/plan.md: dry run: 4 op(s)
  op 0: create temp-1 under 8d88348a-... (direct)
  op 1: move 859d1384-... to the bottom of 8d88348a-...
  op 2: complete 859d1384-...
  op 3: delete 2cfa93ea-... (+1 hidden)
  deleting "Old ideas" (3d437204e52d) also deletes 1 hidden item: "Hedge maze"
```

### `daemon --vault <DIR> [options]`

Watches a vault and syncs continuously (design §9.1: one engine next to Obsidian Sync). Every `*.md` under the
vault whose frontmatter has a `workflowy:` key is a synced file; dot directories are skipped. Files are
identified by root, not by path, so renaming a file needs no handling.

| flag | default | meaning |
|---|---|---|
| `--debounce SECS` | 45 | quiet period of a changed file before it is synced |
| `--poll SECS` | 60 | interval of the cycle that picks up remote changes |
| `--min-export-interval SECS` | 60 | minimum time between two cycles (each cycle exports) |
| `--delete-grace SECS` | 600 | grace period of deferred deletes |
| `--status-note PATH` | `Workflowy Sync Status.md` | the status note, relative to the vault |

The daemon never exits on a per-file error; it logs it (use `-v`) and shows it in the status note. It exits on a
fatal error, such as an unusable state directory or an API 401.
On SIGTERM or SIGINT (Ctrl-C) it finishes the cycle in progress, writes `Stopped at ...` to the status note and
exits 0; a second signal ends it at once (an interrupted cycle is recovered by the next run). Its event log is
kept in the state directory, so the status note's event sections survive a restart.

### `status [FILE.md]...`

Prints the local state without calling the API: for each root its generation, when it last synced, which file
holds it, whether the file has local changes, whether an interrupted cycle awaits recovery, and the pending
deletes. With files, it shows their roots; a file that does not exist prints `<file>: missing`, and one whose
root has no checkpoint in this state directory prints `<file>: no checkpoint for root <root> in this state
directory`. The other files are still shown, and the exit status is 1. `status` only reads: it does not create
the state directory.

```console
$ syncflowy $S status
5e23116b-0a18-4c58-82b5-cbe4cb0343ff: gen 8, last synced at 1791955658 (unix), file /tmp/v/plan.md
  the file is as last written
```

## The Markdown dialect, in brief

Full rules: design §4 and the doc comments of `src/mdblock.rs` and `src/mdinline.rs`. A synced file:

```markdown
---
workflowy: 5e23116b-0a18-4c58-82b5-cbe4cb0343ff
syncflowy: {headings: 1, gen: 8}
---
Top-level note

# Planning %%wf:1a2b3c4d5e6f%%
A note on "Planning": the lines right under the heading.

- [ ] A new local item ^wf-7691c31e3664
	its note (the lines indented under the first line, before any child)
- Draft the **spec**, <u>underlined</u>, [a link](https://example.com) ^wf-472caa53fd65
	- [ ] Write the mock ^wf-1696183c3a62
	- [x] Run the UI scenarios ^wf-77aa12cd0e9f
	- ⚠ my conflicting wording %%wf-conflict:name%%
- A line you just typed, with no tag
- ``` ^wf-0123456789ab
	x = 1
	```
- ⧉ (mirror of a deleted item) ^wf-b2863eed4436-ro
- Weekly review ^wf-3c4d5e6f7a8b
	- ⧉ [[#^wf-472caa53fd65|Draft the spec]] ^wf-11aa22bb33cc-ro
```

* **Frontmatter**: `workflowy:` names the sync root. `syncflowy:` holds options (`headings`, `ids`, `checkboxes`,
  `dates`, `depth`, `paused`, `force`) and `gen`, the render generation the file came from (written by the
  engine; do not edit). Other frontmatter keys are kept as written. An option written outside `syncflowy:`
  (`paused: true` on its own line) is kept too, with a warning; only a top-level `paused: true` takes effect, a
  top-level `force` never does.
* **Items**: `- ` list items, indented with tabs when written (tabs or consistent spaces are read). The first line
  is the node's name, the indented lines after it are its note, nested items are children. The text between the
  frontmatter and the first item is the root's note. The root's own name is not in the file.
* **Id tags** at the end of the first line identify nodes: `^wf-<12 hex>` on list items, `%%wf:<12 hex>%%` on
  headings. Other forms are read too (see `ids` in `src/mdblock.rs`). A line without a tag is a new node and
  gets its tag at the next sync. Do not remove tags from lines you keep. If you do, an untagged line is matched
  back to the node whose tag vanished when it sits under the same parent nearby with similar text (or with the
  old text as its prefix), or when the two are the only unmatched pair under that parent at the same position,
  whatever the text; the report says `line N: untagged item matched to ...`. So deleting a line and typing a new
  one in its place renames the node.
* **Inline text**: `**bold**`, `*italic*`, `~~strike~~`, `` `code` ``, `[text](url)`, bare URLs, `<u>`, colour
  spans and other well-formed WF-HTML as literal HTML, dates as a compact form. Obsidian-only syntax
  (`[[wikilinks]]`, `![[embeds]]`, `![images](url)`, `==highlight==`, `$math$`, `%%comments%%`) is sent to
  Workflowy as literal text and written back unescaped (a `%%` that would read as an id tag stays escaped). A link
  title (`[text](url "title")`) is dropped, and the report says so.
* **Node kinds**: `- [ ] ` boxes appear on todo nodes and leaves (cosmetic; `- ` and `- [ ] ` both mean "not
  completed"); a node whose children are hidden by `depth:` is not a leaf. `checkboxes:` changes this (`todos`,
  `all`, or `none`: no boxes at all, todo nodes included; they keep their layout, and `[x]` still ticks). A
  fenced block is a code-block node (Workflowy code has no language: a fence's info string such as `python` is
  dropped with a warning), `- > text` a quote, `- ---` a divider, `1.` markers are a numbered list. Other layouts
  render as plain items and keep their layout in the checkpoint.
* **Completed nodes are not in the file.** `- [x]` is a command: see ticking below.
* A node whose text cannot be represented exactly is shown with `🔒` and the `-ro` tag flag, and edits to it are
  refused.
* **Mirrors** (see Safety): a mirror shows its origin's content in place, with the mirror's own
  tag. When the same content appears twice in a file, the second placement is a **stub**: `⧉ [[#^wf-<full>|name]]`,
  a read-only link to the line that renders it in full.

## Safety behaviours

* **Ticking is not deleting.** Ticking `[x]` on a line completes the node in Workflowy (only that node, not its
  children) and removes the line and its subtree from the file. It can only be undone in Workflowy, where
  un-completing makes it reappear. **Deleting a line deletes the node** (to the Workflowy trash), together with
  its hidden descendants (completed, or below `depth:`), which you cannot see; the report and the dry run name
  them (the first ten).
* **Mass-change brakes** (§8.3) stop a run before anything is written, report `refused: ...` and exit 1:
  * the file is empty (or has no items) while the base showed items and the merge deletes nodes,
  * the deletes (visible plus hidden) exceed 50, or 20% of the base's visible nodes once there are at least 5,
  * the completions exceed 200, or 50% of the visible nodes once there are at least 10.

  `--force` on the command line, or `force: true` in the frontmatter, overrides them (the engine clears the
  frontmatter key). A missing file only pauses its root; nothing is deleted.
* **Pending deletes** (daemon, §9.1 guard 2). A removed line is not deleted right away. It waits for the grace
  period and one more cycle in which the file is unchanged. Putting the line back cancels it; pasting its tag
  into another synced file turns it into a move. While another synced file has edits waiting for their debounce,
  no pending delete is sent. `status` and the status note list them. Plain `sync` (no grace period) sends
  deletes at once, except the delete of a line whose tag is in another synced file (see `sync`). The brakes
  count a held or deferred delete when it is first seen.
* **Conflict markers** (§6.1). When you and a collaborator both changed a node's name or note, Workflowy's
  version stays and yours is kept as a child line `- ⚠ your text %%wf-conflict:name%%` (or `:note`) before the
  node's real children. It is never synced. Edit the real line if you want your version, then delete the
  marker. Deleting the marker marks the conflict resolved. When a node was moved out of the file in Workflowy
  and both sides edited the same field, Workflowy wins and your text is only logged; likewise when you tick a
  line whose name or note a collaborator changed (the tick is pushed).
* **Beta API only** (§10, DM§10.5). The engine talks to `https://beta.workflowy.com`, which serves the same
  account and data as production but reports mirrors properly. Production reports a mirror as an empty node, so
  edits to it would be invisible to Workflowy users; `--api-url` refuses the production host.
* **Mirrors as their origins** (§15.1). The engine treats a mirror as the node it mirrors, as the web UI does: a mirror inside the synced subtree shows its origin's text and
  children under the mirror's own tag, even when the origin lies outside the subtree; edits, ticks, new items
  and moves under it reach the origin; deleting its line removes just that mirror (`DELETE /nodes/:id/mirror`),
  never the content. Each content node is rendered once per file: a second placement (a mirror of a node that
  is already in the file, or the origin of a mirror shown first) is a read-only **stub** line
  `⧉ [[#^wf-<full>|name]] ^wf-<mirror>-ro`; edits and ticks of a stub are refused and rewritten, items typed
  under it are lifted to its parent and reported, and the choice of which placement is full sticks across
  renders. A delete whose subtree holds an origin that still has mirrors is refused and reported (`--force`
  does not override it: the API refuses too, and removing a mirror is a visible change for Workflowy users);
  remove the mirrors in Workflowy first. A mirror whose origin was deleted (an orphan) renders as a read-only
  placeholder `⧉ (mirror of a deleted item)` with the `-ro` flag: its children are not shown, and edits or
  deletes of it are refused without `--force` (§10). Creating a mirror from Markdown and the UI's promotion of a
  mirror when its origin is deleted are not supported. An empty node is an ordinary, editable item.
* **Safe mode** (§9.1 guard 1). If the file's `gen` is missing, not a number, or no longer in the render history
  (for example a hand-written or stale file), the engine cannot know what you changed. It then pushes text edits
  and creates, but applies no deletes, moves, reorders or ticks, and says so in the report
  (`the file's gen (999) is unknown: safe mode (no deletes, moves, reorders or ticks)`).
* **Other guards.** Every update checks the remote value just before writing it (a collaborator's later edit
  becomes a conflict, not an overwrite). Creates are journaled and nonce-tagged so a crash never duplicates a
  node; an interrupted cycle is recovered by the next run. A file that cannot be parsed (frontmatter) is skipped,
  never read as "everything deleted". Two files claiming one root are both paused, also when they are synced by
  separate `sync` commands (the checkpoint records the file last synced; a rename is fine), and a file whose name
  looks like a sync conflict copy (`(conflicted copy ...)`, `(... conflict ...)`, `.sync-conflict-`) is skipped. A synced file that is a
  symlink stays a symlink and keeps its mode. `paused: true` in the frontmatter stops syncing a file.
* **The status note** (daemon): `Workflowy Sync Status.md` in the vault, rewritten after every cycle. It lists each
  root with its file, last sync, generation and state, the conflicts awaiting resolution, the pending deletes, per-file
  problems, safe-mode episodes, dropped resurrections and stale echoes, API errors, and other events. It has no
  frontmatter, so it is never synced; edits to it are overwritten.

## State directory

```text
<state_dir>/
  daemon-events.json       the daemon's event log (the status note's event sections)
  roots/<root uuid>/
    checkpoint.json        base_md and base_wf of the last write, the generation, the file's config
    pending.json           pending deletes
    history/<gen>.json     the Markdown and remote tree of recent render generations
    journal.jsonl          write-ahead journal of an apply (only while a cycle is in progress)
    lock                   held while a cycle runs on this root
```

It must be outside the vault so that Obsidian Sync never carries it. Losing the checkpoint of a root means
`sync` refuses (`NoCheckpoint`) until `--force` bootstraps it again; `render --force` starts over. Keep it with
the server that talks to Workflowy: there must be only one engine per account.

## Known limitations

* One engine, one writer. Several engines on one root are not supported, and nothing detects them: use one
  state directory per vault and account, and run every `sync`, `render` and `daemon` for those files with it. The
  per-root lock covers only one state directory. Two engines with separate state directories on the same files
  read each other's `gen` as unknown (safe mode, with no hint why) or, worse, as one of their own generations
  with different text, and then merge against the wrong base.
* The engine needs the beta API (§10); it does not run against production. Mirrors are never
  created from Markdown, a stub is read-only, and an origin that still has mirrors cannot be deleted from
  Markdown. A mirror's hidden persisted children (DM§10.5) are never shown and go with the mirror when it is
  removed. Observed on the beta host on 2026-10-13 only through the mock.
* Dates render compactly only for the observed forms; ranges, times, and locale or 24-hour variants stay literal HTML.
* Markdown link titles are dropped (`[a](b "t")` is pushed as a link to `b`, and the report says the title was dropped). Images
  and Obsidian comments are literal text in Workflowy (a `%%x%%` typed there is hidden in Obsidian).
* A node that cannot round-trip is shown read-only (`🔒`); edit it in Workflowy.
* The client sends at most 10 requests per second. A cycle exports once, or twice when it pushes (the second
  export verifies the push and renders the file). The documented export limit of one per minute was not observed
  in production; the daemon spaces its cycles by `--min-export-interval`, and if the limit is ever enforced, a
  429 is retried for up to 90 s in total, waiting as its `Retry-After` header asks (design §12.2).
* The engine has only been run against the mock server, never against the real API. Anything marked UNVERIFIED
  below depends on behaviour nobody has observed.
* A cycle that cannot be settled after its retries (collaborator edits racing the push) commits nothing and is
  retried at the next run.
* Obsidian Sync's real merge behaviour is untested; the guards do not depend on it.

## UNVERIFIED choices

These are the places where the code chooses without evidence (`grep -rn UNVERIFIED src`). Each is explained at its
location.

| where | choice |
|---|---|
| `wfhtml.rs` | extension of `i`, `s`, `code` by analogy; attributes of format tags dropped; `<a>` without href unwrapped; colour classes and date attributes handled as in `wfhtml.py` |
| `mdinline.rs`, `mdinline/*` | how the UI stores code-block names (§13 Q2); a line break typed into a name becomes a space; tag keyword case and spacing; Obsidian's whitespace set, block-id syntax, autolink and inline-math rules (`$10-$20` keeps its `$` escaped); date locale not modelled; character references kept literally |
| `mdblock.rs`, `mdblock/*` | whether Obsidian keeps whitespace-only note lines; code at heading depth; no checkbox on quotes; an explicit null `depth` means unlimited |
| `tree.rs` | a shell whose layout was changed later; ties of equal priorities (server breaks them by creation sequence) |
| `mirrors.rs` | the sticky choice of the full placement; a content whose placements are all hidden keeps the previous choice; a refused mirror delete keeps the whole subtree |
| `merge.rs`, `merge/*` | foreign tags of other roots; remote wins silently for a conflict that has no marker form; moved-out edit conflicts only reported; "nearby" for identity repair, and pairing the only unmatched line and item at one index whatever the text; divider and `---` exceptions; a layout changed on both sides keeps the remote's; diff3 treats adjacent-line edits as mergeable; cross-file bases |
| `plan.rs` | brake thresholds; the nonce in create names; the create-pipeline check against the mock's model; an emptied file that deletes nothing is not braked |
| `apply.rs` | two minutes of clock skew; the extra pre-write GET checks |
| `wfclient.rs` | no gateway 5xx or 429 seen from production, so retry and non-retry rules are guesses |
| `state.rs` | an unreadable config falls back to the default instead of refusing |
| `sync.rs` | bootstrapping without a checkpoint (`--force`) takes the remote as the base and runs in safe mode |
| `daemon.rs`, `daemon/tasks.rs` | which conflict-copy names Obsidian Sync uses; holding pending deletes while another file waits; the Tasks plugin's date fields are stripped from newly ticked lines |

## Module map

| module | what it does |
|---|---|
| `tree` | the outline model: ids, visibility, sibling order, structural edits with the API's priority algorithm |
| `wfhtml` | WF-HTML escape, parse, canonicalise, well-formedness (port of `src/syncflowy/wfhtml.py`) |
| `mdinline` | WF-HTML inline <-> Markdown inline, representability tiers, id-tag syntax |
| `mdblock` | the file format: frontmatter, parser, renderer, the view of a tree |
| `merge` | three-way merge of fields, structure, completion and order; identity repair; conflicts; cross-file moves |
| `mirrors` | mirrors as their origins (§15.1): the per-root view tree, the real tree, and the translation of a merge result between them |
| `plan` | turns the merge result into ordered API operations; guards and mass-change brakes |
| `apply` | executes a plan: journal, pre-write checks, two-step creates, crash recovery, verification |
| `state` | checkpoints, render history, pending deletes, atomic writes, per-root lock |
| `wfclient` | blocking API client: rate limit, retries, typed errors, request log |
| `sync` | one cycle for one or several files; `render_once`; `status` |
| `daemon` | watch mode: discovery, debounce, polling, deferred deletes, status note |
| `testsupport` | runs the Python mock server for tests |
| `error` | the engine-level error type |

The pure core (`tree`, `wfhtml`, `mdinline`, `mdblock`, `merge`, `mirrors`, `plan`) does no I/O; the contract between modules
is the doc comment of `src/lib.rs`.
