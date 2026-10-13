# UI scenario findings (human-in-the-loop)

Results of the scenarios in [ui-scenarios.md](ui-scenarios.md), performed by a human in the Workflowy web UI and
observed via the API with `experiments/ui/ui.py` (snapshots under `experiments/ui/snaps/<scenario>/`).

## A1. Drag-reorder into the middle

Setup `a, b, c, d` = 100/200/300/400 (API bottom creates).

- Step 1 — drag `d` between `a` and `b`: `d.priority` 400 → **150** (midpoint of neighbours). No other sibling's
  priority changed. `modifiedAt` bumped on **`d` and the parent** (sandbox); siblings untouched. List order
  (creation order) unchanged.
- Step 2 — drag `a` to the end (after `c`=300): `a.priority` 100 → **400** (consistent with API bottom
  `ceil((max+100)/100)*100`). Again only the moved node and the parent were bumped; no renumbering.

## A2. Enter to create a sibling mid-list

Setup `a, b, c, d` = 100/200/300/400.

- Step 1 — Enter at end of `b`, type `new-below-b`: new node priority **250** (midpoint), note `null`,
  layout `bullets`, `createdAt` 1 s before `modifiedAt` (typing after creation). **Parent `modifiedAt` NOT bumped**
  (same as API create). No sibling changed. Appended at the end of list order (creation order).
- Step 2 — Enter at the *start* of `c` (creates an empty item above `c`), type `new-above-c`: a **new** node is
  created at priority **275** (midpoint of 250 and 300); `c` keeps its id and is entirely unchanged (not even
  `modifiedAt`). Parent not bumped.
- Step 3 — Enter at end of last item `d` (400), type `new-last`: priority **500** (= API bottom rule). Parent not
  bumped.

## A3. Indent with Tab

Setup `a`(100: `a1`=100, `a2`=200), `b`=200, `c`=300, `d`=400.

- Tab on `b`: same id; `parent_id` → `a`, priority 200 → **300** (= API bottom rule on `a`'s children). `modifiedAt`
  bumped on **`b` and `a` (new parent) only**; the old parent (sandbox) was *not* bumped. Exactly matches the API
  move's bump rule. (List order under `a` became `a1, a2, b` — consistent with creation order, so this does not
  distinguish creation order from arrival order.)

## A4. Outdent with Shift+Tab

- Step 1 — Shift+Tab on `a2` (was `a`'s child, priority 200): lands after `a` in the sandbox, between `a`=100 and
  `c`=300, with priority **200** — ambiguous (midpoint of 100/300 *and* its old value). `modifiedAt` bumped on
  `a2` and the new parent (sandbox) only; old parent `a` not bumped. No siblings rewritten.
  List order under the sandbox: `a, a2, c, d` — confirms list order is global creation order, not arrival order.
- Step 2 — Shift+Tab on `a1` (priority 100) lands between `a`=100 and `a2`=200 with priority **150**: confirms the UI
  assigns the **midpoint of the new neighbours** (not the old value). Bumps: `a1` + new parent only.

**Ordering summary (A1–A4):** every UI placement (drag, Enter, Tab, Shift+Tab) assigns `priority` = integer midpoint of the new
neighbours, or the API bottom rule when placed last; only the placed node and its (new) parent get `modifiedAt` bumps
(creates bump nothing but the node); no sibling is ever rewritten. Ids are always preserved for moves.

## B1. Delete then undo

Setup `a`, `b` (child `b1`), `c`.

- Step 1 — UI delete of `b`: `b` and `b1` both → `404 {"code":"not_found","message":"Item not found"}` (same as an
  API-deleted node). Gone from the parent's list. Parent `modifiedAt` **not** bumped; siblings untouched.
- Step 2 — Ctrl+Z: `b` and `b1` **reappear with the same ids**, same `parent_id`, same priority, same `createdAt`, and
  the same position in list (creation) order. `modifiedAt` is bumped on **every restored node** (`b` and `b1`) to the
  undo time; parent and siblings not bumped.
  **Implication:** a node can 404 and later come back with the same id (soft delete server-side). The sync engine
  must not treat a 404 as permanent tombstoning of an id; the mock should model deletion as soft (and could expose a
  `/_mock/` "UI undo/restore" operation to simulate this).

## B2. Delete then restore from trash

Setup `a`, `b`(`b1`), `c`(`c1`); `c` deleted **via the API** (`DELETE /api/v1/nodes/:id` → 200), then `b` deleted in the UI.

- The web UI has a **Trash** view. **API-deleted nodes appear in it too** — the upstream docs' "Permanently deletes a
  node. This cannot be undone." is false; API delete is the same soft delete as the UI.
- Restoring `b` and `c` from Trash: same ids, same parent, same priorities (200/300), same `createdAt`, original list
  (creation-order) positions; descendants (`b1`, `c1`) restored too. `modifiedAt` bumped on every restored node
  (including descendants); parent not bumped.
- **Mock implication:** delete must be a soft delete (keep the rows, mark deleted, hide from GET/list/export →
  404), with a test-only `/_mock/` restore operation simulating Trash/undo restores (bumping `modifiedAt` of the
  whole restored subtree).

## B4. Cut and paste

Setup `P1` → `x` → `x1`; empty `P2`.

- Attempt 1 (text-level cut of `x`'s text, paste into a new empty child of `P2`): this is a *text* cut/paste, not an
  item move. `x` stayed in place (same id, children intact) with a `modifiedAt` bump (its text was briefly
  edited); the paste produced a **brand-new node** (new id, name `x`, no children) under `P2`, and `P2` was bumped
  (the new empty item was created as a sibling with Enter and then indented with Tab, which is a move into `P2`).
  The stray node was then deleted via the API.
- Attempt 2 (item-level: Shift+Down to select whole items, Ctrl+X, then Ctrl+V into a new empty child of `P2`):
  **cut/paste is a move** — `x` and `x1` keep their ids and `createdAt`; `x.parent_id` → `P2`; `x1` stays under
  `x`. `x.priority` = **50** (= `floor(100/2)`: inserted at the top, above the empty placeholder child at 100, which
  then vanished — the placeholder was presumably deleted; it is not visible in the list).
  `modifiedAt` bumped on `x`, **its descendant `x1`** (unlike the API move, which never bumps descendants) and
  the new parent `P2`; the old parent `P1` was not bumped.
- Step 3 (copy `x` subtree as items, paste into a new empty child of `P1`): a **deep copy with new ids** (`x'`,
  `x1'`), `createdAt == modifiedAt` = paste time. Pasted root gets priority 50 (same placeholder-top pattern);
  the pasted child gets priority **0** and both have **`note: ""`** (not `null`) — the same signature as the API's
  multiline-create children (priorities 0,1,2…, note `""`), suggesting the paste goes through the same
  text-import code path. `P1` bumped (placeholder Tab-indented into it). Originals untouched.

## C1. Parent `modifiedAt` under UI child operations

Setup `X`(`x1`, `x2`), `Y`(`y1`). Snapshots `s0`, `a`…`g`.

| step | UI action | nodes whose `modifiedAt` changed |
|---|---|---|
| a | Enter after `x2`, type `x3` | only the new node (`x3`, priority 300) — **not `X`** |
| b | edit `x1` text | only `x1` |
| c | drag `x2` from `X` into `Y` (below `y1`) | `x2`, `Y` (new parent); not `X` (old parent). `x2` priority 200 (bottom rule after 100) |
| d | drag `y1` from `Y` into `X` (below `x3`) | `y1`, `X`; not `Y`. `y1` priority 400 (bottom rule after 300) |
| e | complete `x1` (Ctrl+Enter) | only `x1`; `completedAt == modifiedAt` |
| f | add note to `x3` | only `x3` (`note` `null` → `"a note"`) |
| g | delete `x3` | nothing (node 404s; parent not bumped) |

**Conclusion:** UI edits follow exactly the same bump table as the API (data-model §8): create, edit, complete, note,
and delete of a child never bump the parent; a move bumps the moved node and the destination parent only.
`modifiedAt` on a parent is therefore *not* a usable "subtree changed" signal.

## C2. Collapse/expand and other view-only actions

Setup `X`(`x1`, `x2`).

- API-created parents appear **collapsed** in the UI by default.
- Expand, collapse, and zoom in/out: **no API-visible change at all** (no `modifiedAt` bumps, no field changes;
  collapse state is not exposed by the API).
- Adding a tag by typing ` #t`: stored as **plain text** in `name` (`'X #t'`, no markup); only `X` bumped.

## D1. Complete a bullet and a todo, then uncomplete

Setup bullet `b`, todo `t` (`layoutMode: "todo"`, via API).

- Ctrl+Enter on `b` and ticking `t`'s checkbox: identical data effect — `completed: true`,
  `completedAt == modifiedAt` = action time; `layoutMode` unchanged. Same as API `/complete`.
- (Accidental, answers D3) uncomplete then re-complete in quick succession: net result `completed: true` with
  `completedAt` **re-stamped** to the re-completion time (and `modifiedAt` equal to it). Same as the API.
- Uncomplete both: `completed: false`, `completedAt: null`, `modifiedAt` bumped. Identical to API `/uncomplete`.

## D2. Completing a parent: is there a cascade?

Setup `P`(`c1`, `c2`(`g1`)).

- Completing `P` in the UI changes **only `P`** (`completed`, `completedAt`, `modifiedAt`). Descendants keep
  `completed: false` and are not bumped. The UI renders `P` struck through and its descendants **greyed out but
  not struck** — i.e. "inside a completed ancestor" is a *display-time* inheritance, not stored data. No cascade,
  same as the API. A sync engine/mock should store per-node completion only.

## E1. Special characters typed in the UI

Sandbox `ui: E` (shared by E1–E8).

| typed in UI | stored `name` |
|---|---|
| `a < b > c & d "q" 'q'` | `a &lt; b &gt; c &amp; d &quot;q&quot; 'q'` |
| `5 > 3` | `5 &gt; 3` |
| `AT&T` | `AT&amp;T` |

**The UI stores `name` as HTML**: it escapes `<`, `>`, `&` and `"` as entities (`'` left raw). Contrast: the API
update path stores raw `<`/`&` verbatim and the create path *decodes* entities. So text written through the API is
not in the UI's canonical form; a sync engine must treat `name` as HTML and escape plain text itself
(`&` → `&amp;`, `<` → `&lt;`, `>` → `&gt;`, `"` → `&quot;`) when writing via update.

## E2. Literal tag-like text

- Typing `<b>not bold</b>` stores `&lt;b&gt;not bold&lt;/b&gt;` — escaped, hence unambiguous vs real markup.

## E3. Formatting shortcuts

| UI format on `WORD` | stored `name` |
|---|---|
| bold (Ctrl+B) | `plain <b>WORD</b> plain` |
| italic (Ctrl+I) | `plain <i>WORD</i> plain` |
| underline (Ctrl+U) | `plain <u>WORD</u> plain` |
| strikethrough | `plain <s>WORD</s> plain` |
| bold then italic | `plain <b><i>WORD</i></b> plain` (bold outermost, properly nested) |
| inline code | `plain <code>CODE</code> plain` |

Tags are bare (no attributes). `<u>` is produced by the UI although the upstream docs do not list it.

## E5. Links

| UI action | stored |
|---|---|
| paste raw URL `https://example.com/a?b=1&c=2` after `see ` | name `see <a href="https://example.com/a?b=1&c=2">https://example.com/a?b=1&amp;c=2</a>` |
| link tool on `here` → `https://example.org/` | name `click <a href="https://example.org/">here</a>` |
| paste raw URL into a note | note `<a href="https://example.net/x">https://example.net/x</a>` |

- Pasted raw URLs are **auto-linked** into `<a href>` in both names and notes.
- `<a>` has only `href` (no `target`/`rel`), double-quoted.
- Quirk: the `href` attribute value keeps a **raw `&`**, while the link text escapes it as `&amp;`.
- **Notes are HTML too** (they get `<a>` markup), not plain text.

## E8. Notes

- Multi-line note typed as `line one`⏎`line two`⏎⏎`has <angle> & bold` (with **bold**) is stored as
  `'line one\nline two\n\nhas &lt;angle&gt; &amp; <b>bold</b>'` — notes use **raw `\n` for line breaks** (no `<br>`),
  and are HTML-escaped with inline tags exactly like names.
- Side observation: the previously edited `click here` node got a `modifiedAt` bump during this step with no change
  to name/note (likely a deferred save when focus left it). **`modifiedAt` can bump without any API-visible content
  change**; sync must compare content, not just timestamps.

## F1/F2. Rendering of API-written HTML and entities

Sandbox `ui: F`: 16 names written via the API **update** path (stored verbatim). Rendering (screenshot by the user):

| node | stored name (after `Fnn `) | UI rendering |
|---|---|---|
| F01 | `<u>underline</u>` | underlined ✔ |
| F02 | `<span class="colored c-red">red span</span>` | red text ✔ (Workflowy's colour markup) |
| F03 | `<script>alert(1)</script>after script` | `alert(1)after script` — tag dropped, inner text kept |
| F04 | `<div>in div</div>` | `in div` — tag dropped |
| F05 | `line<br>break` | **entire bullet blank** (even `F05` missing) |
| F06 | `<img src="x">img` | **entire bullet blank** |
| F07 | `<a href="javascript:alert(1)">js link</a>` | `F07` — the link *and its text* dropped |
| F08 | `<b>unclosed bold` | **entire bullet blank** |
| F09 | `<B>upper</B>` | bold ✔ (case-insensitive) |
| F10 | `<strong>strong</strong>` | bold ✔ |
| F11 | `x<!-- comment -->y` | `xy` |
| F12 | `<mark>mark</mark>` | plain `mark` (no highlight) |
| F13 | `a < b & c > d` (raw) | **entire bullet blank** |
| F14 | `a &amp; b &lt;tag&gt;` | `a & b <tag>` ✔ |
| F15 | `nb&nbsp;sp &#39;q&#39; &quot;dq&quot;` | **entire bullet blank** |
| F16 | `he said "hi"` (raw `"`) | `he said "hi"` ✔ |

**Interpretation:** the UI parses `name` as **XML**, not lenient HTML. Anything not well-formed — void tags without
`/>` (`<br>`, `<img>`), unclosed tags, raw `<` or `&`, or entities undefined in XML (`&nbsp;`; `&#39;` unconfirmed) —
makes the whole name render empty. Well-formed unknown elements are dropped but their text kept (except
`javascript:` links, which are dropped entirely). Hence the UI's own escaping (E1): `&lt; &gt; &amp; &quot;`.
**Sync-engine implication:** anything written to `name` must be well-formed XML using only the UI's tag vocabulary,
or the user will see a blank bullet.

**Step 2 — append ` z` to each in the UI** (the UI re-serialises the name on save):

| node | before | after UI edit |
|---|---|---|
| F01 | `<u>underline</u>` | `<u>underline z</u>` (typing at end extends the trailing format run) |
| F02 | `<span class="colored c-red">red span</span>` | `<span class="colored c-red">red span z</span>` |
| F03 | `<script>alert(1)</script>after script` | `alert(1)after script z` — tag stripped |
| F04 | `<div>in div</div>` | `in div z` |
| F05 | `line<br>break` | `' z'` — **whole name (incl. `F05`) lost** |
| F06 | `<img src="x">img` | `' z'` — **lost** |
| F07 | `<a href="javascript:…">js link</a>` | `F07 z` — link and text lost |
| F08 | `<b>unclosed bold` | `'z'` — **lost** |
| F09 | `<B>upper</B>` | `<b>upper z</b>` — lower-cased |
| F10 | `<strong>strong</strong>` | `<b>strong z</b>` — normalised to `<b>` |
| F11 | `x<!-- comment -->y` | `xy z` |
| F12 | `<mark>mark</mark>` | `mark z` |
| F13 | `a < b & c > d` | `' z'` — **lost** |
| F14 | `a &amp; b &lt;tag&gt;` | unchanged + ` z` |
| F15 | `nb&nbsp;sp &#39;q&#39; &quot;dq&quot;` | `' z'` — **lost** |
| F16 | `he said "hi"` | `he said &quot;hi&quot; z` — `"` escaped |

**Conclusions:**
- A UI edit **canonicalises** the whole name: unknown tags stripped (text kept), `<B>`/`<strong>` → `<b>`,
  comments removed, `"` → `&quot;`. Writing non-canonical (but valid) markup via the API causes the stored name to
  change on the next UI edit — sync churn.
- **Malformed names are destroyed** by any UI edit: the UI sees them as empty, so the saved name is just the newly
  typed text. Writing malformed XML via the API is a **data-loss bug** for a sync engine.
- Canonical form = XML-escaped text (`&amp; &lt; &gt; &quot;`, `'` raw) with tags from
  {`b`, `i`, `u`, `s`, `code`, `a href`, `span class="colored c-…"`, …}, lower-case, properly nested.

## G2. Every layout the UI offers

The UI's layout menu offered (user's labels → stored `data.layoutMode`):

| UI label | `layoutMode` | side effects |
|---|---|---|
| H1 / H2 / H3 | `h1` / `h2` / `h3` | — |
| Paragraph | `p` | — (confirms `p` is a real UI layout, the one the API uses for multiline children) |
| TODO | `todo` | — |
| Number | **`numbered`** | (not in upstream docs) |
| Board | **`board`** | UI auto-creates one empty child (`name ""`, priority 100) |
| Table | **`table`** | UI auto-creates children `Headers` (100) and `Row` (200), each with two empty cells at priorities **0, 1** (multiline-import signature) |
| Quote | `quote-block` | — |
| Divider | **`divider`** | `note` `null` → `""` |

Not offered in this menu: `code-block` (presumably via ```` ``` ```` typing), `bullets` (default), `dashboard`.
`data` never gained keys other than `layoutMode`. Only the node itself was bumped by a layout change; the
auto-created children did not bump their parents (consistent with create semantics).
Real-world values also seen account-wide: `bullet`, `cmnt`, `cmnt-c` (see global-readonly.md).

## H1. How API mirror shells render

Fixture (all via API, sandbox `ui: H1`): origin `O` (`origin O <b>bold</b>`, note `origin note`, children `oc1`, `oc2`);
`D1` → shell `S1` (fresh); `D2` → shell `S2` whose own name/note were then set via API update to
`shell renamed`/`shell note`; `D3` → shell `S3` with an API-created child `shell child (API)`.

- Every shell renders as a **mirror** (diamond bullet) showing the **origin's** name and note — including `S2`,
  whose own API-written name/note are **ignored by the UI**.
- Expanding the mirror in `D3` shows the **origin's children** (`oc1`, `oc2`); the API-created child of the shell
  (`shell child (API)`) is **invisible** in the UI.
- Viewing changed nothing in the API (snapshot diff empty).
- **Implication:** in the UI's model a mirror *is* its origin; the shell record's name/note/children as seen through
  the API are vestigial. Writing to a shell via the API (update, create-under) produces data the user cannot see.
  The sync engine should detect shells (e.g. by the `name=="" && note==""` heuristic + structure) and route edits
  to the origin, never create under a shell.

## H2. Create a mirror in the UI

Setup todo `O` (note `O note`, children `o1`, `o2`) and empty `D`. Alt-drag `O` into `D`.

- The UI creates a shell node under `D`: new id, `name ""`, **`note null`** (API-made shells have `note ""`),
  `layoutMode "bullets"` (does not copy the origin's `todo`), priority 100 (first child), `createdAt == modifiedAt`.
- `modifiedAt` bumped on **`O` (origin)** only — not `D`. Identical to the API's mirror bump rule.
- **Detection heuristic must be `name == "" and note in ("", None)`** (plus: shells have no visible children of
  their own). Still ambiguous with a genuinely empty bullet.

## H3. Edit mirror text vs origin text

On H2's result:
- (a) edit typed into the mirror's **note**: stored on **`O`** (`note` → `O note edited-in-mirror`); shell unchanged.
- (b) edit of the mirror's **name**: stored on **`O`**; shell unchanged (still `name ""`).
- (c) edit of the origin's name: stored on `O`; shell unchanged.

**All UI edits through a mirror are writes to the origin record.** The shell's own fields never change. So via the
API a mirror's content must be read from its origin — but the API exposes no shell→origin link (data-model §10.1),
so a sync engine can't resolve it from the API alone.

## H4. Add a child under a mirror; drag into a mirror

On H2's result (mirror of `O` inside `D`), plus loose node `L`.

- (a) Enter after `o2` inside the expanded mirror, type `new-via-mirror`: created with **`parent_id = O`** (the
  origin), priority 300; nothing bumped except the new node (normal create rule).
- (b) Rename `o1` inside the mirror: writes the real `o1` record.
- (c) Drag `L` into the mirror (below `new-via-mirror`): `L.parent_id` → **`O`**, priority 400, bumps `L` and **`O`**
  (the move's destination parent is the origin); the shell and `D` untouched.

**Conclusion:** in the UI all structure under a mirror is the origin's structure; the shell never gets children.
This matches the API's mirror-*parent* resolution rule for `POST …/mirror` (parent shell → origin) but **contradicts
the API's create/move**, which put nodes under the shell itself (where the UI never shows them).

## H6. Delete the origin of a mirrored node

Fixture (API): `O2` (child `o2c`) at top of sandbox; `E1` → shell `M1`; `E2` → shell `M2` (+ API child `M2child`).
UI: delete `O2`. User sees the mirrored item persist in both `E1` and `E2`.

- **Promotion:** the origin `O2` is **not deleted** — it is *moved* (same id, same `createdAt`, name, children) into
  the place of one of its shells: `O2.parent_id` → `E2`, priority 100 (the shell's priority).
- That shell (`M2`) is deleted (404), **together with its API-created child `M2child`** (invisible in the UI anyway).
- The other shell `M1` remains a shell (now mirroring `O2` at its new location).
- `modifiedAt` bumped: `O2`, `E2` (new parent) **and the sandbox (old parent)** — unlike an ordinary move, which never
  bumps the old parent.
- Which shell is chosen: here `M2`, the most recently created (and last in tree order) of the two; rule unconfirmed.

**Implication:** the API delete leaves orphan shells (data-model §10.4) but the UI promotes a shell. For sync, a UI
"delete" of a mirrored node shows up as: origin moved, one shell vanished.

---

# Summary of P0 UI findings (2026-10-12/13)

1. **Ordering:** every UI placement = integer midpoint of new neighbours (or API bottom rule at the end); no sibling
   renumbering; only the placed node (+ destination parent for moves) bumped. Same id across drag/indent/outdent/cut-paste.
2. **`modifiedAt`:** UI follows the API's bump table exactly; parent `modifiedAt` is not a subtree-change signal; spurious
   bumps without content change happen (E8).
3. **Delete is soft**, for both UI and API deletes: Trash and Undo restore the same ids/priorities/createdAt and bump
   `modifiedAt` on the whole restored subtree. 404 is not permanent.
4. **Copy-paste** creates new ids with the import signature (child priorities 0,1,…; `note ""`).
5. **Completion** is per-node; no cascade (descendants only rendered greyed).
6. **Text is XML/HTML:** the UI escapes `& < > "`, uses `<b> <i> <u> <s> <code> <a href> <span class="colored c-…">`;
   notes are the same format with raw `\n`. The UI parses names as strict XML: malformed names render blank and are
   **destroyed** by the next UI edit; valid-but-noncanonical markup is canonicalised on edit.
7. **Layouts:** `h1 h2 h3 p todo numbered board table quote-block divider` (+ `bullets`, `code-block`); board/table
   auto-create children.
8. **Mirrors:** in the UI a mirror *is* its origin. Shell fields/children written via the API are invisible; all UI
   edits, creates and moves through a mirror target the origin; UI-made shells have `note null`; deleting an origin
   promotes one shell's position (origin moved there, that shell deleted).
