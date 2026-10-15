# syncflowy user guide

syncflowy keeps a Workflowy outline and a Markdown file in step, so that you can work on part of your Workflowy
account from Obsidian (or any text editor) while everyone else keeps using Workflowy.

> **Experimental.** syncflowy has only been exercised against a mock of the Workflowy API, never against the real
> service. Try it on a test subtree first, and keep Workflowy's own Trash and history in mind as your safety net.

This guide has four pages:

| page | read it when |
|---|---|
| This page | you want to know what syncflowy does and get a first file syncing |
| [Command-line reference](cli.md) | you need a flag, an exit code, the daemon or the state directory |
| [The synced file](markdown.md) | you want to know what each line, tag and option in the file means |
| [What your edits do](editing.md) | you want to know what an edit will do in Workflowy, or a report line puzzles you |

## What syncflowy does

You pick one Workflowy node, the **sync root**. syncflowy writes that node's subtree into one Markdown file. From
then on, each sync does two things:

- it sends what you changed in the file to Workflowy (new lines, edited text, ticks, deletes, moves), and
- it brings what changed in Workflowy into the file.

One Workflowy node maps to one Markdown file. You can sync several roots, each into its own file. People who use
Workflowy directly notice nothing special: your edits arrive as ordinary edits.

You can run a sync by hand (`syncflowy sync FILE`), or leave the daemon running (`syncflowy daemon`), which watches
a vault and syncs every synced file in it continuously.

syncflowy talks to Workflowy's beta API (`https://beta.workflowy.com`), which serves the same account and data as
the normal site but reports mirrors properly. It refuses the production API host.

## How to think about the file

The file is a **view** of the subtree, not a copy you own. Keep these points in mind:

- **Each list item is a Workflowy node.** Nesting is the outline. The indented text under an item, before its
  first child, is that node's note.
- **The root's own name is not in the file.** The text before the first item is the root's note.
- **Completed nodes are hidden.** They, and everything under them, are left out of the file. They still exist in
  Workflowy.
- **Ids ride along as tags.** Each line ends with a tag such as `^wf-8069f23e2368`. That tag is how syncflowy
  knows which node a line is. In Obsidian it is a block id, so it is mostly out of the way. Leave tags alone.
- **The engine rewrites the file after a sync** when something you should see changed: a collaborator's edit, the
  tag of a line you added, a conflict. The frontmatter's `gen` number goes up each time it does.

## Where things can surprise you

Skim this list before your first edit. Each point is explained in [What your edits do](editing.md) or
[The synced file](markdown.md).

- **Ticking is not deleting.** `- [x]` completes the node in Workflowy and removes the line (and everything under
  it) from the file. You can only un-complete it in Workflowy.
- **Deleting a line deletes the node**, with its whole subtree, *including children you never saw in the file*:
  completed ones, and ones that the `depth:` option hides. It goes to Workflowy's Trash, from where you can restore
  it in Workflowy.
- **`depth:` hides children, it does not protect them.** Deleting a line whose children are cut off by `depth:`
  deletes them all. The report counts them in `(+N hidden)` and names the first ten.
- **Typing `- [ ]` does not make a Workflowy to-do.** Checkboxes in the file are cosmetic; a new line typed with a box
  is an ordinary bullet.
- **Reverting a line to an earlier text is ignored.** If you rename something, sync, and then type the old text
  back, syncflowy treats it as a stale copy and keeps the newer text. Edit it in Workflowy, or type something
  slightly different.
- **A cut and paste between two files must be synced in one run** (`syncflowy sync a.md b.md`, or the daemon). Syncing
  only the file you cut from holds the delete back with a warning until both are synced together; syncing only
  the file you pasted into creates a new node, and the original is then deleted. A tagged line *copied* (left in
  its old place too) is always a new node.
- **A line without its tag is matched back only nearby.** An untagged line under the same parent as a vanished tag,
  near its place with similar text or alone in its exact place, takes that node over: deleting a line and typing a new one in its place renames the node.
  Elsewhere the node is deleted and a new one created, losing anything only Workflowy knew about it (its layout,
  completed children, history).
- **`%%comments%%` you type are visible text in Workflowy.** They stay comments in Obsidian; a `%%...%%` typed in
  Workflowy is hidden in Obsidian.
- **Typing `- ---` over an item turns it into a divider and empties its text** in Workflowy.
- **`force` goes inside the `syncflowy:` map**: `syncflowy: {gen: 3, force: true}`. A separate `force: true` line is
  ignored, with a warning. (A separate `paused: true` line does pause the file, but write it inside the map too.)
- **A collaborator's change can win without a marker.** If they rename a node while you tick it (and rename it),
  their name is kept; your text appears only in the report, as `conflict on the name of ..., which you ticked`.

## What you need

- **A Workflowy API key.** syncflowy reads it from `WORKFLOWY_API_KEY` or `--api-key`. Note: where to get one is not
  documented in this repository; Workflowy is believed to issue keys at `https://workflowy.com/api-key` while you
  are logged in (to be confirmed). Treat the key like a password.
- **The `syncflowy` binary.** It is a single file. The project's CI builds a static Linux binary
  (`x86_64-unknown-linux-musl`); you can also build it from source, see the
  [engine README](../../engine/README.md#build-and-test).
- **A state directory outside your vault.** syncflowy keeps a checkpoint per root there (what the file and the
  subtree looked like at the last sync). The default is `~/.local/state/syncflowy`. It must not be synced between
  devices by Obsidian Sync or anything else. See [The state directory](cli.md#the-state-directory).
- **One engine per account.** Run syncflowy in exactly one place (one machine, one state directory), and give every
  `render`, `sync` and `daemon` for your files that same state directory. Two copies with separate state
  directories syncing the same root are not supported and nothing stops them.
- **The id of the node you want to sync.** Any of these work: the URL of the node
  (`https://workflowy.com/#/a2e0b9174736`), its 12-character short id (`a2e0b9174736`), or its full UUID. The URL
  form is what the web app shows when you zoom into a bullet (not verified against the live web app).

## Quick start

This takes about five minutes. Every transcript below is real output (from a run against a local mock of the
API). The example root is a node called "Garden plan" with the note "Spring 2027 planting". Other pages of this
guide come from separate test runs, so the same 12-character id can name different nodes on different pages.

### 1. Set up your environment

```sh
export WORKFLOWY_API_KEY=...                       # your key
export SYNCFLOWY_STATE_DIR=$HOME/.local/state/syncflowy   # optional: the default unless XDG_STATE_HOME is set
```

### 2. Render the subtree into a new file

```console
$ syncflowy render 8f5d06e0-e76a-44af-b5d8-a2e0b9174736 vault/garden.md
vault/garden.md: rendered (gen 1)
```

`render` refuses to overwrite an existing file, and refuses a root that is already being synced; both errors
point to `--force`. See [render](cli.md#render).

### 3. Look at the file

```markdown
---
workflowy: 8f5d06e0-e76a-44af-b5d8-a2e0b9174736
syncflowy: {gen: 1}
---
Spring 2027 planting

- Vegetables ^wf-3be53d2763ad
	Raised beds by the fence
	- Tomatoes ^wf-8069f23e2368
		- [ ] Buy stakes ^wf-b0a1fa4da2ae
	- [ ] Lettuce ^wf-ac22287add1f
- [ ] Order seeds ^wf-0bfe464630d8
- [ ] Read [the planting guide](https://example.com/guide) ^wf-0f52eb3567ff
```

- The frontmatter names the root (`workflowy:`) and holds syncflowy's options and its generation (`gen`).
- "Spring 2027 planting" is the root's note. The root's name, "Garden plan", is not in the file.
- "Raised beds by the fence" is the note of "Vegetables".
- Leaves and Workflowy to-dos get a `[ ]` box; parents do not. This is cosmetic.
- The link in Workflowy became a Markdown link.

### 4. Edit, preview, sync

Rename one line and add one:

```diff
 	- Tomatoes ^wf-8069f23e2368
 		- [ ] Buy stakes ^wf-b0a1fa4da2ae
-	- [ ] Lettuce ^wf-ac22287add1f
+	- [ ] Lettuce, two rows ^wf-ac22287add1f
+	- Carrots
 - [ ] Order seeds ^wf-0bfe464630d8
```

`--dry-run` shows what would be sent, and sends nothing:

```console
$ syncflowy sync --dry-run vault/garden.md
vault/garden.md: dry run: 2 op(s)
  op 0: create temp-1 under 1cebb573-ce52-4419-a056-3be53d2763ad (direct)
  op 1: update name of fd4f092e-d71d-4643-a663-ac22287add1f
```

Then sync for real:

```console
$ syncflowy sync vault/garden.md
vault/garden.md: synced: 1 created, 1 updated, 0 moved, 0 reordered, 0 completed, 0 deleted (+0 hidden); file rewritten (gen 2)
  op 0: create temp-1=391d4919-cd99-432b-9ce6-2dab2776c2ac under 1cebb573-ce52-4419-a056-3be53d2763ad (direct): done
  op 1: update name of fd4f092e-d71d-4643-a663-ac22287add1f: done
```

The new line now has its tag, and the file is at generation 2:

```markdown
	- [ ] Lettuce, two rows ^wf-ac22287add1f
	- [ ] Carrots ^wf-2dab2776c2ac
```

Changes made in Workflowy come into the file the same way: run `sync` again, and the file is rewritten with them.
When nothing changed on either side, the report is just `vault/garden.md: unchanged`.

### 5. Check the state

`status` only reads the local state directory (no API calls, no key needed, nothing created):

```console
$ syncflowy status
8f5d06e0-e76a-44af-b5d8-a2e0b9174736: gen 2, last synced at 1792103066 (unix), file /…/vault/garden.md
  the file is as last written
```

Before the sync it said `the file has local changes not yet synced`.

### 6. Keep it running (optional)

Instead of running `sync` by hand, let the daemon watch the whole vault:

```sh
syncflowy daemon --vault ~/vault
```

It syncs a file about a minute after you stop editing it (45 seconds of quiet, then the next cycle), picks up
Workflowy changes every minute, waits at least 10 minutes before sending a delete, and writes a
`Workflowy Sync Status.md` note into the vault. Ctrl-C (or SIGTERM) stops it cleanly after the cycle in progress.
See [The daemon](cli.md#daemon).

## Next steps

- [Command-line reference](cli.md): every command and flag, exit codes, the daemon and its status note, and the
  state directory.
- [The synced file](markdown.md): the frontmatter options, headings mode, id tags, formatting, layouts, read-only
  items, mirrors and conflict markers.
- [What your edits do](editing.md): each kind of edit and its effect in Workflowy, collaborators, the safety
  brakes, safe mode, recovery, and a troubleshooting table.
- For how the engine works inside, see the [design](../sync-design.md) and the
  [engine README](../../engine/README.md).
