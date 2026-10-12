# Workflowy API: consolidated data model and endpoint specification

This is the **authoritative spec for the mock server**. It consolidates the per-area
investigation docs into a single specification that a mock implementer can follow without
reading them. Every rule is tagged:

- **[O: ref]**: OBSERVED against production on 2026-10-12. `ref` points to the
  per-area doc and section, and experiment scripts are listed in that doc.
- **[I: ref]**: INFERRED. The rule fits every observation but was not proven directly.
- **[U]**: UNTESTED or UNKNOWN. The behaviour we chose is a guess, and the open question is in
  [open-questions.md](open-questions.md).

Reference abbreviations:

| abbrev | doc |
|---|---|
| `C` | [create.md](create.md) |
| `U` | [update-and-complete.md](update-and-complete.md) |
| `M` | [ordering-and-move.md](ordering-and-move.md) |
| `D` | [delete-and-mirrors.md](delete-and-mirrors.md) |
| `E` | [errors-and-http.md](errors-and-http.md) |
| `T` | [text-and-content.md](text-and-content.md) |
| `F0`/`F1`/`F2` | [followup-0.md](followup-0.md) / [followup-1.md](followup-1.md) / [followup-2.md](followup-2.md) |
| `UP` | [upstream-api-reference.md](upstream-api-reference.md) (the vendored official docs) |

Where the per-area docs disagree, this document records the resolution (see §13).

---

## 1. Storage model

### 1.1 Node record

| column | type | exposed? | notes |
|---|---|---|---|
| `id` | lowercase hyphenated UUIDv4 string | yes | server-generated. Client-supplied `id` is ignored [O: C§3.4] |
| `parent_id` | UUID or NULL | yes | NULL only for top-level nodes (the scratchpad has `parent_id: null`) [O] |
| `name` | text, never NULL | yes | `""` allowed [O: U, T§10] |
| `note` | text or NULL | yes | NULL vs `""` are distinct, see §6.4 [O: C§3.2, U] |
| `priority` | integer | yes | sparse sort key scoped to one parent, see §7 [O: M] |
| `completed` | bool | yes | |
| `completed_at` | int unix seconds or NULL | yes (`completedAt`) | |
| `layout_mode` | text, never NULL | yes (`data.layoutMode`) | any string [O: C§3.3, U] |
| `created_at` | int unix seconds | yes (`createdAt`) | never changes, not even on move [O: U, M§4] |
| `modified_at` | int unix seconds | yes (`modifiedAt`) | see §8 |
| `create_seq` | monotonically increasing int | **no** | needed for list order, because `createdAt` ties within one second [O: M§6, F0] |
| `mirror_origin_id` | UUID or NULL | **no** | set only on mirror shells. Kept (dangling) after the origin is deleted [O: D§5] |

Deletion is **hard** (the row and its subtree disappear, §9.5). No tombstones are visible
through the API [O: D§1].

Timestamps are `int(time.time())` (floor of wall-clock seconds) [O: U]. The mock
should take an injectable clock.

**Short id** = the last 12 hex characters of the UUID (the dashes removed, then the suffix). It is used
by GET and by `parent_id` resolution [O: E§2].

### 1.2 Serialised node

Key order is **exactly**: [O: C§1.2, E§9]

```json
{"id": "…", "name": "…", "note": null, "parent_id": "…", "priority": 100, "completed": false, "data": {"layoutMode": "bullets"}, "createdAt": 1791834396, "modifiedAt": 1791834396, "completedAt": null}
```

This order differs from the upstream example, which puts `parent_id` second. `data` only ever contains `layoutMode` [O].
No mirror metadata is ever serialised [O: D§2.2].

### 1.3 JSON serialisation

Responses are byte-compatible with Python `json.dumps(obj)` defaults: `", "` and `": "`
separators, and `ensure_ascii=True`, so non-ASCII is written as `\uXXXX` and astral characters as surrogate pairs. `/`
is not escaped [O: E§9]. `Content-Type: application/json` (no charset) [O: E§1].

Wrappers [O]:

| endpoint | 200 body |
|---|---|
| GET node | `{"node": {...}}` |
| list | `{"nodes": [...]}` |
| create | `{"item_id": "<uuid>"}` |
| mirror | `{"item_id": "<uuid>", "origin_id": "<uuid>"}` |
| update / delete / move / complete / uncomplete / delete-mirror | `{"status": "ok"}` (exactly 16 bytes) |

None of the mutating endpoints echo the node back. Status is always 200 (never 201 or 204) [O].

---

## 2. HTTP layer

### 2.1 Routes

All routes also accept a trailing `/`, without a redirect [O: E§1].

| path | methods | 405 style for other methods |
|---|---|---|
| `/api/v1/nodes` | GET (list), POST (create) | JSON, `allowed_methods: ["GET", "POST"]` |
| `/api/v1/nodes/<id>` | GET, POST (update), DELETE | JSON, `["GET", "POST", "DELETE"]` |
| `/api/v1/nodes/<id>/move` | POST | HTML-empty, `Allow: POST` |
| `/api/v1/nodes/<id>/complete` | POST | HTML-empty, `Allow: POST` |
| `/api/v1/nodes/<id>/uncomplete` | POST | HTML-empty, `Allow: POST` |
| `/api/v1/nodes/<id>/mirror` | POST, DELETE | JSON, `["POST", "DELETE"]` |
| `/api/v1/nodes-export` | GET | [U] (route exists: 401 with a bad key [O: E§4]) |
| `/api/v1/targets` | GET | HTML-empty, `Allow: GET, HEAD` [O: E§5] |

- **JSON 405** [O: E§5]: `405 {"error": "Method not allowed", "allowed_methods": [...]}`,
  `application/json`, with no `Allow` header. HEAD and OPTIONS also get 405 (HEAD with no body).
- **HTML-empty 405** [O: E§5, U]: `405`, `content-type: text/html; charset=utf-8`,
  `content-length: 0`, `Allow: POST`.
- **Unknown route** [O: E§5, D§1.4]: 404 `text/html; charset=utf-8`. GET returns a full HTML page
  (`<!DOCTYPE html>…<title>404</title>…`), and other methods return an empty body. This covers
  `/api/v1/trash`, `/nodes/<id>/restore`, `/nodes/<id>/children`, `/nodes/<id>/completed`, `/api/v1/me`,
  and so on. A `%2F` in the path decodes to `/` before routing, so an id containing a URL gives an unknown route [O].
- `<id>` is any single path segment [O].

### 2.2 Check order

[O: E§0.4, E§6.3, D§2.7, F1]

1. Routing (404 HTML)
2. Method (405)
3. Auth (401)
4. Body parse (400 `invalid_json`). Only create, update, move and mirror parse the body.
5. Schema validation (400 `errors[]`, all errors collected)
6. Path-id lookup (404 `Item not found`). For mirror, also origin resolution.
7. `parent_id` lookup (404 `Parent item not found` / `Location '…' not found`)
8. Semantic checks (400 `invalid_request`)

So a bad body sent to a nonexistent id gives 400, and a nonexistent `:id` with a nonexistent
`parent_id` gives `Item not found` [O].

### 2.3 Authentication

[O: E§4]

- Header `Authorization`. The server splits it on whitespace and needs at least 2 tokens. The **second token** must be a valid
  key, and the scheme word is ignored: `Bearer k`, `bearer k`, `Token k` and `Bearer  k` all work. A bare `k`,
  `Basic …`, a missing header or a bad key give 401. [I] for the exact split rule.
- 401 body depends on the **method**:
  - GET: `{"errors": "Invalid Credentials, try again."}`
  - POST and DELETE: `{"code": "invalid_credentials", "message": "Invalid Credentials, try again."}`
- No `WWW-Authenticate` header.

### 2.4 Request bodies

[O: E§6.1, U, C§3.6]

- If `Content-Type` is `application/x-www-form-urlencoded` or `multipart/form-data`, the body is parsed as
  form fields, so every value is a string.
- Otherwise the raw body is parsed with `json.loads`, **whatever the Content-Type** is (also
  `text/plain`, or no header). Duplicate keys: the last one wins.
- An empty or unparsable body gives `400 {"code": "invalid_json", "message": "Invalid JSON format in request body"}`.
- A parsed body that is not an object gives `400 {"errors": [{"type": "model_type", "loc": [], "msg": "Input should be a valid dictionary or instance of <Model>", "input": <body>}]}`.
- The query string never contributes fields to a POST [O].
- `complete`, `uncomplete` and `DELETE` **ignore the body entirely**, even invalid JSON [O: U].
- No size limit: 20 MB bodies were accepted and 1 MB names and notes were stored [O: T§9]. The mock must not limit size,
  or must use a limit of at least 20 MB.

### 2.5 Validation errors

[O: E§6.2, F2]

```json
{"errors": [{"type": "string_type", "loc": ["name"], "msg": "Input should be a valid string", "input": 1}]}
```

- Item key order is `type, loc, msg, input`. There are no `url` or `ctx` keys.
- **All** errors are collected, in **field-declaration order**. The key order in the body does not matter.
- Models and their declared fields:

| endpoint | model | fields, in order | required |
|---|---|---|---|
| create | `CreateItemRequest` | `name, parent_id, note, layoutMode, position` | `name` (non-null) |
| update | `UpdateNodeRequest` | `name, note, layoutMode` | none (null means "not provided") |
| move | `MoveNodeRequest` | `parent_id, position` | none (`parent_id` optional and nullable) |
| mirror | `MirrorNodeRequest` | `parent_id, position` | `parent_id` (non-null) |

- Error types:

| condition | type | msg | input |
|---|---|---|---|
| string field is a non-null non-string (number, bool, list, object) | `string_type` | `Input should be a valid string` | the value |
| required field is null (create `name`, mirror `parent_id`) | `string_type` | same | `null` |
| required field missing | `missing` | `Field required` | **the whole body object** |
| `position` not in `{"top","bottom"}` and not null (case-sensitive; `""`, ints and bools included) | `enum` | `Input should be 'top' or 'bottom'` | the value, in its original JSON type |

- `""` is a valid string everywhere and passes validation, including `parent_id` [O: F2].
- Keys that are not declared fields are **silently ignored and never type-checked**: `priority`, `completed`, `completedAt`,
  `id`, `createdAt`, `modifiedAt`, `data`, `parent_id`/`position` on update, `index`/`after`/`before`/`sibling_id` on
  move, misspellings, and so on [O: C§3.4, U, M§2].

### 2.6 Other error bodies

All are `application/json` [O]:

| status | body |
|---|---|
| 404 | `{"code": "not_found", "message": "Item not found"}` |
| 404 | `{"code": "not_found", "message": "Parent item not found"}` |
| 404 | `{"code": "not_found", "message": "Location '<raw input>' not found"}` (raw input echoed in single quotes, JSON-escaped) |
| 400 | `{"code": "invalid_request", "message": "Cannot mirror a node under itself"}` |
| 400 | `{"code": "invalid_request", "message": "Node is not a mirror"}` |
| 500 | `{"code": "internal_error", "message": "An internal server error occurred"}`. Only seen for a NUL in the create `note` [O: T§2] |

### 2.7 Headers

There are no rate-limit headers, no `Retry-After`, no caching headers (`ETag`, `Last-Modified`), and no CORS headers [O: E§1].
Optional fidelity: `server: nginx`, `x-frame-options: SAMEORIGIN`,
`x-content-type-options: nosniff`, `x-xss-protection: 1; mode=block`, `referrer-policy: same-origin`,
`strict-transport-security: max-age=63072000; includeSubDomains; preload`, `vary: Accept-Encoding, Cookie, Origin`.
The `Accept` header is ignored [O]. We never saw a 429 at up to 9 req/s, and its shape is unknown [U].

### 2.8 Consistency

Production is **strictly read-your-writes**. Every create, update, move, delete, complete and mirror was
visible to the immediately following GET or list in more than 1000 checks [O: C§6, U, M§4, D§1.6, E].
Concurrent writes behave as if serialized in arrival order and never produced duplicate priorities [O: M§8, F0].
The mock should serialize all writes (one SQLite transaction per request, under a global write lock).

---

## 3. Id resolution

### 3.1 Path `:id`

[O: E§2.1, U, D§1.2, M§3]

| endpoint | accepted forms |
|---|---|
| `GET /nodes/:id` | exact lowercase hyphenated UUID, **or** a 12-hex short id (case-insensitive) |
| every other endpoint (update, delete, move, complete, uncomplete, mirror POST/DELETE) | **exact lowercase UUID only** |

- A short id on a mutating endpoint gives `404 Item not found` and **does nothing**. For DELETE, the node is not deleted [O].
- An uppercase UUID or a dashless UUID gives 404 on GET and update [O]. On the other mutating endpoints we inferred the same and did not test it [I].
- An uppercase full UUID does **not** fall through to short-id matching on GET [O].
- Anything else (wrong length, garbage) gives `404 Item not found` [O].
- A deleted node, or a descendant of one, gives `404 Item not found` [O].
- The returned `node.id` is always the canonical lowercase UUID [O].

### 3.2 `parent_id` for list, create and move

[O: E§2.2 revised by F2, C§3.5]

```python
UUID_RE  = r'^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$'   # re.I
URL_RE   = r'^https?://(beta\.)?workflowy\.com/#/([0-9a-f]{12})(?![0-9a-f])'   # re.I, prefix match
SHORT_RE = r'^[0-9a-f]{12}$'                                                   # re.I

def resolve_parent(s):
    if s is None or s == "":            # [U] passes validation; production behaviour untested (probably root)
        ...
    if re.match(UUID_RE, s, re.I):      # any case
        return exact_lookup(s) or 404("Parent item not found")   # exact, case-sensitive: an UPPER UUID always misses
    if m := re.match(URL_RE, s, re.I):
        return short_lookup(m[2].lower()) or 404("Parent item not found")
    # [U] special keys ("None", "inbox", "calendar", "today", "tomorrow", "next_week",
    #     "YYYY", "YYYY-MM", "YYYY-MM-DD") and user shortcut keys go here
    if re.match(SHORT_RE, s, re.I):
        if n := short_lookup(s.lower()): return n
    return 404(f"Location '{s}' not found")
```

| input | result |
|---|---|
| existing UUID | OK [O] |
| UUID-shaped miss, including an UPPERCASE UUID of an existing node | `Parent item not found` [O] |
| deleted node's UUID | `Parent item not found` [O] |
| short id, any case | OK [O] |
| bare 12-hex miss | `Location '<x>' not found` [O] |
| `http(s)://(beta.)workflowy.com/#/<short>`, case-insensitive host and short, followed by `/`, `?q`, or `/<other>` (the first segment wins) | OK [O] |
| the URL form above with a nonexistent short id | `Parent item not found` [O] |
| URL with a full UUID (dashed or dashless, even an existing one), `www.`/`app.`/other host, no scheme, `#/x/<short>`, no `/` before `#` | `Location '<raw>' not found` [O] |
| dashless UUID | `Location '<raw>' not found` [O] |
| surrounding spaces | no trimming, so `Location` [O] |
| non-string | 400 `string_type` (validation) [O] |
| omitted, `null`, `""` | passes validation, and resolution is **untested** [U]. Mock: treat as the account root |
| `None`, `inbox`, calendar keys, shortcut keys | **untested** (out of scope) [U]. Mock: implement per UP, guessing |

The URL boundary check after the 12 hex digits is [I] (a 32-hex dashless UUID after `#/` is rejected).

### 3.3 `parent_id` for mirror

[O: D§2.7, F1, F2] An **exact UUID lookup only**. *Every* miss gives `404 Parent item not found`: short ids,
URLs, uppercase or dashless UUIDs, and garbage strings. Mirror never produces `Location '…'`. If the resolved parent
is a mirror shell, it is redirected to that shell's origin (§10.2).

---

## 4. Endpoints

### 4.1 `GET /api/v1/nodes/:id`

Resolve per §3.1, then return `200 {"node": <node>}`. Otherwise return `404 Item not found` [O].

### 4.2 `GET /api/v1/nodes?parent_id=…` (list)

- `parent_id` is resolved per §3.2. A miss gives `Parent item not found` or `Location`, and a deleted parent gives `Parent item not found` [O].
- Returns `{"nodes": [...]}` with **direct children only** (depth 1), **including completed** nodes [O: E§8].
- **Order: ascending `create_seq`** (creation order). This is NOT priority order, and moves, renames, completes and renumbers do not change it.
  A node created earlier under a different parent and then moved in lists *before* the parent's older-created children
  if it was created earlier [O: M§6, F0§4: 331/331 log responses createdAt-sorted]. Upstream says "unordered", so clients must
  sort by `priority`.
- An empty parent gives `{"nodes": []}` [O].
- If the query parameter is repeated, the last one wins [O]. All other params are ignored (`limit`, `offset`, `cursor`, `page`, `per_page`,
  `depth`, `recursive`, `include_completed`, `completed`, `include_deleted`) [O]. There is no pagination [O].
- Listing a mirror shell returns only the shell's own children (§10) [O].
- Without `parent_id`: [U] (probably the root, per UP).

### 4.3 `POST /api/v1/nodes` (create)

1. Parse and validate (§2.4, §2.5, model `CreateItemRequest`).
2. Resolve `parent_id` (§3.2). A mirror shell as the parent is **not** redirected: the child goes under the shell [O: D§2.5].
3. Transform `name` into blocks (§6.2). Use the `note` verbatim (§6.3). If the note contains NUL, return **500** and create nothing [O: T§2].
4. Block 0 becomes the created node:
   - `priority` = `assign_priority(parent, position, exclude=None)` (§7)
   - `layoutMode` = the `layoutMode` param if it is a non-empty string (any value, stored verbatim; it overrides the prefix layout),
     otherwise the prefix layout, otherwise `"bullets"`. The param values `""` and `null` give `bullets` [O: C§3.3]
   - `note` = the param (`null` if absent or null, `""` stays `""`) [O]
   - `completed` and `completedAt` come from the `- [x]` rule (§6.2.5). The `completed`/`completedAt` params are ignored [O]
   - `createdAt = modifiedAt = now`
5. Blocks 1..n become **direct children of block 0** (flat), with `priority = 0, 1, 2, …`, `note = ""`, the same timestamps,
   a layout from the prefix or else `"p"`, and completion per the leak rule [O: C§5.2, T§5.1]. The `layoutMode` param
   applies to block 0 only [O: C§3.3].
6. The parent's `modifiedAt` is **not** bumped [O: C§1.2, U, F1].
7. Response: `200 {"item_id": "<block-0 uuid>"}`. Children ids are not returned [O].

### 4.4 `POST /api/v1/nodes/:id` (update)

[O: U, T]

1. Parse and validate (`UpdateNodeRequest`). A null `name`/`note`/`layoutMode` means "leave unchanged".
2. Look up `:id` by exact UUID. A miss gives `404 Item not found`.
3. For each of `name`, `note` and `layoutMode` that is present and non-null, store it after deleting
   `[\x00-\x08\x0b\x0c\x0e-\x1f]` (§6.1). No other transformation is applied. Whether layoutMode is also control-stripped is [U].
4. **Always** set `modifiedAt = now`, even for `{}`, a no-op, unknown fields only, or null fields [O].
5. `parent_id`, `priority`, `position`, `completed`, and so on are ignored: update never moves, reorders or completes [O].
6. `200 {"status": "ok"}`.

A `note` can be set to `""` but never back to `null` [O]. `name` may be `""` [O]. Changing `layoutMode` does not affect
completion [O].

### 4.5 `POST /api/v1/nodes/:id/complete` and `/uncomplete`

[O: U§Complete]

- The body is ignored entirely (not even parsed). The `:id` must be an exact UUID, otherwise `404 Item not found`.
- complete: `completed = true`, `completedAt = now`, `modifiedAt = now`. **Re-completing re-stamps `completedAt`**.
- uncomplete: `completed = false`, `completedAt = null`, `modifiedAt = now`. This happens even if the node was never completed.
- Only the target changes. **No cascade** to descendants, and no `modifiedAt` change on them or on the parent.
- Works for any layoutMode and does not change it. Priority is unchanged.
- `200 {"status": "ok"}`.

### 4.6 `DELETE /api/v1/nodes/:id`

[O: D§1, D§4.2, D§7]

- Exact UUID only, otherwise `404 Item not found`. A short id does NOT delete. Deleting twice gives 404.
- Hard-deletes the node and its whole subtree (everything under it then gives `404 Item not found` / `Parent item not found`).
- No sibling renumbering. Nobody's `modifiedAt` is bumped: not the parent, and not the origin if the node is a mirror shell.
- Mirrors elsewhere whose origin lies in the deleted subtree survive as **orphans** (`mirror_origin_id` dangling) [O].
- Deleting a subtree that contains a mirror shell removes the shell and does not bump its origin [O: D§4.2].
- The body is ignored. Returns `200 {"status": "ok"}`.
- There is no trash or restore endpoint [O].

### 4.7 `POST /api/v1/nodes/:id/move`

[O: M§3–5, F0, F1, F2]

1. Parse and validate (`MoveNodeRequest`). `parent_id` is optional and nullable [O]. `{}` passes validation, and **what it does is [U]**
   (probably moves to the root).
2. Look up `:id` by exact UUID, otherwise `404 Item not found`.
3. Resolve `parent_id` per §3.2, otherwise `Parent item not found` or `Location`. A mirror shell is **not** redirected [O: D§2.5].
4. **Move to self** (`parent == node`): `200`, with no change to the parent or the priority (the priority is *not* recomputed), and the node's
   `modifiedAt` bumped, the parent's not [O: M§4, F0, F2].
5. **Move into its own descendant**: production returns 200 and **creates a parent cycle**, which detaches the subtree from
   the outline. GET and list still work on the detached nodes [O: M§4, D§6, E§7]. **Mock policy (decision)**: reject by default
   with the mock-specific `400 {"code": "invalid_request", "message": "Cannot move a node under its own descendant"}`
   (NOT production behaviour), and provide a `faithful_cycles` flag that reproduces production.
6. Otherwise: `priority = assign_priority(new_parent, position, exclude=node)`. This applies even for a same-parent move,
   so a node already at the top/bottom can move *up* or *down* in value [O: M§2]. Set `parent_id`. Set `modifiedAt = now` on **the moved node and the
   destination parent only**. The old parent, ancestors, descendants and siblings (even renumbered ones) are not bumped [O: F1, 3/3].
7. Descendants travel with the node, keeping their ids and priorities [O]. `id` and `createdAt` are kept [O].
8. There is **no API way to insert in the middle**. `priority`, `index`, `after`, `before`, `after_id` and `sibling_id` are ignored [O: M§2].
9. `200 {"status": "ok"}`.

### 4.8 `POST /api/v1/nodes/:id/mirror`

See §10.2.

### 4.9 `DELETE /api/v1/nodes/:id/mirror`

See §10.3.

### 4.10 `GET /api/v1/nodes-export`, `GET /api/v1/targets`

[U] These were never called successfully, because the scope guard forbids it (only a bad-key 401 and `POST /targets` gave 405 [O: E§4–5]).
Mock per UP:
- `nodes-export` returns `{"nodes": [<every node, flat>]}` with the same node shape. Order [U] (suggest `create_seq`). The rate limit is 1 req/min,
  and the 429 shape is [U].
- `targets` returns `{"targets": [{"key", "type": "shortcut"|"system", "name"}]}`. System targets (`inbox`, …) are always present, with
  `name: null` when not yet created.

Whether export includes orphan or detached nodes, or exposes mirror metadata, is [U].

---

## 5. Node field semantics summary

| field | create | update | complete/uncomplete | move | mirror |
|---|---|---|---|---|---|
| name | transformed (§6.2) | verbatim minus C0 (§6.1) | – | – | shell gets `""` |
| note | verbatim, default `null` | verbatim minus C0, null ignored | – | – | shell gets `""` |
| layoutMode | param, else prefix, else `bullets` | any string, null ignored | unchanged | unchanged | shell gets `bullets` |
| completed | only via `- [x]` | ignored | set | – | shell gets `false` |
| priority | §7 | ignored | – | §7 | §7 |
| parent_id | resolved | ignored | – | resolved | resolved + redirect |

---

## 6. Text processing

### 6.1 Update path (name, note, and probably layoutMode)

Delete `[\x00-\x08\x0b\x0c\x0e-\x1f]` and store everything else **byte-for-byte**: no markdown, no entity
decode or encode, no trimming, no newline handling, no HTML sanitising, and no Unicode normalisation [O: T§2, U].
`\r\n` and `\r` are preserved [O]. Re-submitting a name that create produced is a fixed point [O: T§11].

### 6.2 Create path: `name`

This is the only transforming path. Pipeline [I: fitted to ~200 observed cases. T§5–7 and C§4–5 are the test vectors;
`experiments/text-content/model_check.py` reproduces 191/191 single-line cases]:

1. **Strip** `[\x00-\x08\x0e-\x1f]` (VT and FF are kept here and later act as line breaks) [O: T§2].
2. **`html.unescape()`** (Python semantics: `&ampx` → `&x`, unknown entities are kept, a single pass) [O: T§4.2]. This
   runs **before** markdown: `&#42;x&#42;` → `<i>x</i>`, and `&lt;b&gt;` becomes a real `<b>` [O].
3. **Normalise** `\r\n` and `\r` → `\n` [O].
4. **Find code-fence regions** (C§5.4 [I]). A line starting with ```` ``` ```` opens a fence, unless it is longer than 3 and
   also ends with ```` ``` ```` (a one-line fence). The region continues up to, but not including, the next line ending in ```` ``` ````.
5. **Soft-wrap join**, outside fence regions only [O/I: C§5.1]: one non-overlapping, left-to-right
   `re.sub(r"([^.!?\n])\n+([a-z])", r"\1 \2", text)`. Consequences [O]:
   - `a\nb` → `a b`; `a\n\nb` → `a b` (the number of blank lines is irrelevant); `a\nb\nc` → `a b` + child `c` (the
     1-char line `b` was consumed as the "after" char, so it cannot also be the "before" char of the next join);
     `parent\nchild1\nchild2` → one node `parent child1 child2`.
   - There is no join if the next line starts with anything other than ASCII `a-z` (uppercase, digit, `-`, `#`, `*`, `>`,
     `` ` ``, `(`, `"`, whitespace, non-ASCII such as `é`), or if the previous line ends in `.`, `!` or `?`.
   - `a \nb` → `a  b` (two spaces).
   - (T§5.1 describes this as "a line absorbs at most one following non-block-start line". That model contradicts
     the observed `hello\nworld\nagain\nmore` → one node, so use the regex. See §13.)
6. **Split** into lines with `str.splitlines()` semantics: `\n`, `\v`, `\f`, `\x1c`–`\x1e` (already stripped),
   `\x85`, `\u2028`, `\u2029`. Lines split by the non-`\n` separators are never joined [O: T§2, C§4.4].
7. **Strip** each line with Python `str.strip()` (this strips NBSP, U+2003 and U+3000, but not U+FEFF or ZWSP) and **drop empty**
   lines [O: T§3]. Group fenced code lines into a single block (C§5.4 step 3): the content keeps its `\n`s and its
   indentation, blank lines inside are dropped, the info string stays as text (```` ```py\nx = 1\n``` ```` → `py\nx = 1`),
   an unclosed fence eats the rest of the input, and a fence closed by a joined line keeps a literal ```` ``` ```` [O].
   At least one block always exists: `""`, `"   "` and `"\n\n\n"` give one node named `""` [O].
8. **Prefix per block** (only one prefix is consumed) [O: C§4.5, T§5.1]:

   | prefix (on the stripped block) | layoutMode | name |
   |---|---|---|
   | `- [ ]` | `todo` | `block[6:]` (blindly cuts 6 chars: `- [ ]no` → `o`) |
   | `- [x]` (lowercase x only) | `todo`, and sets the **completion flag** | `block[6:]` |
   | `- ` | `bullets` | `block[2:]` (not re-stripped: `-   b` → `  b`) |
   | `# ` / `## ` / `### ` | `h1` / `h2` / `h3` | the rest |
   | `#### ` | **`p`** | `"<b>" + block[4:] + "</b>"` (keeps the leading space: `<b> H4</b>`) |
   | fence | `code-block` | fence content |
   | anything else (`#####…`, `#x`, `* `, `+ `, `1. `, **`> `**, `---`, `- [X]`→ eats `- ` only) | plain | literal |

   A plain block gets `bullets` if it is block 0, and `p` if it is a child [O]. **`> ` does NOT give `quote-block`**, despite UP [O, re-run].
9. **Completion leak** [O: C§5.3]: once a `- [x]` block has been processed, it **and every later block** in the same request
   is created `completed: true, completedAt = createdAt`, whatever its prefix (`- [ ]`, headers and plain text included).
   Blocks before it are unaffected. The `layoutMode` param does not suppress it.
10. **Inline markdown** per block, except in code-block content [I: T§6]. These are naive non-greedy regexes that ignore HTML context
    (they run inside attributes, `<code>` and URLs):

    ```python
    ESC = {"*": "\x00E1", "[": "\x00E2", "]": "\x00E3", "`": "\x00E4", "(": "\x00E5", ")": "\x00E6"}
    s = re.sub(r"\\([*\[\]`()])", lambda m: ESC[m[1]], s)      # only these are escapable; others keep the backslash
    s = re.sub(r"`(.*?)`",     r"<code>\1</code>", s)
    s = re.sub(r"\*\*(.*?)\*\*", r"<b>\1</b>", s)               # bold before italic -> ***x*** = <b><i>x</b></i>
    s = re.sub(r"\*(.*?)\*",   r"<i>\1</i>", s)                 # empty allowed -> **x = <i></i>x
    s = re.sub(r"~~(.*?)~~",   r"<s>\1</s>", s)
    s = re.sub(DATE_RE, time_elem, s)                           # date before link
    s = re.sub(r"\[(.*?)\]\((.*?)\)", r'<a href="\2">\1</a>', s) # lazy: breaks URLs that contain ')'
    for k, v in ESC.items(): s = s.replace(v, k)
    ```

    Not supported (left literal) [O]: `_x_`, `__x__`, `~x~`, autolinks and raw URLs, `![img]` (becomes `!<a…>`), `#tag`, `@mention`.
11. **Dates** [O: T§7, C§4.2]: `DATE_RE = \[(\d{4})-(\d{1,2})-(\d{1,2})(?: (\d{1,2}):(\d{2}))?\]`, converted only if it is a valid
    calendar date (hour < 24, minute < 60), and otherwise left literal:
    `<time startYear="2026" startMonth="1" startDay="15">Thu, Jan 15, 2026</time>`. With a time, add
    `startHour="14" startMinute="30"` and the text ` at 2:30 PM`. Numbers are unpadded in the attributes; the text format is
    `%a, %b {day}, {year}` plus ` at {h12}:{MM} {AM|PM}` (proleptic Gregorian). Seconds, `T`, AM/PM input and ranges are not
    recognised. Locale and timezone dependence is [U].

### 6.3 Create path: `note`

Stored **completely verbatim**: no control-character stripping, trimming, markdown, entity decoding, or child creation.
A **NUL** gives `500 {"code": "internal_error", "message": "An internal server error occurred"}` and creates nothing [O: T§2, T§8].
If absent or null, it is stored as `null`.

### 6.4 Note null vs ""

[O] A node created without a note has `note: null`. Multiline-split children and mirror shells get `""`. Update can set
`""` but cannot restore `null`.

### 6.5 What the server never does

[O: T, U] Sanitise or balance HTML, escape `< > &` on output, normalise Unicode, collapse whitespace, limit length, autolink,
or parse tags and mentions. GET and list return identical strings.

---

## 7. Priority algorithm

[O: C§2, M§1–2, M§7, F0, F1§7]

Priorities are **sparse integers scoped to one parent**; lower values display first. The same function serves create, move
(including same-parent moves) and mirror:

```python
def assign_priority(parent, position, exclude=None):
    # serialized; one transaction
    others = [n for n in children(parent) if n.id != exclude]   # includes mirror shells
    if not others:
        return 100
    if position == "bottom":
        return -(-(max(n.priority for n in others) + 100) // 100) * 100   # ceil((max+100)/100)*100
    # "top", null or absent
    m = min(n.priority for n in others)
    if m > 0:
        return m // 2
    # m == 0: full renumber; siblings' modifiedAt is NOT bumped
    for rank, n in enumerate(sorted(others, key=lambda n: (n.priority, n.create_seq))):  # tie-break [U]
        n.priority = 100 * (rank + 2)
    return 100
```

Evidence:
- top: 100→50→25→12→6→3→1→0, then a renumber (new=100, others 200, 300, …) [O]. Moving self to top with `[self=0, 100, 200]` gives 50 (self excluded) [O].
- bottom: 100/200/300 → 400; {0, 1, 2} → 200; {25, 50} → 200; {50} → 200; {150} → 300; {250} → 400; {0} → 100 [O].
- Nothing else rewrites sibling priorities: not delete (even of the first child), not move-out or move-in [O].
- Multiline-create children get **0, 1, 2, …** outside this function [O].
- The top-level scratchpad has priority 28, so the web UI must use a different algorithm [O/I]. See the UI scenarios.
- Upper overflow handling on bottom is [U] (we never exceeded about 1600).
- Equal priorities never arose through the API, even under concurrency [O: F0]. UI-produced ties are [U].

---

## 8. `modifiedAt` rules

[O: U§modifiedAt, M§5, D§2.4, D§3.5, D§4, F0, F1]

| operation | bumps |
|---|---|
| create | nothing besides the new node(s) (parent **not** bumped) |
| update (any successful request, even `{}` or a no-op) | the node |
| complete / uncomplete (even when redundant) | the node |
| move (cross-parent or same-parent) | the moved node **and the destination parent** (not the old parent, ancestors, descendants, or renumbered siblings) |
| move to self | the node only |
| delete | nothing (not the parent) |
| `POST /:id/mirror` | the **resolved origin** only (not the `:id` mirror if it differs, and not the destination, even when redirected) |
| `DELETE /:id/mirror` | the origin (if it still exists) |
| plain `DELETE` of a mirror shell | nothing |
| renumber (min==0 top insert) | nothing (siblings keep their `modifiedAt`) |
| any operation on or under a shell | never the origin |
| failed request (4xx) | nothing |

`createdAt` never changes [O]. `completedAt` is set by complete and on create via `- [x]` [O].

---

## 9. Structural invariants and anomalies

1. **Hard delete** of the subtree, immediately visible [O].
2. **Cycles via move** are possible in production (§4.7.5) [O].
3. **Orphan mirror shells** (origin deleted) persist and behave as ordinary nodes, except in mirror operations (§10.4) [O].
4. **Recursive mirrors**: mirroring O under O's own child or grandchild is allowed (200) [O: D§2.6, E§7].
   Moving O into its own mirror shell is also allowed (200) [O: D§3.4].
5. **Completed** nodes are never filtered, and completion never cascades [O].

---

## 10. Mirrors

### 10.1 Identity model

[O: D§5]

- A mirror is a **separate node ("shell")** with its own UUID, `createdAt`, `modifiedAt` and priority, and a hidden `mirror_origin_id`.
- A fresh shell has `name ""`, `note ""`, `layoutMode "bullets"`, `completed false` [O]. It does not copy the origin's layout or completion.
- **Through the API, nothing propagates**: edits to the origin do not show on the shell, and the reverse is also true (name, note, layout, completion). We re-checked
  minutes later, so this is not eventual consistency [O]. The upstream "live copy" wording is presumably true only in the UI [U].
- Listing a shell returns only the shell's own children (initially `[]`). Create and move into a shell put the node under
  the shell [O].
- `origin_id` is always a non-mirror node (chains cannot form through the API) [O/I].
- There is **no side-effect-free way** to detect a mirror via the API. `name == "" and note == ""` is only a heuristic [O: F1].

### 10.2 `POST /api/v1/nodes/:id/mirror`

[O: D§2, F1, F2] Check order: JSON parse → validation (`MirrorNodeRequest`, `parent_id` required and non-null, collect all errors) →

1. `src` = exact-UUID lookup of `:id`, otherwise `404 Item not found`.
2. `origin` = follow `mirror_origin_id` from `src`. If the origin no longer exists (an orphan), return `404 Item not found`.
3. `parent` = exact-UUID lookup of `parent_id`, otherwise `404 Parent item not found` (for every form of miss). If `parent` is a shell,
   use its origin. If that origin is gone (an orphan parent), return `404 Parent item not found`.
4. If `parent.id == origin.id`, return `400 {"code": "invalid_request", "message": "Cannot mirror a node under itself"}`.
   Descendants of the origin are **allowed**.
5. Create the shell with `priority = assign_priority(parent, position)` (default top) and `mirror_origin_id = origin.id`.
6. Bump `origin.modifiedAt` only.
7. Return `200 {"item_id": shell.id, "origin_id": origin.id}`.

Mirroring the same node twice into the same parent creates two shells [O].

### 10.3 `DELETE /api/v1/nodes/:id/mirror`

[O: D§4.1, F1]

- Exact UUID, otherwise `404 Item not found` (also when already removed).
- If the node is not a shell (a plain node, an origin, or a plain child of a shell): `400 {"code": "invalid_request", "message": "Node is not a mirror"}`.
- Otherwise delete the shell and its subtree. Bump the origin's `modifiedAt` if it exists. **Orphans succeed** (200).
- `200 {"status": "ok"}`.

### 10.4 Orphans

[O: D§4.3–4.4, F1] When an origin (or one of its ancestors) is deleted, its shells survive with their children and their `modifiedAt`
unchanged. An orphan can be moved, moved into, created under, updated, completed, listed and deleted like an ordinary node. As a mirror
source it gives `Item not found`. As a mirror parent it gives `Parent item not found`. `DELETE /mirror` on it returns 200.

---

## 11. Special locations, root, targets

[U] Everything in this section is untested because of the scope restriction. Per UP:

- `parent_id` keys: `None` (root), `inbox`, `calendar`, `today`, `tomorrow`, `next_week`, `YYYY`, `YYYY-MM`,
  `YYYY-MM-DD`, and user shortcut keys.
- Move and create "materialise" calendar nodes on demand. List with a not-yet-created calendar key gives 404.
- Suggested mock: a root sentinel (`parent_id NULL`), an `inbox` system node, lazily created calendar nodes, and a `targets` table.
  Mark all of it as unverified.

---

## 12. Mock implementation checklist

- [ ] SQLite schema per §1.1, including `create_seq` and `mirror_origin_id`. Hard delete via recursive CTE.
- [ ] Global write lock, one transaction per request. Injectable clock.
- [ ] Routing, 405 styles and HTML 404 per §2.1.
- [ ] Auth per §2.3 (configurable key list).
- [ ] Body parsing per §2.4. Pydantic-like validation per §2.5 (all errors, declaration order, `missing` input = body).
- [ ] Id resolution per §3 (GET-only short ids; parent resolver; mirror exact-only).
- [ ] Create text pipeline per §6.2, using the golden vectors in `experiments/create/*.out.json` and
      `experiments/text-content/results/*.json`.
- [ ] `assign_priority` per §7, shared by create, move and mirror.
- [ ] `modifiedAt` table per §8.
- [ ] Mirrors per §10. Cycle policy per §4.7.5 (reject by default, `faithful_cycles` flag).
- [ ] List ordered by `create_seq`. JSON serialised with `json.dumps` defaults and the node key order from §1.2.
- [ ] Untested areas (§11, empty or null `parent_id`, export, targets): implement plausibly and flag them as unverified.

---

## 13. Reconciled conflicts between area docs

| topic | conflict | resolution |
|---|---|---|
| Multiline join rule | C§5.1 regex `([^.!?\n])\n+([a-z])` vs T§5.1 "absorb at most one non-block-start line" | **Regex (C)**. It reproduces all of T's table *and* C's cases (`hello\nworld\nagain\nmore` → one node; `Hello world.\nthis` and `a\nB` split), which T's model gets wrong. |
| Entity decoding order | C§7 step 4 decodes *after* markdown; T§4.2 decodes *before* | **Before (T)**: `&#42;x&#42;` → `<i>x</i>` is observed. |
| Bottom formula | early docs said `max+100` | `ceil((max+100)/100)*100` [O: F0, F1]. |
| Move to self | E said "pure no-op" | Bumps the node's `modifiedAt` only [O: F2, M]. |
| List order | E originally said priority order | Creation order [O: F0, F2]. |
| URL parent miss message | E's early resolver said `Location` | `Parent item not found` for a URL carrying a 12-hex short id [O: F2]. |
| Create field order | C guessed `parent_id` first | `name, parent_id, note, layoutMode, position` [O: F2]. |
| Cycle policy | E: accept faithfully; M, D: reject | Reject by default, faithful behind a flag (§4.7.5). |
| Mirror error key order | older D tables printed keys sorted | Real order is `type, loc, msg, input` and `error, allowed_methods` [O]. |
