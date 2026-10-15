# Command-line reference

`syncflowy` has four commands: `render` starts syncing a root, `sync` runs one sync, `daemon` syncs a whole vault
continuously, and `status` shows the local state. This page covers them, their options and exit codes, the state
directory, and the choice between the normal Workflowy service and the beta host.

Note: everything here was verified against a mock of the Workflowy API, not the real service. The examples come from several separate test runs, so the same 12-character id can name different nodes in
different examples.

- [Commands at a glance](#commands-at-a-glance)
- [Global options](#global-options)
- [Exit codes](#exit-codes)
- [render](#render)
- [sync](#sync)
- [status](#status)
- [daemon](#daemon)
- [The state directory](#the-state-directory)
- [The real service, the beta host and rate limits](#the-real-service-the-beta-host-and-rate-limits)

## Commands at a glance

```text
Sync Workflowy subtrees with Markdown files

Usage: syncflowy [OPTIONS] <COMMAND>

Commands:
  render  Render a Workflowy subtree into a new Markdown file and start syncing it
  sync    Run one sync cycle over the given files
  daemon  Watch a vault and sync continuously
  status  Show the local sync state (no API calls)
  help    Print this message or the help of the given subcommand(s)
```

`syncflowy --version` prints `syncflowy 0.1.0`. `syncflowy <command> --help` prints each command's options.

## Global options

These work with every command, before or after the command name (`syncflowy -v sync f.md` and
`syncflowy sync -v f.md` are the same).

| option | environment variable | default | what it does |
|---|---|---|---|
| `--api-key KEY` | `WORKFLOWY_API_KEY` | none | Your Workflowy API key. Needed by `render`, `sync` and `daemon`, not by `status`. The flag wins over the variable. |
| `--api-url URL` | `WORKFLOWY_API_URL` | `https://workflowy.com` | Which Workflowy host to talk to. Use `https://beta.workflowy.com` to sync mirrors properly ([below](#the-real-service-the-beta-host-and-rate-limits)). The help text calls this "the mock server for tests"; for you it selects the host. |
| `--state-dir DIR` | `SYNCFLOWY_STATE_DIR` | `$XDG_STATE_HOME/syncflowy` if `XDG_STATE_HOME` is set, else `~/.local/state/syncflowy` | Where checkpoints, history and journals live. Keep it outside the vault ([below](#the-state-directory)). Every command, `status` included, creates it if it is missing. |
| `--request-log FILE` | none | off | Appends one JSON line per API request (method, path, status, timing, response, attempt). The API key is never written. Large bodies are summarised. Useful when reporting a problem. |
| `-v`, `-vv` | `RUST_LOG` | warnings only | `-v` logs info (each operation sent), `-vv` debug. Logs go to stderr with UTC timestamps. If `RUST_LOG` is set, it replaces `-v`/`-vv` entirely. |

A key variable that is set but empty counts as a key, so you get `invalid credentials` rather than "no API key".

## Exit codes

| code | meaning | when |
|---|---|---|
| `0` | success | `render` succeeded; every file of a `sync` was `unchanged`, `synced`, a `dry run`, or `paused (paused: true in the frontmatter)`; `status`. Conflicts, safe mode and refused edits of read-only items also exit 0: read the report. |
| `1` | a file needs attention | some file of a `sync` was `refused: ...` (a safety brake), `skipped: ...` (unreadable frontmatter, no frontmatter, not UTF-8, lock held, retries exhausted), `paused: the file is missing ...`, `paused: several files claim this root ...`, or had a per-file error such as no checkpoint or root not found. The other files of the run are still synced. |
| `2` | fatal | no API key; a bad key (401); the API unreachable after retries; an export failing; an unusable state directory; for `render`: the file or checkpoint exists, unknown or malformed root; for `status FILE`: that root has no checkpoint, or the file is missing; for `daemon`: the vault is not a directory; any command-line usage error. |
| `143` / `130` | daemon stopped | the daemon was killed by SIGTERM (143) or Ctrl-C (130, standard shell behaviour, not tested). It never exits 0 on its own. |

Fatal errors print an `error: ...` line on stderr:

```console
$ syncflowy render 000000000000 vault/nope.md
error: sync root "000000000000" not found
[exit 2]
$ env -u WORKFLOWY_API_KEY syncflowy sync vault/garden.md
error: no API key: pass --api-key or set WORKFLOWY_API_KEY
[exit 2]
$ syncflowy --api-key wrong-key sync vault/garden.md
error: API: invalid credentials: invalid credentials
[exit 2]
$ syncflowy --api-url http://127.0.0.1:9 sync vault/garden.md
error: API: transport error: io: Connection refused (os error 111): transport error: io: Connection refused (os error 111)
[exit 2]
```

(The doubled text is cosmetic. An unreachable API takes several seconds to fail: syncflowy retries four times.)

## render

```text
syncflowy render [--headings N] [--force] <ROOT> <OUT>
```

Renders the subtree under `ROOT` into a new Markdown file `OUT` and records the first checkpoint. From then on the
file is synced. Missing parent directories of `OUT` are created.

| argument / flag | meaning |
|---|---|
| `ROOT` | the sync root: a 12-character short id (`584a2265b1f5`), a full UUID, or a URL `https://workflowy.com/#/584a2265b1f5` (also `http://`, `beta.workflowy.com`, any case, optionally followed by `/...` or `?...`) |
| `OUT` | the Markdown file to create |
| `--headings N` | render depth 1 to N as `#` headings (default 0: everything is a list). Use 0 to 6 only: a higher value is accepted here but makes a file that every later `sync` skips with `bad syncflowy config headings: "7"`. See [headings mode](markdown.md#headings-mode). |
| `--force` | overwrite an existing file and checkpoint |

The `workflowy:` line keeps the root exactly as you typed it:

```console
$ syncflowy render https://workflowy.com/#/1193e6c3f339 vault/reading.md --headings 1
vault/reading.md: rendered (gen 1)
[exit 0]
$ cat vault/reading.md
---
workflowy: https://workflowy.com/#/1193e6c3f339
syncflowy: {headings: 1, gen: 1}
---
# Books %%wf:a7dd06839eb9%%

- [ ] **Middlemarch** ^wf-8ad9f06c144a
	George Eliot

# Articles %%wf:0c4237730edf%%

- [ ] https\://example.org/essay ^wf-b8bc076f3787
```

(`https\://` is how a URL that is plain text in Workflowy, not a link, is written, so that Obsidian does not turn it
into one. See [escaping](markdown.md#escaping).)

`render` refuses to overwrite a file, and refuses a root that already has a checkpoint (even into a different file):

```console
$ syncflowy render cd613e30-d8f1-4adf-91b7-584a2265b1f5 vault/garden.md
error: vault/garden.md exists (use --force)
[exit 2]
$ syncflowy render 584a2265b1f5 vault/garden2.md
error: root cd613e30-d8f1-4adf-91b7-584a2265b1f5 already has a checkpoint; sync its file instead, or use --force
[exit 2]
$ syncflowy render not-an-id vault/nope.md
error: parse: bad workflowy: root "not-an-id": bad workflowy: root "not-an-id"
[exit 2]
```

`render --force` overwrites both the file and the checkpoint. The generation continues rather than restarting:

```console
$ syncflowy render 1193e6c3f339 vault/reading.md --headings 1 --force
vault/reading.md: rendered (gen 2)
```

> **Warning.** `render --force` throws away anything in the file that is not yet in Workflowy, and starts the
> frontmatter over: only `workflowy:`, `headings` and `gen` are written. Your other options (`ids`, `checkboxes`,
> `depth`, ...) and your own keys (`title`, `tags`, `aliases`, ...) are lost.

## sync

```text
syncflowy sync [--force] [--dry-run] <FILES>...
```

Runs one sync over the given files: it reads Workflowy once, merges each file with it, sends your edits, and
rewrites the files where needed. Files named in one command share that one read, which is what makes a cut and
paste between them a move (see [moving between files](editing.md#moving-between-files)).

| flag | meaning |
|---|---|
| `--dry-run` | plan only: print the operations that would be sent and write nothing. It does not recover an interrupted earlier run. |
| `--force` | for this run: override the [mass-change brakes](editing.md#mass-change-brakes) and the placeholder guards, and adopt a file whose root has no checkpoint (in [safe mode](editing.md#safe-mode)). |

### The report

One line per file, then details:

```console
$ syncflowy sync vault/garden.md
vault/garden.md: synced: 0 created, 0 updated, 0 moved, 0 reordered, 1 completed, 1 deleted (+0 hidden); file rewritten (gen 3)
  op 0: complete 9b810e76-6ec9-4286-a3ca-828dd5f4b3b2: done
  op 1: delete b2221a58-008a-45a6-8464-7159c324c985 (+0 hidden): done
```

| part | meaning |
|---|---|
| `created`, `updated`, `moved`, `completed`, `deleted` | what was sent to Workflowy. `updated` counts fields, so a new name and a new note on one node count 2. |
| `reordered` | extra moves needed to put siblings in the right order (the API cannot insert in the middle of a list). |
| `(+N hidden)` | descendants deleted along with a deleted line that were not shown in the file: completed ones, and ones hidden by `depth:`. They are counted, not named, and children hidden by `depth:` may be missing from the count (a test showed `+0 hidden` although two such children were deleted). |
| `file rewritten (gen N)` | the file was rewritten and is now at generation N. |
| `file unchanged (gen N)` | your edits were sent, but nothing you need to see changed, so the file was left alone. |
| `op N: ...: done` | one line per API operation sent. |
| other indented lines | warnings (`warning: ...`), conflicts, refused edits and other events. See [troubleshooting](editing.md#troubleshooting). |

Other one-line outcomes: `unchanged` (nothing to do), `dry run: N op(s)`, `paused ...`, `refused: ...`,
`skipped: ...`, and per-file errors printed as `FILE: error: ...` on stderr. A per-file error does not stop the
other files, and the run exits 1:

```console
$ syncflowy sync misc/unknown.md          # frontmatter: workflowy: 000000000000
misc/unknown.md: error: sync root "000000000000" not found
[exit 1]
$ syncflowy sync misc/nofm.md             # a file with no frontmatter
misc/nofm.md: skipped: the file does not parse: no frontmatter
[exit 1]
```

The `would ...` lines of `--dry-run` are debugging output. Read them for the gist (`Create`, `UpdateName`,
`Complete`, `Delete { ..., hidden_descendants: 1, ... }`); the format may change.

### Two files for one root

If two of the files you name claim the same root, neither is synced:

```console
$ syncflowy sync vault/reading.md misc/reading-copy.md     # two files, same root
[2026-10-15T01:25:07Z WARN  syncflowy::sync] root 05b6e6e3-07d4-4edc-9143-1193e6c3f339 is claimed by several files: ["vault/reading.md", "misc/reading-copy.md"]; not synced
vault/reading.md: paused: several files claim this root: vault/reading.md, misc/reading-copy.md
misc/reading-copy.md: paused: several files claim this root: vault/reading.md, misc/reading-copy.md
[exit 1]
```

> **Caution.** `sync` only compares the files on its command line. Each copy synced on its own is accepted. Only
> the daemon, which scans the whole vault, always catches duplicates. Delete stray copies yourself.

## status

```text
syncflowy status [FILES]...
```

Shows the local state of every synced root, or only of the given files' roots. It makes no API calls and needs
no key. It is safe to run while the daemon is running.

```console
$ syncflowy status
cd613e30-d8f1-4adf-91b7-584a2265b1f5: gen 1, last synced at 1792027196 (unix), file /…/vault/garden.md
  the file is as last written
```

Per root: the generation, the last sync time (Unix seconds), the file's absolute path, then one or more of:

| line | meaning |
|---|---|
| `the file is as last written` | no local edits since the last sync |
| `the file has local changes not yet synced` | you edited the file; the next sync sends the edits |
| `the file cannot be read` | the file is missing (moved, renamed elsewhere or deleted) |
| `an unfinished cycle was interrupted: the next sync recovers it` | a sync was killed part-way; see [after a crash](editing.md#after-a-crash) |
| `pending delete: "Beans" (f3c64af7-75a8-4294-82cd-789a380208a9), first seen at 1792027370` | the daemon is waiting to delete this node |

With no synced roots it prints `no synced roots in <state dir>`. `status FILE` for a file whose root has no
checkpoint, or for a missing file, is a fatal error (exit 2).

## daemon

```text
syncflowy daemon --vault <VAULT> [--debounce S] [--poll S] [--min-export-interval S] [--delete-grace S] [--status-note PATH]
```

Watches a vault and keeps every synced file in it in step with Workflowy. It also writes a status note into the
vault. The first cycle runs immediately.

### Options

All times are in seconds.

| flag | default | in plain terms |
|---|---|---|
| `--vault DIR` | required | The folder to watch, recursively. |
| `--debounce S` | 45 | A file you edited is synced once its content has stayed the same for S seconds. Until then its edits are not sent, and Workflowy changes for that root wait too. |
| `--poll S` | 60 | Even with no edits, a cycle runs every S seconds to bring Workflowy changes into the files (never more often than `--min-export-interval` allows). |
| `--min-export-interval S` | 60 | No two cycles closer than S seconds. Each cycle reads the whole account once, and Workflowy documents a limit of one such read per minute. So a file can wait up to S seconds more after its debounce. Lowering this below 60 risks being rate-limited. |
| `--delete-grace S` | 600 | A line you remove is not deleted at once. It becomes a [pending delete](editing.md#pending-deletes-in-the-daemon) and is deleted at the first cycle after S seconds, once the file has stayed unchanged for one more cycle. |
| `--status-note PATH` | `Workflowy Sync Status.md` | Where to write the status note, relative to the vault. |

With the defaults, an edit you make reaches Workflowy roughly 45 to 60 seconds or more after your last change
reaches the machine running the daemon. A Workflowy edit reaches the file within about a minute. If you use
Obsidian Sync, its own delay comes on top.

The daemon also differs from plain `sync` in two ways: it defers deletes, and it removes the done date that the
Obsidian Tasks plugin adds to a ticked line (`✅ 2026-10-13`, `[completion:: 2026-10-13]`) before completing the
node, so the date does not end up in the Workflowy text (covered by tests, not run live).

### Which files it syncs

- Every `*.md` file under the vault whose frontmatter has a `workflowy:` key. Other files are ignored silently.
- Anything whose name starts with `.` is skipped: `.obsidian`, `.trash`, `.git` and their contents.
- Files are recognised by their root, not their path. Renaming or moving a file needs nothing.
- Files whose names look like sync-conflict copies are never synced and are listed under Problems: names containing
  `.sync-conflict-`, `conflicted copy`, or a bracketed group containing the word `conflict`, such as
  `garden (conflicted copy 2026-10-15).md`. `Plan (1).md` is not treated as a copy. (Whether this matches the names
  Obsidian Sync really uses is not verified.)
- Two files naming one root: that root is paused and reported until only one does.
- A root with a checkpoint but no file in the vault: paused, nothing deleted. Deleting the file is how you stop
  syncing a root.
- A file naming a root that has never been rendered is not adopted automatically. Either run `render`, or put
  `force: true` in its `syncflowy:` map (see [adopting a file](#adopting-a-file-without-the-command-line)).

### The status note

After every cycle the daemon rewrites `Workflowy Sync Status.md` (only when its content changes). Do not edit it:
your changes are overwritten. It has no frontmatter, so it is never synced itself. All times are UTC. This is a
real note from a run with one pending delete and one stray conflict copy:

```markdown
# Workflowy sync status

This note is written by syncflowy after every cycle; edits to it are overwritten.

Last cycle: 2026-10-15 01:22 UTC

## Roots

| File | Root | Last sync | Gen | State |
|---|---|---|---|---|
| [[garden]] | Garden plan `584a2265b1f5` | 2026-10-15 01:22 UTC | 2 | ok |
| [[reading]] | Reading list `1193e6c3f339` | 2026-10-15 01:22 UTC | 3 | ok |

## Conflicts awaiting resolution

None.

## Pending deletes

Removed from a file but not yet deleted in Workflowy (§9.1 guard 2). Put the line back to cancel; paste its tag into another synced file to move it there.

- [[garden]]: "Beans", removed 2026-10-15 01:22 UTC, deleted at the first cycle after 2026-10-15 01:23 UTC with the file unchanged

## Problems

- [[garden (conflicted copy 2026-10-15)]]: looks like a sync conflict copy; never synced (merge it by hand, then delete it)

## Safe-mode episodes

None.

## Dropped resurrections and stale echoes

None.

## API errors

None.

## Other events

None.
```

(The "§9.1 guard 2" in the note refers to the [design document](../sync-design.md); you can ignore it.)

The sections are always in this order:

| section | lists |
|---|---|
| Roots | each root: its file (as a wikilink), its name and short id, last sync, generation, state |
| Conflicts awaiting resolution | each conflict marker still in a file, with its line number |
| Pending deletes | removed lines waiting for their grace period. "Held: another synced file has edits waiting to be synced." means they wait for another file to settle. |
| Problems | duplicate roots, conflict copies, files that do not parse, current per-file errors, and `All files: ...` for errors that affect every file |
| Safe-mode episodes | syncs that ran in [safe mode](editing.md#safe-mode) |
| Dropped resurrections and stale echoes | old text, or lines of completed nodes, brought back (for example by a sync tool's merge) and ignored |
| API errors | failed cycles |
| Other events | brakes, errors, conflicts, deletes sent, cancelled deletes |

The event lists keep the 50 newest events, in memory only: they start empty when the daemon restarts.

The **State** column of the Roots table shows one of:

| state | meaning |
|---|---|
| `ok` | in step |
| `local edits waiting (debounce)` | you edited the file; it will sync after the quiet period |
| ``paused (`paused: true`)`` | you paused it in the frontmatter |
| `paused: no file names this root; nothing is deleted` | the file is gone from the vault |
| `paused: several files name this root` | duplicate files |
| ``refused by a brake: ...; set `force: true` to override once`` | a [mass-change brake](editing.md#mass-change-brakes) stopped it |
| `skipped: ...` | the file could not be read or parsed |
| `error: ...` | a per-file error, such as `error: no checkpoint for root ...` |
| ``not initialised: run `syncflowy render`, or set `force: true` to adopt the file`` | seen before the first cycle for a file with no checkpoint; after a cycle the row shows `error: no checkpoint ...` instead |
| `not synced yet` | no successful cycle yet in this run (also shown while the API is unreachable) |

### Errors

- **Per-file problems never stop the daemon.** A brake, a file that does not parse, a missing checkpoint or a failed
  operation is logged, shown in the status note and retried. A file that failed is retried at every cycle, and the
  failure is logged once, until its content changes.
- **Network and API outages do not stop it either.** The cycle is logged as `the cycle failed: ...`, shown under
  API errors and as `All files: ...` under Problems, and retried at the next cycle:

  ```text
  [2026-10-15T01:24:05Z WARN  syncflowy::daemon] the cycle failed: API: transport error: io: Connection refused (os error 111)
  ```

- **These stop it, with exit code 2:** a rejected API key (401), an unusable state directory, a vault that is not a
  directory, or the file watcher failing. On a 401 it writes a last entry to the status note
  (`fatal: API: invalid credentials; stopping`) and exits:

  ```console
  $ syncflowy --api-key wrong-key daemon --vault vault
  [2026-10-15T01:23:45Z WARN  syncflowy::daemon] fatal: API: invalid credentials; stopping
  error: API: invalid credentials: invalid credentials
  [exit 2]
  ```

- A state directory inside the vault only produces a warning (`the state directory ... is inside the vault; keep it
  outside`). Move it anyway.

Without `-v` the daemon logs only warnings. With `-v` it logs every operation it sends.

### Adopting a file without the command line

To start syncing a root from a device where you cannot run `syncflowy render` (a phone, say), write a file with
just a frontmatter naming the root and `force: true`, inside the `syncflowy:` map:

```markdown
---
workflowy: 3fef701966a0
syncflowy: {force: true}
---
```

At its next cycle the daemon adopts it in [safe mode](editing.md#safe-mode): it fills the file from Workflowy and
deletes nothing that the file does not mention. In the real run, the file became:

```markdown
---
workflowy: 3fef701966a0
syncflowy: {gen: 1}
---
- [ ] Call the plumber ^wf-d2803bab6c39
```

and the status note logged `no checkpoint: bootstrapped from Workflowy in safe mode (--force)` under Safe-mode
episodes.

### Stopping and restarting

Stop the daemon with Ctrl-C or SIGTERM (`kill <pid>`). There is no graceful shutdown and no pid file, and the
status note is not updated on stop. This is safe: every file and state write is atomic, and a sync interrupted
part-way is recovered by the next run, CLI or daemon (see [after a crash](editing.md#after-a-crash)). Pending
deletes are kept in the state directory; the status note's event log is not.

You can run `syncflowy status` while the daemon runs. You can also run `syncflowy sync` by hand next to it on the same
state directory (they take turns through a per-root lock), but remember that `sync` sends deletes at once.

### Running it as a service

The intended set-up is **one daemon on an always-on machine** (for example a small server) that holds the API key
and a copy of the vault, which Obsidian Sync keeps in step with your other devices. syncflowy is a single static
binary: copying it to the server is the installation. It is not an Obsidian plugin; nothing runs inside Obsidian.

What this guide cannot tell you (not covered by the project yet):

- how to keep the server's copy of the vault synced with Obsidian Sync (for example a desktop Obsidian left
  running there); this has not been specified or tested;
- a ready-made service definition: the project ships no systemd unit, launchd file or container image.

If you run it under a supervisor such as systemd, give it a restart delay: a bad API key makes the daemon exit
immediately, and an instant restart would loop. On the server, do not run Obsidian plugins that rewrite lines in
synced files (Tasks, Linter, templaters).

## The state directory

syncflowy keeps what it needs to tell your edits from Workflowy's in the state directory:

```text
<state_dir>/
  roots/<root uuid>/
    checkpoint.json        what the file and the subtree looked like at the last sync
    pending.json           the daemon's pending deletes
    history/<gen>.json     the Markdown and the subtree of recent generations
    journal.jsonl          a write-ahead journal (only while a sync is in progress)
    lock                   held while a sync runs on this root
```

The history keeps the newest 200 generations and at most 30 days. It contains your notes' text, so keep it as
private as the notes themselves.

**Keep it outside the vault**, and never sync it between devices. Two devices sharing one checkpoint amount to two
engines, which is not supported, and its files would show up as notes. The CLI does not check this; the daemon only
warns.

**One engine per account.** The lock only stops two syncs from the same state directory running at the same moment.
A second `sync` then reports `skipped: another sync holds the root's lock` (exit 1), a second `render` fails with
`root ... is locked by another process` (exit 2). Nothing stops two machines with separate state directories from
syncing the same root, so do not do it.

### If you lose it

Without a checkpoint, syncflowy no longer knows what the file looked like at the last sync, so it refuses to guess:

```console
$ syncflowy status
no synced roots in /…/state
[exit 0]
$ syncflowy sync vault/garden.md
vault/garden.md: error: no checkpoint for root cd613e30-d8f1-4adf-91b7-584a2265b1f5; run `syncflowy render` first or use --force
[exit 1]
```

You have two ways back:

1. **Keep the file and adopt it**: `syncflowy sync --force FILE` (or `syncflowy: {force: true}` in the file, for
   the daemon). Workflowy's current subtree becomes the base and the run is in [safe mode](editing.md#safe-mode): new
   lines and text edits in the file are sent; deletes, moves, reorders and ticks are not. The generation restarts
   at 1.

   ```console
   $ syncflowy sync --force vault/garden.md
   vault/garden.md: synced: 1 created, 0 updated, 0 moved, 0 reordered, 0 completed, 0 deleted (+0 hidden); file rewritten (gen 1)
     warning: no checkpoint: bootstrapped from Workflowy in safe mode (--force)
     op 0: create temp-1=1a2b8f1f-f1fd-42a2-9755-d4c13a902931 under cd613e30-d8f1-4adf-91b7-584a2265b1f5 (direct): done
     safe mode: 0 structural change(s) not applied (the file's base is unknown)
   ```

   Caution: text you edited since the last sync may overwrite a collaborator's newer edit of the same line (by
   design; not verified in practice). `--force` also disables the brakes for that run.

2. **Start the file over from Workflowy**: `syncflowy render --force ROOT FILE`. Unsynced edits in the file and your
   custom frontmatter are lost.

## The real service, the beta host and rate limits

**Production** (`https://workflowy.com`, the default) is what you normally use. Its API does not say which nodes are
mirrors: a mirror looks like an empty bullet. syncflowy shows such nodes as read-only `⧉ (empty or mirror)`
placeholders and refuses to change them without `--force` (see [mirrors](markdown.md#mirrors)).

**The beta host** (`--api-url https://beta.workflowy.com`) serves the same account and data, and tells syncflowy
about mirrors. syncflowy notices this by itself (no extra flag) as soon as the account has at least one mirror, and
then shows each mirror with its real content, which you can edit. In an account without any mirror, the beta host
looks exactly like production, and empty bullets are placeholders as there. Its behaviour was observed in October 2026 and is reproduced by the mock; syncflowy itself has
not been run against it.

**Rate limits.** syncflowy sends at most 10 requests per second and retries failures up to four times with growing
waits. Workflowy documents a limit of one full export per minute (it was not enforced when the
project tested production, but may be). Each `sync` run does one or two exports, and the
CLI does not throttle itself, so do not run `sync` in a tight loop; the daemon keeps to the limit with
`--min-export-interval 60`.
