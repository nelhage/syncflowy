# The synced file

This page explains every part of a synced Markdown file: the frontmatter and its options, how the outline is laid
out, the id tags, inline formatting, Workflowy's node kinds, read-only items, mirrors and conflict markers. For what
your edits *do* in Workflowy, see [What your edits do](editing.md).

All examples are real output of syncflowy. Indentation in rendered files is one tab per level.

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
(`syncflowy render 584a2265b1f5 showcase-default.md`). The Workflowy root is called "Projects". The notes after the
example explain each part.

`````markdown
---
workflowy: 584a2265b1f5
syncflowy: {gen: 1}
---
Free text here is the sync root's note.
It has **two** lines & an ampersand.

- Planning ^wf-c4d1c386bbc4
	Note on "Planning": lines right under the heading.
	- Draft the *spec* ^wf-6f447ed4d57b
		A note line.

		After an empty line.
		\- not a child, a note line
		- [ ] Write the mock ^wf-7696a6cecc1b
		- Deep ^wf-7159c324c985
			- Deeper ^wf-3d437204e52d
				- Deeper still ^wf-d4c13a902931
					- Nearly there ^wf-1193e6c3f339
						- [ ] Bottom `x = 1` ^wf-a7dd06839eb9
	- Numbered steps ^wf-8ad9f06c144a
		1. [ ] first ^wf-0c4237730edf
		2. [ ] third ^wf-3fef701966a0
			with a note
	- [ ] Board layout is kept as a plain item ^wf-d2803bab6c39
	- [ ] Paragraph layout ^wf-789a380208a9
	- [ ] An h1 inside the outline ^wf-89d94a2f20aa
- ``` ^wf-a8f0d66b829e
	fn main() {
	    println!("hi & bye");
	}
	```
	runs on save
- > To be or not to be ^wf-9e3fa46d6753
- --- ^wf-67edfe175330
- ⧉ (empty or mirror) ^wf-dc0a1ef2a4f0-ro
- ⧉ (empty or mirror) ^wf-17aeefba91fc-ro
- [ ] 🔒 broken bold ^wf-e35cd47d380d-ro
	a malformed name (UI:F1)
- [ ] 🔒 line one ⏎ line two ^wf-ab134da98f1d-ro
- ```` ^wf-3116e1ea24c4
	x
	```
	y
	````
- [ ] **bold** and unknown markup ^wf-d2bc815a47c5
- [ ] See https://example.com/a?b=1&c=2 on 📅 2026-01-15 and at 📅 2026-01-15 14:30 ^wf-27bc08d6af57
- [ ] <u>underlined</u>, <span class="colored c-red">red</span>, [a link](https://example.com) and #tag @who ^wf-6cbdcc22af58
- [ ] 1\. not a list ^wf-d3d12c4a3698
- [ ] \- not a child ^wf-4bd8b3fa7aa7
- [ ] \# not a heading ^wf-bd795fec898f
- [ ] \[ ] not a box ^wf-621582283d15
- [ ] a \^wf-8e1f00aa2b3c in the text and %\% percents ^wf-107e855c3844
- [ ] A todo with children ^wf-3c25fbb230bb
	- [ ] leaf one ^wf-06d864c2f2e3
	- [ ] leaf two ^wf-90c78092b4d4
		&#32;&#32;indented note line
- [ ] Trailing empty note lines ^wf-8131c541013d
	first
	
	
- [ ] Leading empty note line ^wf-e8f8dc3bf364

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
9. **Placeholders.** `⧉ (empty or mirror)` marks an empty node that may be a mirror. See [Mirrors](#mirrors).
10. **Read-only items.** `🔒` and the `-ro` tag mark nodes whose Workflowy text cannot be shown exactly: broken
    formatting, or a line break inside a name (shown as `⏎`). See [Read-only items](#read-only-items).
11. **Escaping.** Workflowy text that would look like Markdown structure is escaped: `1\.`, `\-`, `\#`, `\[ ]`, a
    tag-like string `\^wf-...`, `%\%`. A code block whose code contains ```` ``` ```` gets a longer fence.
12. **Inline formatting.** Bold, italic, code and links become Markdown; underline and colours stay as HTML; dates
    are `📅 2026-01-15`. See [Inline formatting](#inline-formatting).
13. **Whitespace.** Leading spaces in a note line are written `&#32;`. Empty lines at the end of a note are lines
    that hold only the indentation (a tab), so they are not mistaken for the gap before the next item. An empty line
    at the start of a note is a blank line.

## Frontmatter

The frontmatter is the block between the two `---` lines at the top. It must start on the first line. syncflowy
reads only two keys, `workflowy:` and `syncflowy:`, and keeps every other line as you wrote it.

### Keys

| key | values | default | written by | may you edit it? | effect |
|---|---|---|---|---|---|
| `workflowy` | short id, full UUID, or `https://workflowy.com/#/<short id>` URL (see below) | required | you or `render`; kept verbatim, including a trailing `# comment` | only to point the file at another root (which then needs `render` or `force`) | which node this file syncs |
| `syncflowy.headings` | `0` to `6` | `0` | the engine, when not 0 | yes | render depth 1 to N as headings ([headings mode](#headings-mode)); applied at the next sync |
| `syncflowy.ids` | `auto`, `block`, `comment`, `dataview`, `dataview-hidden`, `html` | `auto` | the engine, when not `auto` | yes | the form of the id tags ([id tags](#id-tags)); applied only at the next rewrite |
| `syncflowy.checkboxes` | `todos-and-leaves`, `todos`, `all`, `none` | `todos-and-leaves` | the engine, when not the default | yes | which items get a `[ ]` ([checkboxes](#checkboxes)); only for items not already in the file |
| `syncflowy.dates` | `true`, `false` | `true` | the engine, when `false` | yes | `true`: dates as `📅 2026-01-15`; `false`: as Workflowy's `<time ...>` HTML |
| `syncflowy.depth` | a positive whole number, or `null` / `~` / empty for unlimited | unlimited | the engine, when set | yes | hide nodes deeper than N (the root is depth 0). Hidden, not deleted. |
| `syncflowy.paused` | `true`, `false` | `false` | you; the engine keeps it | yes | `true`: the file is neither pushed nor pulled until you remove it |
| `syncflowy.force` | `true`, `false` | `false` | you; **the engine removes it** after one sync | yes, one-shot | override the brakes and placeholder guards once, adopt a file with no checkpoint, apply a pending `ids:` change |
| `syncflowy.gen` | a number | none | the engine, every time it rewrites the file | **no** | the generation the file came from; removing it, or changing it to a value the engine does not know, puts the next sync into [safe mode](editing.md#safe-mode) |
| other keys inside `syncflowy:` | anything | | | | a warning (`unknown syncflowy key "colour" (dropped on the next render)`), then dropped |
| any other line (`title`, `tags`, `aliases`, comments, lists) | anything | | you | yes | never read; kept as written, but moved below the two syncflowy keys |

An invalid value makes the whole file `skipped`: nothing is sent and nothing is written. For example:

```console
$ syncflowy sync misc/badcfg.md           # syncflowy: {headings: 9}
misc/badcfg.md: skipped: the file does not parse: bad syncflowy config headings: "9"
[exit 1]
```

`dates: yes` and `depth: 0` are invalid too. A missing or unparseable `gen` is not an error; it means safe mode.

> **Warning.** `paused` and `force` work only **inside** the `syncflowy:` map. Write
> `syncflowy: {gen: 2, paused: true}`. A separate top-level `paused: true` line is treated like any other unknown
> line: it is kept and ignored, and the file is synced.

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

At each sync the engine renders the file with the current options, but it only rewrites the file if the result
differs in meaning. That makes some option changes slow to show:

| you change | what happens |
|---|---|
| `headings`, `depth`, `dates` | the file is rewritten at the next sync |
| `ids` alone | **nothing visible** (`file unchanged`), even with `--force` on the command line. It is applied at the next rewrite for any other reason, or at once if you also set `force: true` in the frontmatter. |
| `checkboxes` alone | no visible change on items already in the file: boxes are "sticky" ([checkboxes](#checkboxes)) |
| `paused: true` | `paused (paused: true in the frontmatter)`, exit 0, nothing done |
| `force: true` | used for one sync, then removed: `syncflowy: {…, gen: 5, force: true}` becomes `syncflowy: {…, gen: 6}` |

`depth: 2` on a file, then `sync`:

```markdown
---
workflowy: d8f33418-f3d4-4711-9804-f92283868a29
syncflowy: {depth: 2, gen: 2}
---
- Alpha ^wf-2cfa93ea5c4e
	- [ ] Alpha child ^wf-4d4244ef7feb
- [ ] Beta ^wf-f87ff5059285
	- [ ] Beta child ^wf-0487c89da11b
- Level 1 ^wf-dca7e2dcaa37
	- Level 2 ^wf-7d8fcf23cae8
```

`Level 3` and `Level 4` are hidden, not deleted. `depth: null` brings them back:

```markdown
- Level 1 ^wf-dca7e2dcaa37
	- Level 2 ^wf-7d8fcf23cae8
		- Level 3 ^wf-ae908fb5262c
			- [ ] Level 4 ^wf-7d0b0e5e18ba
```

`dates: false` shows Workflowy's date HTML instead of the compact form (Workflowy's value is not touched):

```markdown
- [ ] See https://example.com/a?b=1&c=2 on <time startYear="2026" startMonth="1" startDay="15">Thu, Jan 15, 2026</time> and at <time startYear="2026" startMonth="1" startDay="15" startHour="14" startMinute="30">Thu, Jan 15, 2026 at 2:30 PM</time> %%wf:27bc08d6af57%%
```

`render --force` starts the frontmatter over (only `workflowy:`, `headings` and `gen`), so it loses your other
options and keys.

### Files syncflowy will not read

| file | report |
|---|---|
| no frontmatter (or an empty file) | `skipped: the file does not parse: no frontmatter` |
| frontmatter without `workflowy:` | `skipped: the file does not parse: frontmatter has no workflowy: root` |
| closing `---` missing | `skipped: the file does not parse: unterminated frontmatter` |
| a second `workflowy:` line naming a different root | `skipped: the file does not parse: bad workflowy: root "..."` |

A skipped file is never read as "everything was deleted". The body itself never fails to parse.

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
- The engine writes `- ` markers (or `1.` under a numbered parent) and one tab per level. When reading, it accepts
  `-`, `*`, `+`, `1.` and `1)` markers, and tabs or any consistent number of spaces.
- A line that fits nowhere (for example text at column 0 after a list) is attached to the note of the nearest item
  above it, with a warning: `line 22: line is not indented under an item; attached to the preceding item's note`.

A file edited with spaces, other markers, notes and a stray line:

```markdown
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

After `sync` (`7 created, 3 updated`), normalised to tabs and `-`/`1.` markers:

```markdown
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

Note: syncflowy writes empty lines at the end of a note as lines holding only a tab. Whether Obsidian or a
formatting plugin keeps such lines has not been checked.

### Headings mode

With `headings: N` (or `render --headings N`), nodes at depth 1 to N are written as headings (`#` for depth 1, `##`
for depth 2, and so on), whatever their Workflowy layout. A heading's note is the lines right under it, at column 0.
Deeper nodes are lists under their heading. The showcase tree with `--headings 2`:

`````markdown
---
workflowy: 584a2265b1f5
syncflowy: {headings: 2, gen: 1}
---
Free text here is the sync root's note.
It has **two** lines & an ampersand.

# Planning %%wf:c4d1c386bbc4%%
Note on "Planning": lines right under the heading.

## Draft the *spec* %%wf:6f447ed4d57b%%
A note line.

After an empty line.
\- not a child, a note line

- [ ] Write the mock ^wf-7696a6cecc1b
- Deep ^wf-7159c324c985
	- Deeper ^wf-3d437204e52d
		- Deeper still ^wf-d4c13a902931
			- Nearly there ^wf-1193e6c3f339
				- [ ] Bottom `x = 1` ^wf-a7dd06839eb9

## Numbered steps %%wf:8ad9f06c144a%%

1. [ ] first ^wf-0c4237730edf
2. [ ] third ^wf-3fef701966a0
	with a note

## Board layout is kept as a plain item %%wf:d2803bab6c39%%

## Paragraph layout %%wf:789a380208a9%%

## An h1 inside the outline %%wf:89d94a2f20aa%%

# ``` %%wf:a8f0d66b829e%%
fn main() {
    println!("hi & bye");
}
```
runs on save

# > To be or not to be %%wf:9e3fa46d6753%%

# --- %%wf:67edfe175330%%

# ⧉ (empty or mirror) %%wf:dc0a1ef2a4f0-ro%%

# ⧉ (empty or mirror) %%wf:17aeefba91fc-ro%%

# 🔒 broken bold %%wf:e35cd47d380d-ro%%
a malformed name (UI:F1)

# 🔒 line one ⏎ line two %%wf:ab134da98f1d-ro%%

# ```` %%wf:3116e1ea24c4%%
x
```
y
````

# **bold** and unknown markup %%wf:d2bc815a47c5%%

# See https://example.com/a?b=1&c=2 on 📅 2026-01-15 and at 📅 2026-01-15 14:30 %%wf:27bc08d6af57%%

# <u>underlined</u>, <span class="colored c-red">red</span>, [a link](https://example.com) and #tag @who %%wf:6cbdcc22af58%%

# 1\. not a list %%wf:d3d12c4a3698%%

# \- not a child %%wf:4bd8b3fa7aa7%%

# \# not a heading %%wf:bd795fec898f%%

# \[ ] not a box %%wf:621582283d15%%

# a \^wf-8e1f00aa2b3c in the text and %\% percents %%wf:107e855c3844%%

# A todo with children %%wf:3c25fbb230bb%%

## leaf one %%wf:06d864c2f2e3%%

## leaf two %%wf:90c78092b4d4%%
&#32;&#32;indented note line

# Trailing empty note lines %%wf:8131c541013d%%
first

# Leading empty note line %%wf:e8f8dc3bf364%%
second
`````

Differences from list mode:

- Headings carry `%%wf:...%%` tags, because Obsidian does not allow block ids on headings.
- Headings never show a checkbox, even for to-dos ("A todo with children"). You can still tick one: write
  `# [x] Title`.
- A heading's note loses its empty lines at the start and end ("Trailing empty note lines" shows only `first`).
- A code block at heading depth is a heading line with a fence, then the code at column 0, then the closing fence.

> **Caution: headings you type.** Items below a heading belong to that heading. If you type a heading deeper than
> `headings:` allows, it is kept as a node, with a warning
> (`heading at depth 2 but headings: 1; kept as a heading`), and the items after it become **its children**. At
> the next rewrite it is shown as a list item. In a real run, typing `### Skipped level typed by hand` above "Beta child" in a `headings: 1` file moved "Beta child"
> under the new node:
>
> ```markdown
> # Beta %%wf:f87ff5059285%%
> A note typed under the Beta heading
>
> - Skipped level typed by hand ^wf-ba8e0445d656
> 	- item under the h3 ^wf-816ac16e2284
> 	- [ ] Beta child ^wf-0487c89da11b
> ```
>
> A heading that skips a level is first coerced to the next level (`heading level 3 coerced to 2`). A `#` heading
> at column 0 in a file with `headings: 0` also becomes a node, with the items under it as its children.

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

Remember that an `ids:` change alone does not rewrite the file; add `force: true` to apply it at once.

### Flags

A tag can carry a flag after the hex: only `-ro` (read-only) exists, as in `^wf-e35cd47d380d-ro` or
`%%wf:e35cd47d380d-ro%%`. syncflowy sets it; removing it does not unlock the item. An unknown flag is ignored with a
warning.

### Tags you change

Short version (details and real examples in [id-tag accidents](editing.md#id-tag-accidents)):

| you | result |
|---|---|
| type a line with no tag | a new node |
| remove the tag but keep the text (nearly) the same | matched back to its node: `untagged item matched to "Butter (plus marker)" (5cd12d5db79b)` |
| remove the tag and change the text a lot | the old node is **deleted** and a new one created |
| copy a tagged line within the file | the best-matching line keeps the node; the others become new nodes |
| put two tags on one line | the last one wins (`2 id tags on one line; the last one wins`) |
| type a tag that exists nowhere | `treated as a new item`, re-tagged |
| move or copy a tagged line into another synced file | the node **moves** there (when both files sync in one run) |

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
| link | `[text](https://example.com)`; `[text](<url>)` when the URL has spaces or brackets | `<a href="url">text</a>` |
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
| `%%` | `%\%` | reserved for tags and markers |
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
| `%%hidden comment%%` | the literal text, **visible** to Workflowy users | `%\%hidden comment%\%` | **no**: it is now visible text |
| `![img](https://example.com/i.png)` | literal text with the URL as a link | `\![img\](https://example.com/i.png)` | **no**: no longer an image |
| `[a](https://example.com "title")` | literal brackets around a link | `[a\](https://example.com "title")` | **no**: link titles are not supported |
| `__bold__`, `_italic_` | bold, italic | re-spelled `**bold**`, `*italic*` at the next rewrite | yes |
| `<https://example.org/auto>` | a link | the bare URL `https://example.org/auto` | yes |
| `- [/] ...`, `- [-] ...` (Tasks statuses) | `[/] ...` as part of the name | `- [ ] \[/] ...` | no: not a checkbox |
| `- [x]typed` (no space) | `[x]typed` as text | | not a tick |

> **Warning.** Do not use `%%comments%%` in synced files for private notes: they are sent to Workflowy as plain
> text, where every collaborator can read them.

### Lossy values

Some Workflowy text is well-formed but cannot be shown exactly, for example markup from another app
(`<b>bold</b> and <foo>unknown</foo> markup`). It is shown in its nearest Markdown form
(`**bold** and unknown markup`) and stays editable. When you edit it, syncflowy keeps what it can and reports
`the name of "bold and unknown markup, edited" (d2bc815a47c5) lost formatting Markdown could not show`.

## Node kinds and layouts

Workflowy nodes have a layout (bullet, to-do, heading, board, code block, ...). The file shows the ones Markdown can
express; the others render as plain items and keep their layout in Workflowy.

| Workflowy layout | in the file | checkbox (default policy) | can you change it from Markdown? |
|---|---|---|---|
| bullets | `- text` | leaves only | yes: into a code block, quote, divider, or numbered list, by writing that kind |
| to-do | `- [ ] text` | always | no. A new `- [ ]` line is a plain bullet. |
| h1, h2, h3 | plain item | leaves only | no, kept |
| paragraph, board, table, dashboard, and others | plain item; a board's columns and a table's rows are ordinary nested items | leaves only | no, kept |
| code block | ``- ``` `` then the code indented one level, then a closing fence; the note after the fence | never | remove the fence: becomes a bullet. Fence an item: becomes a code block. |
| quote block | `- > text` | never | remove `> `: bullet. Write `- > text`: quote block. |
| divider (empty) | `- ---` | never | write `- ---` over an item: divider, **and its text is emptied** |
| divider with text | plain item | leaves only | |
| numbered (on the parent) | children are `1.`, `2.`, ... (visible ones only) | | switch **all** children to `1.` markers: parent becomes numbered; all back to `-`: bullets |

A file with one node of each kind:

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
[changing a node's kind](editing.md#changing-a-nodes-kind). Two traps: a fence's info string (```` ```python ````)
becomes the first line of the code, and `- # text` inside a list is not a heading but literal text (written back as
`- \# text`).

### Checkboxes

`- ` and `- [ ] ` both mean "not completed". Adding or removing a box sends nothing to Workflowy. The `checkboxes:`
option decides which items get a box:

| `checkboxes:` | boxes on |
|---|---|
| `todos-and-leaves` (default) | to-dos and items with no visible children |
| `todos` | to-dos only |
| `all` | every list item |
| `none` | none (to-dos still get one) |

Headings, code blocks, quotes, dividers and placeholders never get a box. Workflowy to-dos always do.

**Boxes are sticky.** Once a line is in the file, it keeps the form it has there, across rewrites, even when the
node gains or loses children or you change `checkboxes:`. The policy decides only for items that are new to the
file. So changing `checkboxes:` on an existing file changes nothing you can see. In a real run, `checkboxes: all`
added to the showcase file left "Planning", "Deep" and the other parents without a box; only a new file that already
had `checkboxes: all` when it was first filled from Workflowy boxed every item. (One exception: a to-do whose box
you removed gets it back at the next rewrite.)

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

A Workflowy mirror shows the same content in two places. How syncflowy shows one depends on the host you sync with
(see [the real service and the beta host](cli.md#the-real-service-the-beta-host-and-rate-limits)).

### On the normal service: placeholders

The normal API does not say which nodes are mirrors: a mirror looks like a node with an empty name, no note and the
bullets layout. syncflowy cannot tell it from a genuinely empty bullet, so it shows both as a read-only placeholder,
`⧉ (empty or mirror)`, never shows their children, and refuses edits, ticks and deletes of them without `force`.

```markdown
---
workflowy: 382f21e4-a57b-4700-b8ec-2d3446752b5c
syncflowy: {gen: 1}
---
- Local section ^wf-35210c56a92d
	- [ ] Local item ^wf-9f4782fe3a4a
- ⧉ (empty or mirror) ^wf-4d7a28d2e08e-ro
- ⧉ (empty or mirror) ^wf-22bae20cea4a-ro
- [ ] Another section ^wf-5ec1b14b69dc
- ⧉ (empty or mirror) ^wf-26f05f221dfc-ro
```

> **Caution.** A placeholder may be a real mirror that Workflowy users see full of content. Deleting it with
> `--force` removes that mirror for everyone.

### On the beta host: mirrors as their content

On `https://beta.workflowy.com` the API marks mirrors, and syncflowy shows each mirror with its original's content,
even when the original is outside the synced subtree. The same tree:

```markdown
---
workflowy: 382f21e4-a57b-4700-b8ec-2d3446752b5c
syncflowy: {gen: 1}
---
- Local section ^wf-35210c56a92d
	- [ ] Local item ^wf-9f4782fe3a4a
- Shared **checklist** ^wf-4d7a28d2e08e
	kept in the Library
	- [ ] pack charger ^wf-74036988f668
	- [ ] pack passport ^wf-ac4ae82d2fef
- ⧉ [[#^wf-9f4782fe3a4a|Local item]] ^wf-22bae20cea4a-ro
- [ ] Another section ^wf-5ec1b14b69dc
- ⧉ [[#^wf-4d7a28d2e08e|Shared checklist]] ^wf-26f05f221dfc-ro
```

- The first time some content appears in the file, it is shown **in full** under the mirror's own tag. "Shared
  **checklist**" lives in a "Library" node outside the synced subtree; editing it here edits the original.
- Every other placement of the same content is a read-only **stub**: `⧉ [[#^wf-<full line's id>|name]]`, an Obsidian
  link to the full line. The link only works when the full placement is a list item, not a heading. Which placement
  is the full one is meant to stay the same across syncs (a design choice, not verified).
- A mirror whose original was deleted becomes a `⧉ (empty or mirror)` placeholder again.
- On the beta host, genuinely empty bullets are shown as ordinary empty items, not placeholders.

You cannot create a mirror from Markdown. What edits, ticks and deletes do to mirrors and stubs is in
[mirrors](editing.md#mirrors).

## Conflict markers

When you and someone in Workflowy changed the same name or note of the same node between two syncs, Workflowy's
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
