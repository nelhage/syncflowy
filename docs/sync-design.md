# Workflowy ⇄ Markdown sync: design notes

Status: design, 2026-10-13. Implemented under [`engine/`](../engine/README.md) as of 2026-10-14; see "Implementation notes" at the end for where the implementation differs.

This document works out what a Workflowy/Markdown sync engine would look like, given what
[workflowy/](workflowy/) established about the API and the web UI. Facts about Workflowy cite the
spec as `DM§x` ([data-model.md](workflowy/data-model.md)) and the UI findings as `UI:xx`
([ui-findings.md](workflowy/ui-findings.md)).

## 1. Goal and the property that matters

One Workflowy node (the **sync root**) and its subtree is mirrored into one Markdown file. Each run of
the tool:

1. fetches the remote subtree,
2. parses the Markdown file,
3. three-way merges both against the checkpoint saved by the previous run,
4. pushes the local edits to Workflowy through the API,
5. re-renders the Markdown and saves a new checkpoint.

The property that matters most is this: **Workflowy collaborators must be able to work normally and never
lose data, and they should not notice the bridge.** "Unsupported features render badly in Markdown" is
acceptable. "Unsupported features are destroyed by a round-trip" is not. Most of the design below
follows from taking that property seriously.

Two consequences come first:

- **We never write a field the Markdown user did not change.** Detecting change in the Markdown
  domain, against the Markdown we last wrote, is what makes lossy rendering safe (§5).
- **Everything we write must already be in the UI's canonical form.** Otherwise the next UI edit
  rewrites it (churn) or, if it is malformed, blanks it (data loss, UI:F1/F2).

## 2. Constraints imposed by the API (and what each one costs)

These are the facts from the investigation that shape the design most, roughly in order of impact.

| fact | consequence for sync |
|---|---|
| **No change feed, no ETags, no conditional writes** (DM§2.7). `modifiedAt` does not propagate to ancestors (UI:C1), bumps without content changes (UI:E8), and is `null` on old nodes (DM§1.1). | Every pull is a **full snapshot** of the subtree, compared by content. Writes are last-writer-wins, and there is always a race window between our read and our write (§9). |
| **List is depth 1 with no pagination; export is the whole account** (DM§4.2, §4.10). Export took 1 s for 13k nodes / 4.4 MB. Its documented 1 req/min limit was not enforced. | Pull with `nodes-export` and filter to the subtree. A breadth-first list walk costs one request *per node* (leaves included), because you cannot tell a leaf without listing it. That is only viable for small subtrees or for targeted re-checks. |
| **Placement is only `top` or `bottom`** (DM§7). The UI uses midpoints (UI:A1), but the API cannot. | Putting a node in the middle of a sibling list costs `min(k, n−k)` moves (§8). Reorders are the most expensive kind of edit. |
| **Create's `name` goes through a markdown-ish transform**: entities decoded, lines joined or split into children, prefixes eaten, completion "leaks" (DM§6.2). Update and create's `note` are verbatim (DM§6.1, §6.3). | **Never send real text through create's `name`.** Create with a safe name, then `update` (§7.2). |
| **Text is WF-HTML, an XML fragment** (UI:E1–E8, F1/F2): `&amp; &lt; &gt; &quot;`, `<b> <i> <u> <s> <code> <a href> <span class="colored …"> <time …>`, raw `\n` in notes. A malformed name renders blank and is wiped by the next UI edit. | All writes go through `wfhtml` and must be well-formed and canonical (a fixed point of `wfhtml.canonicalize`). |
| **Mutations need the full UUID**; only GET takes short ids (DM§3.1). | The checkpoint maps short id → UUID. Markdown can carry the short id. |
| **A move into one's own descendant returns 200 and creates a cycle** (DM§4.7.5). | The planner must check for cycles against *fresh* remote state before every move. This is a hard safety requirement. |
| **Mirrors cannot be detected or resolved through the API** on production (DM§10.1, UI:H1–H4). The UI shows the origin's content, while the API shows an empty shell. | On production, mirrored content cannot be synced and writes to a shell are invisible to UI users. The beta API fixes this (DM§10.5), so the engine uses the beta API only and follows mirrors on it (§10, §15.1). |
| **Delete is soft** (UI:B1/B2). Deleted ids can come back with the same id. | A 404 is not a tombstone. On the plus side, every delete we push can be recovered from the Workflowy Trash. |
| **10 req/s budget** (self-imposed, no 429 observed). | A full initial upload of N nodes costs about 2N requests (§7.2). |
| **API-created parents appear collapsed in the UI** (UI:C2). Collapse state is not exposed. | A small "tell" to collaborators that we cannot avoid. |

## 3. Architecture

```
            ┌──────────── checkpoint (JSON) ─────────────┐
            │  base_wf: subtree as of last sync           │
            │  base_md: exact text of the .md we wrote    │
            └──────────────┬──────────────────────────────┘
                           │
 nodes-export ──► Remote tree R ──┐                   ┌── parse(current .md) ◄── file
                                  ├─► 3-way merge ◄───┤
          base_wf ───────────────►┘     │             └── parse(base_md)
                                        ▼
                                  merged tree M
                                        │
                 ┌──────────────────────┼─────────────────────────┐
                 ▼                      ▼                         ▼
        plan API ops (M vs R)   render(M) → new .md (CAS write)   new checkpoint
                 │
        journal + apply ──► re-export ──► verify == M
```

The engine is a Rust crate that builds a single static binary (§12.1). Proposed modules:

| module | responsibility |
|---|---|
| `wfclient` | Thin, blocking API client: rate limiting, retries on 5xx and network errors, full-UUID enforcement, export. Port the behaviour of `prod_client`, minus the scope guard. |
| `tree` | An immutable outline model: `Node { id, name_wf, note_wf, layout, completed, children }` plus a projection for the subtree. |
| `wfhtml` | WF-HTML parse, escape, canonicalise, well-formedness. A port of `src/syncflowy/wfhtml.py`, checked against it by a differential test over a corpus. |
| `mdinline` | WF-HTML inline ⇄ Markdown inline (bold, links, `<u>` passthrough, dates, escaping). |
| `mdblock` | Tree ⇄ Markdown document (list structure, headings, notes, id tags, code blocks). |
| `merge` | Three-way merge of fields, structure and sibling order, plus a conflict policy. |
| `plan` | Turns the merged-versus-remote diff into a sequence of API calls, including the ordering algorithm and cycle checks. |
| `apply` | Executes the plan with a write-ahead journal and crash recovery. |
| `state` | Checkpoint format, versioning and the md fingerprint. |

**Keep the core pure.** Parse, render, merge and plan should take values and return values, with no I/O.
That makes them property-testable (`proptest`) and snapshot-testable (`insta`), and leaves I/O to a thin
shell: the API client, file watching, the checkpoint store and the daemon loop.

## 4. The Markdown dialect

### 4.1 Shape

```markdown
---
workflowy: 5e2e76e4a846cfce4c6d          # sync root (short id or URL)
syncflowy: {headings: 1, gen: 412}       # config + the render generation this file came from (§9.1)
---
Free text here is the sync root's note.

# Planning %%wf:1a2b3c4d5e6f%%
Note on "Planning": any paragraphs right under the heading.

- Draft the spec ^wf-8e1f00aa2b3c
    A note line on "Draft the spec".
    - [ ] Write the mock ^wf-0c0ffee12345
    - [ ] Run the UI scenarios ^wf-77aa12cd0e9f
- **Bold** item, <u>underlined</u>, a #tag and a [link](https://example.com) ^wf-a1b2c3d4e5f6
- ```python ^wf-…
  x = 1
  ```

# Later %%wf:2b3c4d5e6f70%%
- …
```

Rules:

- **Sync root**: identified in the frontmatter. Its `name` is not rendered by default (the file name is
  independent). Optionally it becomes a `# H1` or a `title:` field. Its `note` is the preamble before the first node.
- **Heading levels** (`headings: k`, default 0): nodes at depth 1…k render as `#`…`#`×k headings, and deeper
  nodes as nested lists under them. This is *structural* (by depth), not by `layoutMode`. Rationale:
  the user's real account has essentially no `h1`/`h2`/`h3` nodes (global-readonly §5), and
  Markdown headings cannot sit inside a list anyway. Header levels in the file must then be consistent: a
  `###` directly under a `#` is a parse error, or is coerced to depth 2 with a warning.
- **List item first line = `name`.** Indented continuation lines before the first child are the **`note`**,
  with raw `\n` preserved line for line (notes are WF-HTML with raw newlines, UI:E8). Obsidian renders
  them as the item's body, which is the natural rendering.
- **Completed nodes are not in the file.** A completed node and its whole subtree are hidden (§4.5).
  Ticking a checkbox is a one-shot command: the next sync completes the node in Workflowy and removes it
  (and its subtree) from the file (§4.3).
- **Indentation**: render with four spaces per level. Parse tabs or any consistent space width.
- **Id tag** at the end of the item's first line (§4.4).

### 4.2 Inline text

Each WF-HTML construct is mapped to the most faithful Markdown. Anything without a mapping becomes
**literal inline HTML**, which round-trips exactly and which Obsidian renders:

| WF-HTML | Markdown | notes |
|---|---|---|
| text with `&amp; &lt; &gt; &quot;` | plain `& < > "`, backslash-escaped where Markdown would misread it | `&lt;b&gt;` (typed literally in the UI, UI:E2) must render as `\<b>` so that it stays distinguishable from real markup |
| `<b>`, `<i>`, `<s>`, `<code>` | `**…**`, `*…*`, `~~…~~`, `` `…` `` | Parse also accepts `__ _` and normalises them. Code spans need care with backticks in content. |
| `<u>` | `<u>…</u>` (passthrough) | Markdown has no underline. |
| `<a href="u">u</a>` (autolink, UI:E5) | bare `u` | Must reproduce the UI quirk on write: the href keeps a raw `&`, the text has `&amp;`. |
| `<a href="u">text</a>` | `[text](u)` | Escape `)`, `]`, and spaces in `u` (angle form `<u>`). |
| `<span class="colored c-red">` | passthrough `<span class="colored c-red">…</span>` | Optional: a CSS snippet in Obsidian to colour it. Highlight classes are unobserved (UI E4 not run). |
| `<time startYear=… startMonth=… startDay=…>Thu, Jan 15, 2026</time>` | a compact form such as `📅 2026-01-15`, **only if** the text equals what we would regenerate from the attributes (DM§6.2 step 11). Otherwise passthrough. | Ranges and 24-hour or locale variants are unobserved (E6), so passthrough is the safe default. |
| anything else that is well-formed | passthrough | |
| `#tag`, `@mention` | the same text | WF tags are plain text (UI:C2), and Obsidian tags have the same syntax, which is a happy accident. |

Things in Markdown with no WF meaning (`[[wikilinks]]`, `==highlight==`, `$math$`, `![[embeds]]`, `![images](…)`,
`%%comments%%`, footnotes) are sent to Workflowy as **literal text**. WF users then see `[[Page]]`. That is acceptable but visible.
They are rendered back unescaped, so they keep working in Obsidian after a sync.

**Escaping is the trickiest part of the dialect.** WF text such as `1. foo`, `> x`, `# x`, `[ ] x`,
`- x` or `---` at the start of a name or note line, `*` and `_` mid-text, a trailing `^abc` or `[wid:: …]`, and
a `%%wf:` would all be misread by Obsidian or by our parser. Render escapes minimally (to limit visual noise), and
parse accepts both escaped and unescaped forms wherever that is unambiguous. Line-leading escapes matter most
in notes, where a WF note line starting with `- ` would otherwise become a child node.

### 4.3 Node kinds, `layoutMode` and completion

`layoutMode` is **preserved in the checkpoint** and changed from Markdown only when the Markdown explicitly
expresses a different kind:

| `layoutMode` | rendered as | can Markdown change it? |
|---|---|---|
| `bullets` (≈ all real nodes), `p`, `bullet`, `cmnt`, `cmnt-c`, `board`, `table`, `dashboard`, unknown | `- ` or `- [ ] ` (see below) | no: kept as is |
| `todo` | `- [ ] ` | yes: optional; see below |
| `code-block` | a fenced block inside the item | yes: a fence makes it, unfencing removes it |
| `quote-block` | `- > …` | yes |
| `numbered` (on a parent) | its children use `1.` markers | yes, if *all* children switch markers |
| `h1`–`h3` inside a list | `- # …` is legal CommonMark, but Obsidian rendering must be checked; otherwise keep the layout in the checkpoint only | maybe |
| `divider` | `- ---`, if Obsidian renders it acceptably | maybe |

**Completion: hidden, and a checkbox is a command.** Completed nodes and everything under them are
not rendered. This matches how most Workflowy users work, with "hide completed" turned on. It also keeps the
file small: about half of the user's account is completed (6,786 of 12,972 nodes, global-readonly §5), almost
all of it plain `bullets`. Consequences:

- **Every visible node is incomplete**, so a checkbox in the file never *displays* state. It is only an
  affordance for completing. `- ` and `- [ ]` both parse as "not completed", and which one is rendered is
  cosmetic: `- [ ]` for `todo` layout and (by default) for leaves, `- ` for interior nodes. Because the marker
  form is not data, a leaf gaining a child in Workflowy (`- [ ]` turning into `- `) never shows up as an edit.
- **`[x]` on any item means "complete it"**, including a plain bullet the user changed from `- ` to `- [x] `.
  The sync calls `/complete` on that node only (completion never cascades, UI:D2) and then drops the node and
  its subtree from the re-rendered file. Any other edits made to the node or its descendants in the same
  session are pushed first.
- **Uncompleting is Workflowy-only.** A hidden node cannot be unticked from Markdown. When a WF user
  uncompletes it, it reappears in the file at its position (a remote "appear", §6.2).
- **Ticking is not deleting.** Deleting a line deletes the node (to the WF Trash). Ticking completes it. Both
  remove the line from the file, so the difference should be documented for the Markdown user.
- **Headings** cannot hold a checkbox. Accept `# [x] Title` (or `# [ ] Title`, if rendered) as the completion
  command for heading-level nodes, and render them without a box by default.
- **An unticked box is cheap.** If the user ticks and then unticks before a sync, nothing happens.

New nodes typed in Markdown get layout `bullets` by default, or `todo` if typed with `- [ ]` and
configured so. A new item typed directly as `- [x] …` is created and then completed: it goes to Workflowy and
immediately disappears from the file. That is odd, but consistent.

### 4.4 Embedding ids

Each line carries the node's 12-hex short id. Candidate encodings:

| encoding | Obsidian reading / live preview | plain Markdown viewers | headings | extra |
|---|---|---|---|---|
| `[wid:: 8e1f00aa2b3c]` (Dataview) | Dataview: a "wid" pill. Without Dataview: literal text on every line | visible | works | queryable with Dataview |
| `(wid:: 8e1f00aa2b3c)` | Dataview shows only the value | visible | works | |
| `^wf-8e1f00aa2b3c` (block id) | hidden or dimmed; native | a short visible tail | **not allowed** on headings | enables `[[file#^wf-…]]` links that survive syncs, because the id is the Workflowy id |
| `%%wf:8e1f00aa2b3c%%` | hidden in reading view, shown while editing | visible | works | |
| `<!-- wf:8e1f00aa2b3c -->` | hidden when rendered | hidden when rendered | works | the most portable, but the most verbose |

Recommendation: make the encoding a config option, **parse every form**, and default to block ids for
list items and `%%wf:…%%` for headings. Verify that Obsidian keeps a block id on a list item that has
continuation lines. The short id suffices in the file because the checkpoint holds the full UUID, and a
GET by short id recovers it if the checkpoint is lost (DM§3.1).

Ids in a freely edited text file are fragile. The engine must handle all of these:

- **Duplicated tag** (a line was copied): the occurrence that best matches the base (text similarity,
  then position) keeps the identity. An other occurrence whose line carries a further valid tag that no
  line wins (its own tag next to a copied one) keeps that node; the rest are new nodes, and their tags
  are rewritten on output.
- **Deleted tag, text kept**: this looks like delete plus create, which would destroy the node's WF history
  and, worse, re-create the node's WF-only features (colour, mirror status, `layoutMode`) as lost. Fallback:
  pair each unmatched new item with an unmatched deleted base item of the same parent, position and similar
  text (or text that keeps the old name as its prefix), and treat the pair as an edit. When the two are the
  only unmatched pair under their parent and at the same index, pair them whatever the text: a real "delete
  a line, type a new one in its place" then becomes a rename, which keeps the old node's history (the smaller
  loss). The report names every such pairing.
- **Tag of a node outside this file**: the user pasted from another synced file or typed a tag. This is a
  cross-file move if the id exists remotely (§6.4), and is ignored with a warning otherwise.
- **Edits that make a tag ambiguous** (text after the tag, two tags on a line): take the last valid one.

### 4.5 Projections: what is not in the file is not deleted

The file deliberately does not show everything. **Completed nodes and their entire subtrees** (including
incomplete descendants) are the main case. Orphan-shell placeholders (§10), nodes past a depth limit and unrepresentable
content are the others. All of these are **hidden nodes**, and the projection is a core mechanism from day one,
not an option.

- **Visibility is computed from the remote tree**: a node is visible if it and all its ancestors (up to the
  sync root) are incomplete and nothing else hides it. The sync root itself is always rendered, even if
  completed (with a warning).
- **Local side**: what the Markdown user could see is exactly what is in `base_md`. A node in `base_md` and
  missing from the current file was deleted (or ticked, which is not missing but `[x]`). A node *not* in
  `base_md` was hidden, and its absence means nothing.
- **Remote side**: a node that was visible in the base and is now hidden (completed, or under a newly completed
  ancestor) is a **disappear**, not a delete. A node hidden in the base and visible now (uncompleted, or
  its ancestor was uncompleted) is an **appear**, and is rendered at its remote position.
- The checkpoint stores the full remote subtree including hidden nodes, so `diff(base_wf, R)` is uniform,
  but only visible nodes have Markdown counterparts.
- **Hidden nodes are never written by the sync** except as a side effect the user asked for (a tick, or a delete
  of a visible ancestor, which takes its hidden descendants with it). In particular the reorder planner
  never moves them (§8.1).

Deleting a visible parent deletes its hidden (completed) children too. That is correct Workflowy semantics,
but the Markdown user cannot see what they are deleting. The delete brake (§8.3) should count hidden
descendants, and the sync report should name them (the engine names the first ten, in the report and in a dry
run).

## 5. Fidelity: how lossy rendering stays lossless

### 5.1 Change detection in two domains

The checkpoint stores **the exact text of the last Markdown we wrote** (`base_md`) next to the WF values
it was rendered from (`base_wf`).

- **Local changes** = `diff(parse(base_md), parse(current_md))`. Both sides are parsed by the *same, current*
  parser, so a change in the parser or renderer never produces phantom edits. If the file is byte-identical
  to `base_md`, there are no local changes at all, which is the common fast path.
- **Remote changes** = `diff(base_wf, R)`, compared field by field on raw WF strings.

A node whose Markdown is unchanged is never written, however lossy its rendering. Loss can only affect
nodes that the Markdown user edited.

### 5.2 Writing an edited node without dropping what Markdown could not show

For an edited field, the naive write is `md_to_wf(new_md)`. That drops whatever the rendering could not
express (leading or trailing whitespace, an unrecognised attribute and so on) **across the whole field, not
just the edited part**. Refinement: let `b` be the base WF value and `b' = md_to_wf(render(b))` its lossy
round-trip. If `b' == b`, write `md_to_wf(new_md)`. Otherwise compute the edit from `b'` to
`md_to_wf(new_md)` and apply it to `b` with a three-way text merge (character-level for names, line-level for notes).
If that merge does not apply cleanly, fall back to the naive write and log what was lost. Nothing is logged
when the user saw all of `b`: the item changed kind (a code block unfenced, say) and `b` round-trips in its
old kind, or a read-only value (§5.3) was edited with `force`.

### 5.3 Representability tiers

At render time, classify each field:

1. **Exact**: `md_to_wf(render(x)) == x`. This is the normal case for UI-produced text.
2. **Exact via passthrough**: it round-trips only because parts are emitted as raw inline HTML. Fine, but
   uglier in Obsidian.
3. **Not representable**: newlines in a non-code name, malformed WF-HTML written by some other API client,
   constructs our dialect cannot hold. Render a best-effort, clearly marked read-only form (for example a
   `🔒` and an `ro` flag in the id tag), refuse local edits to it with a warning, and keep the real value in
   the checkpoint.

Tier 1 or 2 should cover everything the UI produces. Property tests should enforce it over a corpus
built from the UI findings and the user's real account.

### 5.4 Convergence

After a push, the file is re-rendered from the new remote state. If the user typed `__x__` we store
`<b>x</b>` and re-render `**x**`: the file normalises once. Required properties:

- `render(parse(render(t))) == render(t)` (rendering is a fixed point)
- `parse(render(parse(m))) ≅ parse(m)` (normalising Markdown does not change meaning)
- everything written is well-formed, canonical WF-HTML: `canonicalize(w) == w`

## 6. The three-way merge

Inputs: base `B` (the checkpoint), remote `R` (the export filtered to the subtree), local `L` (the parsed file).
Identity comes from UUIDs in `B`/`R` and short-id tags in `L` (mapped through `B`).

### 6.1 Per-node field merge (`name`, `note`, `layout`)

`completed` is not a merged field. Locally it can only go from false to true (a tick), and a tick always applies
(completing an already-completed node is skipped, so that `completedAt` is not re-stamped, DM§4.5).

| local | remote | result |
|---|---|---|
| unchanged | changed | take remote |
| changed | unchanged | push local (§5.2) |
| changed | changed to the same value | nothing |
| changed | changed differently | **conflict** |

Conflict policy, with Workflowy as the shared source of truth and its users unaware of us: **remote wins in
Workflowy**, and the local version is kept *in the Markdown only*, as a visible conflict marker next to
the node (for example a `%%conflict: <local text>%%` line, or a child item `⚠ local edit: …` that is
excluded from sync until the user resolves it). For notes, try a line-level diff3 first: concurrent edits
to different lines merge cleanly.

### 6.2 Structure

- **Create** (local, no id; or remote, unknown id): add it. Creates never conflict.
- **Delete vs. edit**: deletes lose. If one side deleted a node and the other edited it or added
  descendants under it, keep the node and the edited content. If the remote deleted it, resurrect it with a
  local create (new id). The original is in the Trash, but the API cannot restore it. Report it.
- **Move vs. move**: remote wins.
- **Move vs. delete**: the move wins (the node survives).
- **Moves out of the subtree** (remote): export shows where the node went, so it is a removal from the file,
  not a delete. **Moves in**: an addition.
- **Cycles**: a local move of A under B combined with a remote move of B under A must be caught here. Remote wins
  and the local move is dropped. The planner re-checks independently (§8.2), because the API will happily corrupt the tree.

Completion and hiding interact with all of these (§4.5):

| local | remote | result |
|---|---|---|
| ticked | edited | push nothing but the completion, and keep the remote edit. The node disappears from the file; a local edit of the same field is reported with its text. |
| edited (and/or children added) | completed it, or an ancestor | push the edits and creates anyway: the nodes exist and WF users can see them greyed under the completed item. Then they disappear from the file. **Report this**, because the Markdown user's fresh work vanishes from view. |
| deleted | completed it | completion counts as an edit, so the delete loses: the node stays (completed, hidden). |
| moved it | completed it | apply the move. The node is hidden at its new position. |
| moved a node under X | completed X | apply it. The moved node becomes hidden. Report it. |
| ticked | deleted it | nothing to do. |
| anything | uncompleted a hidden node | an "appear": render it at its remote position, which never conflicts with local edits because the local side could not see it. |

### 6.3 Sibling order

Order is a sequence of ids per parent, *not* priorities. Priorities are rewritten by API moves and are
not comparable across snapshots. Derive the order by sorting `R` by `(priority, id)`. Ties are possible only
through UI races and the UI's tie order is unknown (DM§7), so treat a tie as unstable and never write because
of one.

Merge per parent: compute each side's *moved set* relative to `B` (the elements not in the LCS of the base and
that side). Start from the remote order and re-insert each locally moved or created node after its nearest
local predecessor that also exists in the result. The merge only constrains the order of **visible** nodes.
Hidden siblings (often numerous, since completed items accumulate under active lists) stay wherever they are in
Workflowy (§8.1). A new item typed between visible A and B, with hidden C between them in Workflowy, can end up
on either side of C. Nobody in Markdown can tell, and WF users with "show completed" on see only a completed item shift.

### 6.4 Several files, one run

A single export serves every synced file in the vault, so run all of them in one pass with a global id map.
This turns "deleted in `a.md`, appears with its tag in `b.md`" into a **move** instead of a delete plus an
unknown id. A tag that is still on its line in its own file was copied, not moved: the pasted line is a new
node, as for a duplicated tag within one file (§4.4). When files are synced one at a time (the CLI), a run
that would delete a node whose tag (or a descendant's) is in another synced file outside the run (found
through the files the other checkpoints record) holds the delete back as a pending delete (§9.1 guard 2) and
asks for both files to be synced together (and, if that file is paused, which leaves it out of any run, for it
to be unpaused first); a stale or missing recorded file only costs the check. When such a delete is finally
sent, its line is no longer in the base render, so all its descendants count and are named as hidden, also
those that were visible when it was cut (an over-report in the safe direction, as for deferred deletes). Sync roots
must not nest. Alternatively, a nested root could render in the outer file as a link or
an `![[embed]]` and be treated there as a hidden subtree.

## 7. Applying changes to Workflowy

### 7.1 Order of operations

1. **Creates**, top-down in document order, each at `position: bottom` of its (possibly new) parent.
   In pre-order this produces the right order with no extra moves.
2. **Moves** (re-parenting), each preceded by a cycle check against the current remote tree.
3. **Field updates** (`update` for name, note and layout).
4. **Sibling reorders** (§8.1).
5. **Completions** (`complete` for ticked nodes). They come after the edits to the same subtree, so a
   half-applied sync never hides work that has not been pushed yet. The API never uncompletes.
6. **Deletes** last, children before parents (or just the top-most deleted node, since delete is recursive).

Before each update, optionally `GET` the node and abort that operation if it no longer equals `R`. This
narrows the race with collaborators to the gap between that GET and the write. It cannot be eliminated (§9).

After applying, re-export and check that the remote now equals the merged tree `M`. Any difference is a bug
or a concurrent edit, and the next cycle picks it up as a remote change.

### 7.2 Creating a node safely

Create's `name` is transformed (markdown, entities, line splitting, completion leak), so:

1. `POST /nodes {parent_id: <uuid>, name: "⟳syncflowy:<nonce>", note: <wf note or omitted>, layoutMode: <layout>,
   position: "bottom"}`. The nonce (§7.3) is a fixed point of the create pipeline, so it creates exactly one node with
   that name. `note` and `layoutMode` are verbatim on create.
2. `POST /nodes/:id {name: <wf name>}`.
3. `POST /nodes/:id/complete` if the item was typed already ticked (rare).

That is 2 requests per node. Optimisation: if a name is provably a fixed point of the create pipeline
(a conservative character whitelist: no `&*[]`\`~#-` at risky positions, no leading/trailing space, no
newline), send it directly and skip step 2. Check the whitelist against the mock's `text.py`, which implements the
pipeline: a test sends whitelisted names through the mock's create and asserts they come back unchanged.

During step 1–2, collaborators can briefly see the nonce as the name. A crash between the steps leaves a
node named by its nonce, which the journal handles.

### 7.3 Journal and crash recovery

Creates are not idempotent, and a crash after `POST /nodes` but before the new UUID is recorded would
create a duplicate on retry. Approach:

- Write each planned operation to a journal (in the checkpoint directory) before sending it, and record
  the result after.
- For creates, put a nonce in the first request, as the name (`name: "⟳syncflowy:<nonce>"`, replaced in
  step 2), so that recovery can find a half-created node by scanning the export for the nonce.
  The cost is that the nonce is visible for a moment. Not in the note: the API cannot write a `null` note
  back (DM§6.4), so a nonce note would turn the note of every note-less new node into `""`, while the name
  is overwritten in step 2 anyway. A name that is sent directly (the whitelist path of §7.2) has no nonce;
  recovery finds it as an unknown child of the parent with exactly that name.
- On startup with a non-empty journal, reconcile it against a fresh export before doing anything else.

## 8. Ordering and structural safety

### 8.1 Reordering with only top and bottom

Given the target order `T` and the current order `C` of one parent: nodes that do not move must form a block
that is **contiguous in `T`** and **in increasing order in `C`**. Every node before the block is moved
to the top (in reverse order), and every node after it is moved to the bottom (in order). Choose the longest
such block, which is a linear scan for the longest run of `T` whose positions in `C` increase. The cost is
`n − |block|` moves.

**Only visible nodes are in `T` and `C`**, and hidden (completed) siblings are never moved. Moving visible
nodes to the top or bottom leaves the hidden ones in place, and the visible order is still exact. This keeps
`n` at the number of *visible* siblings, which matters because completed items pile up in active lists and would
otherwise make every mid-list insert more expensive. It also means we never bump `modifiedAt` on, or reorder,
nodes the Markdown user cannot see.

| edit | cost |
|---|---|
| append at the end | 0 extra (create at bottom) |
| insert at index k of n | `min(k, n−k)` moves |
| move one item from the end to the front | 1 |
| move one item from the front into the middle | `min(k, n−k)` |
| reverse a list | `n−1` |

Side effects: every moved node and the parent get a `modifiedAt` bump (DM§8). Repeated top moves halve
the minimum priority until it reaches 0, which triggers a server-side renumber (free, no bumps). Each move
also widens the window in which a collaborator's concurrent reorder of the same list can be clobbered.

This cost is **fundamental**. No API call places a node between two siblings. If Workflowy ever adds an
`after`/`index` parameter, it is a one-function change.

### 8.2 Cycle and boundary guards

Before every move, against the latest known remote tree (updated as we apply operations):

- the destination must not be the node or any of its descendants (the API would return 200 and detach the subtree);
- the destination must be inside a sync root (we never move nodes into parts of the account we do not manage);
- `parent_id` must always be sent explicitly as a full UUID (an omitted `parent_id` is accepted and its
  behaviour is unknown, open question S1).

### 8.3 Mass-change brakes

If the local diff deletes more than some fraction of the file's nodes (for example 20%, or more than 50 nodes),
or if the file is missing or empty, **refuse and require `--force`**. Count hidden (completed) descendants of
deleted nodes toward the threshold, since the user cannot see them. Ticks are not deletes and do not count,
though a separate, higher threshold for mass completion is cheap insurance. Truncated files from editor crashes,
sync conflicts or bad merges are the most likely way to cause real damage. A missing file means "pause this
root and report" (§9.1), never "delete everything". The Trash makes accidents recoverable, but only by hand, one subtree at a time.

## 9. Concurrency

| race | handling |
|---|---|
| A WF user edits a node between our export and our write | Pre-write GET check per update (§7.1). The remaining window is about one round-trip. A loss means their keystrokes are overwritten by ours, last writer wins. This cannot be closed without conditional writes in the API. |
| A WF user reorders a list we are reordering | Ours wins for the nodes we move. Keep reorder sequences short (§8.1), and re-export after. |
| The Markdown file changes while we sync | Read the file, merge, and **compare-and-swap** on write: re-read just before writing. If it changed, discard the render and rerun the merge (the remote work is already done and the checkpoint is updated). |
| Obsidian is mid-edit when we rewrite the file | With the engine on the server, the editing happens on another device, and our write reaches it through Obsidian Sync. The hazards are those of §9.1. How Obsidian merges a synced-in change into an open, dirty buffer should be tested. |
| Several engines, or a checkpoint that is itself synced between devices | Not supported. Run exactly one engine and keep its checkpoint outside the synced vault. §9.1 covers the supported multi-device setup. |

### 9.1 Deployment: one engine behind Obsidian Sync

The intended topology is a single engine on a headless server that holds the Workflowy credentials. Obsidian
Sync carries the Markdown files to and from every client device.

```
 phone / laptop ──Obsidian Sync──► server vault ◄──► engine ◄──► Workflowy API
 (edit .md)        (minutes of delay,   (files)     (checkpoint +
                    file-level merges)               base history: server-only, outside the vault)
```

This removes the multi-engine problems. There is one checkpoint, one writer to Workflowy, and credentials on
one machine. It adds a second, slow, merging sync layer between the user and the engine. In the simple case
it is fine, and the engine just sees a file that changed. The hazards all come from the engine seeing a file
that **is not the result of a user editing the latest render**. Obsidian Sync can produce such files in three ways:

- **Stale overwrite**: a client that was offline (or slow) uploads a version based on an old render, and it
  wins outright.
- **Text-merged file**: Obsidian Sync merges concurrent changes to a Markdown file at the character level. For
  example, the engine's re-render meets a client's edits. The result can have lines in odd places, duplicated
  lines, or a half-merged id tag.
- **Split arrival**: changes the user made together across two files (cut from `a.md`, paste into `b.md`) reach
  the server minutes apart.

The exact merge behaviour of Obsidian Sync (when it merges, when the last writer wins, and when it creates a
conflict copy) should be checked experimentally. The guards below do not depend on it.

#### Guard 1: the file names its own base (render generations)

Every render that changes the file increments a generation number and stamps it in the frontmatter
(`gen: 412`). The engine keeps, server-side, a **history of recent renders**: for each generation, the
Markdown text and the remote tree it was rendered from. When a file arrives, the local diff is computed
against **the generation the file says it came from**, not against the latest render:

```
local_changes = diff(parse(history[file.gen].md), parse(file))
```

That diff is what the user actually did, relative to what they saw. Everything that changed between
`file.gen` and the latest generation is a remote change that the merge already knows about. So a stale
overwrite from gen 400 no longer looks like "the user deleted everything added since 400, and reverted every
remote edit since 400". It looks like "the user made these three edits to gen 400", which are then merged
forward as normal. This is essentially a rebase, and it is the most important guard in this topology.

Details:

- The `gen` field changes only when the file content does, so idle files do not generate sync traffic.
- A text-merged file normally carries the newer `gen`, because the client never touches that line and the merge
  takes the engine's side. That is right when the merge was good. Guard 3 covers bad merges.
- If `gen` is missing, unparseable, or older than the kept history (keep, say, 30 days or 200 generations),
  use **safe mode**: field edits are applied only where the text differs from *every* known version of that
  node (Guard 3), and there are no deletes, no moves, no reorders and no ticks (a line typed as `[x]` is
  created unticked). Report it.

#### Guard 2: deferred deletes

A node that disappears from a file is not deleted at once. It goes into a **pending-delete** state for a grace
period (for example 10 minutes, and at least one more sync cycle in which that file is unchanged):

- If its id tag shows up in another file in the meantime (split arrival of a cut and paste), it is a **move**.
- If the line comes back (a transient bad merge, or an undo on a client), nothing happens.
- If an untagged line with matching text appears near where it was, the deleted-tag fallback (§4.4) rematches it.
- Otherwise the `DELETE` is sent when the grace period ends.

Pending deletes are not rendered back into the file, because the user removed them. The cost is that Workflowy
users see a deleted item linger for a few minutes. Deferral matters more here than anywhere else, because a
delete followed by a re-create loses the node's id, its WF-only attributes and any mirrors of it, and the API
cannot restore from the Trash.

Ticks do not need deferring: a completion is reversible from the Workflowy side and loses nothing.

#### Guard 3: stale-value detection per node

Even with the right base, a bad text merge can put an old value of a node back into the file. The base
history allows a cheap check: if a node's text in the file **equals a value the engine rendered for that node
in an earlier generation**, and differs from the base generation's value, treat it as a stale echo, not an
edit. Do not push it, and log it. A user who genuinely retypes an old version of a line is the false
positive, and that is rare and harmless (the next edit goes through).

The same idea applies to structure. A node that is in the file but **not** in the base generation (so the user
could not have seen it there) and is hidden remotely (completed, or under a completed ancestor) is a
resurrection by a merge, not a user action. For example: the user ticks P, the engine drops P and its children,
and then a merge re-inserts a child line under the wrong parent. Drop it from the file and never interpret it
as a move.

#### Guard 4: fewer, safer engine writes to the file

Every engine write to a file is a chance for a concurrent merge with a client. Write only when there is
something the user needs to see:

- **Remote changes or new id tags**: write.
- **Changed `syncflowy:` options** (for example `ids:` or `checkboxes:`): write, so the new form shows at once.
  A new checkbox policy is applied to every line, not only to new ones.
- **Pure normalisation** (`__x__` → `**x**`, re-escaping): defer it until the file next needs a write for
  another reason.
- **Debounce on the server side**: push and re-render a file only after it has been unchanged for some quiet
  period (for example 30–60 s). Edits arriving from clients come in bursts, and this avoids pushing half-typed
  lines to Workflowy (which collaborators would see) and avoids writing back into a file mid-burst.
- **Parse by content, not mtime**: mtimes from other devices are not trustworthy. Make sure a file is
  complete before using it (stable for the debounce period, frontmatter present, root id matches). If it does
  not parse, skip the cycle and retry. Never treat a parse failure as "everything deleted".
- **Atomic replacement vs. in-place writes**: test which one the server's Obsidian Sync handles better (a rename
  may look like delete plus create).

New-item id write-back is the most frequent collision. The user types an item on a phone, the engine creates
it and writes back `^wf-…`, while the user keeps typing on that line. A merge may then put the tag mid-line
or drop it. So the parser has to find a valid tag anywhere in an item's first line (not only at the end). The
fallback rematch for untagged lines (§4.4) must be reliable, because "tag lost in a merge, node created twice"
is the likeliest glitch in this topology.

#### Guard 5: files are identified by root, not path

- Find synced files by the `workflowy:` root in their frontmatter. A **rename or move** of the file on any
  device then needs no special handling.
- If **two files claim the same root** (a copy, or a sync conflict copy), stop syncing that root and report.
  Never merge both.
- If a **file is missing**, pause that root and report. Do not re-render it unasked, because deleting it on a
  client is the obvious way to say "stop syncing this", and the engine must not delete anything in response.
- Files whose names look like conflict copies are never synced.
- `syncflowy sync`, which is given files instead of scanning a vault, also compares a file with the one recorded
  in the root's checkpoint: if that one still exists under another path and still names the root, the root is
  claimed twice and the file is not synced. A rename (the old path is gone, or names another root) is accepted.

#### Guard 6: other writers on the clients

Plugins that rewrite lines on any device also race with the engine. Examples: the Tasks plugin appending
`✅ 2026-10-13` when an item is ticked, Linter or other formatters, and templating plugins. Run none of them on
synced files on the server. Map or strip known Tasks-plugin fields on ticked lines (otherwise the done date
would be pushed into the Workflowy name just before the node is completed).

#### Control and status through the vault

With the engine on a headless server, the vault is also its user interface:

- The engine maintains a status note (for example `Workflowy Sync Status.md`, engine-owned and overwritten
  every cycle). It shows the last successful sync per root, conflicts awaiting resolution, pending deletes and
  when they fire, safe-mode episodes, resurrections dropped, and API errors.
- Per-root controls live in the file's frontmatter, so they can be set from any device. Examples:
  `syncflowy: {paused: true}` and `syncflowy: {force: true}`, the latter for a one-time override of the
  mass-change brake, which the engine clears after use. An option typed outside `syncflowy:` gets a warning;
  a top-level `paused: true` is honoured anyway (pausing is the safe direction), a top-level `force` never is.
- Obsidian Sync's own version history is a second safety net for the Markdown side. The engine's base history
  is the safety net for "what did we believe and push", and it should be kept long enough to reconstruct any
  push from the sync logs.

## 10. Mirrors

**Decision (2026-10-15): the engine uses the beta API only** (`beta.workflowy.com`, DM§10.5), which serves the same
account and data as production. The CLI refuses the production host.

The reason is that production exposes no mirror metadata (DM§10.1). A shell looks like an empty node (`name ""`,
`note ""` or `null`), and the UI shows the origin's content in its place (UI:H1). On production:

- A shell in the synced subtree would render as an empty bullet, while WF users see real content there.
- A Markdown user who types into that "empty" bullet writes to the shell, which **nobody in the UI will ever
  see** (UI:H1, H3). A user who deletes it removes a mirror that WF users rely on.
- Detection is only a heuristic: `name == ""` and `note in ("", None)` (UI:H2). Genuinely empty bullets also
  exist, and API-made shells can have children of their own.

An earlier version of the engine supported production by rendering every node that matched the heuristic as a
read-only placeholder (`- ⧉ (empty or mirror)`), never rendering its children, and refusing edits and deletes
without `--force`. That left mirrors as a hole and made genuinely empty bullets read-only. On the beta API a shell
is known, not guessed, so that heuristic is gone: an empty node is an ordinary item, and mirrors are synced as
their origins (§15.1).

**Placeholders** remain for one case: an **orphan shell** (DM§10.4, `data.mirror.origin_id: null`), a mirror
whose origin was deleted. It has no content to show. It renders as a marked, **read-only** placeholder
`- ⧉ (mirror of a deleted item)` with the `-ro` flag; its children are never rendered, and edits, ticks and deletes
of it are not pushed without `--force`. Items typed under it are created under its parent instead.

## 11. What collaborators could notice

| tell | avoidable? |
|---|---|
| A brief `⟳syncflowy:<nonce>` name while a node is created | Shorten the gap; the whitelist path avoids it. |
| Parents we create appear collapsed | No (UI:C2). |
| Several siblings shuffle when we insert mid-list | No (§8.1). |
| Text in a slightly non-UI form | Yes: always write canonical WF-HTML. |
| `[[wikilinks]]`, `==hl==` and other Obsidian-isms in their text | Partly: map what can be mapped, or lint and warn the Markdown user. |
| An edit of theirs overwritten in a narrow race | Narrow it, but it cannot be eliminated. |
| Deleted items in their Trash | Only from deliberate deletes; these are a safety net. |

## 12. Major design decisions

### 12.1 Language and host

Decided: **Rust**, built as a single static binary (for example the `x86_64-unknown-linux-musl` target), so
deployment is copying one file to the server. The engine runs as a **standalone daemon** on the headless
server next to the vault (§9.1). It is not an Obsidian plugin, and nothing runs inside Obsidian on any device:
Obsidian is only an editor of files that Obsidian Sync moves around.

Why Rust over Go (the other single-binary option):

- The core is mostly data types and case analysis: merge outcomes, plan operations, line kinds, tag forms,
  conflict results. Enums with exhaustive `match` make each new case (completion interactions, mirror stubs)
  a compile error everywhere it needs handling, which matters in an engine that can delete data.
- Go would be quicker to write, and its cross-compiling is simpler. Neither outweighs the above.

Consequences:

- **Synchronous by design.** The daemon is one sequential loop (debounce, pull, merge, apply, write), so use a
  blocking HTTP client (for example `ureq`) and a file watcher (`notify`), and no async runtime.
- **The Python stays as the test oracle.** The mock is plain HTTP and serves the Rust engine unchanged. The
  conformance suite keeps checking the mock against the real API, and `wfhtml.py` and the mock's `text.py`
  are the references that the Rust `wfhtml` port and the create-name whitelist (§7.2) are tested against.
- Markdown parsing is our own line-based parser in either case (the 3-way diff needs exact source text), so
  no general Markdown library is on the critical path.

### 12.2 Pull strategy

Export everywhere (one request, consistent, cheap at 13k nodes) versus a per-subtree list walk (proportional
to the subtree, but N requests). Recommend export, with a 60 s minimum interval in case the documented limit
is ever enforced. Hiding completed nodes makes the list walk more attractive for small roots, because it
never needs to list below a completed node: its cost is the number of *visible* nodes, not the subtree size.
Pushes do not need a pull first if the checkpoint is fresh: plan against `B`, guard each
write with a GET, and reconcile at the next export.

What the engine does: the daemon spaces its cycles by `--min-export-interval` (default 60 s), but a cycle that
pushes exports twice (before the merge, and again to verify and re-render), and the CLI never waits between
runs. A proactive wait would make every pushing `sync` take a minute for a limit that has never been seen
enforced. Instead an enforced limit degrades gracefully: the client retries a 429 until its waits add up to
`rate_limit_budget` (default 90 s), waiting what `Retry-After` asks (else the backoff), so the verification
export of a run whose writes are done is delayed by up to a minute rather than failed.

### 12.3 Heading mapping

Structural (depth ≤ k becomes a heading, by config) versus semantic (`layoutMode h1–h3` becomes a heading).
Recommend structural, plus preserving `layoutMode` in the checkpoint. A semantic mapping produces headings at
arbitrary depths, which Markdown cannot nest under list items cleanly, and the user's data barely uses them.

### 12.4 Completion display

Decided: completed nodes and their subtrees are hidden, and a tick is a one-shot "complete" command (§4.3,
§4.5). What remains open is cosmetic: which incomplete items get a `[ ]` box (default: todos and leaves). A
possible later option is a read-only "recently completed" section, rendered from the remote tree and ignored by
the parser, for users who want to see what was ticked off.

### 12.5 Conflict surface

Remote wins in Workflowy, and the local version is preserved in the Markdown with a marker. The alternative
(local wins) would make the Markdown user's view authoritative over collaborators who do not know the bridge exists.

## 13. Open questions that matter for sync

Some could be settled with more UI scenarios ([ui-scenarios.md](workflowy/ui-scenarios.md)):

1. **Shared nodes.** If a collaborator shares *their* node with this account, does this account's API key see it, in
   list and in export? Is it writable? Nothing has tested this yet, and the whole collaboration goal
   depends on it.
2. How the UI stores **code-block names** (are `<`/`&` escaped? are newlines raw?) (UI scenario G4).
3. **Highlight and colour** span classes (E4), and **date** variants: ranges, times, 24-hour or locale (E6).
4. The display order of **equal priorities** (A10/A11).
5. How the UI renders a non-code name containing `\n`.
6. Whether Obsidian keeps a `^block-id` on a list item with continuation lines, how it renders `- # h`
   and `- ---`, and how it handles external writes to an open, dirty file.
7. **Obsidian Sync's merge behaviour** for Markdown: when it text-merges, when the last writer wins, when it
   creates a conflict copy (and how that copy is named), and whether it preserves mtimes. Test it with two clients
   and the server, using deliberately concurrent edits to one list (§9.1).
8. **Mirrors in the UI** (for §15.1): does ticking a mirror complete the origin or the shell? Is `layoutMode` taken
   from the origin? How does the UI render an orphan shell? Which shell does H6 promote?
9. Whether `nodes-export` is ever rate-limited or cached (the export freshness test was inconclusive, global-readonly §4).

## 14. Suggested build order

1. Cargo crate skeleton and the static-binary build. Port `wfhtml`, with the differential test against
   `wfhtml.py`.
2. `mdinline`/`mdblock` render and parse with property tests: fixed points, `wfhtml` canonical output, and the
   tier classifier run over a corpus (UI findings plus an anonymised account export).
3. Read-only `syncflowy render <root> out.md`, plus checkpoint writing.
4. Local-only push (Markdown edits to Workflowy) against the **mock**, using the mock's `/_mock/ui/*` routes to
   simulate concurrent UI edits, canonicalisation and restores.
5. Full three-way sync, journal, brakes, then multi-file runs.
6. The daemon: watch mode, debounce, render generations and history, deferred deletes, the status note (§9.1).

## 15. Future work

### 15.1 Mirrors as their origins (on the beta API)

**Status: implemented (2026-10-14; see the implementation notes for what is narrowed). The engine uses the beta
API only (§10).** This needed the API to return each shell's origin on GET, list and export. The beta API (`beta.workflowy.com`, DM§10.5) does that and more: GET, list and
export carry `data.mirror.origin_id` on shells and `data.mirror.mirror_ids` on origins. A shell reports its origin's
content, a list of a shell returns the origin's children, and update, complete and create through a shell reach the
origin. This was observed on the beta host on 2026-10-13 (open-questions R1–R11). The mock implements it behind
`beta_mirrors`. An export without any `data.mirror` is an account without mirrors, whose view tree is the export
itself.

Three caveats were also observed:
- `mirror_ids` is a registry stored on the origin. The UI can leave a live shell out of it, and a production-host
  delete of a shell leaves a **ghost** in it. So the engine should build the origin → shells map from each shell's
  `origin_id`.
- A ghost makes its origin undeletable through beta, so the engine must remove shells only with `DELETE /mirror`
  or a beta plain delete.
- A shell's own persisted children are invisible in lists, but export includes them, and they are deleted along
  with the shell.

Two beta rules change the plan below. First, the engine can write through a shell, but it does not need to, because the
content node's own id always works. Second, **deleting a node fails while it, or any node in its subtree, is an origin
that still has mirrors**. So a Markdown delete of a subtree that contains origins must first delete their mirror roots
(`DELETE /nodes/S/mirror`) or decline the delete. Removing a mirror that WF users rely on is a visible change, so the
safer default is to decline the delete and report it.

For production, a workaround exists and has been rejected: `POST /nodes/S/mirror` into a scratch parent returns the
resolved `origin_id` (DM§10.2), and the throwaway shell can then be deleted. That costs two requests per
candidate and has side effects visible to collaborators: the origin's `modifiedAt` is bumped, a brief extra
mirror exists, and an entry lands in the Trash.

Goal: treat a mirror exactly as the UI does, as the mirrored node itself (UI:H1–H4). A synced subtree that
contains a mirror of a node from elsewhere lets the Markdown user edit that node in place, as a WF user can.

#### The view tree

The Markdown renders a **view tree**. Each item is a **placement** (a real node: an ordinary node, an origin
or a shell) showing a **content node**: the placement itself, or a shell's origin. The children of a shell
placement are the children of its origin. A shell's own fields and children are never rendered and never
written.

Because export returns the whole account, origins outside the sync root and their subtrees are already in
hand. The synced scope is therefore the closure of the root under "follow mirrors".

**Each content node is rendered once.** Walk the view tree in pre-order. The first placement of a content
node gets the full rendering: text, children and all. Every later placement of the same content node,
whether a shell or the origin itself, is a **mirror stub**:

```markdown
- Weekly review ^wf-3c4d5e6f7a8b
    - ⧉ [[#^wf-9a8b7c6d5e4f|Inbox triage]] ^wf-11aa22bb33cc
```

- The stub's own tag is its placement id, so every tag in the file is unique (Obsidian requires this of block
  ids), and every content node's children appear exactly once.
- The link points at the fully rendered placement, so in Obsidian the stub is a click-through.
- The stub's text is derived and its contents are ignored. Edits to its text are dropped, and the stub is
  rewritten in canonical form on the next engine write. Children typed under a stub are not dropped silently:
  they are kept as a conflict marker (§12.5) and reported, since they show the user intended to add something.
- **Cycles end in a stub.** Mirroring a node into its own descendants is allowed (DM§10.2), and the UI shows
  that recursively. The render-once rule turns the recursion into a stub.

Which placement is "first" depends on tree order, so a reorder or move can swap the full rendering and the
stub. That rewrites lines without anyone editing them. The ids are placement ids, so the rebase against
the file's generation (§9.1, guard 1) still reads both versions correctly, but a swap is a real file rewrite.
Make the choice sticky: keep the previous full placement while it still exists and is visible.

Visibility (§4.5) is computed on the view tree. Whether the UI completes the origin or the shell when a
mirror is ticked is untested (§13), so for now treat completion as a property of the content node, like name
and note. A shell's own `completed` flag is ignored.

#### Mapping Markdown edits

The local diff is computed on placements (lines and tags), and then translated to the real graph:

| Markdown edit | Workflowy operation |
|---|---|
| Edit the text of a fully rendered placement `p` | Update `content(p)`, which may be outside the sync root, as in the UI (H3). |
| Add, delete, reorder or move children under `p` | The same operation on `content(p)`'s children (H4). |
| Move a stub, or move a fully rendered shell placement | Move the shell itself. That is a move of the mirror, as when it is dragged in the UI. |
| Delete a shell placement (stub or full) | Remove that mirror only: `DELETE /nodes/S/mirror`. The content survives, as in the UI. |
| Delete the origin's own placement while shells exist | Emulate the UI (H6): move the origin into one shell's position (same parent, adjacent), then delete that shell. A plain API delete would orphan every shell, which the UI never does. |
| Delete the last remaining placement | Delete the content node, subject to the usual deferral and brakes. |
| Tick a placement | Complete `content(p)` (see the completion caveat above). |

When the full rendering of a node moves to a different placement (a swap or a deletion), the content node's
children come with it in the file, but nothing moves in Workflowy. The diff sees "children moved under the
same content node" and does nothing.

Cycle checks (§8.2) run against real `parent_id`s, not the view tree. Moving a line under an instance of
its own ancestor is a cycle even if the two look far apart in the file. The mass-delete brake (§8.3) counts
deletes of content nodes outside the root like any other.

Remote changes (§6) are diffed on the content nodes, and then re-projected. A remote UI delete of a mirrored
origin arrives as "origin moved, one shell gone" (H6). The view tree shows that as one placement disappearing
and the content staying, which is what the WF user saw.

#### Orphans, and creating mirrors

- **Orphan shells** only arise from API deletes, and the UI behaviour for them is unknown. Render them as
  §10's read-only placeholders (`- ⧉ (mirror of a deleted item)`).
- **Creating a mirror from Markdown** is a natural later feature: a stub line `- ⧉ [[#^wf-…]]` with no tag of
  its own, placed by the user, becomes `POST /nodes/<origin>/mirror`. It is not needed for the first version.

## Implementation notes

The engine under `engine/` follows this design. The places where it narrows or departs from it, or decides
something the design left open, are below. Each is marked `UNVERIFIED` in the code where it rests on a guess; the
module docs have the details.

- **Never run against the real API.** All behaviour was developed and tested against the mock server. The open
  questions of §13 are still open, and the mock's choices stand in for them.
- **Mass-change brakes (§8.3).** The thresholds are 50 deletes, or 20% of the visible nodes from 5 deletes on, and
  200 completions or 50% from 10 on. An empty file brakes only when the base had items and the merge deletes
  something. A file emptied because all its items were cut into another file of the run is not braked, because
  it deletes nothing and braking would keep remote changes out of it.
- **Beta API only (§10).** The default `--api-url` is `https://beta.workflowy.com`, and the production host is
  refused. There is no shell heuristic: an empty node is an ordinary item.
- **Mirrors (§15.1).** In `engine/src/mirrors.rs` the view tree of §15.1 is built per sync root, with the full placement chosen as the
  first visible one in pre-order and kept sticky from the previous base; the merge runs on it and its result is
  translated to the real nodes before planning. Narrowed from the design: a stub is read-only (its text edits are
  refused and rewritten rather than dropped silently, a tick of it is refused instead of completing the content, and
  items typed under it are lifted to its parent and reported, like items under a §10 placeholder, instead of being
  kept as a conflict marker); deleting the origin's own placement while shells exist is refused and reported, like any
  delete whose subtree holds an origin that still has mirrors (the UI's promotion of a shell, H6, is not emulated),
  and the refused subtree is kept whole; a refused mirror delete is not overridden by `--force`. Orphan shells are
  §10's placeholders, `⧉ (mirror of a deleted item)`. Mirrors are not created from Markdown. A shell's hidden persisted children (DM§10.5) are dropped from
  every tree the engine looks at, so they are never shown and are not counted when the shell is removed.
- **Nodes moved out of the file in Workflowy.** The design does not cover edits to such a node. The engine still
  pushes your edits and ticks to it where it now is. If both sides edited the same field, Workflowy wins and the
  local text is only logged (a marker cannot be shown because the line leaves the file).
- **Cross-file moves (§6.4)** compare the pasted line against the source file's base, so a collaborator's edit is
  not reverted. Without such a base nothing is pushed for the move. A node whose delete is pending in the source
  file is compared against its line rendered from the source's `base_wf`, which equals the line that was cut
  (a remote change cancels a pending delete).
- **Files synced one at a time (§6.4).** The CLI holds back a delete whose node is tagged in another synced file
  outside the run (read through `Checkpoint::file`), as a pending delete, so that a cut and paste synced in two
  commands is not a delete and a create. The brakes count it when first seen, as they count the daemon's
  deferred deletes; a file emptied into another file and synced alone is therefore still braked.
- **Safe mode (§9.1 guard 1)** also applies to a bootstrap with `--force` and no checkpoint, which takes the remote
  as the base.
- **Deferred deletes (guard 2)** are held while another synced file has edits waiting for its debounce, which the
  design does not ask for. Plain `sync` has no grace period and sends deletes at once.
- **The create nonce is in the name (§7.3)**, not in the note as first sketched, because the API cannot write a
  `null` note back.
- **Creates are never resent after a 5xx.** Any 5xx other than the application's own `internal_error` is treated as
  "maybe applied" and settled by the nonce scan of §7.3. No gateway 5xx or 429 has been observed.
- **Dry run** does not recover a leftover journal, since recovery writes.
- **Code blocks** with a ``` line are written with a longer fence and stay editable; the design left this open.
  A fence's info string (a language typed in Obsidian) is dropped with a warning, since Workflowy code blocks
  have none; the API's create pipeline would have made it the first line of the code.
- **Obsidian syntax** (`[[x]]`, `![[x]]`, `![alt](img)`, `==x==`, `$x$`, `%%comments%%`) is written back
  unescaped (§4.2), except a `$` before a digit when the value holds another `$`, to keep prices from reading as
  math, and a `%%` that would read as a tag (`%%wf:`, `%%wf-conflict:`) or has no partner in the value (it would
  pair with a `%%wf:…%%` tag). An image is literal text exactly as typed (no link inside). A `%%x%%` typed by a
  Workflowy user is therefore hidden in Obsidian's reading view.
- **Dates and locale variants** have no compact or structured form; they stay literal. A **link title**
  (`[a](u "t")`) has no WF form either: the link is pushed without it and the report says the title was dropped.
- **Checkbox policy, id tag form, depth, headings** are frontmatter options (`checkboxes`, `ids`, `depth`,
  `headings`); the design only recommended making the tag form configurable. `checkboxes: none` shows no boxes,
  `todo` nodes included (their layout is kept). A node whose children are hidden by `depth` counts as interior
  for the box, so it does not look like a leaf.
- **Files** are written in place through symlinks and keep their mode. The state directory layout (checkpoint,
  pending deletes, history, journal, lock) is described in `engine/src/state.rs`.
- **Not done:** several engines on one account, any
  Obsidian Sync experiment (§13 Q7),
  and the optional title or `# H1` rendering of the root's name (§4.1).
