# Open questions

These are the unknowns that remain after the API investigation of 2026-10-12. Each item says why it is open and how it could be
resolved. **UI** means a scenario in [ui-scenarios.md](ui-scenarios.md). **Scope** means we could not test it because the
scope rules confine every request to the scratchpad. **Prod** means a few more API requests would settle it.
"Mock default" is what [data-model.md](data-model.md) does until the question is answered.

## 1. Blocked by the scope restriction

Read-only GETs outside the scratchpad were authorised on 2026-10-12 and are written up in [global-readonly.md](global-readonly.md).
Writes outside the scratchpad remain out of scope, so materialisation is still unobserved.

| # | question | mock default |
|---|---|---|
| S1 | What do `parent_id` **omitted, `null` or `""`** resolve to on **create** and **move**? All three pass validation (followup-2). **Partly answered (list only):** omitted → the root, `""` → `Location '' not found` (global-readonly §1). Create and move presumably share the resolver, but that is unobserved. A sync engine must always send `parent_id` on move. | omitted/null → root, `""` → Location |
| S2 | Special keys. **Mostly answered (read-only):** `None` = the root; calendar names `📆 Calendar` / `2026` / `07` / `<time …>Sun, Jul 5, 2026</time>`, `bullets`, `note: null`, UUIDv5 ids, chronological priorities; a not-yet-created key → `Location '<key>' not found` on list and `Item not found` on GET; a never-materialised `inbox` lists as `[]`; impossible dates and year 9999 → 500 on list; the first week of year 1 → 500 (global-readonly §2). **GET `/nodes/:key` also resolves existing keys.** Still open: what create and move materialise (the API path, the inbox name and position), the priority of a node moved there, the timezone of `today`, and the resolver's relative order (unobservable, since no key is URL- or short-id-shaped). | as observed; materialisation per UI calendar |
| S3 | **User shortcut keys** as `parent_id` and on GET: the resolution and the not-found message after the key is deleted, and their position in `/targets`. The account has **no shortcuts** (global-readonly §3), so a human would have to create one (UI I2). | lookup in a targets table, listed first |
| S4 | ~~`GET /nodes` with no `parent_id`~~ **Answered: the root**, the same as `parent_id=None` (global-readonly §1). | root |
| S5 | **`GET /nodes-export`**. **Mostly answered** (global-readonly §4): `{"nodes": [...]}` with records identical to GET; no mirror metadata or collapse state; shells look like plain nodes; completed included; the order is unspecified (not creation, priority, id or tree order) but stable. **The 1 req/min limit was not enforced** (200s 10 s apart, no rate-limit headers). Still open: the 429 body and `Retry-After` if a limit exists, whether detached cycles are exported (none existed), and whether responses are cached. | creation order; rate limit off by default |
| S6 | ~~**`GET /targets`**~~ **Answered** (global-readonly §3): system targets `inbox` (null, never materialised), `today` "Today", `tomorrow` "Tomorrow", `next_week` "Next Week" (fixed labels, although no day node exists), `calendar` "📆 Calendar", in that order. Shortcut entries: see S3. | as observed |
| S7 | Do mutating endpoints accept special or shortcut keys as the path `:id`? **GET does** for existing nodes (global-readonly §2), which makes this less certain. It cannot be tested read-only. | no |
| S8 | ~~Uppercase or dashless UUIDs of an **existing** node on delete, move, complete, uncomplete and mirror.~~ **Answered: 404** (`Item not found`, nothing changed) on update, complete, uncomplete, move, mirror POST/DELETE (conformance golden `ids_mutations_need_exact_uuid`) and plain DELETE (`delete_by_short_id_is_noop`). | 404 |

## 2. Needs a human in the UI

| # | question | scenario |
|---|---|---|
| H1 | How does the UI assign priorities (midpoint, shift, renumber, fractional values)? Can it make duplicates? Evidence for the **integer midpoint**: the scratchpad got 28 between 12 and 44, and a calendar day got 550 between 500 and 600 (global-readonly §1, §2.1). | A1–A5, A10 |
| H2 | How does the UI encode `<`, `>`, `&` and formatting in `name` and `note`? Is `name` HTML? | E1–E5, E8 |
| H3 | Does the UI rewrite or sanitise API-written HTML, entities or markdown on edit (sync churn)? | F1–F3 |
| H4 | What does the UI show for a mirror shell (origin content or the shell's own fields)? Where do UI-created children of a mirror go? Which record does an edit through a mirror change? | H1–H4 |
| H5 | Does deleting an origin in the UI promote a mirror, or does it leave orphans? | H6 |
| H6 | Can undo or trash restore deleted nodes with the **same ids**? This includes API-deleted nodes. | B1, B2 |
| H7 | Does UI completion cascade to descendants in the data? | D2 |
| H8 | Which UI actions bump which `modifiedAt` (child ops, collapse, tags)? | C1, C2 |
| H9 | What is the real set of `layoutMode` values? How do unknown values render? The user's own (non-scratchpad) data contains only `bullets`, `bullet`, `p`, `cmnt` and `cmnt-c` (global-readonly §5). | G1, G2 |
| H10 | Is cut/paste a move or a delete+create? | B4 |
| H11 | Date `<time>` format with ranges and locale/24h settings. | E6 |
| H12 | Can a note be returned to `null` from the UI? | E8 |
| H13 | Does the UI refuse a move into a descendant? Did our briefly-existing API cycles leave any damage? | B5 |
| H14 | URL forms produced by "Copy link". | I1 |

## 3. Could be settled with more API requests (in scope)

| # | question | notes |
|---|---|---|
| P1 | ~~Date regex edges~~ **Answered** (golden `text_dates_edges`): minutes are 1–2 digits (`9:5` → `9:05 AM`), `\d` is Unicode (Arabic-Indic/full-width digits convert), 1–3 and 5 digit years and `0000` stay literal, `0001` converts. Still open: whether the text depends on the account's timezone/locale (H11). | data-model §6.2 step 11 |
| P2 | ~~Create-path entity/fence details~~ **Answered** (golden `text_entities_structural`): entities are decoded inside fences, fence content gets no markdown (also single-line ```` ```**x**``` ````), and `&#10;`/`&#13;`/`&#35; `/`&#45; [x]` act as line breaks and prefixes. | data-model §6.2 step 2 |
| P3 | ~~Where does `note` go for a multi-block create?~~ **Answered: block 0**; children get `""` (golden `text_multiline_note_and_layout_param`). | |
| P4 | ~~Is `layoutMode` on update also control-character stripped?~~ **Answered: no**, it is stored verbatim. | conformance golden `update_layoutmode_control_chars` |
| P5 | ~~Huge garbage body on complete/uncomplete?~~ **Answered: ignored, 200** (50 KB, golden `complete_huge_garbage_body`). | |
| P6 | ~~URL `parent_id` boundary~~ **Answered** (golden `ids_url_parent_boundaries`): only end, `/` or `?` may follow the 12 hex digits; `-`, `#`, `.`, `g`, `_`, space, `&` and a hex digit give `Location '…' not found`. | data-model §3.2 |
| P7 | Do PUT and DELETE on `/move` give 405 HTML like GET does? | DELETE, HEAD, OPTIONS: **yes** (405 HTML, `Allow: POST`, also on `/complete` and `/uncomplete`; conformance golden `http_405_html_routes`). PUT and PATCH also **yes** (golden `move_wrong_methods`). |
| P8 | Is the NUL-in-create-note **500** stable, or a bug? Re-check occasionally. The same 500 now also covers NUL in `layoutMode` (create and update) and lone surrogates in the create note and `layoutMode` (goldens `create_layoutmode_controls_nul_and_surrogates`, `update_nul_layoutmode_and_lone_surrogates`). | mock copies it |
| P9 | Request body limit above 20 MB, and maximum name length. | mock: no limit |
| P10 | Does a renumber also happen at large bottom priorities (overflow)? We never went above about 1600. Costly to test via the API, because each bottom insert is +100. | none |

## 4. Not resolvable by black-box testing, or design decisions

| # | item | decision / note |
|---|---|---|
| D1 | **Cycle policy**: production accepts a move into a descendant (200) and corrupts the tree. | Mock rejects by default with a mock-specific 400 and offers a `faithful_cycles` flag (data-model §4.7). **Decided 2026-10-12 (user): keep rejection as the default**; `faithful_cycles` remains available. |
| D2 | Why mirror creation and child creation do not bump the destination parent's `modifiedAt` while move-in does. | Copy the observed table. |
| D3 | Short-id collisions (two nodes with the same last 12 hex digits). | Cannot be constructed. Mock: first match by `create_seq`. |
| D4 | Narrow races (more than 4 concurrent writes) producing duplicate priorities, and the renumber's tie order. | Mock serialises writes. Tie-break `(priority, create_seq)`. |
| D5 | Whether the API's mirror divergence is intended or a bug that may change. | **Partly answered (2026-10-13):** it is being fixed. The beta API (`beta.workflowy.com`) has mirror-aware semantics, observed in data-model §10.5 (questions R1–R11). Production still diverges. Mock: production by default, beta behind `beta_mirrors`. The sync engine follows mirrors when the export carries mirror metadata (sync-design §15.1) and keeps production's read-only placeholders otherwise. |

## 5. Beta API mirrors

The first draft of data-model §10.5 was written from the beta reference alone. On 2026-10-13 the beta host was probed through
`ProdClient(base_url=BETA_URL)` inside the scratchpad (`experiments/beta-mirrors/`, log tag `beta-mirrors`). That settled most of
these questions, and several of the draft's guesses were wrong (struck answers below).

| # | question | answer / mock default (`beta_mirrors`) |
|---|---|---|
| R1 | Does production still behave as observed on 2026-10-12? | **Yes** on 2026-10-13: raw shell rows, no metadata, and origin deletes allowed. When it will switch is unknown. |
| R2 | Shape details | **Answered:** `data: {layoutMode, mirror}`, a list of a shell is `{"nodes", "mirror"}`, `mirror_ids` is **sorted by id**, and the key is absent once no mirrors remain. |
| R3 | Listing a shell with persisted children | **Answered:** they are **not listed** (only the origin's children). An orphan lists as `{"nodes": []}` without the `mirror` key. |
| R4 | `modifiedAt` | **Answered:** writes through a shell bump the origin, not the shell row. Create through a shell bumps nothing. Mirror create and `DELETE /mirror` bump the origin, as in production. **New:** a plain delete of a shell (or of a subtree with shells) bumps the origin. A shell reports the origin's `modifiedAt`. |
| R5 | Writes through an orphan | **Answered:** update, complete and uncomplete give `404 Item not found`, and create under it gives `404 Parent item not found`. Move, move-into and delete work. |
| R6 | The delete-blocked error | **Answered:** `400 invalid_request` "Cannot delete a node that has mirrors", and nothing is bumped. Mirrors inside the subtree block too, as does a mirrored persisted child of a shell. The check reads the origin's mirror **registry** (R10). `DELETE /mirror` skips the check and can orphan. |
| R7 | Mirror-relative GET; short ids and URLs of a shell | No mirror-relative path was found. Short ids and URLs of a shell resolve as before, and on create they redirect to the origin. |
| R8 | Do the `mirror` endpoints change? | **No** (self check, redirects, orphans as source or parent). |
| R9 | Move into a shell | It bumps the shell row and the node becomes a hidden persisted child. A recursive case was not tried. |
| R10 | `mirror_ids` is a registry on the origin. Can an origin whose registry misses a live shell be deleted? What else desynchronises it? | **Answered** (04–06). Yes: the H6 origin was deleted on beta, which orphaned its shell. A **production plain delete** of a shell (or its parent) leaves a **ghost** entry, which blocks every beta delete of the origin and cannot be cleared on beta (`DELETE /<ghost>/mirror` → 404). Only a production delete of the origin gets past it. `DELETE /mirror` on either host and a beta plain delete clean up. The mock models the registry (`mirror_registry`). Still open: which UI operations leave ghosts (3 ghosts in the user's own data have no API cause). |
| R11 | `nodes-export` on beta | **Answered** (04): every record equals the beta GET record. Origin children appear once, and a shell's hidden persisted children are included under the shell. Both hosts export the same ids. |
| R12 | **New.** Does `DELETE /mirror` (either host) unregister the shells *nested* inside the removed shell's subtree, or leave ghosts? Does a UI delete of a shell unregister it? Does restoring a deleted shell from the Trash re-register it? | Mock: beta drops nested shells and production only the target; the UI delete unregisters; restore does nothing. |

## 6. Process notes for the coordinator

- `prod_client.AllowList.contains` is case-sensitive. The create agent used `unscoped=True` once, to send an uppercase short id
  of its own sandbox. Suggestion: normalise case (and dashes) in `contains`, which would also unlock S8.
- The guard cannot parse repeated query parameters (list-of-tuples params). The errors-http agent used `unscoped=True` once
  with two of our own sandbox ids.
- Two production cycles were created during testing (one by accident) and repaired within about 30 s. The UI should be
  checked (scenario B5).
