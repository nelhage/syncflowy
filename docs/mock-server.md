# Mock Workflowy API server

`syncflowy.mock` is a local, SQLite-backed imitation of the Workflowy REST API (`https://workflowy.com/api/v1`).
It aims to behave **byte-for-byte like production** as far as we know it, quirks and bugs included, so the sync engine
can be developed and tested without touching a real account.

The behavioural spec is [workflowy/data-model.md](workflowy/data-model.md) (cited below as `§x.y`). The per-area docs in
[workflowy/](workflowy/) hold the evidence, and [workflowy/open-questions.md](workflowy/open-questions.md) lists what is
still unknown. Where production behaviour is unknown, the mock picks the most plausible behaviour, marks the code with an
`# UNVERIFIED: ...` comment, and lists the choice in [Known deviations and UNVERIFIED choices](#known-deviations-and-unverified-choices) below.

---

## Running it

### Command line

```sh
direnv exec . uv run syncflowy-mock --db ./mock.sqlite3            # http://127.0.0.1:8765
direnv exec . uv run syncflowy-mock --db ./mock.sqlite3 --port 9000 --api-key k1,k2 --api-key k3
direnv exec . uv run python -m syncflowy.mock --db ./mock.sqlite3  # same thing
```

| flag | default | meaning |
|---|---|---|
| `--db PATH` | required | SQLite file. It is created if missing, and its data persists across restarts. |
| `--host` | `127.0.0.1` | bind address |
| `--port` | `8765` | bind port |
| `--api-key KEY` | `$SYNCFLOWY_MOCK_API_KEY`, else `test-key` | accepted keys. Repeatable, comma-separated, or both. |
| `--faithful-cycles` | off | accept a move into the node's own descendant and create a parent cycle, as production does (see [deviations](#known-deviations-and-unverified-choices)) |
| `--export-min-interval S` | `0` | minimum seconds between `nodes-export` calls per key. `0` (the default) disables the limit: production did not enforce its documented 1 req/min. |
| `--id-seed N` | off | generate node ids from a seeded RNG (well-formed UUIDv4s, reproducible run to run; the sequence restarts on `POST /_mock/reset`) |
| `--xml-check MODE` | `warn` | mock-only WF-HTML well-formedness check of API-written names/notes: `off`, `warn` (record in `GET /_mock/warnings`; responses unchanged) or `reject` (mock-specific 400). See [xml_check](#xml_check-mock-only). |
| `--log-level` | `info` | uvicorn and app log level |

Then point any client at it with `Authorization: Bearer <key>`:

```sh
curl -H 'Authorization: Bearer test-key' -H 'Content-Type: application/json' \
     -d '{"parent_id": "None", "name": "hello"}' http://127.0.0.1:8765/api/v1/nodes
```

(Without `-H 'Content-Type: application/json'`, curl's `-d` sends a form-encoded body. The mock then parses it as form
fields, as production does. See §2.4.)

### In tests

```python
from syncflowy.mock.testing import running_mock

with running_mock() as base_url:                       # "http://127.0.0.1:<free port>"
    ...

with running_mock(db_path="x.sqlite3", api_key="k", clock=my_clock) as base_url:
    base_url.app                                       # the live MockApp (base_url is a str subclass)
```

`running_mock(db_path=None, api_key="test-key", clock=None, *, faithful_cycles=False, export_min_interval=0.0, extra_api_keys=(), id_seed=None, xml_check="warn")`
starts uvicorn in a daemon thread on a free port and yields the base URL. With `db_path=None` it uses a temporary file
that is removed on exit. `clock` is a zero-argument callable that returns float unix time. Every timestamp is
`int(clock())`, plus any offset set through `POST /_mock/clock`. `id_seed` makes node ids deterministic (see
[Using the mock as a sync-engine test double](#using-the-mock-as-a-sync-engine-test-double)).

The repo's own test suite (`tests/mock/`) uses one server per test session and calls `POST /_mock/reset` before each
test. See `tests/mock/conftest.py` for the `WF` helper client and the `FakeClock`.

---

## Architecture

```
src/syncflowy/mock/
  app.py      ASGI app: routing, 404/405, auth, body parsing, error rendering, admin routes
  ops.py      endpoint semantics: id/parent resolution, priority algorithm, mirrors, special keys
  models.py   pydantic v2 request models (CreateItemRequest, UpdateNodeRequest, MoveNodeRequest, MirrorNodeRequest)
  text.py     create-path name pipeline (markdown, multiline, dates) and the update-path strip
  store.py    SQLite schema, the transaction/lock, and row helpers
  testing.py  running_mock()
  cli.py      syncflowy-mock entry point (also python -m syncflowy.mock)
```

- **No web framework.** `app.py` is a small hand-written ASGI callable served by uvicorn. This gives exact control over
  routing (`%2F` decodes before routing, one trailing slash is accepted, there is no redirect), the two 405 styles,
  method-dependent 401 bodies, the HTML 404s and the JSON byte format.
- **Check order** (§2.2) is: routing → method → auth (all in `MockApp.handle`) → body parse (`parse_body`) → pydantic
  validation (`models.validate`) → path-id lookup → `parent_id` lookup → semantic checks (all in `Ops`).
- **Validation** uses pydantic v2 models with production's class names and field declaration order. So `model_type`
  messages ("…instance of CreateItemRequest"), `missing` errors whose `input` is the whole body, error ordering and
  "collect all errors" come out of pydantic exactly as production emits them. `position` is a `str` `Enum` (giving
  `"type": "enum"`), not a `Literal` (which would give `literal_error`).
- **JSON** responses are `json.dumps(obj)` with default settings (`", "`/`": "` separators and `ensure_ascii`), with
  `content-type: application/json` (no charset) and an explicit `content-length`. Node dicts are built in production's
  key order (`Node.to_api`).
- **Serialisation of requests.** All handler logic is synchronous and runs inside `Store.transaction()`, which holds one
  global `RLock` and one `BEGIN IMMEDIATE … COMMIT` SQLite transaction. Concurrent requests therefore behave as if
  serialised in arrival order, and any exception rolls back the whole request. (Example: a NUL in a create note gives
  500 with nothing created, including no materialised calendar node.)
- **Clock.** `Ops.now()` is `int(clock())`. The clock is the injected callable, or `time.time`, plus an adjustable
  offset.
- **TCP_NODELAY** is set on the listening socket. Without it, uvicorn's separate header and body writes hit
  Nagle/delayed-ACK stalls of about 40 ms per request.

## Storage

SQLite via stdlib `sqlite3`, in WAL mode. The schema is in `store.py`:

| table.column | type | exposed? | notes |
|---|---|---|---|
| `nodes.id` | TEXT PK | yes | lowercase hyphenated UUIDv4 |
| `nodes.short_id` | TEXT, indexed | – | last 12 hex chars of `id`. Used for GET and `parent_id` short-id/URL lookups. |
| `nodes.parent_id` | TEXT NULL | yes | `NULL` = top level (the "root") |
| `nodes.name` | TEXT | yes | |
| `nodes.note` | TEXT NULL | yes | `NULL` and `""` are distinct (§6.4) |
| `nodes.priority` | INTEGER | yes | §7 |
| `nodes.completed`, `completed_at` | INTEGER | yes | |
| `nodes.layout_mode` | TEXT | yes (`data.layoutMode`) | any string |
| `nodes.created_at`, `modified_at` | INTEGER | yes | unix seconds |
| `nodes.create_seq` | INTEGER UNIQUE | **hidden** | monotonically increasing. Defines list order (§4.2). Comes from `meta.next_create_seq`, so it never goes backwards after deletes. |
| `nodes.mirror_origin_id` | TEXT NULL | **hidden** | set on mirror shells only. Never cleared, so orphans keep a dangling link (§10). |
| `nodes.system_key` | TEXT UNIQUE NULL | **hidden, mock-only** | `inbox`, `calendar`, `calendar:YYYY`, `calendar:YYYY-MM`, `calendar:YYYY-MM-DD` for materialised special locations. Calendar nodes get deterministic UUIDv5 ids derived from the key, as in production |
| `shortcuts.key`, `node_id`, `created_seq` | | via `/targets` | user "jump-to" shortcuts. Created through the admin API, because the real API cannot create them. |
| `trash.*` | every `nodes` column, plus `deleted_at`, `delete_op`, `op_root` | **hidden, mock-only** | soft-deleted rows, exactly as they were. One `delete_op` per delete operation; `op_root` marks the node the delete was issued on (what the UI's Trash lists). |
| `meta.key`, `value` | | – | `schema_version` (now `2`), `next_create_seq`, `next_delete_op` |

**Deletes are soft** (ui-findings B1/B2: the UI's Trash and undo restore API- and UI-deleted subtrees with the same
ids). A delete *moves* the subtree's rows from `nodes` to `trash` (a recursive CTE with `UNION`, so it terminates even
inside a faithful-mode cycle). Because `nodes` always holds exactly the live outline, every API-visible query — GET,
list, export, short-id and URL resolution, error bodies — is byte-identical to the former hard delete; deleted ids 404.
We chose a separate table over a `deleted_at` column so that no existing query needed a filter. Restores go through
`POST /_mock/ui/restore`. `create_seq` is preserved, so a restored node regains its list (creation-order) position.

**Migration:** opening a schema-v1 file (no `trash` table) upgrades it in place (`CREATE TABLE IF NOT EXISTS` plus a
`schema_version` bump); nodes deleted under v1 were hard-deleted and cannot be restored.

---

## Admin (test-only) routes

These routes are under `/_mock/`, need **no auth**, and always answer with JSON. They exist only in the mock.

| route | body | effect |
|---|---|---|
| `POST /_mock/reset` | – or `{"all": true}` | delete all nodes, trash and shortcuts, reset `create_seq`, clear export rate-limit state and `xml_check` warnings, and restart a seeded id sequence. Config and clock are unchanged, unless `{"all": true}` is sent: then the config returns to its startup values and the clock offset to 0. |
| `GET /_mock/dump` | – | `{"nodes": [...], "trash": [...], "shortcuts": [...], "config": {...}, "now": int}`. Nodes are in creation order and include the hidden `_create_seq`, `_mirror_origin_id` and `_system_key` fields; trash rows also carry `_delete_op`, `_op_root`, `_deleted_at`. |
| `GET /_mock/config` | – | current config |
| `POST /_mock/config` | any of `{"faithful_cycles": bool, "export_min_interval": float, "week_start": 0-6, "xml_check": "off"\|"warn"\|"reject"}` | update the config (`week_start` 0 = Monday … 6 = Sunday; default 6) |
| `GET /_mock/warnings` | – | `{"warnings": [{"op", "node_id", "field", "value", "time"}, ...]}` recorded by `xml_check: "warn"` |
| `DELETE /_mock/warnings` | – | clear the warnings |
| `POST /_mock/ui/<action>` | see [UI simulation](#ui-simulation-routes-_mockui) | apply an edit with web-UI semantics |
| `GET /_mock/ui/trash` | – | `{"trash": [{"delete_op", "id", "deleted_at", "count"}, ...]}`, most recent first |
| `GET /_mock/clock` | – | `{"now": float, "offset": float}` |
| `POST /_mock/clock` | `{"advance": s}` / `{"now": t}` / `{"offset": s}` | shift the server clock (it is added to the injected clock) |
| `POST /_mock/shortcuts` | `{"key": "home", "node_id": "<uuid>"}` | create or replace a user shortcut target (404 if the node does not exist) |
| `DELETE /_mock/shortcuts/<key>` | – | remove a shortcut (404 if it is absent) |

---

## Spec rule → implementation

| spec rule | where |
|---|---|
| §1.1 node record, hidden `create_seq` / `mirror_origin_id`, short id | `store.py` schema, `short_id_of`, `Tx.insert` |
| §1.1 timestamps = `int(time.time())`, injectable clock | `Ops.now`, `ops.Clock` |
| §1.2 node key order, `data` only has `layoutMode`, no mirror metadata | `Node.to_api` |
| §1.3 `json.dumps` defaults, `application/json` without charset, wrappers, `{"status": "ok"}` = 16 bytes | `app.dumps`, `Response.json`, `ops.OK` |
| §2.1 routes, trailing slash, `%2F` = unknown route, `<id>` = one segment | `app.route` |
| §2.1 JSON 405 (`error`, `allowed_methods`, no `Allow`) vs empty-HTML 405 with `Allow` | `app.ROUTES`, `MockApp.handle` |
| §2.1 unknown route → HTML 404 | `MockApp.handle` (body-presence rule, see deviations) |
| §2.2 check order | `MockApp.handle` → `MockApp.dispatch` → `Ops.*` |
| §2.3 auth: header must split into exactly 2 whitespace tokens (`Bearer k extra` → 401), second token is the key, scheme word ignored (`BEARER k` works), 401 body by method, no `WWW-Authenticate` | `MockApp._api_key`, `MockApp.handle` |
| §2.4 form/multipart parsed as fields, otherwise `json.loads` whatever the content-type, `invalid_json`, duplicate keys (last wins), no size limit, query never contributes to a POST | `app.parse_body`, `_parse_multipart` |
| §2.4 complete/uncomplete/DELETE never parse the body | `MockApp.dispatch` (no `_body` call) |
| §2.5 validation errors: model names, field order, all errors, `missing` input = body, `enum`, undeclared keys ignored | `models.py` |
| §2.6 error bodies (`not_found` ×3, `invalid_request` ×2, `internal_error`) | `ops.not_found`, `invalid_request`, `internal_error` |
| §2.7 static headers (`server: nginx`, `x-frame-options`, …), no rate-limit/cache/CORS headers | `app.FIDELITY_HEADERS` |
| §2.8 read-your-writes, serialised writes | `Store.transaction` |
| §3.1 GET accepts exact UUID or 12-hex short id (any case); mutating endpoints accept exact UUID only | `Ops.get_node`, `Ops._lookup_exact` |
| §3.2 `parent_id` resolver (UUID exact/case-sensitive → `Parent item not found`; URL with short id followed by end, `/` or `?` → `Parent item not found`; bare short id; otherwise `Location '<raw>' not found`) | `Ops.resolve_parent`, `UUID_RE`, `URL_RE`, `SHORT_RE` |
| §3.3 mirror `parent_id`: exact UUID only, every miss → `Parent item not found` | `Ops.mirror` |
| §4.1 GET node | `Ops.get_node` |
| §4.2 list: depth 1, completed included, **creation order**, last repeated param wins, other params ignored | `Ops.list_children`, `Tx.children`, `MockApp.dispatch` |
| §4.3 create: block 0 = node, flat children with priorities 0..n-1, note `""`, layout param overrides the prefix (block 0 only), parent not bumped, NUL in note → 500 and nothing created | `Ops.create` |
| §4.4 update: verbatim minus C0, null = unchanged, always bumps `modifiedAt`, never moves/completes; NUL or lone surrogate in `layoutMode` → 500, nothing changed | `Ops.update`, `text.strip_update_text` |
| §4.5 complete/uncomplete: re-stamp, no cascade | `Ops.complete` |
| §4.6 delete: soft (trash), recursive, no renumber, bumps nothing, orphans shells | `Ops.delete`, `Tx.delete_subtree` |
| §4.7 move: priority recomputed (even same-parent), move-to-self only bumps the node, bumps node + destination, descendants travel | `Ops.move` |
| §4.7.5 cycles: reject (mock-specific 400) or create them with `faithful_cycles` | `Ops.move`, `Ops._is_descendant`, `Config.faithful_cycles` |
| §5 field semantics table | `Ops.create/update/complete/move/mirror` |
| §6.1 update strip `[\x00-\x08\x0b\x0c\x0e-\x1f]` on name/note, lone surrogates → `?`; update `layoutMode` stored verbatim | `text.UPDATE_STRIP_RE`, `text.replace_lone_surrogates`, `Ops.update` |
| §6.2 steps 1–3: strip (keeping VT/FF), lone surrogates → `?`, `html.unescape` before markdown (and before fences, splitting, prefixes), CR normalisation | `text.transform_create_name` |
| §6.2 steps 4–5: fence regions (only *closed* fences; an unclosed opener is an ordinary line and joins, e.g. ```` ```\nunclosed ```` → `" unclosed"`), soft-wrap join regex `([^.!?\n])\n+([a-z])` outside fences | `text._soft_join` |
| §6.2 steps 6–7: `splitlines`, strip, drop empty lines, fence grouping (info string kept, blank lines dropped, unclosed fence eats the rest) | `text._group` |
| §6.2 step 8: prefixes (`- [ ]`/`- [x]` cut 6 chars, `- `, `#`/`##`/`###`, `####` → `p` + `<b> …</b>`, `>` not recognised), plain → `bullets` for block 0 / `p` for children | `text.transform_create_name`, `Ops.create` |
| §6.2 step 9: `- [x]` completion leak | `text.transform_create_name` (`leak`) |
| §6.2 step 10: naive inline regexes and escapes | `text.inline_markdown` |
| §6.2 step 11: `<time>` dates (Unicode `\d`, 1–2 digit minutes) | `text._DATE_RE`, `text._time_elem`, `format_date_text` |
| §6.3 create note and `layoutMode` verbatim; NUL or lone surrogate in either → 500, nothing created | `Ops.create`, `text.has_lone_surrogate` |
| §7 `assign_priority` (top halving, renumber at min 0 without bumping, bottom `ceil((max+100)/100)*100`, shells included, exclude self on move) | `Ops.assign_priority` |
| §8 `modifiedAt` table | `Ops.create/update/complete/move/mirror/delete_mirror` (`tx.touch` calls) |
| §9 invariants (soft delete, cycles, orphans, recursive mirrors, completion) | `Ops`, `Tx.delete_subtree` |
| §10.1 mirror identity: an independent empty shell plus a hidden origin link | `Ops.mirror` (`mirror_origin_id`) |
| §10.2 mirror check order: src → origin (orphan → `Item not found`) → parent (exact, then redirect a shell to its origin; orphan → `Parent item not found`) → self check; bumps the origin only | `Ops.mirror`, `Ops._follow_mirror` |
| §10.3 `DELETE /mirror`: `Node is not a mirror`, orphans OK, bumps the origin if it exists | `Ops.delete_mirror` |
| §11 root, `None`, `inbox`, calendar keys, shortcuts, targets, export | `Ops.resolve_parent`, `get_node`, `_special_chain`, `_calendar_priority`, `_materialise`, `targets`, `export` (read paths observed, see below) |

---

## Known deviations and UNVERIFIED choices

### Deliberate deviations from production

1. **Moves into one's own descendant are rejected by default** with
   `400 {"code": "invalid_request", "message": "Cannot move a node under its own descendant"}`. Production returns 200
   and creates a parent cycle that detaches the subtree (§4.7.5, open-questions D1). `--faithful-cycles` /
   `faithful_cycles=True` / `POST /_mock/config {"faithful_cycles": true}` reproduces production. The user
   confirmed this default on 2026-10-12 (open-questions D1).
2. **The 404 HTML page** is a short stub. It has the same `<!DOCTYPE html>` and `<title>404</title>` markers, but it is
   not production's ~20 KB marketing page.
3. **The HTML 404 body rule** is "full page unless the request carried a body, then empty" (data-model.md §2.1,
   corrected from an earlier "GET → page, other methods → empty"). It fits all 27 HTML 404s in the request log and
   golden `http_unknown_routes`; it is listed here only because the rule is fitted [I], not derived.
4. **Admin routes** under `/_mock/` do not exist in production. In production that path would be an unknown route.
5. Optional headers. The mock sends the static security headers, `server: nginx` and `vary`. It omits
   `content-security-policy`, `set-cookie`, gzip and chunked encoding, and always sends `content-length`.

### UNVERIFIED (production behaviour unknown; the mock guesses)

| topic | mock behaviour | ref |
|---|---|---|
| `parent_id` omitted / `null` / `""` on **create and move** | omitted/`null` → the root, `""` → `Location '' not found`: the same as list, which is observed | S1 |
| Materialisation by create/move | the missing chain is created: inbox `"Inbox"` at the **top** of the root; the calendar chain `📆 Calendar` (bottom of root) → `2026` → `07` → `<time …>Sun, Jul 5, 2026</time>` with year/month/day siblings in chronological priority order. Names and ordering are copied from the UI-made calendar; that the API path does the same is inferred. | S2 |
| `today` / `tomorrow` / `next_week` | dates from the mock clock in **UTC**. `next_week` = the next `week_start` day (default Sunday, inferred from the observed year-1 500 window). | S2 |
| Impossible dates / year 9999 on **create and move** | 500, the same as list (observed for list only) | S2 |
| User shortcut keys | only creatable through `POST /_mock/shortcuts`. They resolve to their node for list, create, move and GET. A shortcut whose node was deleted → `Location '<key>' not found`, and it is dropped from `/targets`, where shortcuts are listed first. | S3 |
| Special or shortcut keys as the path `:id` of **mutations** | not accepted (`404 Item not found`), as for short ids | S7 |
| `GET /nodes-export` detached cycles; order | included; creation order (production's order is an unspecified stable storage order) | S5 |
| `nodes-export` rate limit | off by default. With `export_min_interval > 0`: `429 {"code": "rate_limited", "message": "Rate limit exceeded. Please try again later."}` without `Retry-After` (the shape is a guess; production never returned a 429) | S5 |
| `nodes-export` / `targets` non-GET methods | empty-HTML 405 with `Allow: GET, HEAD` (observed for `POST /targets` only) | – |
| `modifiedAt: null` | never produced (production has it on some pre-2024 nodes; clients must accept it) | – |
| Indented fence openers / trailing spaces on fence closers | the fence checks use the stripped line | C§5.4 |
| Whitespace-only lines inside a fence | dropped, like blank lines | C§5.4 |
| Date display text | always en-US, `"Thu, Jan 15, 2026 at 2:30 PM"`, with no account timezone or locale | P1, H11 |
| Renumber tie-break | `(priority, create_seq)` | D4 |
| Renumber on large bottom priorities | never | P10 |
| Short-id collisions | first match by `create_seq` | D3 |
| Other unexpected storage failures | `500 internal_error`, rolled back | – |
| Request body size, name length | unlimited | P9 |

### Observed with read-only GETs on 2026-10-12 ([global-readonly.md](workflowy/global-readonly.md))

The mock now copies these (they were guesses before):

- `GET /nodes` without `parent_id` and `parent_id=None` list the root. `parent_id=` (empty) is `Location '' not found`. `none` and `null` are Location misses. `GET /nodes/None` is `Item not found`.
- **GET `/nodes/:key`** resolves an existing special-location node (`calendar`, `2026`, `2026-07`, `2026-07-05`, a materialised inbox) and never creates one. Misses, impossible dates and `None` give `Item not found`.
- List of a never-materialised `inbox` → `200 {"nodes": []}`. List of a valid date key or `today` without a node → `404 Location '<key>' not found`.
- Impossible dates (`2026-13`, `2026-02-30`, `0000`) and any year-9999 key: list → 500, GET → 404. A date whose week starts before 0001-01-01 (`0001`, `0001-01`, `0001-01-01`…`06` with a Sunday week start): 500 on both.
- Calendar node names `📆 Calendar` / `2026` / `07` / `<time startYear="2026" startMonth="7" startDay="5">Sun, Jul 5, 2026</time>`, `note: null`, UUIDv5 ids.
- `/targets`: system targets in the order `inbox`, `today`, `tomorrow`, `next_week`, `calendar`. `today`, `tomorrow` and `next_week` always carry the labels `Today`, `Tomorrow` and `Next Week`. `inbox` and `calendar` carry their node's name, or `null`.
- `/nodes-export`: `{"nodes": [...]}` with records identical to GET, no mirror metadata, completed included. Production did not rate-limit it.

### Settled by the 2026-10-12 review (no longer guesses)

These were UNVERIFIED rows above and are now backed by conformance goldens; the mock was changed where it differed:

| behaviour | production | golden | mock before |
|---|---|---|---|
| URL `parent_id` boundary (P6) | only end, `/` or `?` may follow the 12 hex digits | `ids_url_parent_boundaries` | accepted any non-hex follower (`-x`, `#y`, `.`, …) **fixed** |
| Date minutes and digits (P1) | `(\d{1,2})` minutes, Unicode `\d` | `text_dates_edges` | `\d{2}` minutes, ASCII only **fixed** |
| NUL in update `layoutMode` | 500, nothing changed | `update_nul_layoutmode_and_lone_surrogates` | stored it **fixed** |
| Lone surrogates in create name, update name/note | replaced by `?` | both surrogate goldens | 500 **fixed** |
| Lone surrogates in create note, create/update `layoutMode` | 500 | both surrogate goldens | 500 (by accident, via SQLite's encoder) — now explicit |
| NUL / C0 controls in create `layoutMode` | NUL → 500; other controls verbatim | `create_layoutmode_controls_nul_and_surrogates` | same (was a guess) |
| Entities inside fences and as structure (P2) | decoded before fences/splitting/prefixes; no markdown in fences | `text_entities_structural` | same (was a guess) |
| Note placement in multi-block create (P3) | block 0; children `""` | `text_multiline_note_and_layout_param` | same |
| Exact-UUID path ids on all mutations (S8) | 404 for short/upper/dashless | `ids_mutations_need_exact_uuid` | same |
| Garbage body on complete/uncomplete (P5) | ignored | `complete_huge_garbage_body` | same |
| Lone surrogate in `parent_id` | create/move: `Location '…' not found` (echoed); mirror: 500 | `ids_parent_id_lone_surrogate` | 500 everywhere (SQLite encoder) **fixed** |

### Things the API cannot see that the mock does not model

These are UI-only concepts: collapse state (no API-visible effect, ui-findings C2), sharing, and UI-side mirror
*rendering*. Trash/undo, UI placement and UI text canonicalisation are now modelled by the
[UI simulation routes](#ui-simulation-routes-_mockui).

---

## UI simulation routes (`/_mock/ui/*`)

Production's web UI writes the same data model with different semantics (see
[workflowy/ui-findings.md](workflowy/ui-findings.md)). These test-only routes (`src/syncflowy/mock/ui.py`) let a
sync-engine test produce exactly what a human editing in the UI would. No auth; JSON object in, JSON out; ids are exact
lowercase UUIDs. Errors: `404 {"code": "not_found", …}` for unknown or deleted ids, `400 {"code": "bad_ui_request", …}`
for bad arguments.

**Placement spec** (create, move, copy, mirror): `parent_id` (`null`/`"None"` = top level) and at most one of
`after_id` / `before_id` (a sibling; the parent may then be omitted), `index` (0-based slot in display order) or
`position` (`"top"` / `"bottom"`, the default). Priority rule [O: A1–A4, B4]: integer midpoint of the new neighbours
(placed first: midpoint of 0 and the first sibling); placed last: `ceil((max+100)/100)*100`; empty parent: 100. No
sibling is rewritten. A mirror shell as `parent_id` resolves to its origin [O: H4].

| route | body | effect (observations in brackets) |
|---|---|---|
| `POST /_mock/ui/create` | placement + `text` (plain; escaped like the UI) or `name` (HTML; canonicalised), optional `note`/`note_text`, `layoutMode` | Enter + typing. Bumps only the new node; note `null` [A2]. → `{"id"}` |
| `POST /_mock/ui/edit` | `id` + `name`/`text` or `append_text`, and/or `note`/`note_text` (null clears to `""`) | edit through a shell edits the origin [H3]. Names are canonicalised; `append_text` types at the end (inside a trailing format run; a malformed name counts as empty) [F step 2]. Bumps the edited node only [C1]. → `{"status", "id"}` (the edited id) |
| `POST /_mock/ui/move` | `id` + placement + `method`: `"drag"` (default) or `"cut_paste"` | same id; bumps the node and destination parent, not the old parent [A1–A4, C1]; `cut_paste` also bumps every descendant [B4]. Refuses moves into the own subtree. → `{"status", "priority"}` |
| `POST /_mock/ui/copy` | `id` + placement | deep copy, new ids; descendants get priorities 0, 1, …; null notes become `""`; createdAt = modifiedAt = now; nothing else bumped [B4]. → `{"id", "ids": {old: new}}` |
| `POST /_mock/ui/complete`, `/uncomplete` | `id` | node only, no cascade; re-completing re-stamps `completedAt` [D1, D2] |
| `POST /_mock/ui/mirror` | `id` + placement | new shell: name `""`, **note `null`**, `bullets`; bumps the origin only [H2]. → `{"id", "origin_id"}` |
| `POST /_mock/ui/set_layout` | `id`, `layoutMode` | bumps the node. `board` adds one empty child; `table` adds `Headers`/`Row`, each with two empty cells at 0, 1; `divider` turns a null note into `""` [G2]. → `{"status", "id", "created"}` |
| `POST /_mock/ui/delete` | `id` | soft delete, bumps nothing [B1, C1]. For an origin with live shells: **promotion** — the origin moves into its newest shell's place (parent, priority), that shell and its subtree are deleted, and origin, new parent and old parent are bumped [H6]. → `{"status", "deleted", "delete_op", "promoted"}` |
| `POST /_mock/ui/restore` | `{"id": <delete root>}` or `{"last": true}` (undo) | same ids, parent, priorities, createdAt, list position; every restored node bumped; parent not bumped [B1, B2]. Works on API deletes. Restoring an origin re-attaches its orphaned shells. → `{"status", "restored"}` |
| `GET /_mock/ui/trash` | – | the Trash: delete operations, most recent first |

## xml_check (mock-only)

The web UI parses names and notes as strict XML ([workflowy/ui-findings.md](workflowy/ui-findings.md) F1/F2): a
malformed name renders as an **empty bullet** and is destroyed by the next UI edit. Production's API accepts anything,
so the mock offers a check of the **stored** values of API `create` (every block's name, and the note) and `update`
(name, note), using `syncflowy.wfhtml.is_well_formed`:

- `"off"`: no check.
- `"warn"` (default): record `{"op", "node_id", "field", "value", "time"}` at `GET /_mock/warnings`. API responses are
  unchanged (byte-identical to production). Note that the API's create path *decodes* entities, so `AT&amp;T` sent to
  create is stored as `AT&T` and warns.
- `"reject"`: a mock-specific `400 {"code": "mock_malformed_xml", "message": …}` after all production checks
  (validation, 404s), with nothing written.

Set it with `--xml-check`, `running_mock(xml_check=…)` or `POST /_mock/config`.

`src/syncflowy/wfhtml.py` (shared with the future sync engine) provides `escape_text`, `is_well_formed`,
`canonicalize` (what a UI save stores), `to_plain_text` and `append_typed_text`; `tests/test_wfhtml.py` pins it to the
E and F tables of ui-findings.md.

### UNVERIFIED choices in the UI simulation and wfhtml

| choice | why unverified |
|---|---|
| No integer gap between neighbours → renumber siblings 100, 200, … in display order (no bumps) | A5 not run |
| Which shell is promoted when an origin with several shells is deleted: the most recently created; shells inside the origin's own subtree are never chosen | H6 had two shells; the newer won |
| UI delete of a shell deletes just the shell, no origin bump | H7 not run |
| Completing through a shell completes the origin | H5 not run |
| Dragging a shell moves the shell itself; moves into the own subtree are refused | H8, B5 not run |
| An orphan shell (origin deleted) used as a parent/edit target acts as an ordinary node | H11 not run |
| Restore when the original parent is gone → top level; restoring a non-root member of a delete is refused; undo of a promotion is not modelled | not observed |
| Board/table side effects only on childless nodes | G2 nodes had no children |
| Copy keeps layouts and completion; shells inside a copied subtree are copied as shells | B4 used plain bullets |
| `edit` with a null note stores `""`; notes are canonicalised like names; a note-only edit leaves the name as is | not observed |
| Typing at the end extends trailing `<b>/<i>/<u>/<s>/<code>/<span>` runs but not `<a>`/`<time>` | F01/F02/F09/F10 only (`<u>`, `<span>`, `<b>`) |
| wfhtml: numeric character references (`&#39;`) are accepted | F15 mixed them with the rejected `&nbsp;` |
| wfhtml: a bare `&` inside an attribute value is tolerated (the UI writes `href="…?b=1&c=2"` itself, E5) | unknown whether the UI parser re-accepts it |
| wfhtml: `<em>` → `<i>`; attributes on `<b>` etc. dropped; `<a>` without `href` unwrapped; spans kept only with a `colored…` class; `<time>` kept with all attributes | not observed |

## Using the mock as a sync-engine test double

Review notes (2026-10-12) on the properties a sync-engine test suite relies on:

- **Concurrency.** All endpoint logic is synchronous and runs inside `Store.transaction()`: one process-wide `RLock` plus
  one `BEGIN IMMEDIATE … COMMIT`. uvicorn runs the app on a single event loop, so requests are serialised in arrival
  order and each is atomic; a failure (4xx, 500) rolls everything back, including lazily materialised calendar nodes.
  The lock also protects direct use of `url.app.ops` from test threads. A failing `COMMIT` now rolls back too, so the
  connection can never be left inside a transaction. Two processes on one file also work (WAL, `BEGIN IMMEDIATE`,
  Python's default 5 s busy timeout), though that is not a tested configuration.
- **Persistence.** SQLite in WAL mode with `synchronous=NORMAL`: committed requests survive a process crash or restart
  (`test_special_admin.py` restarts on the same file). `create_seq` lives in the `meta` table, so list order keeps
  working across restarts. Schema v1 files are migrated to v2 (trash table) on open; otherwise delete the file when the
  schema changes.
- **Reset between tests.** `POST /_mock/reset` empties the outline in one transaction. Send `{"all": true}` to also
  restore the startup config (e.g. undo a test's `faithful_cycles`) and zero the clock offset. The pytest fixture in
  `tests/mock/conftest.py` shows the pattern: one server per session, reset per test.
- **Deterministic clock.** Pass `clock=` (a zero-arg callable, e.g. a `FakeClock`) to `running_mock`; every timestamp is
  `int(clock() + offset)`. `POST /_mock/clock` moves the offset for black-box clients. Calendar keys use the same clock.
- **Deterministic ids.** `running_mock(id_seed=N)` / `--id-seed N` draws UUIDv4s from `random.Random(N)`; the sequence
  restarts on every reset, so a test sees the same ids on every run. Without it ids are random `uuid4()`, as in
  production.
- **Keep-alive.** `running_mock` and the CLI set uvicorn's `timeout_keep_alive` to 3600 s. With uvicorn's 5 s default, a
  long-lived client that reuses a pooled connection just as the server closes it gets a spurious
  `RemoteProtocolError: Server disconnected`. This is the most likely cause of the one unexplained test failure seen in an
  earlier round (never reproduced in more than 20 runs since; the failing test was not captured).

## Tests

`tests/mock/` (pytest, about 2,500 tests, about 30 s). Everything goes through HTTP against `running_mock`, except a
few pure-function tests of `text.py`.

| file | covers |
|---|---|
| `test_http.py` | serialisation bytes, routing, both 405 styles, HEAD content-length, auth variants and 401 shapes, check order, body encodings, invalid JSON, query strings, large bodies, 500 shape |
| `test_validation.py` | every validation-error shape for all four models |
| `test_ids.py` | path-id and `parent_id` resolution matrices, every not-found variant, mirror's exact-only parent |
| `test_create.py` | **golden vectors**: about 500 create-path names replayed from `experiments/create/*.out.json` and `experiments/text-content/results/*.json`, plus update-path vectors, defaults, note handling, layout overrides, completion leak, multiline structure |
| `test_update_complete.py` | partial updates, null semantics, C0 stripping, completion |
| `test_ordering_move.py` | the priority algorithm (halving, renumber, bottom formula), list order, move semantics, cycles (both modes), concurrency |
| `test_mirror_delete.py` | delete, mirror identity, redirects, orphans, and the full `modifiedAt` table |
| `test_special_admin.py` | root, special keys (GET/list matrix, calendar names and priorities, date-key 500s, week start), inbox, shortcuts, targets, export, admin routes, persistence across restart, CLI key parsing |
| `test_review.py` | the behaviours settled by the 2026-10-12 review (URL boundary, date edges, NUL and lone surrogates), full reset, seeded ids |
| `test_ui_sim.py` | soft delete (API delete → trash → restore, undo, orphan re-attachment, v1→v2 migration), every `/_mock/ui/*` route against the ui-findings scenarios (A1–A4, B1/B2/B4, C1, D1/D2, E1, F, G2, H2–H4, H6), and `xml_check` (warn/reject/off) |
| `tests/test_wfhtml.py` | `syncflowy.wfhtml` against the E (typed text, UI output) and F (render + canonical form after a UI edit) tables |
| `test_replay_log.py` | **replays `.prod-state/requests.jsonl`**: about 236 state-independent responses (validation, invalid_json, 401, 405, HTML 404) are re-sent and must match byte-for-byte in content, and about 860 logged creates are re-run in a fresh sandbox and compared with production's first observation of the node and its multiline children. Skipped if the log is absent. |

Run them with:

```sh
direnv exec . uv run pytest tests/mock -q
```
