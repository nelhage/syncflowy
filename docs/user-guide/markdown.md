# The synced file

This page explains every part of a synced Markdown file: the frontmatter and its options, how the outline is laid
out, the id tags, inline formatting, Workflowy's node kinds, read-only items, mirrors and conflict markers. For what
your edits *do* in Workflowy, see [What your edits do](editing.md).

All examples are real output of syncflowy. Indentation in rendered files is four spaces per level. The examples come from several separate test runs, so the same 12-character id can name different nodes in
different examples. Small examples are often excerpts: they leave out the frontmatter or the lines around the part
being shown.

- [A complete example](#a-complete-example)
- [Frontmatter](#frontmatter)
- [Body structure](#body-structure)
- [Id tags](#id-tags)
- [Inline formatting](#inline-formatting)
- [Node kinds and layouts](#node-kinds-and-layouts)
- [Read-only items](#read-only-items)
- [Mirrors](#mirrors)
- [Conflict markers](#conflict-markers)

## A complete example

This is one tree that shows almost everything, rendered with the default options
(`syncflowy render b01d2bbab3ba showcase-default.md`). The Workflowy root is called "Projects". The notes after the
example explain each part.

`````markdown
---
workflowy: b01d2bbab3ba
syncflowy: {gen: 1}
---
Free text here is the sync root's note.
It has **two** lines & an ampersand.

- Planning ^wf-72ad9784e804
    Note on "Planning": lines right under the heading.
    - Draft the *spec* ^wf-849919fed983
        A note line.

        After an empty line.
        \- not a child, a note line
        - [ ] Write the mock ^wf-b57e156452ec
        - Deep ^wf-2823ff185804
            - Deeper ^wf-e34adc6a7b07
                - Deeper still ^wf-619ae27f6dac
                    - Nearly there ^wf-90c9785a74b5
                        - [ ] Bottom `x = 1` ^wf-0bdf96b84cd7
    - Numbered steps ^wf-d00fee1fe835
        1. [ ] first ^wf-9bcc841e525e
        2. [ ] third ^wf-aed9db72096c
            with a note
    - [ ] Board layout is kept as a plain item ^wf-86971bae29e9
    - [ ] Paragraph layout ^wf-f97041907338
    - [ ] An h1 inside the outline ^wf-bd335f1b7ce7
- ``` ^wf-0faec1e4bcd6
    fn main() {
        println!("hi & bye");
    }
    ```
    runs on save
- > To be or not to be ^wf-d1821a36f41f
- --- ^wf-eb6d09f5af6f
- ^wf-1b0fd2f68e02
    - [ ] child of an empty bullet ^wf-61c09709524a
- ⧉ (mirror of a deleted item) ^wf-c4893308254a-ro
- [ ] 🔒 broken bold ^wf-75bd70bb31f8-ro
    a malformed name (UI:F1)
- [ ] 🔒 line one ⏎ line two ^wf-e22e3e0ac1bb-ro
- ```` ^wf-a8df5f2dca1d
    x
    ```
    y
    ````
- [ ] **bold** and unknown markup ^wf-e3ce6b0bec62
- [ ] See https://example.com/a?b=1&c=2 on 📅 2026-01-15 and at 📅 2026-01-15 14:30 ^wf-3c213108eeaa
- [ ] <u>underlined</u>, <span class="colored c-red">red</span>, [a link](https://example.com) and #tag @who ^wf-f00cd531bb01
- [ ] 1\. not a list ^wf-7e69db56700d
- [ ] \- not a child ^wf-b5f5131ad9a3
- [ ] \# not a heading ^wf-de3705f4633d
- [ ] \[ ] not a box ^wf-ed5f704494c4
- [ ] a \^wf-8e1f00aa2b3c in the text and %\% percents ^wf-94b71ec8514b
- [ ] A todo with children ^wf-669bcc63fb32
    - [ ] leaf one ^wf-6fcd396ad25e
    - [ ] leaf two ^wf-02bcde313b0c
        &#32;&#32;indented note line
- [ ] Trailing empty note lines ^wf-ecf22665551e
    first
    
    
- [ ] Leading empty note line ^wf-17a44ac81198

    second
`````

What to notice:

1. **Frontmatter.** `workflowy:` names the root, exactly as it was given to `render`. `syncflowy:` holds the
   options (none here, so only `gen`, the generation the engine wrote). See [Frontmatter](#frontmatter).
2. **Root note.** The two lines before the first item are the note of "Projects". The name "Projects" itself is not
   in the file.
3. **Items, notes, children.** "Planning" has a note (the indented line under it) and children (the nested items).
   "Draft the *spec*" has a note of three lines with an empty line between the first two.
4. **Hidden completed nodes.** Workflowy also has "Run the UI scenarios", "second (done)" and "Done stuff", all
   completed. They are not shown, and neither is anything under them, even incomplete items.
5. **Checkboxes.** Leaves and Workflowy to-dos ("Write the mock") get `[ ]`; parents ("Planning", "Deep") do not.
   This is cosmetic. See [checkboxes](#checkboxes).
6. **Numbered list.** "Numbered steps" has Workflowy's numbered layout, so its children are `1.`, `2.`. Only visible
   children are counted ("second (done)" is completed).
7. **Other layouts.** A board, a paragraph and an h1 node render as plain items. Their layout is kept in Workflowy.
8. **Code block, quote, divider.** A Workflowy code block is a fenced block (its note, "runs on save", follows the
   closing fence). A quote block is `- > ...`. A divider is `- ---`.
9. **Empty items and placeholders.** `- ^wf-1b0fd2f68e02` is a node with an empty name, an ordinary item like any
   other (it has a child, so no box). `⧉ (mirror of a deleted item)` is a read-only placeholder for a mirror whose
   original was deleted. See [Mirrors](#mirrors).
10. **Read-only items.** `🔒` and the `-ro` tag mark nodes whose Workflowy text cannot be shown exactly: broken
    formatting, or a line break inside a name (shown as `⏎`). See [Read-only items](#read-only-items).
11. **Escaping.** Workflowy text that would look like Markdown structure is escaped: `1\.`, `\-`, `\#`, `\[ ]`, a
    tag-like string `\^wf-...`, a `%%` that does not pair up (`%\%`). A code block whose code contains ```` ``` ```` gets a longer fence.
12. **Inline formatting.** Bold, italic, code and links become Markdown; underline and colours stay as HTML; dates
    are `📅 2026-01-15`. See [Inline formatting](#inline-formatting).
13. **Whitespace.** Leading spaces in a note line are written `&#32;`. Empty lines at the end of a note are lines
    that hold only the indentation (spaces), so they are not mistaken for the gap before the next item. An empty line
    at the start of a note is a blank line.

## Frontmatter

The frontmatter is the block between the two `---` lines at the top. It must start on the first line. syncflowy
reads only two keys, `workflowy:` and `syncflowy:`, and keeps every other line as you wrote it.

### Keys

| key | values | default | written by | may you edit it? | effect |
|---|---|---|---|---|---|
| `workflowy` | short id, full UUID, or `https://workflowy.com/#/<short id>` URL (see below) | required | you or `render`; kept verbatim, including a trailing `# comment` | only to point the file at another root (which then needs `render` or `force`) | which node this file syncs |
| `syncflowy.headings` | `0` to `6` | `0` | the engine, when not 0 | yes | render depth 1 to N as headings ([headings mode](#headings-mode)) |
| `syncflowy.ids` | `auto`, `block`, `comment`, `dataview`, `dataview-hidden`, `html` | `auto` | the engine, when not `auto` | yes | the form of the id tags ([id tags](#id-tags)) |
| `syncflowy.checkboxes` | `todos-and-leaves`, `todos`, `all`, `none` | `todos-and-leaves` | the engine, when not the default | yes | which items get a `[ ]` ([checkboxes](#checkboxes)) |
| `syncflowy.dates` | `true`, `false` | `true` | the engine, when `false` | yes | `true`: dates as `📅 2026-01-15`; `false`: as Workflowy's `<time ...>` HTML |
| `syncflowy.depth` | a positive whole number, or `null` / `~` / empty for unlimited | unlimited | the engine, when set | yes | hide nodes deeper than N (the root is depth 0). Changing `depth` deletes nothing, but **deleting a line also deletes everything under it that `depth` hides**. A line whose children are hidden this way counts as a parent, not a leaf (no `[ ]` by default). |
| `syncflowy.paused` | `true`, `false` | `false` | you; the engine keeps it (`render --force` drops it) | yes | `true`: the file is neither pushed nor pulled until you remove it |
| `syncflowy.force` | `true`, `false` | `false` | you; **the engine removes it** after one sync | yes, one-shot | override the brakes and the mirror-placeholder guards once, adopt a file with no checkpoint |
| `syncflowy.gen` | a number | none | the engine, every time it rewrites the file | **no** | the generation the file came from; removing it, or changing it to a value the engine does not know, puts the next sync into [safe mode](editing.md#safe-mode) |
| other keys inside `syncflowy:` | anything | | | | a warning (`line 3: unknown syncflowy key "colour" (dropped on the next render)`), then dropped |
| any other line (`title`, `tags`, `aliases`, comments, lists) | anything | | you | yes | never read; kept as written, but moved below the two syncflowy keys (an option name on its own line is warned about: see below) |

An invalid value makes the whole file `skipped`: nothing is sent and nothing is written. For example:

```console
$ syncflowy sync misc/badcfg.md           # syncflowy: {headings: 9}
misc/badcfg.md: skipped: the file does not parse: bad syncflowy config headings: "9"
[exit 1]
```

`dates: yes` and `depth: 0` are invalid too. A missing or unparseable `gen` is not an error; it means safe mode.

Options belong **inside** the `syncflowy:` map: write `syncflowy: {gen: 2, paused: true}`. An option written on its
own top-level line is kept, with a warning, and ignored, except that a top-level `paused: true` still pauses the
file (the safe direction):

```console
$ syncflowy sync d/doc.md                 # a top-level "force: true" line
d/doc.md: synced: 0 created, 0 updated, 0 moved, 0 reordered, 0 completed, 0 deleted (+0 hidden); file unchanged (gen 11)
  warning: line 4: "force" is a syncflowy option; write it as syncflowy: {force: true} (ignored here)
[exit 0]
```

### The `workflowy:` value

All of these are accepted, and `render` writes whichever you gave it:

| form | example |
|---|---|
| short id (last 12 hex digits of the UUID, any case) | `workflowy: f92283868a29` |
| full UUID | `workflowy: d8f33418-f3d4-4711-9804-f92283868a29` |
| URL, also `http://` and `beta.workflowy.com`, optionally followed by `/...` or `?...` | `workflowy: https://workflowy.com/#/f92283868a29` |

A URL without the scheme (`workflowy.com/#/f92283868a29`) is rejected. So is one with a full UUID, a `www.` or
`app.` host, or `#/x/<short>`.

### How the engine writes the frontmatter

The engine always writes `workflowy:` first, then `syncflowy:` as one line in a fixed order
(`headings, ids, checkboxes, dates, depth, paused, force, gen`), leaving out default values, then all your other
lines. You may write `syncflowy:` in block style; it is rewritten as one line. Comments inside `syncflowy:` are lost;
a comment after `workflowy:` is kept.

Before (hand-edited):

```markdown
---
title: My plan
workflowy: f92283868a29   # the Projects node
aliases:
  - plan
  - roadmap
syncflowy:
  headings: 1   # top level as headings
  checkboxes: todos
  gen: 1
tags: [work, sync]
---
```

After the next sync:

```markdown
---
workflowy: f92283868a29   # the Projects node
syncflowy: {headings: 1, checkboxes: todos, gen: 2}
title: My plan
aliases:
  - plan
  - roadmap
tags: [work, sync]
---
```

### When an option change takes effect

A change to `headings`, `ids`, `checkboxes`, `dates` or `depth` rewrites the file at the next sync, even when nothing
else changed (`file rewritten`). The others:

| you change | what happens |
|---|---|
| `checkboxes` | every box is chosen again by the new policy; boxes you added or removed by hand are forgotten ([checkboxes](#checkboxes)) |
| `paused: true` | `paused (paused: true in the frontmatter)`, exit 0, nothing done |
| `force: true` | used for one sync, then removed: `syncflowy: {…, gen: 5, force: true}` becomes `syncflowy: {…, gen: 6}` |

`depth: 2` on a file, then `sync`:

```markdown
---
workflowy: 42258477-b1af-49ab-a5a9-e7f709f1414e
syncflowy: {depth: 2, gen: 2}
---
- Alpha ^wf-abce1a2a9ed2
    - [ ] Alpha child ^wf-77cb8348b570
- Beta ^wf-4572d2015b35
    - [ ] Beta child ^wf-97136e83cfc1
- Level 1 ^wf-f3cd60486bc9
    - Level 2 ^wf-000593e20343
```

`Level 3` and `Level 4` are only hidden: Workflowy still has them. `depth: null` brings them back (excerpt):

```markdown
- Level 1 ^wf-f3cd60486bc9
    - Level 2 ^wf-000593e20343
        - Level 3 ^wf-96acbcc043c4
            - [ ] Level 4 ^wf-36f880ded786
```

> **Warning.** `depth:` hides nodes from the file; it does not protect them. Deleting the line "Level 2" above
> deletes "Level 3" and "Level 4" too. The report counts and names what was hidden, and so does `--dry-run`; here,
> with `depth: 1`, the line "Level 1" was deleted:
>
> ```console
> $ syncflowy sync d/doc.md
> d/doc.md: synced: 0 created, 0 updated, 0 moved, 0 reordered, 0 completed, 1 deleted (+3 hidden); file unchanged (gen 4)
>   op 0: delete 33d4fcc5-e84e-41cc-b341-f3cd60486bc9 (+3 hidden): done
>   deleting "Level 1" (f3cd60486bc9) also deletes 3 hidden items: "Level 2", "Level 3", "Level 4"
> [exit 0]
> ```
>
> Hidden items count towards the brakes: deleting a line "Big" whose 60 children were never shown was stopped
> with `refusing to delete 1 visible and 60 hidden nodes out of 3 (use --force)`. Run `sync --dry-run` before
> deleting a line in a file with `depth:`.

`dates: false` shows Workflowy's date HTML instead of the compact form (Workflowy's value is not touched; this line is
from a run that also set `ids: comment`, hence the `%%wf:` tag):

```markdown
- [ ] See https://example.com/a?b=1&c=2 on <time startYear="2026" startMonth="1" startDay="15">Thu, Jan 15, 2026</time> and at <time startYear="2026" startMonth="1" startDay="15" startHour="14" startMinute="30">Thu, Jan 15, 2026 at 2:30 PM</time> %%wf:27bc08d6af57%%
```

`render --force` on a file that names the same root keeps your `syncflowy:` options (except `paused` and `force`)
and your other frontmatter lines; `--headings`, if given, changes only the heading depth. Delete the file first for
a clean start.

### Files syncflowy will not read

| file | report |
|---|---|
| no frontmatter (or an empty file) | `skipped: the file does not parse: no frontmatter` |
| frontmatter without `workflowy:` | `skipped: the file does not parse: frontmatter has no workflowy: root` |
| closing `---` missing | `skipped: the file does not parse: unterminated frontmatter` |
| a second `workflowy:` line naming a different root | `skipped: the file does not parse: bad workflowy: root "..."` |
| a name that looks like a sync conflict copy: `(conflicted copy ...)`, `(... conflict ...)`, `.sync-conflict-` | `skipped: looks like a sync conflict copy` |
| a second file naming the same root as another synced file (also across separate `sync` commands) | each of them in the run: `paused: several files claim this root: d/doc.md, d/doc2.md` (exit 1) |

A skipped or paused file is never read as "everything was deleted". The body itself never fails to parse.

## Body structure

### The root note

Everything between the frontmatter and the first item or heading is the root's note. Blank lines at its start and
end are dropped; blank lines inside are kept. The root's **name** is never in the file, so renaming the root in
Workflowy, or renaming the file, changes nothing.

### Items and notes

- Each list item is a node. Its first line (after the marker, checkbox and tag) is the node's **name**. Names are
  always one line.
- The lines indented under it, **before its first child**, are its **note**, one Markdown line per note line. A
  blank line between them is an empty note line.
- Nested items are its children.
- The engine writes `- ` markers (or `1.` under a numbered parent) and four spaces per level. When reading, it accepts
  `-`, `*`, `+`, `1.` and `1)` markers, and tabs or any consistent number of spaces.
- A line that fits nowhere (for example text at column 0 after a list) is attached to the note of the nearest item
  above it, with a warning: `line 22: line is not indented under an item; attached to the preceding item's note`.

A file edited with spaces, other markers, notes, a second root-note line and a stray line:

```markdown
---
workflowy: f0e642f4-3328-4d08-8ded-3c9691eb79fa
syncflowy: {gen: 1}
---
Root note line 1
Root note line 2, typed in Obsidian

- Groceries ^wf-95dd81355c53
  A note typed under Groceries
  second note line

  third note line after a blank line
  - [ ] Milk ^wf-b9af5b569643
  - Eggs
  * Bread (star marker)
  + Butter (plus marker)
- [ ] Errands ^wf-9ebe89d9bf02
  1) first step
  2) second step
- [ ] A brand new top-level item
    - a child indented with four spaces
stray text at column 0 after the list
```

After `sync` (`7 created, 3 updated`; the updates include the new root-note line), normalised to four-space indentation and `-`/`1.`
markers:

```markdown
---
workflowy: f0e642f4-3328-4d08-8ded-3c9691eb79fa
syncflowy: {gen: 2}
---
Root note line 1
Root note line 2, typed in Obsidian

- Groceries ^wf-95dd81355c53
    A note typed under Groceries
    second note line

    third note line after a blank line
    - [ ] Milk ^wf-b9af5b569643
    - [ ] Eggs ^wf-6c9a9cc9af4e
    - [ ] Bread (star marker) ^wf-440407295e42
    - [ ] Butter (plus marker) ^wf-5cd12d5db79b
- [ ] Errands ^wf-9ebe89d9bf02
    1. [ ] first step ^wf-308cdc6b13ab
    2. [ ] second step ^wf-17e3cc0e95ee
- A brand new top-level item ^wf-3dd6415af341
    - [ ] a child indented with four spaces ^wf-3d26ac512b01
        stray text at column 0 after the list
```

"Errands" became a numbered list in Workflowy, because all its children were typed with numbered markers. The stray
line became the note of the last item.

Note: syncflowy writes empty lines at the end of a note as lines holding only the indentation. Whether Obsidian or a
formatting plugin keeps such lines has not been checked.

### Headings mode

With `headings: N` (or `render --headings N`; N is 0 to 6), nodes at depth 1 to N are written as headings (`#` for
depth 1, `##` for depth 2, and so on), whatever their Workflowy layout. A heading's note is the lines right under it, at column 0.
Deeper nodes are lists under their heading. The showcase tree with `--headings 2`:

`````markdown
---
workflowy: b01d2bbab3ba
syncflowy: {headings: 2, gen: 1}
---
Free text here is the sync root's note.
It has **two** lines & an ampersand.

# Planning %%wf:72ad9784e804%%
Note on "Planning": lines right under the heading.

## Draft the *spec* %%wf:849919fed983%%
A note line.

After an empty line.
\- not a child, a note line

- [ ] Write the mock ^wf-b57e156452ec
- Deep ^wf-2823ff185804
    - Deeper ^wf-e34adc6a7b07
        - Deeper still ^wf-619ae27f6dac
            - Nearly there ^wf-90c9785a74b5
                - [ ] Bottom `x = 1` ^wf-0bdf96b84cd7

## Numbered steps %%wf:d00fee1fe835%%

1. [ ] first ^wf-9bcc841e525e
2. [ ] third ^wf-aed9db72096c
    with a note

## Board layout is kept as a plain item %%wf:86971bae29e9%%

## Paragraph layout %%wf:f97041907338%%

## An h1 inside the outline %%wf:bd335f1b7ce7%%

# ``` %%wf:0faec1e4bcd6%%
fn main() {
    println!("hi & bye");
}
```
runs on save

# > To be or not to be %%wf:d1821a36f41f%%

# --- %%wf:eb6d09f5af6f%%

# %%wf:1b0fd2f68e02%%

## child of an empty bullet %%wf:61c09709524a%%

# ⧉ (mirror of a deleted item) %%wf:c4893308254a-ro%%

# 🔒 broken bold %%wf:75bd70bb31f8-ro%%
a malformed name (UI:F1)

# 🔒 line one ⏎ line two %%wf:e22e3e0ac1bb-ro%%

# ```` %%wf:a8df5f2dca1d%%
x
```
y
````

# **bold** and unknown markup %%wf:e3ce6b0bec62%%

# See https://example.com/a?b=1&c=2 on 📅 2026-01-15 and at 📅 2026-01-15 14:30 %%wf:3c213108eeaa%%

# <u>underlined</u>, <span class="colored c-red">red</span>, [a link](https://example.com) and #tag @who %%wf:f00cd531bb01%%

# 1\. not a list %%wf:7e69db56700d%%

# \- not a child %%wf:b5f5131ad9a3%%

# \# not a heading %%wf:de3705f4633d%%

# \[ ] not a box %%wf:ed5f704494c4%%

# a \^wf-8e1f00aa2b3c in the text and %\% percents %%wf:94b71ec8514b%%

# A todo with children %%wf:669bcc63fb32%%

## leaf one %%wf:6fcd396ad25e%%

## leaf two %%wf:02bcde313b0c%%
&#32;&#32;indented note line

# Trailing empty note lines %%wf:ecf22665551e%%
first

# Leading empty note line %%wf:17a44ac81198%%
second
`````

Differences from list mode:

- Headings carry `%%wf:...%%` tags, because Obsidian does not allow block ids on headings.
- Headings never show a checkbox, even for to-dos ("A todo with children"). You can still tick one: write
  `# [x] Title`.
- A heading's note loses its empty lines at the start and end ("Trailing empty note lines" shows only `first`).
- A code block at heading depth is a heading line with a fence, then the code at column 0, then the closing fence.

> **Caution: headings you type.** Items below a heading belong to that heading. If you type a heading deeper than
> `headings:` allows, it becomes a node, and the items after it become **its children**; the warning says how many.
> At the next rewrite it is shown as a list item. In a real run, typing `### Skipped level typed by hand` and
> `- item under the h3` above "Beta child" in a `headings: 1` file moved "Beta child" under the new node:
>
> ```console
> $ syncflowy sync d/doc.md
> d/doc.md: synced: 2 created, 1 updated, 1 moved, 0 reordered, 0 completed, 0 deleted (+0 hidden); file rewritten (gen 11)
>   warning: line 12: heading level 3 coerced to 2
>   warning: line 12: heading at depth 2 but headings: 1; it becomes an item, and the 2 items after it become its children
> ...
> ```
>
> ```markdown
> # Beta %%wf:4572d2015b35%%
> A note typed under the Beta heading
>
> - Skipped level typed by hand ^wf-0cf49189011c
>     - [ ] item under the h3 ^wf-d5155a9268c7
>     - [ ] Beta child ^wf-97136e83cfc1
> ```
>
> A heading that skips a level is first coerced to the next level, as the first warning shows. A `#` heading at
> column 0 in a file with `headings: 0` also becomes a node, with the items under it as its children.

Typing a new heading at a level that `headings:` allows simply creates a node (with the items under it as children)
and it gets a `%%wf:...%%` tag.

## Id tags

Each line ends with a tag holding the node's short id (the last 12 hex digits of its Workflowy UUID). That is how
syncflowy knows which node a line is. A line without a tag is a new node, and gets a tag at the next sync.

### Forms

The `ids:` option chooses how tags are written:

| `ids:` | list items | headings |
|---|---|---|
| `auto` (default), `block` | `^wf-c4d1c386bbc4` (an Obsidian block id) | `%%wf:c4d1c386bbc4%%` (an Obsidian comment) |
| `comment` | `%%wf:c4d1c386bbc4%%` | `%%wf:c4d1c386bbc4%%` |
| `dataview` | `[wid:: 7d8fcf23cae8]` (a Dataview field) | same |
| `dataview-hidden` | `(wid:: 7d8fcf23cae8)` (a Dataview field shown as its value only) | same |
| `html` | `<!-- wf:7d8fcf23cae8 -->` | same |

The same file with `headings: 1` and `ids: html`:

```markdown
# Alpha (edited in Workflowy) <!-- wf:2cfa93ea5c4e -->

- [ ] Alpha child <!-- wf:4d4244ef7feb -->

# Beta <!-- wf:f87ff5059285 -->

- [ ] Beta child <!-- wf:0487c89da11b -->
```

and with `ids: dataview-hidden`:

```markdown
# Alpha (edited in Workflowy) (wid:: 2cfa93ea5c4e)

- [ ] Alpha child (wid:: 4d4244ef7feb)
```

All five forms are read anywhere on the first line, whatever `ids:` says, and rewritten in the configured form at the
next rewrite. The hex is case-insensitive. A backslash-escaped tag (`\^wf-...`) is text, and `#^wf-...` is a link,
not a tag. Text in Workflowy that looks like a tag is escaped when written, so your text can never turn into a tag.

### Flags

A tag can carry a flag after the hex: only `-ro` (read-only) exists, as in `^wf-e35cd47d380d-ro` or
`%%wf:e35cd47d380d-ro%%`. syncflowy sets it; removing it does not unlock the item. An unknown flag is ignored with a
warning.

### Tags you change

Short version (details and real examples in [id-tag accidents](editing.md#id-tag-accidents)):

| you | result |
|---|---|
| type a line with no tag | a new node |
| remove the tag but keep the text (nearly) the same, nearby | matched back to its node: `line 5: untagged item matched to "Butter (plus marker)" (c4b53949080b)` |
| remove the tag and change the text a lot, in the same place (it is the only untagged line where the node was) | also matched back: the node is **renamed** (deleting a line and typing a new one in its place does the same) |
| remove the tag, change the text a lot and move the line | the old node is **deleted** and a new one created |
| copy a tagged line within the file | the best-matching line keeps the node; the others become new nodes |
| put two tags on one line | the last one wins (`2 id tags on one line; the last one wins`) |
| type a tag that exists nowhere | `treated as a new item`, re-tagged |
| cut a tagged line and paste it into another synced file | the node **moves** there when both files sync in one command. Syncing only the file you cut from holds the delete: `"Butter (plus marker)" (c4b53949080b) is in i2.md; sync both files in one command to move it` |
| copy a tagged line into another synced file, keeping the original | a new node there: `id tag 557b5d9ee3dd is still in the file it comes from; treated as a copy (a new item)` |

## Inline formatting

### Workflowy text and Markdown

| in Workflowy | written in the file | you may also type |
|---|---|---|
| bold | `**x**` | `__x__`, `<b>x</b>` |
| italic | `*x*` | `_x_`, `<i>x</i>` |
| bold italic | `**_x_**` | `***x***` (re-written as `*__x__*`) |
| strike-through | `~~x~~` | `<s>x</s>` |
| inline code | `` `x` `` | `<code>x</code>` |
| underline | `<u>x</u>` (Markdown has no underline) | the same |
| link | `[text](https://example.com)`; `[text](<url>)` when the URL contains spaces, parentheses or angle brackets | `<a href="url">text</a>` |
| a link whose text is its URL (what Workflowy makes when you paste a URL) | the bare URL: `https://example.com/x?a=1&b=2` | a bare `https://` URL, or `<https://...>` |
| text colour | `<span class="colored c-red">x</span>` | the same HTML |
| highlight colour | passed through as HTML like colours (Workflowy's real class names were not observed) | |
| date | `📅 2026-03-01` | `📅 2026-03-01` |
| date with time | `📅 2026-03-01 09:05` (24-hour) | `📅 2026-02-03 17:45` |
| a date in a form syncflowy does not recognise, or any date with `dates: false` | literal `<time ...>...</time>` HTML | |
| `#tag`, `@mention` | the same text | the same (Obsidian reads `#tag` as its own tag) |
| unknown markup | dropped from the display; see [lossy values](#lossy-values) | other HTML (`<strong>`, `<em>`, `<div>`, `<script>`, an unclosed `<b>`) is sent as literal text |

Dates are shown compactly only when they are in the English form Workflowy's API produces. A typed `📅 2026-02-30` is
not a date and stays text. Other locales, 24-hour account settings and date ranges have not been checked and stay as
HTML.

Real examples of formatting written from Workflowy:

```markdown
- [ ] **bold** ^wf-54ef2904acec
- [ ] *italic* ^wf-eb43ad9cedde
- [ ] <u>underline</u> ^wf-1bf13879399b
- [ ] ~~strike~~ ^wf-daffcc3d5506
- [ ] `code` ^wf-02ec39235bc0
- [ ] **_bold italic_** ^wf-6a576518093d
- [ ] [link text](https://example.com) ^wf-9ae80f0c8a89
- [ ] https://example.com/x?a=1&b=2 ^wf-433ff7c882f4
- [ ] [spaces and parens](<https://example.com/a b (c)>) ^wf-28004e6f5a94
- [ ] <span class="colored c-red">red text</span> ^wf-110e4f73fd94
- [ ] 📅 2026-03-01 ^wf-5022409a8a78
- [ ] 📅 2026-03-01 09:05 ^wf-1d97e0f3a7ef
- [ ] <time startYear="2026" startMonth="3" startDay="1">1 March</time> ^wf-000bd1c51f86
```

### Escaping

syncflowy escapes only what would otherwise change meaning in Markdown, so plain Workflowy text stays readable:

| Workflowy text | written as | why |
|---|---|---|
| `a * b and 2*3*4` | `a * b and 2\*3\*4` | a lone `*` is safe; a pair would be italics |
| `snake_case_name` | `snake_case_name` | underscores inside words are safe |
| `*args and **kwargs` | `\*args and \*\*kwargs` | would be emphasis |
| `<b>not bold</b>` typed as text | `\<b>not bold\</b>` | keeps it apart from real formatting |
| a name starting `1)`, `* `, `+ `, `# `, `> `, `---`, `[ ]` | `1\)`, `\*`, `\+`, `\#`, `\>`, `\---`, `\[ ]` | would be structure |
| `#hashtag at start` | unchanged | a tag, not a heading |
| `50%% off` (a `%%` that does not pair up), or `%%wf:` | `50%\% off`, `%\%wf:` | would pair with an id tag, or be one |
| `%%seen in WF%%` | unchanged | an Obsidian comment: **hidden** in Obsidian's reading view |
| `price $10-$20` | `price \$10-\$20` | would be maths in Obsidian |
| `http://plain-http.example.com/path` as plain text (not a link) | `http\://plain-http.example.com/path` | stops Obsidian turning it into a link |
| leading or trailing spaces | `&#32;&#32;leading and trailing spaces&#32;&#32;` | Markdown would drop them |
| `AT&T and &copy;` | `AT&T and \&copy;` | `&copy;` would become © |

When you type: `&amp; &lt; &gt; &quot;` and numeric references like `&#169;` are decoded; other named references are
kept literally. Leading and trailing spaces you type are removed.

### Obsidian-only syntax

Workflowy has no equivalent for these. Here is what happened to each in a real run, typed in Obsidian and synced:

| you type | Workflowy gets | the file afterwards | still works in Obsidian? |
|---|---|---|---|
| `[[Some Note]]`, `[[Some Note\|alias]]`, `![[image.png]]` | the literal text | unchanged | yes |
| `==highlight==` | the literal text | unchanged | yes |
| `$E=mc^2$` | the literal text | unchanged | yes |
| `footnote[^1]` | the literal text | unchanged | yes |
| `#tag`, `@person` | the literal text | unchanged | yes |
| `%%hidden comment%%` | the literal text, **visible** to Workflowy users | unchanged | yes (hidden in Obsidian only) |
| `![img](https://example.com/i.png)` | the literal text | unchanged | yes |
| `[a](https://example.com "title")` | a link, without the title | `[a](https://example.com)`; the report says `the name of "a" (ea24575da7fb, new) was written without its link title: Workflowy links have none` | yes, without the title |
| `__bold__`, `_italic_` | bold, italic | re-spelled `**bold**`, `*italic*` at the next rewrite | yes |
| `<https://example.org/auto>` | a link | the bare URL `https://example.org/auto` | yes |
| `- [/] ...`, `- [-] ...` (Tasks statuses) | `[/] ...` as part of the name | `- [ ] \[/] ...` | no: not a checkbox |
| `- [x]typed` (no space) | `[x]typed` as text | `- [ ] [x]typed` | not a tick |

> **Warning.** Do not use `%%comments%%` in synced files for private notes: they are sent to Workflowy as plain
> text, where every collaborator can read them. The reverse also holds: a `%%...%%` typed in Workflowy is hidden
> in Obsidian's reading view.

### Lossy values

Some Workflowy text is well-formed but cannot be shown exactly, for example markup from another app
(`<b>bold</b> and <foo>unknown</foo> markup`). It is shown in its nearest Markdown form
(`**bold** and unknown markup`) and stays editable. When you edit it, syncflowy keeps what it can and reports
`the name of "bold and unknown markup, edited" (e3ce6b0bec62) lost formatting Markdown could not show`.

## Node kinds and layouts

Workflowy nodes have a layout (bullet, to-do, heading, board, code block, ...). The file shows the ones Markdown can
express; the others render as plain items and keep their layout in Workflowy.

| Workflowy layout | in the file | checkbox (default policy) | can you change it from Markdown? |
|---|---|---|---|
| bullets | `- text` | leaves only | yes: into a code block, quote, divider, or numbered list, by writing that kind |
| to-do | `- [ ] text` | always (none with `checkboxes: none`) | no. A new `- [ ]` line is a plain bullet. |
| h1, h2, h3 | plain item | leaves only | no, kept |
| paragraph, board, table, dashboard, and others | plain item; a board's columns and a table's rows are ordinary nested items | leaves only | no, kept |
| code block | ``- ``` `` then the code indented one level, then a closing fence; the note after the fence | never | remove the fence: becomes a bullet. Fence an item: becomes a code block. |
| quote block | `- > text` | never | remove `> `: bullet. Write `- > text`: quote block. |
| divider (empty) | `- ---` | never | write `- ---` over an item: divider, **and its text is emptied** |
| divider with text | plain item | leaves only | |
| numbered (on the parent) | children are `1.`, `2.`, ... (visible ones only) | | switch **all** children to `1.` markers: parent becomes numbered; all back to `-`: bullets (only for a parent whose layout is bullets or numbered) |

An excerpt of a file with one node of each kind (the h2, h3, table, dashboard and other plain-item layouts are left
out; they render like the h1 and board lines):

````markdown
- [ ] A bullets node ^wf-b61550332cb8
- [ ] A todo node ^wf-d1a6e9d40f2b
- [ ] An h1 node ^wf-1bfbf8449560
- [ ] A paragraph (p) node ^wf-25d9b02d3504
- > A quote-block node ^wf-af97425375be
- ``` ^wf-d03d4eac98d6
    print("code-block node")
    second line
    ```
- --- ^wf-041814d4954e
- [ ] a divider that has a name ^wf-6560fbbe9381
- A numbered parent ^wf-b4035596dfde
    1. [ ] step one ^wf-c1ba943863a5
    2. [ ] step two ^wf-1b333ebebe3e
    3. step three ^wf-a3653e67026c
        - [ ] a bullet under step three ^wf-27478d1bc13a
- A board ^wf-f255133bb4c2
    - Column To do ^wf-c22a4a7347fa
        - [ ] card 1 ^wf-87f07e465b19
    - Column Done ^wf-d73b2778507c
        - [ ] card 2 ^wf-f07ccb8409d6
- [ ] A todo parent ^wf-02f350e9e079
    - [ ] open subtask ^wf-3076cdc98666
````

What you can and cannot change from Markdown, with real results, is in
[changing a node's kind](editing.md#changing-a-nodes-kind). Two traps: Workflowy code blocks have no language, so
a fence's info string (```` ```python ````) is dropped with a warning
(`the code block's language "python" was dropped (Workflowy code blocks have none)`), and `- # text` inside a list
is not a heading but literal text (written back as
`- \# text`).

### Checkboxes

`- ` and `- [ ] ` both mean "not completed". Adding or removing a box sends nothing to Workflowy. The `checkboxes:`
option decides which items get a box:

| `checkboxes:` | boxes on |
|---|---|
| `todos-and-leaves` (default) | to-dos and items with no visible children |
| `todos` | to-dos only |
| `all` | every list item |
| `none` | no item, not even to-dos (they stay to-dos in Workflowy, and `[x]` still ticks them) |

An item whose children are all hidden by `depth:` counts as a parent, not a leaf. Headings, code blocks, quotes,
dividers, placeholders and stubs never get a box. Workflowy to-dos always do, except with `checkboxes: none`.

**Boxes are sticky.** Once a line is in the file, it keeps the form it has there, across rewrites, even when the
node gains or loses children. The policy decides only for items that are new to the file, with one exception:
changing `checkboxes:` rewrites the file with every box chosen by the new policy. In a real run, `checkboxes: all`
added to the showcase file boxed "Planning", "Deep" and every other item at the next sync. (A to-do whose box you
removed gets it back at the next rewrite, unless `checkboxes: none`.)

You may add or remove boxes by hand; the choice sticks. Here a box was added to "Interior bullet" and removed
from "Leaf A" and "Todo item" (a to-do). The sync sent nothing (`file unchanged (gen 1)`):

```markdown
- [ ] Interior bullet ^wf-db95d4350b28
    - Leaf A ^wf-9c1966ad51fd
    - [ ] Leaf B ^wf-47a73bc8996b
- Todo item ^wf-bf628758ff4d
- [ ] Leaf C ^wf-a9d3a61a59e3
```

After a later rewrite, caused by a rename and a new child in Workflowy, the hand-made choices stayed, the to-do got
its box back, and the new item followed the policy:

```markdown
- [ ] Interior bullet (renamed in WF) ^wf-db95d4350b28
    - Leaf A ^wf-9c1966ad51fd
    - [ ] Leaf B ^wf-47a73bc8996b
    - [ ] Leaf D added in WF ^wf-c4b2afbf5310
- [ ] Todo item ^wf-bf628758ff4d
- [ ] Leaf C ^wf-a9d3a61a59e3
```

### Completed items and `- [x]`

Completed nodes, and everything under them, are not in the file. Writing `- [x]` (or `- [X]`, `1. [x]`, or
`# [x] Title` for a heading) completes that node at the next sync and removes it, with its subtree, from the file.
Only that node is completed, not its children. You can only un-complete a node in Workflowy, and it then reappears in
the file. See [ticking](editing.md#ticking).

## Read-only items

An item is read-only when syncflowy cannot show its Workflowy text exactly:

- its name or note has broken formatting (for example `broken <b>bold`, an unclosed tag), or
- its name contains a line break, shown as ` ⏎ `.

It is shown with `🔒 ` before the text and `-ro` on its tag:

````markdown
- [ ] 🔒 A fine name ^wf-50b370d87824-ro
    but a broken note
- ``` ^wf-225a1f1bdba2
    x
    ```
- 🔒 ``` ^wf-03307ef71ba2-ro
    code with unclosed
    ```
````

Edits to its text are refused, and the line is put back. Ticking or deleting it **is** allowed, and so is editing
its children or adding new ones. Removing the `🔒` and `-ro` does not unlock it. Fix such a node in Workflowy, or
force your edit with `--force`. Examples are in [read-only items](editing.md#read-only-items).

The root's note cannot be marked; if it has broken formatting it is shown in its nearest form, without a marker.

## Mirrors

A Workflowy mirror shows the same content in two places. syncflowy talks only to Workflowy's beta API
(`https://beta.workflowy.com`, the same account and data), which says which nodes are mirrors, and shows each
mirror with its original's content, even when the original is outside the synced subtree:

```markdown
---
workflowy: f253cb87-e739-4614-9ee2-bacc26f79523
syncflowy: {gen: 1}
---
- Local section ^wf-9f707b66a5ec
    - [ ] Local item ^wf-4390738a065f
- Shared **checklist** ^wf-40707d64a59d
    kept in the Library
    - [ ] pack charger ^wf-88089a945f59
    - [ ] pack passport ^wf-e70226fd8cac
- ⧉ [[#^wf-4390738a065f|Local item]] ^wf-a583f12cc087-ro
- [ ] Another section ^wf-03d6331bd7f1
- ⧉ [[#^wf-40707d64a59d|Shared checklist]] ^wf-35636ee56b11-ro
- [ ] ^wf-d404d48f69f7
- ⧉ (mirror of a deleted item) ^wf-13e8a79a7565-ro
```

- The first placement of some content in the file is shown **in full**, under that placement's tag. When the
  placement is a mirror, that is the mirror's own tag: "Shared **checklist**" (`40707d64a59d`, a mirror) lives in a
  "Library" node outside the synced subtree, and editing it here edits the original. "Local item" is shown in full
  under its own tag, and its mirror `a583f12cc087` is the stub.
- Every other placement of the same content is a read-only **stub**: `⧉ [[#^wf-<full line's id>|name]]`, an Obsidian
  link to the full line. The link only works when the full placement is a list item, not a heading. Which placement
  is the full one is meant to stay the same across syncs (a design choice, not verified).
- A mirror whose original was deleted is a read-only placeholder, `⧉ (mirror of a deleted item)`. Its children are
  not shown, and edits, ticks and deletes of it are refused without `force`
  (`"" (13e8a79a7565) is a mirror placeholder; its delete was not sent (use force)`).
- A node with an empty name (`- [ ] ^wf-d404d48f69f7`) is an ordinary item, not a mirror.

You cannot create a mirror from Markdown. What edits, ticks and deletes do to mirrors and stubs is in
[mirrors](editing.md#mirrors).

## Conflict markers

When you and someone in Workflowy changed the same name, or the same line of the same note, between two syncs
(edits to different lines of one note merge cleanly), Workflowy's
version is kept and yours is saved in the file as a **conflict marker**: a child line starting with `⚠`, with a
`%%wf-conflict:name%%` or `%%wf-conflict:note%%` tag. A real file after a sync where both sides edited Leaf B's name,
Leaf C's note and the root note:

```markdown
---
workflowy: fbb41d14-4255-4a33-a374-75e120087497
syncflowy: {gen: 3}
---
root note from Workflowy

- ⚠ local note %%wf-conflict:note%%
    my local root note
- [ ] Interior bullet (renamed in WF) ^wf-db95d4350b28
    - Leaf A ^wf-9c1966ad51fd
    - [ ] Leaf B, the *Workflowy* wording ^wf-47a73bc8996b
        - ⚠ Leaf B, my **local** wording %%wf-conflict:name%%
    - [ ] Leaf D added in WF ^wf-c4b2afbf5310
- [ ] Todo item ^wf-bf628758ff4d
- [ ] Leaf C ^wf-a9d3a61a59e3
    a note typed in Workflowy
    - ⚠ local note %%wf-conflict:note%%
        my local note
        two lines
```

The shape:

- **Name conflict**: `- ⚠ <your name> %%wf-conflict:name%%`, as the node's first child line.
- **Note conflict**: `- ⚠ local note %%wf-conflict:note%%`, with your note as its indented lines.
- Markers come before the node's real children, name before note. For the root note, the marker is at the top of
  the body.
- A code block's name (or a name that would look like a fence or a tag) is written as a fenced marker:
  ```` - ⚠ ``` %%wf-conflict:name%% ````, the code, and a closing fence.
- Markers are never sent to Workflowy and stay until you delete them. Deleting a marker resolves the conflict.

How to resolve one is in [conflicts](editing.md#conflicts).
