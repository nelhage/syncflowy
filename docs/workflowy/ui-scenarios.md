# UI scenarios: actions for a human in the Workflowy web UI

These are things we **cannot** learn through the API alone. A human performs an action in the web UI, and we
then read the result through the API (`GET /api/v1/nodes/:id`, `GET /api/v1/nodes?parent_id=…`) with
`ProdClient`. This list deduplicates the UI scenarios proposed in every per-area doc. Each one names its sources.

**Ground rules**

- Work only inside the scratchpad (`a846cfce4c6d`). Do not touch "Child A/B/C".
- "Fresh sandbox" means: before the session, Claude creates `sandbox: ui: <scenario id>` under the scratchpad via the API
  together with the listed fixture children, and gives the human a link (`https://workflowy.com/#/<short id>`).
- "Snapshot" means: Claude records `id, name, note, parent_id, priority, completed, completedAt, layoutMode,
  createdAt, modifiedAt` for every node in the sandbox, plus list order, **before and after** the UI action.
  The human waits at least 2 seconds between steps that we need to tell apart by `modifiedAt` (1-second resolution).
- Priority: **P0** blocks a correct sync engine or mock. **P1** is important fidelity. **P2** is a curiosity or a
  check that existing API-made fixtures render sanely.

Existing sandboxes referred to below (all children of the scratchpad):

| short name | id |
|---|---|
| ordering create priorities | `0417219b-648d-4d3a-9025-9d94be99bee3` |
| ordering move params (cycle repaired) | `7f261b8d-2e2b-4a1b-a776-207c6f28b6d1` |
| errors-http cycles (repaired) | `2fad4e15-7f03-4a6b-873e-96b06159ad51` |
| update-complete UI fixtures (U1–U6, L1–L7, C1) | `df1de294-3283-493b-a85d-29da5138c2d4` |
| update-complete 08 misc (mirror of "A") | `4e154634-c6b8-4814-b119-1726462dd495` |
| update-complete 02 modifiedAt | `da468ad9-0454-4d09-a2fa-1bd2576779ce` |
| create 04 markdown in name | `ad5d7893-baae-4d6e-bc37-41105cd98fb3` |
| create 05 markdown followups | `c126b45e-64a3-436d-afc8-74b18fb07c1f` |
| create 08 line-join rule | `dcd920ea-7d5c-4699-89df-8cf39699f825` |
| create 11 layoutMode param | `987b7b0c-05ea-4b67-ac24-0e1e605bf823` |
| create 12 params misc (long names) | `25aced16-b8ae-4ad3-92ee-f12898538af2` |
| text 01 unicode (update path) | `f5e56ba9-6c8a-4d65-93f3-405859111baf` |
| text 02 html tags (create path) | `cf56792e-0c7f-4c7d-b0f8-0d86ac07f1ed` |
| text 02 html tags (update path) | `f6e10a18-3647-451d-9da8-be05b9824bd7` |
| text 06 multiline blocks | `2a52c23c-5d30-4e7b-a33e-e0ac72761fda` |
| text 07 multiline deeper | `be9bca4e-05e5-4ff5-b879-a4eccb0d2dff` |
| errors-http body encodings (layout "nonsense") | `80974c63-b2f3-43d0-8c1c-f9b1380b752d` |
| delete-mirror mirror basics | `44da10e1-e3a5-42a2-a130-f113a6799f13` |
| delete-mirror mirror edits | `1ad462f1-96e8-43b4-9c1d-eb495dd5c32e` |
| delete-mirror mirror identity + deletion | `532a07fa-2a20-477d-86df-0d721ef92956` |
| delete-mirror followups | `2882eb89-25a3-4d38-94be-5335447681ab` |
| followup-0 top into min-0 | `6e1d20bf-de32-43b8-b751-d35525071dc3` |
| followup-1 shell side effects | `2d97a9b5-1623-4fe8-970f-4a1bd793d07d` |
| followup-1 mirror check order (orphan N) | `468bb6aa-5067-4224-98a2-a26cd59333e6` |
| followup-1 bottom formula | `845155fe-8e21-4026-b1ab-275cf67b3bf1` |
| followup-2 move-to-self | `4192a400-d3f2-4006-a8b8-1eb278304db4` |

---

## Summary table

| # | theme | scenario | P |
|---|---|---|---|
| A1 | Ordering | Drag-reorder into the middle | P0 |
| A2 | Ordering | Enter to create a sibling mid-list, above and below | P0 |
| A3 | Ordering | Indent (Tab) | P0 |
| A4 | Ordering | Outdent (Shift+Tab) | P0 |
| A5 | Ordering | Repeated keyboard move until the gap is exhausted | P1 |
| A6 | Ordering | "Move to" dialog | P1 |
| A7 | Ordering | Drag onto another bullet (make child) | P1 |
| A8 | Ordering | Reorder the 0/1/2 multiline children | P1 |
| A9 | Ordering | Make an odd max priority, then API bottom | P1 |
| A10 | Ordering | Duplicate priorities from two tabs | P1 |
| A11 | Ordering | Display order equals ascending priority | P1 |
| A12 | Ordering | Sort children (if it exists) | P2 |
| B1 | Delete/undo | Delete, then undo | P0 |
| B2 | Delete/undo | Delete, then restore from trash | P0 |
| B3 | Delete/undo | Delete the middle sibling | P1 |
| B4 | Delete/undo | Cut and paste | P0 |
| B5 | Structure | Drag into own descendant; inspect repaired cycles | P1 |
| C1 | Timestamps | Parent `modifiedAt` under UI child operations | P0 |
| C2 | Timestamps | Collapse/expand and view-only actions | P0 |
| D1 | Completion | Complete a bullet and a todo, then uncomplete | P0 |
| D2 | Completion | Complete a parent: does it cascade? | P0 |
| D3 | Completion | Uncomplete then re-complete | P2 |
| D4 | Completion | Show/hide completed | P2 |
| D5 | Completion | Completion-leak fixtures | P2 |
| E1 | Text | Type `<`, `>`, `&`, quotes | P0 |
| E2 | Text | Type literal tag text `<b>not bold</b>` | P0 |
| E3 | Text | Formatting shortcuts (bold/italic/underline/strike/code) | P0 |
| E4 | Text | Colour and highlight | P1 |
| E5 | Text | Links: custom text and raw pasted URL | P0 |
| E6 | Text | Dates: picker, with time, range, locale | P1 |
| E7 | Text | Tags and mentions | P1 |
| E8 | Text | Notes: multi-line, formatting, cleared note | P0 |
| E9 | Text | Clear a name entirely; leading/trailing whitespace | P1 |
| E10 | Text | Multi-line paste | P1 |
| E11 | Text | Unicode (emoji/RTL/combining) and long text | P2 |
| F1 | Rendering | API-written HTML tags, then a UI edit | P0 |
| F2 | Rendering | API-written entities and specials, then a UI edit | P0 |
| F3 | Rendering | API-written raw markdown and embedded `\n` in a name | P1 |
| F4 | Rendering | Long names and the large-note node | P2 |
| G1 | Layout | Render the non-standard layoutModes | P1 |
| G2 | Layout | Set every layout the UI offers | P0 |
| G3 | Layout | Markdown shortcuts typed in the UI (`# `, `> `, ```` ``` ````, `- [ ] `) | P1 |
| G4 | Layout | Multi-line code block | P1 |
| H1 | Mirrors | Render API mirror shells | P0 |
| H2 | Mirrors | Create a mirror in the UI | P0 |
| H3 | Mirrors | Edit mirror text vs origin text | P0 |
| H4 | Mirrors | Add a child under a mirror; drag into a mirror | P0 |
| H5 | Mirrors | Complete a mirror / origin | P1 |
| H6 | Mirrors | Delete the origin | P0 |
| H7 | Mirrors | Delete or remove a mirror | P1 |
| H8 | Mirrors | Move a mirror | P1 |
| H9 | Mirrors | Mirror of a mirror | P2 |
| H10 | Mirrors | Recursive mirror / drag into own mirror | P2 |
| H11 | Mirrors | Orphan shells | P1 |
| H12 | Mirrors | Convert mirror to copy | P2 |
| I1 | Ids | Copy-link URL forms | P1 |
| I2 | Ids | Create a shortcut (jump-to) key | P2 |

---

## A. Ordering and priority

The web UI clearly assigns priorities differently: the scratchpad itself has priority 28, which is not in the API's
halving sequence. Through the API the only placements are top and bottom (data-model.md §7). The sync engine must
read and preserve UI-made priorities, and the mock must know what values to expect.

### A1. Drag-reorder into the middle (P0)
- **Setup**: fresh sandbox with children `a, b, c, d` created via API at bottom (100, 200, 300, 400).
- **UI action**: drag `d` between `a` and `b`. Then drag `a` between `c` and `d`.
- **Observe**: `d`'s new priority (midpoint 150? something else?), whether any **other** sibling's priority changed (a shift
  or a full renumber), whose `modifiedAt` changed (moved node, siblings, parent), and list order.
- **Why**: core of the priority model. Decides whether the sync engine can assume "only the moved node changes", and
  what values the mock should accept or display. Sources: M-UI1, C-UI1.

### A2. Enter to create a sibling mid-list (P0)
- **Setup**: as A1 (100/200/300/400).
- **UI action**: put the cursor at the end of `b` and press Enter, then type `new-below-b`. Put the cursor at the start of `c` and press Enter (which creates
  an item above `c`). Also press Enter at the end of the last item.
- **Observe**: priorities of the new nodes and any sibling rewrites. The parent's `modifiedAt` (the API's create does not bump it).
  `createdAt` and list position of the new nodes.
- **Why**: UI-created nodes are the most common input to sync. Sources: M-UI3, C-UI1.

### A3. Indent with Tab (P0)
- **Setup**: fresh sandbox with `a, b, c` (100/200/300), where `a` already has children `a1, a2` (100/200).
- **UI action**: select `b` and press Tab, so it becomes the last child of `a`.
- **Observe**: `b.parent_id` and `b.priority` (`ceil((max+100)/100)*100` = 300 like API bottom, or something else?).
  `modifiedAt` of `b`, `a` (new parent) and the sandbox (old parent). Note that the API's move bumps only the node and the new parent.
- **Why**: indent is a move. The sync engine must recognise it as one (same id), and the mock's move bumps must agree.
  Sources: M-UI4, F1-UI5.

### A4. Outdent with Shift+Tab (P0)
- **Setup**: as A3 before the indent, and also add `d` (400) to the sandbox.
- **UI action**: select `a2` and press Shift+Tab, so it lands between `a` and `b` in the sandbox.
- **Observe**: `a2.priority` (the API cannot put it in the middle), whether `b`, `c`, `d` were rewritten, and the `modifiedAt` of
  `a`, the sandbox and `a2`.
- **Why**: middle insertion into a different parent. Sources: M-UI5, F1-UI5.

### A5. Keyboard move until the gap is exhausted (P1)
- **Setup**: sandbox with `x` (100), `y` (200), `z` (300).
- **UI action**: move `z` up one step (Ctrl/Cmd+Shift+Up) so it sits between `x` and `y`, then move it down and up again. Repeat
  about 10 times. Also repeatedly create new items between the same two neighbours.
- **Observe**: the priority sequence. Is it fractional (non-integer JSON numbers?), and when and how does a renumber happen?
- **Why**: priorities may become non-integers or ties. The mock's column type and the sync engine's comparison depend on this. Source: M-UI2.

### A6. "Move to" dialog (P1)
- **Setup**: two sandbox children `P1` (with `n`) and `P2` (with `q1`, `q2` at 100/200).
- **UI action**: use the UI's move command (if available) to move `n` to `P2`.
- **Observe**: `n.priority` (top 50, bottom 300, or something else?), and bumps of `P1`, `P2` and `n`.
- **Why**: compares UI placement with API top/bottom. Source: M-UI6.

### A7. Drag onto another bullet to make a child (P1)
- **Setup**: `t` (with children 100/200) and `s` as siblings.
- **UI action**: drag `s` onto `t`'s bullet.
- **Observe**: whether `s` goes first or last among `t`'s children, and its priority. Source: M-UI7.

### A8. Reorder multiline-create children (priorities 0, 1, 2) (P1)
- **Setup**: create via API `name: "P\n\n- k0\n\n- k1\n\n- k2"`, which gives children at 0, 1, 2.
- **UI action**: first confirm the display order k0, k1, k2. Then drag `k2` between `k0` and `k1`.
- **Observe**: whether the UI renumbers 0/1/2, and what priority `k2` gets (between 0 and 1 there is no integer).
- **Why**: integer exhaustion right away, and these unusual priorities exist in production data. Sources: C-UI3.

### A9. Produce an odd max priority in the UI, then API bottom (P1)
- **Setup**: sandbox with children 100, 200, 300 via API.
- **UI action**: reorder (drag the first between the others, drag one to the end, press Enter after the last item) until the API shows
  a **largest** priority that is not a multiple of 100 (e.g. 350). Stop and tell Claude.
- **Observe**: Claude then sends API `create` and `move` with `position: bottom`, and checks for `ceil((max+100)/100)*100` (350 → 500).
- **Why**: confirms the bottom formula on UI-made values. Sources: F0-U1, M-UI15.

### A10. Duplicate priorities from concurrent UI edits (P1)
- **Setup**: sandbox with 5 children.
- **UI action**: open the sandbox in two browser tabs, or take one tab offline. Reorder the same items differently in each, then reconnect.
- **Observe**: any equal priorities. If there are ties, Claude top-inserts into a min-0 version to see the renumber's tie order.
- **Why**: the sync engine needs a tie-break rule, and the mock's renumber tie-break is unknown. Sources: F0-U2, M-UI16.

### A11. Displayed order equals ascending priority (P1)
- **Setup**: existing sandboxes `ordering create priorities` (list order differs heavily from priority order), `followup-0 top into
  min-0` (mirror shells renumbered), and `followup-1 bottom formula` (150/300, 250/400 pairs).
- **UI action**: just look.
- **Observe**: the UI order matches ascending `priority`, not list or creation order.
- **Why**: confirms that `priority` is the only ordering key. Sources: M-UI14, F0-U3, F1-UI7.

### A12. Sort children (P2)
- **Setup**: sandbox with `c, a, b`.
- **UI action**: sort alphabetically, if the UI has such a command.
- **Observe**: rewritten priorities and `modifiedAt` values. Source: M-UI9.

---

## B. Delete, undo, cut/paste, structure

### B1. Delete then undo (P0)
- **Setup**: sandbox with `a, b, c`, where `b` has child `b1`.
- **UI action**: delete `b` (with its child) and wait 3 s, **Claude reads** `b`, then the human presses Ctrl/Cmd+Z.
- **Observe**: GET `b` while deleted (404?), then after the undo: the same UUID or a new one? Priority, `createdAt`, `modifiedAt`, list position
  (creation order) and `b1` all restored?
- **Why**: if undo resurrects ids, the sync engine must handle "404 then reappears", and the mock may need soft delete.
  Sources: M-UI10, D-U11, E-UI7.

### B2. Delete then restore from trash (P0)
- **Setup**: as B1.
- **UI action**: delete `b`, then restore it from the trash or "recently deleted" view (if one exists). Also try restoring a node that was
  **deleted via the API** (Claude deletes `c` first).
- **Observe**: as B1. Also whether API-deleted nodes appear in the UI trash at all.
- **Why**: the API delete is documented as permanent. Sources: D-U12, D open question 5.

### B3. Delete the middle of three siblings (P1)
- **Setup**: `a, b, c` (100/200/300).
- **UI action**: delete `b`.
- **Observe**: whether `a` and `c` keep 100/300, and the parent's `modifiedAt` (API delete bumps nothing). Source: D-U13.

### B4. Cut and paste (P0)
- **Setup**: `P1` with `x` (which has child `x1`), and `P2` empty.
- **UI action**: cut `x` from `P1` and paste it into `P2`. Separately, copy and paste `x` (a duplicate).
- **Observe**: is the cut-paste a move (same id) or a delete+create (new ids, for `x1` too)? Priorities and timestamps.
- **Why**: the sync engine sees either a move or a delete+create. Source: M-UI11.

### B5. Cycles: the UI's guard and the repaired API cycles (P1)
- **Setup**: existing sandboxes `ordering move params` (P/p1/p1-child), `errors-http cycles` (X/X1/X2) and `followup-2
  move-to-self` (Y0/Y1). Plus a fresh `Q` with child `Q1`.
- **UI action**: (a) look at the existing sandboxes and confirm nothing is duplicated, missing or odd. (b) Try to drag `Q` into
  `Q1`, and onto itself.
- **Observe**: (a) render sanity. (b) Does the UI refuse? Any API-visible change?
- **Why**: the mock's cycle policy (reject vs faithful) and confirmation that past API corruption left no damage.
  Sources: M-UI8, M-UI13, E-UI2, F2-UI2/3.

---

## C. Timestamps

### C1. Parent `modifiedAt` under UI child operations (P0)
- **Setup**: sandbox with `X` holding children `x1`, `x2`, and a sibling `Y` with child `y1`.
- **UI action**, waiting at least 2 s between steps so that Claude can snapshot each one: (a) add a child to `X`; (b) edit `x1`'s text; (c) drag `x2` out
  of `X` into `Y`; (d) drag `y1` into `X`; (e) complete `x1`; (f) add a note to `x1`; (g) delete `x1`.
- **Observe**: `X`'s and `Y`'s `modifiedAt` after each step. Compare with the API table: create, update, complete and delete of a child do **not**
  bump the parent, and move-in does.
- **Why**: the sync engine may use `modifiedAt` for change detection, so it needs to know which UI edits are visible where. Sources: U-UI16, F1-UI5.

### C2. Collapse/expand and other view-only actions (P0)
- **Setup**: `X` with children.
- **UI action**: collapse `X`, expand it, zoom into it, and add a tag `#t` to `X`'s name.
- **Observe**: `modifiedAt` of `X` and its children after each step, and the priorities of the children. Is the collapse state exposed anywhere (it is not
  in `data`)?
- **Why**: if collapse bumps `modifiedAt`, change detection will see noise. Sources: M-UI12, U-UI16(e,f).

---

## D. Completion

### D1. Complete a bullet and a todo, then uncomplete (P0)
- **Setup**: `b` (bullets) and `t` (layoutMode `todo`) created via the API.
- **UI action**: complete `b` (Ctrl/Cmd+Enter) and tick `t`'s checkbox. Then uncomplete both.
- **Observe**: `completed`, `completedAt`, `modifiedAt` and `layoutMode` after each step. Compare with the API `/complete` and with a `- [x]`-created
  node.
- **Why**: confirms that UI completion maps to the same fields. Sources: U-UI10, C-UI9.

### D2. Completing a parent: is there a cascade? (P0)
- **Setup**: `P` with children `c1`, `c2` (not completed). Also view the existing fixture C1 in `update-complete UI fixtures`
  (a completed parent with an uncompleted child).
- **UI action**: complete `P` in the UI. Look at how C1's child is displayed.
- **Observe**: the children's `completed` flags and `modifiedAt` (the API never cascades).
- **Why**: if the UI cascades in the data, the sync engine and the mock must model it. Source: U-UI9.

### D3. Uncomplete then re-complete quickly (P2)
- **UI action**: on a completed node, uncomplete and then re-complete it.
- **Observe**: whether `completedAt` is re-stamped (the API re-stamps it). Source: U-UI12.

### D4. Show/hide completed (P2)
- **UI action**: toggle "show completed". Complete a node while completed items are hidden.
- **Observe**: no API change. The list still returns the hidden node. Sources: U-UI11, E-UI8.

### D5. Completion-leak fixtures (P2)
- **Setup**: existing `create 11 layoutMode param`, node `p` with children a/B/C (B and C completed via the leak), and the `- [x] done` node
  in `text 06 multiline blocks`.
- **UI action**: confirm they display as completed, then uncomplete `B`.
- **Observe**: `completed`, `completedAt`, `modifiedAt` and `name`. Sources: C-UI4, T-UI16.

---

## E. Text typed in the UI (how the UI encodes `name` and `note`)

The API stores whatever it is sent, and the create path converts markdown. What the **UI** writes decides what
the sync engine will read, and whether `name` is HTML or "text with a few tags".

### E1. Special characters (P0)
- **UI action**: in a fresh sandbox, create bullets typed as `a < b > c & d "q" 'q'`, `5 > 3`, `AT&T`.
- **Observe**: raw `<` `&`, or `&lt;` `&amp;`?
- **Why**: the single most important encoding question for a round-trip-safe sync engine. Sources: U-UI2, T-UI1.

### E2. Literal tag-like text (P0)
- **UI action**: type `<b>not bold</b>` literally.
- **Observe**: how it is stored, and whether it can be told apart from real bold. Source: T-UI2.

### E3. Formatting shortcuts (P0)
- **UI action**: on separate nodes, apply bold, italic, underline, strikethrough and inline code to part of the text with keyboard shortcuts or the toolbar.
  Apply bold and italic together to one word.
- **Observe**: the exact tags (`<b>` vs `<strong>`, `<u>`?, nesting order, attributes). Sources: U-UI7, T-UI3.

### E4. Colour and highlight (P1)
- **UI action**: apply a text colour and a highlight to part of a name.
- **Observe**: the representation (`<span class="colored c-red">`?). Sources: U-UI4, T-UI4.

### E5. Links (P0)
- **UI action**: paste a raw URL into a name. Create a link with custom text through the UI link tool. Do the same in a note.
- **Observe**: whether a raw URL is wrapped in `<a>` in storage, and the `<a>` attributes (`target`? quoting?). Source: T-UI5.

### E6. Dates (P1)
- **UI action**: insert a date with the date picker, a date with a time, and a date range (if supported). If the account has a
  24-hour or date-format preference, change it and check the stored names again.
- **Observe**: the `<time>` attributes (any `end*`?) and the text, compared with the API's `[2026-01-15]` conversion. Sources: T-UI6, C-UI2.

### E7. Tags and mentions (P1)
- **UI action**: type `#urgent`, `@alice` and `#tag-with_dash`.
- **Observe**: plain text or markup in `name`? Source: T-UI7.

### E8. Notes (P0)
- **UI action**: (a) add a note to a node that had none. (b) Type a multi-line note (Shift+Enter) with a blank line, bold text and a link.
  (c) Delete all the text of an existing note.
- **Observe**: (a) `null` → string, and `modifiedAt`. (b) Newlines encoded as `\n` or `<br>`, and the formatting tags. (c) `null` or `""`?
- **Why**: the API cannot set `note` back to `null`, so it matters whether the UI can. Sources: C-UI7, U-UI6, U-UI14, T-UI10.

### E9. Empty names and whitespace (P1)
- **UI action**: clear a node's name entirely, leaving the bullet. Type a name with leading and trailing spaces and an NBSP.
- **Observe**: `""`? Is the text trimmed (the API create path trims, update does not)? Sources: U-UI15, T-UI13.

### E10. Multi-line paste (P1)
- **UI action**: paste `parent\n\nchild1\n\nchild2`, `a\nb\nc` and `Hello.\nworld` (as plain text) into a bullet.
- **Observe**: the node structure (siblings? children? joined?) compared with the API's create join rule (data-model.md §6.2.5).
- **Why**: a mock is only expected to copy the API, but the sync engine should know whether the UI produces different
  structures. Sources: C-UI6, T-UI11.

### E11. Unicode and long text (P2)
- **UI action**: type emoji (ZWJ sequences), RTL text and combining accents. Paste about 100k characters into a note.
- **Observe**: exact preservation, and any truncation (the API accepts 1M). Sources: T-UI14, T-UI15.

---

## F. Rendering of API-written content (and whether a UI edit rewrites it)

For each: look first, then **make a one-character edit and undo it** (or add and remove a space), then Claude re-reads it.
This shows whether the UI normalises API-stored strings on save, which would cause sync churn.

### F1. HTML tags (P0)
- **Setup**: `text 02 html tags` (create and update paths), and U4/U5 in `update-complete UI fixtures`, and
  `<script>alert(1)</script>` in `create 04 markdown in name`.
- **Observe**: how `<u>`, `<span>`, `<script>`, `<div>`, `<br>`, `<img>`, `javascript:` hrefs, unclosed `<b>`, `<B>`, `<strong>`,
  comments and `<mark>` render, and whether an edit sanitises, balances or escapes them. Sources: U-UI4/5, T-UI8, C-UI8.

### F2. Entities and raw specials (P0)
- **Setup**: U1 `a < b & c > d` and U2 `a &amp; b &lt;tag&gt;` in `update-complete UI fixtures`. The `&lt;b&gt;` node in `create 05
  markdown followups`. `text 01 unicode (update path)`, which includes control characters.
- **Observe**: rendered literally or decoded? Does an edit re-encode them? Sources: U-UI1, T-UI9, C-UI8.

### F3. Raw markdown and embedded newlines in names (P1)
- **Setup**: U3 (`**md bold** [md link](…)`) and U6 (`line1\nline2`) in `update-complete UI fixtures`. The `py\nx = 1` code block in
  `text 07 multiline deeper`.
- **Observe**: rendered literally? Does an edit convert markdown or drop or keep the `\n`? Sources: U-UI3, U-UI6, T-UI12.

### F4. Long names and the large-note node (P2)
- **Setup**: the 10 000-char and 100 KB names in `create 12 params misc`, and A1 in `errors-http misc errors` (its note was briefly 1.1 MB).
- **Observe**: UI truncation, slowness, and whether viewing them changes anything. Sources: C-UI10, E-UI5.

---

## G. Layout modes

### G1. Non-standard layoutMode values (P1)
- **Setup**: L1–L7 in `update-complete UI fixtures` (`board`, `p`, `table`, `dashboard`, `nonsense`, `""`, `H1`).
  `create 11 layoutMode param` (`p`, `board`, `dashboard`, `H1`, `quote-block`). The `nonsense` node in `errors-http body encodings`.
  The `p` children and `<b> H4</b>` in `text 06/07`.
- **UI action**: look. Then change one of each to a plain bullet in the UI.
- **Observe**: the rendering, and the stored value after the UI change. Sources: U-UI8, C-UI5, E-UI4, T-UI12.

### G2. Set every layout the UI offers (P0)
- **Setup**: a sandbox with about 10 plain bullets.
- **UI action**: set each node to a different layout from the UI menu: bullets, todo, H1, H2, H3, paragraph, board, code block,
  quote block, table, dashboard, and anything else offered.
- **Observe**: `data.layoutMode` for each, which gives the **real enum**. Also whether `data` ever gains other keys.
- **Why**: the mock must know which values the UI produces. The API accepts any string. Source: U-UI8.

### G3. Markdown shortcuts typed in the UI (P1)
- **UI action**: in new bullets, type `# `, `## `, `### `, `- [ ] `, `> `, ```` ``` ````, `**bold**` and `[2026-01-15]`.
- **Observe**: the stored `name` and `layoutMode`. Does `> ` produce `quote-block` (the API create path does not)? Does the UI ever
  produce `p`? Source: C-UI2.

### G4. Multi-line code block (P1)
- **UI action**: create a code block with 3 lines in the UI.
- **Observe**: a single node with `\n` in `name`, or several nodes? Source: T-UI12.

---

## H. Mirrors

Through the API a mirror is an independent, empty "shell" node, and edits never propagate (data-model.md §10). These scenarios
establish what the UI does, which is essential for deciding how the sync engine represents mirrors.

### H1. How API mirror shells render (P0)
- **Setup**: `delete-mirror mirror basics` (3 shells of "O origin **bold**" plus a shell of oc1). `delete-mirror mirror edits` (shell M whose
  API name, note and layout were set directly, with 3 API-created children). `update-complete 08 misc` (a mirror of "A" under
  "trailing slash", renamed via the API). `followup-1 shell side effects` (shell renamed "shell renamed", completed, with child L).
- **UI action**: look.
- **Observe**: does each shell render the origin's content and children, or its own API-written fields? Are the API-created
  children of a shell visible, and where? Re-GET the shells afterwards to confirm nothing changed.
- **Why**: decides whether shell fields are meaningful at all. Sources: D-U1, D-U4, U-UI13, F1-UI8.

### H2. Create a mirror in the UI (P0)
- **Setup**: `O` (name, note, layout `todo`, 2 children) and an empty `D`.
- **UI action**: mirror `O` into `D` (Alt-drag or the Mirror command).
- **Observe**: the new node's `name`/`note` (`""` like the API, or a copy of the origin's text?), layoutMode, priority, and whether `O`'s and `D`'s
  `modifiedAt` bumped (the API bumps the origin only). Source: D-U2.

### H3. Edit mirror text vs origin text (P0)
- **Setup**: H2's result.
- **UI action**: (a) edit the **mirror's** text. (b) Wait, then edit the **origin's** text. (c) Edit the note in the mirror.
- **Observe**: which record's `name`/`note`/`modifiedAt` changes after each: origin, shell, or both? Sources: D-U3, U-UI13, F1-UI2.

### H4. Add a child under a mirror, and drag into a mirror (P0)
- **Setup**: H2's result, plus a loose node `L`.
- **UI action**: add a child under the mirror. Rename an existing child of the mirror. Drag `L` into the mirror.
- **Observe**: `parent_id` of the new child and of `L` (shell or origin?), and whether the origin's `modifiedAt` changed.
  Sources: D-U5, D-U18, F1-UI1, F1-UI4.

### H5. Complete a mirror, then the origin (P1)
- **UI action**: complete the mirror. Then uncomplete it and complete the origin.
- **Observe**: `completed`/`completedAt` on both records. Source: D-U6.

### H6. Delete the origin of a mirrored node (P0)
- **Setup**: `O2` with 2 mirrors in different parents, one of which has an API-created child.
- **UI action**: delete `O2`.
- **Observe**: do the shells survive as orphans (as with the API delete), or is one **promoted** (name filled in, id reused)?
- **Why**: the orphan semantics in the mock. Source: D-U7.

### H7. Delete or remove a mirror (P1)
- **UI action**: on one mirror, press Delete. On another mirror of the same origin, use "remove mirror" (if it exists).
- **Observe**: the shell gives 404, the origin is intact, and whether the origin's `modifiedAt` bumps in each case (the API bumps it only for
  `DELETE /mirror`). Sources: D-U8, D-U20, F1-UI3.

### H8. Move a mirror (P1)
- **UI action**: drag a mirror to another parent.
- **Observe**: the shell's `parent_id` and `priority`, and whether the origin is bumped. Source: D-U17.

### H9. Mirror of a mirror (P2)
- **UI action**: mirror an existing mirror.
- **Observe**: the new shell's name, and that it relates to the true origin (behaviour of a later API `POST /mirror` on it). Source: D-U15.

### H10. Recursive mirrors and dragging into one's own mirror (P2)
- **Setup**: `delete-mirror mirror identity + deletion` and `delete-mirror followups` (mirrors of O inside O's descendants).
- **UI action**: look. In a fresh sandbox, try to mirror `R` into its own child, and try to drag `R` into its own mirror.
- **Observe**: whether the UI renders recursion safely, and whether the UI allows these (the API does). Sources: D-U10, D-U16, E-UI3.

### H11. Orphan shells (P1)
- **Setup**: N1 in `delete-mirror mirror identity + deletion`. "orphan mirror renamed" in `delete-mirror followups`. Orphan N with
  child nc in `followup-1 mirror check order` (D there holds a mirror of nc).
- **UI action**: look, and try to open, edit and expand them.
- **Observe**: rendering, and whether the UI repairs or converts them (re-GET afterwards). Sources: D-U9, D-U19, F1-UI6.

### H12. Convert a mirror to a copy (P2)
- **UI action**: use "convert to copy" or "unmirror", if such a command exists.
- **Observe**: does the shell keep its id and gain the origin's name, and do the copied children get new ids? Source: D-U14.

---

## I. Ids, links, shortcuts

### I1. Copy-link URL forms (P1)
- **UI action**: copy the link of a node (bullet menu → Copy link), both at top level and while zoomed into a deep node. Paste the strings to Claude.
- **Observe**: the exact URL form (host, `#/<short>`, nested `#/a/b`?). Claude passes each string as `parent_id` to list.
- **Why**: confirms that the URL resolver covers real UI links. Sources: E-UI1, F2-UI1.

### I2. Shortcut (jump-to) key (P2)
- **UI action**: define a shortcut key (e.g. `zq`) pointing at a fresh sandbox node. Later, delete it.
- **Observe**: Claude tests `parent_id: "zq"` for list, create and move while it exists (this needs a scope-guard exception for that exact key), and the
  not-found message after deletion. `GET /targets` would also be informative, but is out of scope unless the coordinator allows it.
- **Why**: shortcut keys are part of the `parent_id` resolver we could not test. Source: E-UI6.

---

## Source key

`C-UIn` create.md §9 · `U-UIn` update-and-complete.md · `M-UIn` ordering-and-move.md · `D-Un` delete-and-mirrors.md §9 ·
`E-UIn` errors-and-http.md §13 · `T-UIn` text-and-content.md · `F0-Un`/`F1-UIn`/`F2-UIn` followup-0/1/2.
