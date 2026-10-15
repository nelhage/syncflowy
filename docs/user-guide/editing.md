# What your edits do

This page goes through each kind of edit you can make in a synced file, and what it does in Workflowy. Each section
shows a real before and after. The last section, [troubleshooting](#troubleshooting), maps report lines and messages
to their causes.

Note: all of this was verified against a mock of the Workflowy API, not the real service. The examples come from several separate test runs, so the same 12-character id can name different nodes in
different examples.
Rendered files are often shown without their frontmatter; other cuts are marked with `…` or described in the text.

- [Before you start](#before-you-start)
- [Adding lines](#adding-lines)
- [Editing text, notes and formatting](#editing-text-notes-and-formatting)
- [Ticking](#ticking)
- [Deleting](#deleting)
- [Moving, indenting and reordering](#moving-indenting-and-reordering)
- [Moving between files](#moving-between-files)
- [Changing a node's kind](#changing-a-nodes-kind)
- [Id-tag accidents](#id-tag-accidents)
- [Working alongside collaborators](#working-alongside-collaborators)
- [Mass-change brakes](#mass-change-brakes)
- [Paused, missing, broken and duplicate files](#paused-missing-broken-and-duplicate-files)
- [Safe mode](#safe-mode)
- [Read-only items](#read-only-items)
- [Mirrors](#mirrors)
- [Pending deletes in the daemon](#pending-deletes-in-the-daemon)
- [After a crash](#after-a-crash)
- [Troubleshooting](#troubleshooting)

## Before you start

- **Each sync reports what it sent**, one line per file:
  `synced: N created, N updated, N moved, N reordered, N completed, N deleted (+N hidden); file rewritten (gen G)`.
  `file unchanged` means your edits were sent but the file did not need rewriting. See
  [the report](cli.md#the-report).
- **Workflowy changes always come in** at the next sync of the file: renames, new nodes, completions (the line
  disappears), un-completions (it reappears), moves out of the subtree (it disappears).
- **The engine sometimes re-spells your Markdown** when it rewrites the file: `__bold__` becomes `**bold**`, spaces
  become tabs, `*` markers become `-`. The meaning stays the same.
- **`sync` sends deletes at once. The daemon waits** ([pending deletes](#pending-deletes-in-the-daemon)).
- Use `syncflowy sync --dry-run FILE` to see what an edit would do before sending it.

The examples below mostly use one file, `plan.md`, rendered from a node "Plan":

```markdown
---
workflowy: 1e2feb89-414c-443c-9027-c4d1c386bbc4
syncflowy: {gen: 1}
---
- Alpha ^wf-6f447ed4d57b
	- [ ] Alpha child ^wf-7696a6cecc1b
- [ ] Beta ^wf-2c8218072e8c
	a note on Beta
- [ ] Gamma ^wf-828dd5f4b3b2
```

## Adding lines

**A line without a tag is a new node.** Type it where you want it; nest it to make a child. After the sync it has a
tag.

Typed into `plan.md`: a child of Alpha, an item between Beta and Gamma, a three-level subtree, and a line typed with
a box:

```markdown
- Alpha ^wf-6f447ed4d57b
	- [ ] Alpha child ^wf-7696a6cecc1b
	- Nested new item
- [ ] Beta ^wf-2c8218072e8c
	a note on Beta
- Between Beta and Gamma
- [ ] Gamma ^wf-828dd5f4b3b2
- New at the end
	- its child
		- grandchild
- [ ] A typed checkbox item
```

```console
$ syncflowy sync vault/plan.md
vault/plan.md: synced: 6 created, 0 updated, 0 moved, 3 reordered, 0 completed, 0 deleted (+0 hidden); file rewritten (gen 2)
  op 0: create temp-1=b2221a58-008a-45a6-8464-7159c324c985 under 78e51061-7311-48a3-82ce-6f447ed4d57b (direct): done
  ...
  op 8: move temp-6=afbd67f9-6196-49cf-a198-8ad9f06c144a to the bottom of 1e2feb89-414c-443c-9027-c4d1c386bbc4: done
```

```markdown
- Alpha ^wf-6f447ed4d57b
	- [ ] Alpha child ^wf-7696a6cecc1b
	- [ ] Nested new item ^wf-7159c324c985
- [ ] Beta ^wf-2c8218072e8c
	a note on Beta
- [ ] Between Beta and Gamma ^wf-3d437204e52d
- [ ] Gamma ^wf-828dd5f4b3b2
- New at the end ^wf-d4c13a902931
	- its child ^wf-1193e6c3f339
		- [ ] grandchild ^wf-a7dd06839eb9
- [ ] A typed checkbox item ^wf-8ad9f06c144a
```

- The order in Workflowy matches the file. New nodes are created at the bottom and their neighbours moved into
  place, which shows up as `reordered`.
- **A line typed with `- [ ]` becomes an ordinary bullet**, not a Workflowy to-do. There is no option to change this.
- A line typed as `- [x] ...` is created and immediately completed.
- A `# Heading` typed at column 0 also becomes a node, with the items under it as its children (see
  [headings you type](markdown.md#headings-mode)).
- Code blocks, quotes and dividers can be typed too ([changing a node's kind](#changing-a-nodes-kind)).

## Editing text, notes and formatting

**Change the text and sync.** The name is the first line of an item. The note is the indented lines under it, before
its first child.

```markdown
- Alpha (renamed) ^wf-6f447ed4d57b
	A brand-new note on Alpha
	second note line
	- [ ] Alpha child ^wf-7696a6cecc1b
	- [ ] Nested new item ^wf-7159c324c985
- [ ] Beta ^wf-2c8218072e8c
	a note on Beta, edited
- [ ] Between Beta and Gamma ^wf-3d437204e52d
- [ ] **Gamma** with *italic*, `code`, ~~strike~~ and [a link](https://example.com) ^wf-828dd5f4b3b2
```

```console
$ syncflowy sync vault/plan.md
vault/plan.md: synced: 0 created, 4 updated, 0 moved, 0 reordered, 0 completed, 0 deleted (+0 hidden); file unchanged (gen 2)
  op 0: update name of 78e51061-7311-48a3-82ce-6f447ed4d57b: done
  op 1: update note of 78e51061-7311-48a3-82ce-6f447ed4d57b: done
  op 2: update name of 9b810e76-6ec9-4286-a3ca-828dd5f4b3b2: done
  op 3: update note of e4b06ce6-0741-47a8-bce4-2c8218072e8c: done
```

Workflowy now has:

```text
  [6f447ed4d57b] 'Alpha (renamed)'  note='A brand-new note on Alpha\nsecond note line'
  [2c8218072e8c] 'Beta'  note='a note on Beta, edited'
  [828dd5f4b3b2] '<b>Gamma</b> with <i>italic</i>, <code>code</code>, <s>strike</s> and <a href="https://example.com">a link</a>'
```

- Deleting all the note lines empties the note.
- Formatting maps as described in [inline formatting](markdown.md#inline-formatting). Obsidian-only syntax such as
  `[[Other note]]` and `==highlight==` reaches Workflowy as literal text; `%%comments%%` become visible text there.
- **Reverting is ignored.** If you change a line back to a text the engine wrote into the file earlier, syncflowy takes
  it for a stale copy (as a sync tool might bring back) and keeps the newer text:

  ```console
  $ syncflowy sync vault/garden.md      # "Compost bin (two)" typed back to "Compost bin"
  vault/garden.md: synced: 0 created, 0 updated, 0 moved, 0 reordered, 0 completed, 0 deleted (+0 hidden); file rewritten (gen 5)
    the name of "Compost bin (two)" (d4c13a902931) equals an older render; ignored as a stale echo
  ```

  To really go back, edit it in Workflowy, or type something slightly different.

## Ticking

**`- [x]` completes the node in Workflowy and removes the line and its subtree from the file.** It does not delete
anything. Only the ticked node is completed; its children stay as they are, hidden with it.

Between the previous example and this one, Beta's note was removed and the next line was edited to use `__Beta__`
and Obsidian-only syntax (`2 updated`). Two lines are then ticked (the Alpha lines above them are unchanged and left
out here):

```markdown
- [x] Beta ^wf-2c8218072e8c
- [ ] Between __Beta__ and Gamma, see [[Other note]] and ==highlight== ^wf-3d437204e52d
- [ ] **Gamma** with *italic*, `code`, ~~strike~~ and [a link](https://example.com) ^wf-828dd5f4b3b2
- [x] New at the end ^wf-d4c13a902931
	- its child ^wf-1193e6c3f339
		- [ ] grandchild ^wf-a7dd06839eb9
- [ ] A typed checkbox item ^wf-8ad9f06c144a
```

```console
$ syncflowy sync vault/plan.md
vault/plan.md: synced: 0 created, 0 updated, 0 moved, 0 reordered, 2 completed, 0 deleted (+0 hidden); file rewritten (gen 3)
  op 0: complete e4b06ce6-0741-47a8-bce4-2c8218072e8c: done
  op 1: complete 1a2b8f1f-f1fd-42a2-9755-d4c13a902931: done
```

```markdown
---
workflowy: 1e2feb89-414c-443c-9027-c4d1c386bbc4
syncflowy: {gen: 3}
---
- Alpha (renamed) ^wf-6f447ed4d57b
	A brand-new note on Alpha
	second note line
	- [ ] Alpha child ^wf-7696a6cecc1b
	- [ ] Nested new item ^wf-7159c324c985
- [ ] Between **Beta** and Gamma, see [[Other note]] and ==highlight== ^wf-3d437204e52d
- [ ] **Gamma** with *italic*, `code`, ~~strike~~ and [a link](https://example.com) ^wf-828dd5f4b3b2
- [ ] A typed checkbox item ^wf-8ad9f06c144a
```

Both ticked lines and the subtree under "New at the end" are gone, and `__Beta__` was re-spelled `**Beta**`.

In Workflowy, "Beta" and "New at the end" are completed; "its child" and "grandchild" are not.

- `[X]`, `1. [x]` and, in headings mode, `# [x] Title` work too.
- Edits you made to the same line are sent first, then the tick.
- **You cannot un-complete from Markdown.** Un-complete it in Workflowy and it comes back at the next sync, with its
  subtree, at its Workflowy position:

  ```console
  $ syncflowy sync vault/plan.md
  vault/plan.md: synced: 0 created, 0 updated, 0 moved, 0 reordered, 0 completed, 0 deleted (+0 hidden); file rewritten (gen 4)
    "New at the end" (d4c13a902931) reappeared (uncompleted in Workflowy)
  ```

- If a collaborator renamed the node while you ticked and renamed it, the tick wins and **your rename is dropped**
  silently: the node is completed with their name.
- Many ticks at once can trip the [completion brake](#mass-change-brakes).

## Deleting

**Deleting a line deletes the node and its whole subtree in Workflowy, including descendants you never saw in the
file: completed ones, and ones that the `depth:` option hides.** Workflowy puts deleted nodes in its Trash, from where you can restore them in Workflowy (not from
Markdown).

Here a collaborator had completed "Alpha child", so Alpha had a hidden child. Deleting the Alpha line (with its note
and its visible child) and the Gamma line:

```console
$ syncflowy sync --dry-run vault/plan.md
vault/plan.md: dry run: 2 op(s)
  would Delete { node: NodeId("78e51061-7311-48a3-82ce-6f447ed4d57b"), hidden_descendants: 1, mirror: false }
  would Delete { node: NodeId("9b810e76-6ec9-4286-a3ca-828dd5f4b3b2"), hidden_descendants: 0, mirror: false }
$ syncflowy sync vault/plan.md
vault/plan.md: synced: 0 created, 0 updated, 0 moved, 0 reordered, 0 completed, 2 deleted (+1 hidden); file unchanged (gen 6)
  op 0: delete 78e51061-7311-48a3-82ce-6f447ed4d57b (+1 hidden): done
  op 1: delete 9b810e76-6ec9-4286-a3ca-828dd5f4b3b2 (+0 hidden): done
```

> **Warning.** `(+N hidden)` counts descendants that went with the delete but were not shown in the file, without
> naming them. Run `--dry-run` first and look for `hidden_descendants` greater than 0 if you are unsure. To finish
> something rather than remove it, tick it instead.
>
> **The count can miss children hidden by `depth:`.** In a test with `depth: 1`, deleting a line "A" also deleted
> its hidden children B and C, but the report said `1 deleted (+0 hidden)` and the dry run `hidden_descendants: 0`.
> A line whose children are cut off by `depth:` looks like a leaf (it has a `[ ]`). In another test, such a line
> hid 60 children; deleting it was stopped by the brake as
> `refusing to delete 1 visible and 60 hidden nodes out of 2 (use --force)`. In a file with `depth:`, check a node
> in Workflowy before you delete its line.

- Deleting only a parent line, and leaving its indented children in place, re-parents the children to the line
  above (as Markdown indentation reads) and deletes the parent alone.
- If a collaborator changed the node (or its subtree) meanwhile, your delete is not sent and the line comes back:
  `the delete of "..." (c8d60b21fbac) was not sent: it changed in Workflowy`.
- Large deletes are stopped by the [brakes](#mass-change-brakes). In the daemon, deletes wait for a
  [grace period](#pending-deletes-in-the-daemon).

## Moving, indenting and reordering

**Move a line with its tag, and the node moves.** Indenting makes it a child of the line above; outdenting moves it
up a level; moving it up or down reorders it. Nodes keep their ids, notes and children.

Before:

```markdown
- Item A ^wf-3fef701966a0
	- [ ] A1 ^wf-d2803bab6c39
	- [ ] A2 ^wf-789a380208a9
- [ ] Item B ^wf-89d94a2f20aa
- [ ] Item C ^wf-a8f0d66b829e
- [ ] Item D ^wf-9e3fa46d6753
- [ ] Item E ^wf-67edfe175330
```

After moving Item E to the top, outdenting A1, cutting A2 and pasting it under Item C, and indenting Item D under
Item C:

```markdown
- [ ] Item E ^wf-67edfe175330
- Item A ^wf-3fef701966a0
- [ ] A1 ^wf-d2803bab6c39
- [ ] Item B ^wf-89d94a2f20aa
- [ ] Item C ^wf-a8f0d66b829e
	- [ ] A2 ^wf-789a380208a9
	- [ ] Item D ^wf-9e3fa46d6753
```

```console
$ syncflowy sync vault/moves.md
vault/moves.md: synced: 0 created, 0 updated, 3 moved, 3 reordered, 0 completed, 0 deleted (+0 hidden); file unchanged (gen 1)
```

Workflowy now has the same order and nesting. ("Item A" kept its `- ` although it is now a leaf: box forms are
sticky.)

## Moving between files

**A cut and paste between two synced files is a move, but only if both files are synced in the same run.** Use one
`sync` command naming both files, or let the daemon do it (it always syncs the whole vault together).

Cutting "Item B" from `moves.md` and pasting it, with its tag, under "X in other" in `other.md`:

```console
$ syncflowy sync vault/moves.md vault/other.md
vault/moves.md: synced: 0 created, 0 updated, 0 moved, 0 reordered, 0 completed, 0 deleted (+0 hidden); file unchanged (gen 1)
vault/other.md: synced: 0 created, 0 updated, 1 moved, 0 reordered, 0 completed, 0 deleted (+0 hidden); file unchanged (gen 1)
  op 0: move 6a8ac4ba-0580-4975-ad2f-89d94a2f20aa to the bottom of 803468b6-b610-49f7-b927-0f4eb8b333a8: done
  "Item B" (89d94a2f20aa) moved into this file from the file of "Moves" (b8bc076f3787)
```

> **Warning: sync both halves together.** Synced in two separate commands, the same cut and paste is a delete and a
> new node. The first run deletes the original (to the Trash), the second creates a fresh copy with a new id:
>
> ```console
> $ syncflowy sync vault/moves.md
> vault/moves.md: synced: 0 created, 0 updated, 0 moved, 0 reordered, 0 completed, 1 deleted (+0 hidden); file unchanged (gen 1)
>   op 0: delete 4be03db0-dc25-44bd-b940-67edfe175330 (+0 hidden): done
> $ syncflowy sync vault/other.md
> vault/other.md: synced: 1 created, 0 updated, 0 moved, 0 reordered, 0 completed, 0 deleted (+0 hidden); file rewritten (gen 2)
>   op 0: create temp-1=3099fdf5-ab99-454a-a901-e35cd47d380d under e5446dd4-552b-42f6-be3e-dc0a1ef2a4f0 (direct): done
>   line 8: id tag 67edfe175330 does not belong to this file; treated as a new item
> ```
>
> Everything Workflowy-only about the original (its completed children, colour, mirrors, history) stays with the
> trashed node. Syncing the destination first does the same.

**Copying a tagged line into another file moves the node too.** A tag stands for one node, and there is only ever one.
Pasting `- A2 ^wf-789a380208a9` into `other.md` while it stayed in `moves.md`:

```console
$ syncflowy sync vault/moves.md vault/other.md
vault/moves.md: unchanged
vault/other.md: synced: 0 created, 0 updated, 1 moved, 0 reordered, 0 completed, 0 deleted (+0 hidden); file unchanged (gen 2)
  op 0: move f3c64af7-75a8-4294-82cd-789a380208a9 to the bottom of e5446dd4-552b-42f6-be3e-dc0a1ef2a4f0: done
  "A2" (789a380208a9) moved into this file from the file of "Moves" (b8bc076f3787)
$ syncflowy sync vault/moves.md vault/other.md
vault/moves.md: synced: 0 created, 0 updated, 0 moved, 0 reordered, 0 completed, 0 deleted (+0 hidden); file rewritten (gen 6)
  "A2" (789a380208a9) was moved out of this file in Workflowy (now under "Other file root" (dc0a1ef2a4f0))
vault/other.md: unchanged
```

The source file still showed the line after the first run, and dropped it at the next. **To copy text into another
file, copy it without the tag**; it then becomes a new node.

## Changing a node's kind

Some Workflowy layouts can be set from Markdown; the rest are kept as they are.

| you change | in Workflowy |
|---|---|
| `- ` to `- [ ] ` or back | nothing; boxes are cosmetic. A to-do stays a to-do without its box; a bullet does not become a to-do with one. |
| fence an item (```` - ``` ````, code indented, closing fence) | code block; the code becomes the (multi-line) name |
| remove the fence | bullet; the text becomes the name |
| `- > text`, or remove the `> ` | quote block, or bullet |
| change **all** children's markers to `1.`, or back to `- ` | the parent becomes numbered, or bullets (only for a bullets or numbered parent) |
| `- ---` over an item | divider, **and the name is emptied** |
| `- # text` inside a list | **not** a heading: the name becomes the literal text `# text` (written back `\# text`) |
| a fence with an info string (```` ```python ````) | a code block whose first code line is `python` |
| Workflowy-only layouts (to-do, h1 to h3, board, paragraph, ...) | kept as they are when you edit the text (a to-do shows `[ ]`; the others are plain items) |

A real run that changed several kinds at once:

````markdown
- [ ] Item A ^wf-3fef701966a0
- ``` ^wf-d2803bab6c39
	A1 = code()
	second_line()
	```
- [ ] Item C ^wf-a8f0d66b829e
	1. A2 ^wf-789a380208a9
	2. Item D ^wf-9e3fa46d6753
- > Quote me ^wf-ab134da98f1d
- # Heading me ^wf-3116e1ea24c4
- --- ^wf-d2bc815a47c5
- [ ] Todo me ^wf-27bc08d6af57
- > A new quote
- ```
	new_code()
	```
- ---
````

```console
$ syncflowy sync vault/moves.md
vault/moves.md: synced: 3 created, 7 updated, 0 moved, 0 reordered, 0 completed, 0 deleted (+0 hidden); file rewritten (gen 3)
  ...
  op 4: set layout of ad45f23d-3b1a-41df-987f-d2803bab6c39 to code-block: done
  op 5: update name of da711448-96c8-4a19-a4b2-d2bc815a47c5: done
  op 6: set layout of da711448-96c8-4a19-a4b2-d2bc815a47c5 to divider: done
  op 7: set layout of ec148cb4-8e73-4a47-aa90-a8f0d66b829e to numbered: done
  op 8: update name of f0dfb4a5-d8a0-44df-bfd6-3116e1ea24c4: done
  op 9: set layout of f9341c68-966b-4ea1-88be-ab134da98f1d to quote-block: done
```

```text
  [d2803bab6c39] 'A1 = code()\nsecond_line()'  (layout=code-block)
  [a8f0d66b829e] 'Item C'  (layout=numbered)
  [ab134da98f1d] 'Quote me'  (layout=quote-block)
  [3116e1ea24c4] '# Heading me'
  [d2bc815a47c5] ''  (layout=divider)
```

The divider's old text ("Divider me") is gone; only Workflowy's history has it. Unfencing a code block may report
`the name of "A1 unfenced" (d2803bab6c39) lost formatting Markdown could not show` even when no formatting was
involved; you can ignore it then.

## Id-tag accidents

**Leave tags alone.** If you lose one anyway:

| what happened | result |
|---|---|
| you removed a tag but kept the text (or changed it slightly) | the line is matched back to its node and the tag restored: `line 10: untagged item matched to "Item D" (9e3fa46d6753)` |
| you removed a tag **and** changed the text a lot | the old node is **deleted** (to the Trash) and a new one created, losing its Workflowy-only attributes such as a to-do layout |
| you copied a tagged line within the file | `id tag 789a380208a9 appears on lines 8, 9; line 8 keeps it, the others are new items`; the copy becomes a new node |
| a line ended up with two tags | `2 id tags on one line; the last one wins`. If the winning tag belongs to another line, this line can lose its node: the original is deleted and a new one created. |
| you typed a tag that exists nowhere | `id tag aaaaaaaaaaaa does not belong to this file; treated as a new item` |
| you pasted a tagged line into another synced file | the node moves there ([moving between files](#moving-between-files)) |

The matching rule: an untagged line counts as the vanished tagged line when it is under the same parent, at most 3
positions away, and its text is at least 60% similar. "Quote me" changed to "Quote me, slightly edited" (untagged) was
not similar enough, and became a delete plus a create.

## Working alongside collaborators

### Clean merges

**Edits to different nodes, or to different lines of one note, merge cleanly.** In one sync, a collaborator renamed
"Other task" and edited the first line of the note of "Noted task"; you edited "Task they complete" and the third
line of the same note. Both of you also renamed "Shared task" (that part is a [conflict](#conflicts), below). The file
after the sync:

```markdown
---
workflowy: 78255d68-0792-4986-bb96-8a437d5c8dfc
syncflowy: {gen: 2}
---
- [ ] Shared task (their wording) ^wf-c8d60b21fbac
	- ⚠ Shared task (my wording) %%wf-conflict:name%%
- [ ] Other task, renamed in Workflowy ^wf-3c25fbb230bb
- [ ] Task they complete, edited locally ^wf-06d864c2f2e3
- [ ] Task they delete ^wf-90c78092b4d4
- [ ] Task they move out ^wf-8131c541013d
- [ ] Task they move inside ^wf-e8f8dc3bf364
- [ ] Noted task ^wf-f92283868a29
	line one (edited in WF)
	line two
	line three (edited locally)
```

Workflowy has the same text: their rename of "Other task", yours of "Task they complete", and the note with both
line edits.

### Conflicts

**When you both changed the same name, or the same note line, Workflowy's version wins and yours is kept in the file
as a conflict marker.** In the sync above, both sides had renamed "Shared task":

```console
$ syncflowy sync vault/collab.md
vault/collab.md: synced: 0 created, 2 updated, 0 moved, 0 reordered, 0 completed, 0 deleted (+0 hidden); file rewritten (gen 2)
  op 0: update name of 2b28fef0-2b9c-414e-a5ac-06d864c2f2e3: done
  op 1: update note of d8f33418-f3d4-4711-9804-f92283868a29: done
  conflict on the name of "Shared task (their wording)" (c8d60b21fbac): Workflowy's version kept, yours is in the file
  conflict on the Name of d92a4aa2-b410-493c-8efb-c8d60b21fbac: resolve the marker in the file
```

The marker is the `⚠` line under "Shared task" in the file above. A note conflict looks like this, with your whole
note under the marker:

```markdown
- [ ] Noted task ^wf-f92283868a29
	line one (edited in WF)
	line two THEIRS
	line three (edited locally)
	- ⚠ local note %%wf-conflict:note%%
		line one (edited in WF)
		line two MINE
		line three (edited locally)
```

A conflict exits 0. The marker is never sent to Workflowy and stays (`unchanged`) until you deal with it. To resolve:

- **Keep theirs**: delete the marker line (and its indented note lines). Nothing is sent.
- **Keep yours, or a mix**: edit the real line (or note) to what you want, then delete the marker.

  ```markdown
  - [ ] Shared task (my wording, final) ^wf-c8d60b21fbac
  ```

  ```console
  $ syncflowy sync vault/collab.md
  vault/collab.md: synced: 0 created, 1 updated, 0 moved, 0 reordered, 0 completed, 0 deleted (+0 hidden); file unchanged (gen 2)
    op 0: update name of d92a4aa2-b410-493c-8efb-c8d60b21fbac: done
  ```

The daemon lists open conflicts in its status note under "Conflicts awaiting resolution".

If the node was moved out of your file in Workflowy, a conflict cannot carry a marker: Workflowy wins and your text
is only printed in the report (`... which was moved out of this file in Workflowy: Workflowy's version kept; yours was
"..."`; covered by tests, not run live). Copy it from there if you need it.

### When they completed, deleted or moved what you edited

| the collaborator | you | result |
|---|---|---|
| completed the node | edited its text, added a child | your edits are sent to the completed node, then the line leaves the file: `... was pushed but is hidden: ... was completed in Workflowy` |
| deleted the node | edited its text | it is **re-created as a new node** with a new id from your line (`90c78092b4d4 was deleted in Workflowy but edited in Markdown: re-created as a new node (temp-2)`); the original stays in the Trash |
| moved it out of your subtree | edited its text | your edit is sent to it where it is now; the line leaves the file (`... was moved out of this file in Workflowy (now top-level)`) |
| moved it within your subtree | edited its text | both happen: the line moves in the file and carries your edit |
| edited it | deleted the line | your delete is not sent and the line comes back with their text |
| renamed it | ticked it and renamed it | it is completed with **their** name; your rename is dropped without a marker |

So typed text is not lost when a collaborator deletes or completes a node, but it can end up somewhere you no longer
see it (a completed node, a new node, outside the subtree).

## Mass-change brakes

**A sync that would delete or complete a lot at once is refused, and nothing is written** (not to Workflowy, not to
the file, not to the checkpoint). The run exits 1. The thresholds, compared with the number of nodes the file showed
at the last sync:

| brake | refused when | message |
|---|---|---|
| empty file | the file has no items left and the sync would delete something, or **every** visible node would be deleted (also when you replaced all lines with new ones) | `the file is empty but N nodes were visible (use --force)` |
| deletes | more than 50 deletes, or at least 5 deletes that are more than 20% of the visible nodes. Descendants not shown in the file (completed, or never shown because of `depth:`) count. | `refusing to delete V visible and H hidden nodes out of N (use --force)` |
| ticks | more than 200 ticks, or at least 10 ticks that are more than 50% of the visible nodes | `refusing to complete C nodes out of N (use --force)` |

Real results on a 12-item file where "Item 1" had a hidden completed child:

```console
== body emptied (frontmatter kept)
$ syncflowy sync vault/big.md
vault/big.md: refused: the file is empty but 12 nodes were visible (use --force)
  warning: brake: the file is empty but 12 nodes were visible (use --force)
[exit 1]
== Items 2-5 deleted: 4 deletes, under the minimum of 5, so allowed
== Items 1-4 deleted: 4 visible + 1 hidden = 5, more than 20% of 12
$ syncflowy sync vault/big.md
vault/big.md: refused: refusing to delete 4 visible and 1 hidden nodes out of 12 (use --force)
  warning: brake: refusing to delete 4 visible and 1 hidden nodes out of 12 (use --force)
[exit 1]
```

Ticks: 7 of 12 went through (fewer than 10); 11 of 17 were refused:

```console
$ syncflowy sync vault/ticks.md
vault/ticks.md: refused: refusing to complete 11 nodes out of 17 (use --force)
  warning: brake: refusing to complete 11 nodes out of 17 (use --force)
[exit 1]
```

**If the change is what you meant**, override the brake once:

- on the command line: `syncflowy sync --force FILE`. Try `--dry-run --force` first to see what it would do.
- in the file (works with the daemon too): `syncflowy: {gen: 1, force: true}`. The engine removes `force: true` after
  that one sync.

```console
$ syncflowy sync --force vault/big.md
vault/big.md: synced: 0 created, 0 updated, 0 moved, 0 reordered, 0 completed, 4 deleted (+1 hidden); file unchanged (gen 1)
  op 0: delete f463b337-d20b-4d59-9b61-0487c89da11b (+1 hidden): done
  op 1: delete c7038069-84c8-4999-a116-7d8fcf23cae8 (+0 hidden): done
  op 2: delete f320cd57-6d14-475b-b49a-ae908fb5262c (+0 hidden): done
  op 3: delete 5d5f576c-deb8-4c4c-bb29-7d0b0e5e18ba (+0 hidden): done
```

**If it is not what you meant** (an editor or sync tool emptied the file, say), restore the file from a backup or undo
in your editor, and sync again. In the daemon, a brake fires at once (it does not wait for the delete grace period),
the root's state reads ``refused by a brake: ...; set `force: true` to override once``, and it is retried each cycle
until the file changes.

`force` also overrides the placeholder guards ([mirrors](#mirrors)) and lets a forced edit of a read-only item
through, so use it for one deliberate sync, not as a habit.

## Paused, missing, broken and duplicate files

### Pausing a file

Put `paused: true` **inside** the `syncflowy:` map:

```markdown
---
workflowy: 62397bc7-0176-4741-bab9-f87ff5059285
syncflowy: {gen: 2, paused: true}
---
- [ ] Item 10 edited while paused ^wf-440407295e42
```

```console
$ syncflowy sync vault/big.md
vault/big.md: paused (paused: true in the frontmatter)
[exit 0]
```

Nothing is sent and nothing comes in while paused. Remove it, and the next sync does both: your edit went to
Workflowy and the collaborator's rename of "Item 11" came into the file:

```markdown
- [ ] Item 10 edited while paused ^wf-440407295e42
- [ ] Item 11 edited in WF while paused ^wf-5cd12d5db79b
```

### A missing file

If the file is gone, the root is paused and **nothing is deleted**:

```console
$ syncflowy sync vault/big.md            # file moved away
[2026-10-15T01:24:25Z WARN  syncflowy::sync] vault/big.md: missing; root paused (nothing is deleted)
vault/big.md: paused: the file is missing; nothing was changed in Workflowy
[exit 1]
```

Files are known by their `workflowy:` root, not their path: a renamed or moved file just works when you sync it
under its new name (`vault/renamed-big.md: unchanged`). To stop syncing a root for good, delete its file; the daemon
then shows it as `paused: no file names this root; nothing is deleted`.

### A broken frontmatter

Nothing is sent and the run exits 1. A broken frontmatter is `skipped`; a root that does not exist is an `error`:

```console
== closing --- removed
vault/renamed-big.md: skipped: the file does not parse: unterminated frontmatter
== syncflowy: {gen: 3, headings: 99}
vault/renamed-big.md: skipped: the file does not parse: bad syncflowy config headings: "99"
== workflowy: line removed
vault/renamed-big.md: skipped: the file does not parse: frontmatter has no workflowy: root
== workflowy: names a root that does not exist
vault/renamed-big.md: error: sync root "aaaaaaaa-0176-4741-bab9-f87ff5059285" not found
```

Two cases are not errors: an unknown key in `syncflowy:` (a warning, `unknown syncflowy key "colour" (dropped on the
next render)`), and a damaged `gen` ([safe mode](#safe-mode)). Fix the frontmatter and sync again.

### Two files for one root

A copy of a synced file (for example a sync-conflict copy) names the same root. Synced together, both are paused:

```console
$ syncflowy sync vault/renamed-big.md "vault/renamed-big (conflict copy).md"
[2026-10-15T01:24:39Z WARN  syncflowy::sync] root 62397bc7-0176-4741-bab9-f87ff5059285 is claimed by several files: ["vault/renamed-big.md", "vault/renamed-big (conflict copy).md"]; not synced
vault/renamed-big.md: paused: several files claim this root: vault/renamed-big.md, vault/renamed-big (conflict copy).md
vault/renamed-big (conflict copy).md: paused: several files claim this root: vault/renamed-big.md, vault/renamed-big (conflict copy).md
[exit 1]
```

> **Caution.** The command line only notices duplicates among the files you name in one command: the copy synced on
> its own was accepted (`synced: ... file rewritten (gen 5)`). The daemon checks the whole vault, and never syncs
> files named like conflict copies. Merge any edits from the copy into the real file by hand, then delete the copy.

## Safe mode

**When syncflowy cannot tell which version of the file you edited, it only applies changes that are safe to apply
without knowing.** This happens when `gen` is missing, damaged or unknown (a stale or hand-written file), and when a
file is adopted with `force` after a lost checkpoint.

In safe mode:

- **applied**: text edits (where the text differs from every version of that line still in the engine's history) and new
  lines (added at the end of their parent);
- **not applied**: deletes, moves, reorders, **and ticks**.

A file with `gen: 999`, where you deleted Item 10, renamed Item 11, moved Item 15 above Item 12, ticked Item 13,
indented and renamed Item 14, and added a line:

```console
$ syncflowy sync vault/renamed-big.md
vault/renamed-big.md: synced: 1 created, 2 updated, 0 moved, 0 reordered, 0 completed, 0 deleted (+0 hidden); file rewritten (gen 8)
  warning: the file's gen (999) is unknown: safe mode (no deletes, moves or reorders)
  op 0: create temp-1=4c41d9c0-f075-44fe-aacc-110e4f73fd94 under 62397bc7-0176-4741-bab9-f87ff5059285 (direct): done
  op 1: update name of 0c250a03-e023-433d-b64e-433ff7c882f4: done
  op 2: update name of 2e47dc0e-959f-4a51-8cfe-5cd12d5db79b: done
  safe mode: 4 structural change(s) not applied (the file's base is unknown)
```

```markdown
---
workflowy: 62397bc7-0176-4741-bab9-f87ff5059285
syncflowy: {gen: 8}
---
- [ ] Item 10 edited while paused ^wf-440407295e42
- [ ] Item 11 renamed in safe mode ^wf-5cd12d5db79b
- [ ] Item 12 ^wf-308cdc6b13ab
- [ ] Item 13 ^wf-9ae80f0c8a89
- [ ] Item 14 indented under 13 ^wf-433ff7c882f4
- [ ] Item 15 ^wf-28004e6f5a94
- [ ] a new item typed in safe mode ^wf-110e4f73fd94
```

The deleted line is back, the tick undone, the move and indent undone; the renames and the new line went through.
Note that the warning says "no deletes, moves or reorders" but **ticks are dropped too**. The file now has a good
`gen`, so redo the structural changes and sync again.

Safe mode also lets you start syncing a hand-written file. With `--force` (or `force: true`), syncflowy adopts it,
sends its new lines, and deletes nothing it does not mention:

```console
$ syncflowy sync --force vault/hand.md
vault/hand.md: synced: 2 created, 0 updated, 0 moved, 0 reordered, 0 completed, 0 deleted (+0 hidden); file rewritten (gen 1)
  warning: no checkpoint: bootstrapped from Workflowy in safe mode (--force)
  op 0: create temp-1=973082d6-09b4-45d2-99bc-1d97e0f3a7ef under 909ff497-6a8a-43ef-a880-4790be6c6fe9 (direct): done
  op 1: create temp-2=e69bae29-f652-4008-b7b4-000bd1c51f86 under 909ff497-6a8a-43ef-a880-4790be6c6fe9 (direct): done
  safe mode: 1 structural change(s) not applied (the file's base is unknown)
```

```markdown
---
workflowy: 4790be6c6fe9
syncflowy: {gen: 1}
---
- [ ] Existing child ^wf-5022409a8a78
- [ ] My first line ^wf-1d97e0f3a7ef
- [ ] My second line ^wf-000bd1c51f86
```

"Existing child", which the hand-written file did not mention, was kept (that is the "1 structural change not
applied").

## Read-only items

**Text edits to a `🔒` item are refused and the line is put back. Everything else works.** A read-only item is one
whose Workflowy text syncflowy cannot show exactly ([read-only items](markdown.md#read-only-items)).

```markdown
- [ ] 🔒 Two-line ⏎ name ^wf-e066de26e655-ro
- 🔒 unclosed bold ^wf-7dafb43adc4f-ro
	- [ ] child of malformed ^wf-e73d60c290d0
```

Editing the locked text of the first item, editing the child of the second, and adding a new child under the
second:

```console
$ syncflowy sync vault/ro.md
vault/ro.md: synced: 1 created, 1 updated, 0 moved, 0 reordered, 0 completed, 0 deleted (+0 hidden); file rewritten (gen 2)
  op 0: create temp-1=7e0ab2ed-31b1-427e-9766-99cc6ed5d1bf under 0994940e-8245-4cc8-9f7a-7dafb43adc4f (direct): done
  op 1: update name of 1959b9ef-58d0-4674-b34d-e73d60c290d0: done
  line 5: "Two-line
name" (e066de26e655) is read-only (or a mirror placeholder); the edit was not sent
```

(The refusal quotes the two-line name, so it spans two lines.)

| on a `🔒` item | result |
|---|---|
| edit its name or note | refused, line put back (exit 0) |
| remove `🔒` and `-ro` and edit | still refused: syncflowy remembers |
| tick it | allowed: `1 completed` |
| delete it | allowed |
| edit or add its children | allowed |
| edit with `--force` | sent; the lock goes away. Workflowy's original formatting is lost (`lost formatting Markdown could not show`). |

The safest fix is to correct the node in Workflowy; the lock disappears at the next sync.

## Mirrors

### Placeholders (normal service)

**On the normal service, `⧉ (empty or mirror)` lines cannot be edited, ticked or deleted without `force`.** Typing over
one, adding a child under it, and deleting another:

```markdown
- [ ] Plain item ^wf-4b10a7415111
- typing into the placeholder ^wf-6cf376e39b51-ro
	- a child typed under it
- [ ] Genuinely empty bullet follows ^wf-59b4f1d47eb7
```

```console
$ syncflowy sync vault/prod-mirrors.md
vault/prod-mirrors.md: synced: 1 created, 0 updated, 0 moved, 2 reordered, 0 completed, 0 deleted (+0 hidden); file rewritten (gen 2)
  ...
  line 6: "" (6cf376e39b51) is read-only (or a mirror placeholder); the edit was not sent
  "" (04f6664d637c) is a mirror placeholder; its delete was not sent (use force)
```

```markdown
- [ ] Plain item ^wf-4b10a7415111
- ⧉ (empty or mirror) ^wf-6cf376e39b51-ro
- [ ] a child typed under it ^wf-a218479ebcd6
- [ ] Genuinely empty bullet follows ^wf-59b4f1d47eb7
- ⧉ (empty or mirror) ^wf-04f6664d637c-ro
```

The text was put back, the deleted placeholder came back, and the child typed under the placeholder was created next
to it instead (with no report line of its own). `sync --force` sends the delete:

```console
$ syncflowy sync --force vault/prod-mirrors.md     # second placeholder line deleted again
vault/prod-mirrors.md: synced: 0 created, 0 updated, 0 moved, 0 reordered, 0 completed, 1 deleted (+0 hidden); file unchanged (gen 2)
  op 0: delete 1a021cb2-9afe-4b27-9f1e-04f6664d637c (+0 hidden): done
```

> **Warning.** A placeholder may be a real mirror that your collaborators see full of content. Deleting it with
> `--force` removes that mirror for them.

### Mirrors and stubs (beta host)

On the beta host, a mirror is shown with its original's content ([mirrors](markdown.md#mirrors)):

```markdown
- Local item ^wf-cca73ecd17ef
	- [ ] LI child ^wf-fb2195a8b59a
- Origin O ^wf-202a602ede7a
	- [ ] O child 1 ^wf-6f401eac4729
	- [ ] O child 2 ^wf-c936472a0d2e
- Weekly review ^wf-4135a26028aa
	- ⧉ [[#^wf-cca73ecd17ef|Local item]] ^wf-09800d3d649a-ro
```

| you | result |
|---|---|
| edit a mirror's text or its children, add children | the **original** changes, wherever it lives (here "Origin O" is outside the synced subtree) |
| edit a stub (`⧉ [[#^wf-...\|...]]`) | refused, put back: `"Local item" (09800d3d649a) is read-only (or a mirror placeholder); the edit was not sent` |
| tick a stub | refused, the box is removed again |
| type a child under a stub | created under the stub's parent instead |
| delete a mirror's line | only that mirror is removed (`op 0: remove mirror 47a33b1a-c0e3-4e2b-b84f-202a602ede7a (+0 hidden): done`); the original and its children stay |
| delete an original that still has mirrors | refused: `"Local item" (cca73ecd17ef) still has mirrors in Workflowy; its delete was not sent (remove the mirrors first)`. `--force` does not override this. |
| create a mirror | not possible from Markdown |

Exit code 0 in all these cases.

## Pending deletes in the daemon

**The daemon does not delete a removed line at once.** It becomes a pending delete: gone from the file, still in
Workflowy, and listed by `syncflowy status` and in the status note. It is deleted at the first cycle after the grace
period (`--delete-grace`, 10 minutes by default), once the file has also stayed unchanged for one more cycle.

```console
$ syncflowy status
3ac3c97d-8a31-4a35-a924-0941684245a1: gen 2, last synced at 1792027659 (unix), file /…/dvault/daemon.md
  the file is as last written
  pending delete: "Delete me" (92614ca4-a385-448e-9dca-0fe0eabd1ccb), first seen at 1792027656
  pending delete: "Delete and restore me" (b5c6b14f-8493-451d-a833-77463c3933c3), first seen at 1792027656
```

The status note shows:

```markdown
## Pending deletes

Removed from a file but not yet deleted in Workflowy (§9.1 guard 2). Put the line back to cancel; paste its tag into another synced file to move it there.

- [[daemon]]: "Delete and restore me", removed 2026-10-15 01:27 UTC, deleted at the first cycle after 2026-10-15 01:27 UTC with the file unchanged
- [[daemon]]: "Delete me", removed 2026-10-15 01:27 UTC, deleted at the first cycle after 2026-10-15 01:27 UTC with the file unchanged
```

**To cancel a pending delete, put the line back, with its tag** (undo in your editor, or paste it back). The node was
never deleted, so it keeps its id and everything Workflowy knows about it:

```text
[2026-10-15T01:27:50Z INFO  syncflowy::daemon] dvault/daemon.md: the pending delete of "Delete and restore me" (77463c3933c3) was cancelled
[2026-10-15T01:27:56Z INFO  syncflowy::apply] op 0: delete 92614ca4-a385-448e-9dca-0fe0eabd1ccb (+0 hidden): done
[2026-10-15T01:27:56Z INFO  syncflowy::daemon] dvault/daemon.md: deleted 1 item(s) in Workflowy (with 0 hidden)
```

- **Pasting the tagged line into another synced file** turns the pending delete into a move, even if the paste
  arrives later than the cut (for example through Obsidian Sync): `"Keep me" (94ab41b5beb8) moved into this file from
  the file of 0941684245a1`.
- While any other synced file has edits waiting for their debounce, no pending delete is sent ("Held: another synced
  file has edits waiting to be synced." in the note), in case it holds the other half of a cut and paste.
- The brakes still apply, at once: emptying a file is refused straight away, not after the grace period.

## After a crash

**If a sync is interrupted (killed, crash, power loss), the next run finishes it without duplicating anything.** You
do not need to do anything special. syncflowy writes every operation to a journal before sending it, and the next
`sync` (or daemon cycle) first recovers from that journal. `status` shows that recovery is due:

```console
$ syncflowy status vault/hand.md
909ff497-6a8a-43ef-a880-4790be6c6fe9: gen 1, last synced at 1792027504 (unix), file vault/hand.md
  the file has local changes not yet synced
  an unfinished cycle was interrupted: the next sync recovers it
```

In a real test, a sync creating 120 nodes was killed after 30. The next sync adopted those 30 and created the other
90, leaving exactly 120:

```console
$ syncflowy sync vault/hand.md
vault/hand.md: synced: 90 created, 0 updated, 0 moved, 0 reordered, 0 completed, 0 deleted (+0 hidden); file rewritten (gen 2)
  create of temp-31 was not applied (not found in the export)
  op 0: create temp-1=3e091676-cb54-42bc-93f9-b6ebda8b3ad0 under 909ff497-6a8a-43ef-a880-4790be6c6fe9 (direct): done
  …
  line 8: matched to "bulk item 1" (c68f910e60bb) created by an interrupted sync
  line 9: matched to "bulk item 2" (8b8ffa6e2e94) created by an interrupted sync
  …
```

- Run a real `sync`, not `--dry-run`: a dry run does not recover, so its plan looks bigger than it is.
- While a sync runs, Workflowy users may briefly see a new node named `⟳syncflowy:` followed by 16 hex digits. That
  is a temporary name used to recognise half-made nodes; it is replaced at once. After a crash, the next run adopts
  such a node and gives it its real name.
- File and state writes are atomic, so a crash never leaves a half-written file.

## Troubleshooting

| you see | cause | what to do |
|---|---|---|
| `unchanged` | nothing changed on either side | nothing |
| `file unchanged (gen N)` after `synced:` | your edits were sent; nothing you need to see changed | nothing |
| `refused: the file is empty but N nodes were visible (use --force)` | the file lost all its items, or every visible node would be deleted | restore the file; or, if intended, `--force` / `force: true` ([brakes](#mass-change-brakes)) |
| `refused: refusing to delete V visible and H hidden nodes out of N (use --force)` | too many deletes at once (descendants not shown in the file count, including ones `depth:` hides) | check with `--dry-run`; then `--force` if intended |
| `refused: refusing to complete C nodes out of N (use --force)` | too many ticks at once | `--force` if intended, or tick in smaller batches |
| `skipped: the file does not parse: no frontmatter` | the file has no frontmatter, or is empty | restore it; to start syncing a new root, use `render` |
| `skipped: the file does not parse: unterminated frontmatter` | the closing `---` is missing | add it |
| `skipped: the file does not parse: frontmatter has no workflowy: root` | the `workflowy:` line is gone | put it back |
| `skipped: the file does not parse: bad workflowy: root "..."` | the root value is malformed, or a second `workflowy:` line names another root | fix the value ([forms](markdown.md#the-workflowy-value)) |
| `skipped: the file does not parse: bad syncflowy config headings: "7"` | an option has an invalid value (here `headings` above 6, which `render --headings 7` writes) | fix the value ([keys](markdown.md#keys)) |
| `skipped: another sync holds the root's lock` | another syncflowy (perhaps the daemon) is syncing this root right now | try again in a moment |
| `warning: ... unknown syncflowy key "colour" (dropped on the next render)` | a key syncflowy does not know, inside `syncflowy:` | check the spelling; it will be removed |
| `paused (paused: true in the frontmatter)` (exit 0) | you paused it | remove `paused: true` to resume |
| `paused: the file is missing; nothing was changed in Workflowy` | the file was moved or deleted | sync it under its new name, or `render --force ROOT FILE` to start a new file from Workflowy; nothing was deleted |
| `paused: several files claim this root: ...` | two files name one root | merge and delete the copy |
| ``error: no checkpoint for root ...; run `syncflowy render` first or use --force`` | the root was never rendered, or the state directory was lost or changed | check `--state-dir`; then `sync --force` (or `force: true`) to adopt the file, or `render --force` to start over ([state directory](cli.md#if-you-lose-it)) |
| `error: sync root "..." not found` | the root does not exist (deleted, or a typo in `workflowy:`) | fix `workflowy:`; restore the node from Workflowy's Trash if needed |
| `error: no API key: pass --api-key or set WORKFLOWY_API_KEY` | no key | set the variable |
| `error: API: invalid credentials: invalid credentials` | the key is wrong, revoked, or set to an empty string | check the key |
| `error: API: transport error: ...` | network or Workflowy unreachable | try later; the daemon retries by itself |
| `warning: the file's gen (...) is unknown: safe mode (no deletes, moves or reorders)` | `gen` was removed, damaged, or the file is stale | redo deletes, moves and ticks after this sync ([safe mode](#safe-mode)) |
| `safe mode: N structural change(s) not applied (the file's base is unknown)` | deletes, moves, reorders or ticks were dropped in safe mode | redo them now that the file has a good `gen` |
| `warning: no checkpoint: bootstrapped from Workflowy in safe mode (--force)` | a file was adopted with `--force` | check the file, redo any structural changes |
| `conflict on the name of "..." (...): Workflowy's version kept, yours is in the file` | you and a collaborator changed the same text | resolve the `⚠` marker ([conflicts](#conflicts)) |
| `the name of "..." (...) equals an older render; ignored as a stale echo` | you typed back an earlier text | edit in Workflowy, or type something slightly different |
| `line N: "..." (...) is read-only (or a mirror placeholder); the edit was not sent` | you edited a `🔒` item, a placeholder or a stub | fix it in Workflowy; `--force` for a `🔒` item |
| `"" (...) is a mirror placeholder; its delete was not sent (use force)` | you deleted a `⧉ (empty or mirror)` line | if it really is an empty bullet, `--force`; otherwise leave it |
| `"..." still has mirrors in Workflowy; its delete was not sent (remove the mirrors first)` | beta host: you deleted an original that is mirrored | remove the mirrors in Workflowy first |
| `the delete of "..." (...) was not sent: it changed in Workflowy` | a collaborator changed what you deleted | delete it again if you still want to |
| `... was deleted in Workflowy but edited in Markdown: re-created as a new node` | a collaborator deleted what you edited | keep or delete the new node |
| `... was pushed but is hidden: ... was completed in Workflowy` | a collaborator completed what you edited | your edit is on the completed node in Workflowy |
| `line N: id tag ... does not belong to this file; treated as a new item` | a pasted or typed tag from elsewhere; or half of a cross-file cut synced alone | sync both files together next time ([moving between files](#moving-between-files)) |
| `id tag ... appears on lines N, M; line N keeps it, the others are new items` | a tagged line was copied | remove the extra line if it was a mistake (that deletes the new node) |
| `line N: untagged item matched to "..." (...)` | you removed a tag; it was matched back | nothing |
| `line N: line is not indented under an item; attached to the preceding item's note` | a stray line at column 0 | indent it, or make it an item |
| `line N: heading at depth 2 but headings: 1; kept as a heading` | you typed a heading deeper than `headings:` allows | check that the items after it went where you wanted ([headings](markdown.md#headings-mode)) |
| `... lost formatting Markdown could not show` | Workflowy had formatting the file could not show; your edit replaced it | check the node in Workflowy if formatting mattered |
| a line came back after you deleted it | safe mode, a placeholder, a collaborator's edit of that node, or (beta host) an original that still has mirrors | see the report line. (In the daemon, a removed line stays out of the file while its delete is pending; see `syncflowy status`.) |
| a ticked line came back | safe mode; a placeholder or mirror stub (the box is removed again); or someone un-completed it in Workflowy (`... reappeared (uncompleted in Workflowy)`) | after safe mode, tick it again; placeholders and stubs cannot be ticked |
| a line you reverted keeps the newer text | stale-echo protection | see above |
| the daemon's note shows `not synced yet` for every root | the API cannot be reached; see "API errors" in the note | wait, or check the network |
