# Open questions

These are the unknowns that remain after the API investigation of 2026-10-12. Each item says why it is open and how it could be
resolved. **UI** means a scenario in [ui-scenarios.md](ui-scenarios.md). **Scope** means we could not test it because the
scope rules confine every request to the scratchpad. **Prod** means a few more API requests would settle it.
"Mock default" is what [data-model.md](data-model.md) does until the question is answered.

## 1. Blocked by the scope restriction

| # | question | mock default |
|---|---|---|
| S1 | What do `parent_id` **omitted, `null` or `""`** resolve to on **create** and **move**? All three pass validation (followup-2). The root is likely, and so is `""` → `Location '' not found`. A sync engine must always send `parent_id` on move. | root |
| S2 | Special keys: `None` (root), `inbox`, `calendar`, `today`, `tomorrow`, `next_week`, `YYYY`, `YYYY-MM`, `YYYY-MM-DD`. What do they resolve to for list, create and move? What priority does a node moved there get? Are calendar nodes materialised, and with what name and layoutMode? What exactly does the 404 for a not-yet-created calendar key say? Where does the resolver check them relative to the URL and short-id forms? | implement per upstream, unverified |
| S3 | **User shortcut keys** as `parent_id`: the resolution and the not-found message after the key is deleted (UI I2 could unlock this with a narrow guard exception). | lookup in a targets table |
| S4 | `GET /nodes` with no `parent_id`: does it list the root or return an error? | root |
| S5 | **`GET /nodes-export`**: response order, whether it includes detached cycles and orphan shells, whether it exposes mirror metadata (`mirror_origin_id`?) or collapse state, and the 1 req/min limit's **429 body and `Retry-After`**. | flat `{"nodes": [...]}` in `create_seq` order, no metadata. The 429 is a guess |
| S6 | **`GET /targets`**: the real shape, which system targets exist, and `name` values. | per upstream |
| S7 | Do mutating endpoints accept special or shortcut keys as the path `:id`? Probably not, given that short ids fail. | no |
| S8 | ~~Uppercase or dashless UUIDs of an **existing** node on delete, move, complete, uncomplete and mirror.~~ **Answered: 404** (`Item not found`, nothing changed) on update, complete, uncomplete, move, mirror POST/DELETE (conformance golden `ids_mutations_need_exact_uuid`) and plain DELETE (`delete_by_short_id_is_noop`). | 404 |

## 2. Needs a human in the UI

| # | question | scenario |
|---|---|---|
| H1 | How does the UI assign priorities (midpoint, shift, renumber, fractional values)? Can it make duplicates? | A1–A5, A10 |
| H2 | How does the UI encode `<`, `>`, `&` and formatting in `name` and `note`? Is `name` HTML? | E1–E5, E8 |
| H3 | Does the UI rewrite or sanitise API-written HTML, entities or markdown on edit (sync churn)? | F1–F3 |
| H4 | What does the UI show for a mirror shell (origin content or the shell's own fields)? Where do UI-created children of a mirror go? Which record does an edit through a mirror change? | H1–H4 |
| H5 | Does deleting an origin in the UI promote a mirror, or does it leave orphans? | H6 |
| H6 | Can undo or trash restore deleted nodes with the **same ids**? This includes API-deleted nodes. | B1, B2 |
| H7 | Does UI completion cascade to descendants in the data? | D2 |
| H8 | Which UI actions bump which `modifiedAt` (child ops, collapse, tags)? | C1, C2 |
| H9 | What is the real set of `layoutMode` values? How do unknown values render? | G1, G2 |
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
| D1 | **Cycle policy**: production accepts a move into a descendant (200) and corrupts the tree. | Mock rejects by default with a mock-specific 400 and offers a `faithful_cycles` flag (data-model §4.7). **Coordinator to confirm.** |
| D2 | Why mirror creation and child creation do not bump the destination parent's `modifiedAt` while move-in does. | Copy the observed table. |
| D3 | Short-id collisions (two nodes with the same last 12 hex digits). | Cannot be constructed. Mock: first match by `create_seq`. |
| D4 | Narrow races (more than 4 concurrent writes) producing duplicate priorities, and the renumber's tie order. | Mock serialises writes. Tie-break `(priority, create_seq)`. |
| D5 | Whether the API's mirror divergence is intended or a bug that may change. | Mock copies it. The sync engine should avoid writing through shells. |

## 5. Process notes for the coordinator

- `prod_client.AllowList.contains` is case-sensitive. The create agent used `unscoped=True` once, to send an uppercase short id
  of its own sandbox. Suggestion: normalise case (and dashes) in `contains`, which would also unlock S8.
- The guard cannot parse repeated query parameters (list-of-tuples params). The errors-http agent used `unscoped=True` once
  with two of our own sandbox ids.
- Two production cycles were created during testing (one by accident) and repaired within about 30 s. The UI should be
  checked (scenario B5).
