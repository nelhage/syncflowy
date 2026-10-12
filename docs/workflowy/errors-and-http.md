# Workflowy API: errors, id resolution, and HTTP-level behaviour

Area owner tag: `errors-http`. Experiments: `experiments/errors-http/exp*.py`
(with captured output in `experiments/errors-http/exp*.out`). Every request is
also in `.prod-state/requests.jsonl` with `"tag": "errors-http"` (~460 requests
total, all at < 9 req/s through `ProdClient`).

All facts below are **observed** against production on 2026-10-12 unless
marked *inferred*. "Deterministic" means re-run at least twice with the same
result.

---

## 0. TL;DR for the mock server

1. **Id resolution is endpoint-specific.**
   - `GET /nodes/:id` accepts an exact lowercase hyphenated UUID **or** a
     12-hex-char short id (case-insensitive). Nothing else.
   - All *mutating* path ids (`POST /nodes/:id`, `DELETE /nodes/:id`, `/move`,
     `/complete`, `/uncomplete`, `/mirror`, `DELETE .../mirror`) accept **only
     the exact UUID**. Short ids give `404 Item not found` (observed on all of
     them). Uppercase and dashless UUIDs of an existing node were observed to
     give 404 only on **update**. On the other mutating endpoints that is
     *inferred* (see §2.1 and §12).
   - `parent_id` in list / create / move accepts: exact UUID, short id
     (case-insensitive), or a workflowy URL (`https?://(beta.)?workflowy.com/#/<short>…`,
     case-insensitive). `mirror`'s `parent_id` accepts only the exact UUID.
   - Not-found messages for `parent_id` (list/create/move): a uuid-shaped
     value **or a well-formed URL with a 12-hex short id** that misses gives
     `Parent item not found`. Any other miss, including a *bare* 12-hex short
     id and a URL that embeds a full UUID, gives `Location '<raw>' not found`.
     (Corrected by followup-2, see §2.2.)
   - `parent_id` is **optional and nullable** in `CreateItemRequest` and
     `MoveNodeRequest`, but **required and non-null** in `MirrorNodeRequest`.
     `""` passes validation in all three. (followup-2)
2. **Four error body shapes** (all `application/json` unless noted):
   - `{"code": "not_found", "message": "<msg>"}` (404)
   - `{"code": "invalid_request", "message": "<msg>"}` (400, semantic)
   - `{"code": "invalid_json", "message": "Invalid JSON format in request body"}` (400)
   - `{"errors": [{"type", "loc", "msg", "input"}, ...]}` (400, pydantic-style validation)
   - 401: `{"errors": "Invalid Credentials, try again."}` for **GET**,
     `{"code": "invalid_credentials", "message": "Invalid Credentials, try again."}` for **POST/DELETE**.
   - 405: `{"error": "Method not allowed", "allowed_methods": [...]}` on
     `/nodes` and `/nodes/:id` and `/nodes/:id/mirror`; **empty `text/html` body
     with `Allow: POST`** on `/move`, `/complete`, `/uncomplete`.
   - Unknown route: 404 `text/html` (full HTML page for GET, empty body for POST).
3. **JSON is serialised exactly like Python's `json.dumps(obj)`** with defaults:
   `", "` / `": "` separators, `ensure_ascii=True` (`ö` → `ö`, emoji as
   surrogate pairs `😀`), `/` not escaped, key order as documented below.
4. **Order of checks**: routing (404 HTML) → method (405) → auth (401) → body
   parse (400 invalid_json) → schema validation (400 errors[]) → id/parent
   lookup (404) → semantic checks (400 invalid_request).
5. **Read-your-writes is immediate**: create/move/rename/delete are visible to
   the very next GET/list (3/3 trials, no delay).
6. **List is depth-1**, includes completed nodes, ignores all pagination
   params (`limit`, `offset`, `cursor`, `page`, `per_page`, `depth`, ...).
7. **DANGER (prod bug):** `POST /nodes/:id/move` with a `parent_id` that is a
   *descendant* of `:id` returns `200 {"status": "ok"}` and creates a parent
   cycle, detaching the subtree from the outline. Moving a node under itself
   returns `200` and changes nothing **except bumping the node's
   `modifiedAt`** (followup-2 re-checked this twice). See §7.
8. **List order is creation order, not priority order** (§8). Clients must
   sort by `priority` themselves.
9. **`MoveNodeRequest.parent_id` is optional.** `{}` and `{"parent_id": null}`
   pass validation. On a real node they would presumably move it to the
   account root (untested, out of scope). A sync engine must always send
   `parent_id` on move.

---

## 1. HTTP basics

| Property | Observed |
|---|---|
| Base URL | `https://workflowy.com/api/v1/` |
| Server header | `server: nginx` |
| Content type of every API JSON response | `content-type: application/json` (no charset) |
| Compression | gzip when client sends `Accept-Encoding: gzip` and body is large enough; small bodies (e.g. `{"status": "ok"}`, errors) come back uncompressed with `content-length`. With `Accept-Encoding: identity`, `content-length` is always present. |
| Rate-limit headers | **None** observed on any response (no `x-ratelimit-*`, no `retry-after`). No 429 was ever triggered at ≤ 9 req/s. |
| Caching headers | **None** (`cache-control`, `etag`, `last-modified`, `expires` absent). No conditional request support observed. |
| CORS | No `access-control-*` headers; `OPTIONS` → 405 (see §5). |
| Other headers (always) | `date`, `connection: keep-alive`, `content-security-policy: … 'unsafe-inline' 'unsafe-eval'…` (directive order varies between responses), `referrer-policy: same-origin`, `service-worker-allowed: /`, `vary: Accept-Encoding, Cookie, Origin`, `x-content-type-options: nosniff`, `x-frame-options: SAMEORIGIN`, `x-xss-protection: 1; mode=block`, `strict-transport-security: max-age=63072000; includeSubDomains; preload` |
| Cookies | The first response on a fresh connection/cookie-less client carries `set-cookie: _bess=…; Path=/, did=<uuid>; Domain=.workflowy.com; …HttpOnly; Max-Age=31536000; …; SameSite=Lax; Secure`. Irrelevant to API semantics. |
| `Accept` header | Ignored: `Accept: text/html` or `application/xml` still returns JSON 200. |
| Trailing slash | Accepted everywhere tried: `GET /nodes/` (=list), `POST /nodes/` (=create), `GET /nodes/:id/`, `POST /nodes/:id/` (=update), `POST /nodes/:id/complete/`. No redirect. |
| Repeated query param | `?parent_id=A&parent_id=B` → **last one wins** (lists B's children). |
| Query params on POST | Ignored. `POST /nodes/:id?note=x` with body `{}` does not change note; `POST /nodes?name=q` with body name `b` creates `b`. |

### Latency (observed, n≈450, from a residential US connection)

| Operation | median | p90 |
|---|---|---|
| GET node / list (200) | 145–150 ms | ~160 ms |
| Any 4xx error | 140–147 ms | ~150 ms |
| Create (200) | 200 ms | ~440 ms |
| Update / move / complete (200) | 175–190 ms | ~210 ms |
| Delete (200) | 200 ms | — |

First request on a new connection adds ~230 ms (TLS). Writes are
~40–50 ms slower than reads/errors (*inferred*: a persistence step).

---

## 2. Node id formats and resolution

Test node `T` = `32e0beb1-dda6-434c-bd35-bc4527011317`, short id `bc4527011317`
(experiment `exp01_id_formats.py`, `exp02_short_ids_other_endpoints.py`).

### 2.1 Path id (`/api/v1/nodes/<id>…`)

| Spelling | GET `/nodes/:id` | POST `/nodes/:id` (update) | DELETE, `/move`, `/complete`, `/uncomplete`, `/mirror` |
|---|---|---|---|
| full lowercase UUID | 200 | 200 | 200 |
| full UPPERCASE UUID | 404 `Item not found` | 404 `Item not found` | **not tested** for an existing node. *Inferred* 404 (same lookup). See §12 |
| UUID without dashes (either case) | 404 `Item not found` | 404 `Item not found` | **not tested** for an existing node |
| 12-char short id, lowercase | **200** | **404 `Item not found`** | **404 `Item not found`** (all five tested) |
| 12-char short id, UPPERCASE | **200** | 404 | — |
| 11 / 13 trailing chars, last 8, first 12 chars | 404 `Item not found` | — | — |
| URL `https://workflowy.com/#/<short>` percent-encoded into the path | **404 text/html** (the `%2F` decodes to `/` before routing, so it is an unknown route). GET → HTML page, POST → empty body | 404 text/html empty | — |
| garbage (`garbage-zq9x`, `12345`, `{uuid}`, `uuid+x`, truncated uuid, `zzzzzzzzzzzz`, `nöde`, `a b`, 300×`a`) | 404 `Item not found` | 404 `Item not found` | 404 `Item not found` (DELETE tested) |
| `..` | httpx normalises to `/api/v1` → 404 HTML | | |

Response body for every JSON 404 above (byte-exact):

```
{"code": "not_found", "message": "Item not found"}
```

So **GET is the only endpoint that resolves short ids in the path.** The
returned `node.id` is always the canonical lowercase UUID.

### 2.2 `parent_id` (list query param; create/move body)

| Spelling | list `?parent_id=` | create body | move body | mirror body |
|---|---|---|---|---|
| full lowercase UUID | 200 | 200 | 200 | 200 |
| full UPPERCASE UUID | 404 `Parent item not found` | 404 `Parent item not found` | 404 `Parent item not found` | 404 `Parent item not found` |
| no-dash UUID | 404 `Location '<x>' not found` | 404 `Location '<x>' not found` | — | — |
| short id (lower/UPPER) | 200 | 200 | 200 | **404 `Parent item not found`** |
| `https://workflowy.com/#/<short>` | 200 | 200 | 200 | **404 `Parent item not found`** |
| `https://workflowy.com/#/<SHORT>` (upper short) | 200 | — | — | — |
| `http://workflowy.com/#/<short>` | 200 | 200 | — | — |
| `https://WORKFLOWY.COM/#/<short>` | 200 | — | — | — |
| `https://beta.workflowy.com/#/<short>` | 200 | — | — | — |
| `https://workflowy.com/#/<short>/` (trailing slash) | 200 | 200 | — | — |
| `https://workflowy.com/#/<short>?q=x` | 200 | — | — | — |
| `https://workflowy.com/#/<shortA>/<shortB>` | 200, resolves to **shortA** (first segment) | — | — | — |
| `https://www.workflowy.com/#/<short>` | 404 Location | — | — | — |
| `https://app.workflowy.com/#/<short>` | 404 Location | — | — | — |
| `https://evil.example.com/#/<short>` | 404 Location | — | — | — |
| `workflowy.com/#/<short>` (no scheme) | 404 Location | — | — | — |
| `#/<short>` | 404 Location | — | — | — |
| `https://workflowy.com#/<short>` (no `/` before `#`) | 404 Location | — | — | — |
| `https://workflowy.com/?q=1#/<short>` | 404 Location | — | — | — |
| `https://workflowy.com/#/x/<short>` | 404 Location | — | — | — |
| short id with surrounding spaces / UUID with spaces | 404 Location (no trimming) | — | — | — |
| 11/13/8 chars, first 12 chars of UUID, `zzzzzzzzzzzz`, garbage, 300 chars | 404 Location | 404 Location (create, `zzzzzzzzzzzz`, `garbage-zq9x`, `ffffffffffff`) | 404 Location (`garbage-zq9x`) | 404 `Parent item not found` |
| number `123456789` | — | 400 `string_type` | 400 `string_type` | — |
| array `[uuid]` | — | 400 `string_type` | — | — |
| `https://workflowy.com/#/<random 12 hex>` (lower or UPPER) | 404 **`Parent item not found`** | 404 **`Parent item not found`** | 404 **`Parent item not found`** | 404 `Parent item not found` |
| `https://workflowy.com/#/<full UUID>` (random, dashless random, **or the UUID of an existing sandbox**) | 404 `Location '<raw url>' not found` | 404 Location | 404 Location | 404 `Parent item not found` |
| random 12 hex, bare | 404 `Location '<x>' not found` | 404 Location | 404 Location | 404 `Parent item not found` |
| random UUID, UPPERCASE | 404 `Parent item not found` | 404 `Parent item not found` | 404 `Parent item not found` | 404 `Parent item not found` |
| random UUID, no dashes | 404 Location | 404 Location | 404 Location | 404 `Parent item not found` |
| omitted | — (untested) | **passes validation**; resolution untested | **passes validation** (random `:id` gives `Item not found`); resolution untested | 400 `missing`, `loc:["parent_id"]`, `input` = whole body |
| `null` | — (untested) | passes validation; resolution untested | passes validation; resolution untested | 400 `string_type`, `input: null` |
| `""` | — (untested) | passes validation; resolution untested | passes validation; resolution untested | passes validation (random `:id` gives `Item not found`) |
| `None`, `inbox`, calendar keys, shortcut keys | **untested** (would resolve outside the scratchpad / to the root) | | | |

The rows for the URL forms, the bare random short id, the uppercase or dashless
random UUID, and `null`/omitted/`""` were added by followup-2
(`experiments/followup-2/01_validation_and_parent_resolution.py`). The URL
rows were repeated with two different random short ids. For move and mirror,
`:id` was a real node in our sandbox and the parent could not exist. The node
was confirmed unmoved afterwards.

Exact not-found messages (byte-exact bodies):

```
{"code": "not_found", "message": "Parent item not found"}
{"code": "not_found", "message": "Location 'garbage-zq9x' not found"}
```

The `Location '…'` message echoes the raw input inside single quotes, with no
escaping of quotes beyond normal JSON escaping, non-ASCII as `\uXXXX`:
`{"code": "not_found", "message": "Location 'nöde-zq9x' not found"}`.

**Inferred resolution algorithm for `parent_id`** (consistent with every row
above; revised by followup-2. The earlier version wrongly sent URL misses to
the `Location` message):

```python
UUID_RE  = r'^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$'  # re.I
URL_RE   = r'^https?://(beta\.)?workflowy\.com/#/([0-9a-f]{12})'             # re.I, prefix match
SHORT_RE = r'^[0-9a-f]{12}$'                                                  # re.I

def resolve_parent(s):
    # (s is None / "" / omitted: passes validation; behaviour untested, probably root)
    if re.match(UUID_RE, s, re.I):            # uuid-shaped (any case)
        node = nodes_by_exact_id.get(s)       # exact, case-sensitive lookup
        return node or 404("Parent item not found")
    m = re.match(URL_RE, s, re.I)
    if m:                                     # URL with a 12-hex short id after #/
        node = node whose id (dashes removed) ends with m.group(2).lower()
        return node or 404("Parent item not found")
    if s in SPECIAL_KEYS (None, inbox, calendar, today, ..., YYYY[-MM[-DD]]): ...  # untested
    if s is a user shortcut key: ...                                           # untested
    if re.match(SHORT_RE, s, re.I):
        node = node whose id (dashes removed) ends with s.lower()
        if node: return node
    return 404(f"Location '{s}' not found")
```

Notes:
- A *bare* short id that names a nonexistent node gives the `Location '…'`
  message (e.g. `ffffffffffff` gives `Location 'ffffffffffff' not found`).
  The same short id wrapped in `https://workflowy.com/#/…` gives
  `Parent item not found` (observed on list, create and move, with two
  different random short ids).
- A URL whose fragment is a full UUID (with or without dashes) is **not**
  understood, even when the UUID exists. `https://workflowy.com/#/<sandbox UUID>`
  gave `Location '<url>' not found` on list. URL_RE needs exactly 12 hex
  characters and then something that is not a hex digit (or the end). Since
  `#/<uuid>` starts with 8 hex digits and a dash, it fails the regex.
- Whether URL_RE checks the character after the 12 hex digits is *inferred*.
  `#/<shortA>/<shortB>`, `/`, and `?q=x` suffixes were accepted. A 32-hex
  dashless UUID after `#/` was rejected, which suggests a boundary check (or
  an exact 12-char segment).
- Order inside the resolver between the URL check and the special keys
  cannot be told apart by any observation we have.

For **mirror**, `parent_id` is looked up by exact id only; any miss (short
id, URL, URL with a full UUID, uppercase, dashless, garbage) → `Parent item not found`
(followup-2 re-confirmed every form).

For **GET path id**: exact UUID lookup; else if 12 hex chars (any case) →
short-id lookup; else 404 `Item not found`. Note uppercase full UUID does *not*
fall through to short-id resolution.

---

## 3. Nonexistent ids on every endpoint

Random v4 UUIDs (`exp03_nonexistent_and_malformed.py`). All deterministic.

| Request | Status | Body |
|---|---|---|
| `GET /nodes/<rand>` | 404 | `{"code": "not_found", "message": "Item not found"}` |
| `GET /nodes?parent_id=<rand>` | 404 | `{"code": "not_found", "message": "Parent item not found"}` |
| `POST /nodes {"parent_id": <rand>, "name": "x"}` | 404 | `{"code": "not_found", "message": "Parent item not found"}` |
| `POST /nodes/<rand> {"name": "x"}` | 404 | `{"code": "not_found", "message": "Item not found"}` |
| `POST /nodes/<rand> {}` | 404 | `Item not found` |
| `DELETE /nodes/<rand>` | 404 | `Item not found` |
| `DELETE` an already-deleted real node | 404 | `Item not found` |
| `GET` a just-deleted node | 404 | `Item not found` |
| `POST /nodes/<rand>/move {"parent_id": <real>}` | 404 | `Item not found` |
| `POST /nodes/<real>/move {"parent_id": <rand>}` | 404 | `Parent item not found` |
| `POST /nodes/<rand>/move {"parent_id": <rand>}` | 404 | `Item not found` (item checked before parent) |
| `POST /nodes/<rand>/complete` | 404 | `Item not found` |
| `POST /nodes/<rand>/uncomplete` | 404 | `Item not found` |
| `POST /nodes/<rand>/mirror {"parent_id": <real>}` | 404 | `Item not found` |
| `POST /nodes/<real>/mirror {"parent_id": <rand>}` | 404 | `Parent item not found` |
| `DELETE /nodes/<rand>/mirror` | 404 | `Item not found` |
| `DELETE /nodes/<real non-mirror>/mirror` | **400** | `{"code": "invalid_request", "message": "Node is not a mirror"}` (node untouched) |
| `POST /nodes/<real>/mirror {"parent_id": <real itself>}` | **400** | `{"code": "invalid_request", "message": "Cannot mirror a node under itself"}` |
| `POST /nodes/<rand>/mirror {"parent_id": <rand>}` | 404 | `Item not found` (item checked before parent; followup-1) |
| `POST /nodes/<orphan mirror>/mirror {"parent_id": <rand>}` | 404 | `Item not found` (origin resolution comes before the parent lookup; followup-1) |
| `POST /nodes/<orphan mirror>/mirror {"parent_id": <orphan itself>}` | 404 | `Item not found`, not the self error (followup-1) |
| `POST /nodes/<real>/mirror {"parent_id": <orphan mirror>}` | 404 | `Parent item not found` (followup-1) |
| `DELETE /nodes/<plain child of a mirror shell>/mirror` | 400 | `Node is not a mirror` (followup-1) |
| any of the above + an invalid `position` | 400 | the `enum` validation error. Validation always wins (followup-1, followup-2) |

---

## 4. Authentication

`exp04_auth_methods_routes.py`, `exp06_precedence_list_timing.py`,
`exp07_misc_errors.py`. Probes targeted random UUIDs, so a "success" shows up
as `404 Item not found`.

| `Authorization` header | Result |
|---|---|
| absent (bare client) | 401 |
| `Bearer not-a-real-key-0000` | 401 |
| `Bearer` (no token) | 401 |
| `<key>` (no scheme) | 401 |
| `Basic dXNlcjpwYXNz` | 401 |
| `Bearer <key>` | accepted |
| `bearer <key>` (lowercase scheme) | **accepted** |
| `Token <key>` | **accepted** |
| `Bearer  <key>` (two spaces) | **accepted** |
| `Bearer ` (trailing space) | not sendable (httpx rejects illegal header) |

*Inferred*: the server splits the header on whitespace and uses the second
token as the key, ignoring the scheme word; a single token is rejected.
(`Basic …` presumably fails only because its token is not a valid key.)

**401 body depends on HTTP method** (deterministic, byte-exact):

| Method / endpoint | Body | content-length |
|---|---|---|
| GET `/nodes/:id`, GET `/nodes`, GET `/nodes-export`, GET `/targets` | `{"errors": "Invalid Credentials, try again."}` | 45 |
| POST `/nodes`, POST `/nodes/:id`, DELETE `/nodes/:id`, POST `…/move`, POST `…/complete`, DELETE `…/mirror` | `{"code": "invalid_credentials", "message": "Invalid Credentials, try again."}` | 77 |

No `WWW-Authenticate` header.

Precedence involving auth:
- Unknown route without auth → 404 HTML (routing first).
- Wrong method with bad auth → 405 (method check before auth), for both the
  JSON-405 and HTML-405 style routes.
- Invalid JSON body with bad auth → 401 (auth before body parsing).
- Nonexistent id with bad auth → 401 (auth before lookup).

---

## 5. Methods and routes

| Request | Status | content-type | Body / headers |
|---|---|---|---|
| `PUT`/`PATCH /nodes/:id` | 405 | application/json | `{"error": "Method not allowed", "allowed_methods": ["GET", "POST", "DELETE"]}` |
| `HEAD /nodes/:id` | 405 | application/json | (no body, content-length 77) |
| `OPTIONS /nodes/:id` | 405 | application/json | same as PUT |
| `PUT`/`PATCH`/`DELETE /nodes` | 405 | application/json | `{"error": "Method not allowed", "allowed_methods": ["GET", "POST"]}` |
| `HEAD /nodes?parent_id=` | 405 | application/json | (no body, content-length 67) |
| `GET /nodes/:id/mirror` | 405 | application/json | `{"error": "Method not allowed", "allowed_methods": ["POST", "DELETE"]}` |
| `GET`/`DELETE /nodes/:id/move`, `GET /nodes/:id/complete`, `GET /nodes/:id/uncomplete`, `PUT /nodes/:id/complete` | 405 | `text/html; charset=utf-8` | empty body, `content-length: 0`, **`allow: POST`** |
| `POST /targets` (bad key) | 405 | `text/html; charset=utf-8` | empty, `allow: GET, HEAD` |
| `GET /api/v1/foo`, `/api/v1/`, `/api/v1`, `/api/v2/nodes`, `/api/v1/me`, `/api/v1/user`, `/nodes/:id/children`, `/nodes/:id/foo` | 404 | `text/html; charset=utf-8` | full HTML "404" page (`<!DOCTYPE html>…<title>404</title>…`) |
| `POST /api/v1/foo`, `POST /nodes/:id/mirror/x`, `POST /nodes/<url-with-%2F>` | 404 | `text/html; charset=utf-8` | empty body, `content-length: 0` |

The 405 `allowed_methods` arrays and their order are exactly as shown. The JSON
405s have no `Allow` header; the HTML 405s have one (two different frameworks
— *inferred*: the `/nodes`, `/nodes/:id`, `/nodes/:id/mirror` routes are
custom dispatchers; `/move`, `/complete`, `/uncomplete`, `/targets` use
Django's `require_http_methods`-style decorators).

Wrong-method probes did not modify the node.

---

## 6. Request bodies

`exp05_bodies_and_types.py`, `exp06_precedence_list_timing.py`.

### 6.1 Encodings (tested on update, then create where safe)

| Body | Content-Type | Result |
|---|---|---|
| `{"note": "…"}` | `application/json` | 200, applied |
| `{"note": "…"}` | `application/json; charset=utf-8` | 200, applied |
| `{"note": "…"}` | `text/plain` | 200, applied |
| `{"note": "…"}` | empty `Content-Type:` | 200, applied |
| `note=form-encoded-note` | `application/x-www-form-urlencoded` | 200, **applied** |
| `parent_id=<sb>&name=form-created` | `application/x-www-form-urlencoded` (create) | 200, node created |
| multipart form with `note` field | `multipart/form-data` | 200, **applied** |
| `{"note": ` (truncated) | application/json | 400 `{"code": "invalid_json", "message": "Invalid JSON format in request body"}` |
| empty body | application/json | 400 `invalid_json` (same body) |
| `[{"note": "array"}]` | application/json | 400 `{"errors": [{"type": "model_type", "loc": [], "msg": "Input should be a valid dictionary or instance of UpdateNodeRequest", "input": [{"note": "array"}]}]}` |
| `"hello"` | application/json | 400 same shape, `"input": "hello"` |
| `null` | application/json | 400 same shape, `"input": null` |

*Inferred*: the server tries form/multipart parsing when the content type says
so, otherwise parses the raw body as JSON regardless of content type. The
model class names leak into `model_type` messages. All four are **observed**
(followup-2 sent `[1]`, `"s"` and `null` to each):

| Endpoint | Model name |
|---|---|
| `POST /nodes` (create) | `CreateItemRequest` (note: *Item*, not Node) |
| `POST /nodes/:id` (update) | `UpdateNodeRequest` |
| `POST /nodes/:id/move` | `MoveNodeRequest` |
| `POST /nodes/:id/mirror` | `MirrorNodeRequest` |

An empty body on move or mirror gives 400 `invalid_json`, the same as on update.
(An earlier version of this doc guessed `CreateNodeRequest`. That was wrong.)

Note `POST /nodes/:id/complete` with no body at all works (200), so complete /
uncomplete don't parse a body. (Empty body on update → `invalid_json`.)

### 6.2 Validation errors (pydantic v2 style, but trimmed)

Format: `{"errors": [ {"type": T, "loc": [field], "msg": M, "input": <the offending value>} , ...]}`.
No `url`/`ctx` keys (unlike stock pydantic). All errors are reported at once,
in **field-declaration order**. The order of keys in the request body has no
effect (followup-2 sent the same fields in reversed key order and got
identical bodies). Declaration orders observed:

| Model | Declared (validated) fields, in order | Required |
|---|---|---|
| `CreateItemRequest` | `name, parent_id, note, layoutMode, position` | `name` only. `parent_id` is optional and nullable |
| `UpdateNodeRequest` | `name, note, layoutMode` | none (all nullable; `parent_id`, `position`, `completed`, `priority` are **not** type-checked, so they are not fields) |
| `MoveNodeRequest` | `parent_id, position` | none. `{}`, `{"parent_id": null}` and `{"parent_id": ""}` all pass validation |
| `MirrorNodeRequest` | `parent_id, position` | `parent_id`, non-null (`null` gives `string_type` with `input: null`; omitted gives `missing` with `input` = whole body). `""` passes |

Error item key order is always `type, loc, msg, input`.

| Input | type | msg |
|---|---|---|
| `name`/`note`/`layoutMode`/`parent_id` not a string (number, bool, list, object) | `string_type` | `Input should be a valid string` |
| create with `name: null` | `string_type` | `Input should be a valid string` (input `null`) |
| create without `name`, mirror without `parent_id` | `missing` | `Field required`. `input` is the **whole request object**, e.g. `{"parent_id": "…"}` |
| mirror `parent_id: null` | `string_type` | `Input should be a valid string` (input `null`) |
| `position` not `"top"`/`"bottom"` (`7`, `"middle"`, `"TOP"`) | `enum` | `Input should be 'top' or 'bottom'` |
| top-level not an object | `model_type`, `loc: []` | `Input should be a valid dictionary or instance of UpdateNodeRequest` |

Example (create with three bad fields):

```json
{"errors": [{"type": "string_type", "loc": ["name"], "msg": "Input should be a valid string", "input": 1}, {"type": "string_type", "loc": ["note"], "msg": "Input should be a valid string", "input": 2}, {"type": "enum", "loc": ["position"], "msg": "Input should be 'top' or 'bottom'", "input": 3}]}
```

Example (create, every field bad, body keys in reverse order
`{"position":"q","layoutMode":3,"note":2,"name":1,"parent_id":1}`; followup-2):

```json
{"errors": [{"type": "string_type", "loc": ["name"], "msg": "Input should be a valid string", "input": 1}, {"type": "string_type", "loc": ["parent_id"], "msg": "Input should be a valid string", "input": 1}, {"type": "string_type", "loc": ["note"], "msg": "Input should be a valid string", "input": 2}, {"type": "string_type", "loc": ["layoutMode"], "msg": "Input should be a valid string", "input": 3}, {"type": "enum", "loc": ["position"], "msg": "Input should be 'top' or 'bottom'", "input": "q"}]}
```

Example (create `{"parent_id": 1}`: a `missing` error and a `string_type` error together):

```json
{"errors": [{"type": "missing", "loc": ["name"], "msg": "Field required", "input": {"parent_id": 1}}, {"type": "string_type", "loc": ["parent_id"], "msg": "Input should be a valid string", "input": 1}]}
```

Accepted-but-noteworthy inputs:

| Input | Behaviour |
|---|---|
| create `position: null` | accepted, treated as default (`top`) |
| create `name: ""` | accepted, empty name |
| update `name: null` | 200, **no-op** (name unchanged; `modifiedAt` still bumped) |
| update `note: null` | 200, **no-op** (note unchanged — you cannot clear a note with null) |
| update `note: ""` | 200, note becomes `""` (not `null`) |
| `layoutMode: "nonsense"` (create or update) | **accepted and stored verbatim**: `data.layoutMode == "nonsense"` |
| unknown fields (`foo`, `unknown_field`, `id`, `priority`, `completed`, `parent_id` in update) | silently ignored. `completed: true` does not complete; `priority` does not set priority; `parent_id` in update does not move |
| update `{}` | 200 `{"status": "ok"}`, `modifiedAt` bumped |
| 100 kB and 1.1 MB note | 200, stored in full (no 413 at 1.1 MB; larger untested) |

### 6.3 Check precedence

Validation happens **before** existence checks:
- update nonexistent id + `name: 123` → 400 `string_type` (not 404)
- update nonexistent id + invalid JSON → 400 `invalid_json`
- create with nonexistent parent and no name → 400 `missing`
- create/move with nonexistent ids and bad `position` → 400 `enum`

Then item (path id) before parent (`parent_id`), then semantic checks
(`Cannot mirror a node under itself`, `Node is not a mirror`).

---

## 7. Move / mirror cycles (important)

`exp07_misc_errors.py`, `exp08_cycles_isolated.py` (each cycle repaired
immediately afterwards by moving the node back under its sandbox).

| Request | Response | Effect |
|---|---|---|
| move `Y` under `Y` | 200 `{"status": "ok"}` | parent and priority unchanged, but **`modifiedAt` is bumped** (followup-2 `02_move_to_self_mtime.py`, 2/2 trials, with `position: bottom`; ordering-and-move.md saw the same) |
| move `X` under its child `X1` | 200 `{"status": "ok"}` | **creates a cycle**: `X.parent_id = X1`, `X1.parent_id = X`. `X` disappears from its old parent's list; the subtree is unreachable from the root. Observed twice. |
| move `X` under its grandchild `X2` | 200 | 3-cycle `X → X2 → X1 → X` |
| move `X` back under the sandbox | 200 | repairs it; `X1`/`X2` keep their parent pointers and reappear under `X` |
| mirror `Z` under `Z` | 400 `{"code": "invalid_request", "message": "Cannot mirror a node under itself"}` | none |
| mirror `Z` under its child `Z1` | 200 `{"item_id": …, "origin_id": Z}` | creates a recursive mirror (a mirror of Z inside Z's own subtree). We removed both instances we created with `DELETE …/mirror` (200). |

**Mock implementation note:** the other docs (ordering-and-move.md,
delete-and-mirrors.md) recommend *rejecting* moves into a descendant. This doc
originally recommended accepting them faithfully. **The coordinator has to
choose.** Followup-2 suggests making faithful acceptance (cycle creation) the
behaviour behind a flag that defaults to rejecting. With rejection on, a sync
engine bug that issues such a move fails loudly in tests rather than silently
corrupting the mock tree. Either way, the sync engine must never issue such a
move, because production will corrupt the tree. If rejecting, there is no
observed error body to copy. Use something clearly mock-specific, e.g.
400 `{"code": "invalid_request", "message": "Cannot move a node under its own descendant"}`,
and document it as *not* production behaviour.

---

## 8. List semantics

| Question | Answer (observed) |
|---|---|
| Depth | **Depth-1 only.** Grandchildren never appear. |
| Completed children | **Included**, with `completed: true` and `completedAt`. |
| Ordering | **Not priority order.** Children come back in ascending `createdAt` order (*inferred*: creation order). Followup-2 checked every multi-node list response in `.prod-state/requests.jsonl`: 306 responses, 149 of them **not** sorted by priority, 0 not sorted by `createdAt`. A client must sort by `priority` itself. The mock should return creation order (keep an insertion sequence number for ties within one second). An earlier version of this row said "priority order". That was wrong: early observations happened to have priorities in creation order. followup-0 `05_list_order.py` re-confirmed with nodes moved in from other parents: a node created earlier elsewhere and moved in at bottom lists first, one created later and moved in at top lists last; same-second ties follow creation-request order (ordering-and-move.md §6). |
| Pagination | None. `limit`, `offset`, `cursor`, `page`, `per_page`, `depth`, `recursive`, `include_completed`, `completed` are all silently ignored. |
| Empty parent | `{"nodes": []}` |
| Response shape | `{"nodes": [<node>, …]}` |
| Without `parent_id` | **untested** (blocked by scope guard; probably lists the root). |

---

## 9. Response serialisation (byte-level)

- Python `json.dumps` defaults: separators `", "` and `": "`, `ensure_ascii=True`.
  E.g. name `nöde ☃ "quoted" </script> 😀` comes back as
  `"nöde ☃ \"quoted\" </script> 😀"`.
- Node key order: `id, name, note, parent_id, priority, completed, data, createdAt, modifiedAt, completedAt`
  (note: **differs from the upstream docs' example order**, which puts
  `parent_id` second). `data` is `{"layoutMode": "…"}`.
- Wrapper keys: `{"node": {...}}`, `{"nodes": [...]}`, `{"item_id": "…"}`,
  `{"item_id": "…", "origin_id": "…"}`, `{"status": "ok"}`.
- Timestamps are integer Unix seconds.

---

## 10. Incidental observations (other areas; verify with their owners)

- **Priority allocation on `position: top`** (default): new node gets
  `floor(first_sibling_priority / 2)`; observed sequence 100 → 50, 25, 12, 6,
  3, 1, 0. After further top-inserts (trigger *inferred*: first sibling at 0), the
  server **renumbers all siblings to 100, 200, 300, …** in display order
  (renumbering did not bump their `modifiedAt`). Bottom inserts get
  `ceil((max+100)/100)*100` on create (create.md §2.1), which is `max+100`
  only when `max` is a multiple of 100. The owner is create.md /
  ordering-and-move.md. An earlier version of this note said "last+100".
- A mirror node, read via GET, shows `"name": ""` and `"note": ""` (its own
  row; content lives on the origin).
- Every update (even no-op `{}` or `name: null`) bumps `modifiedAt`.

---

## 11. Mock server implementation notes

Precise behaviour to implement (in this order for each request):

1. **Routing** (paths with or without trailing `/`):
   - `/api/v1/nodes` — GET (list), POST (create)
   - `/api/v1/nodes/<id>` — GET, POST (update), DELETE
   - `/api/v1/nodes/<id>/move|complete|uncomplete` — POST
   - `/api/v1/nodes/<id>/mirror` — POST, DELETE
   - `/api/v1/nodes-export`, `/api/v1/targets` — GET
   - `<id>` is any single path segment (percent-decoded; `%2F` splits segments).
   - Anything else → 404 `text/html; charset=utf-8`: for GET a HTML page
     (`<!DOCTYPE html>…<title>404</title>…`), for other methods an empty body.
2. **Method check**:
   - `/nodes`, `/nodes/<id>`, `/nodes/<id>/mirror`: 405 JSON
     `{"error": "Method not allowed", "allowed_methods": [...]}` with lists
     `["GET", "POST"]`, `["GET", "POST", "DELETE"]`, `["POST", "DELETE"]`. HEAD
     and OPTIONS also 405 (HEAD with no body).
   - `/move`, `/complete`, `/uncomplete`: 405, `text/html; charset=utf-8`,
     empty body, `Allow: POST`. `/targets`: `Allow: GET, HEAD`.
3. **Auth**: header `Authorization` split on whitespace; need ≥ 2 tokens; the
   second token must equal a known API key (scheme word ignored). Failure →
   401, body by method: GET → `{"errors": "Invalid Credentials, try again."}`;
   otherwise `{"code": "invalid_credentials", "message": "Invalid Credentials, try again."}`.
4. **Body parsing** (create/update/move/mirror only; complete/uncomplete and
   delete ignore the body): if content-type is form-urlencoded or multipart,
   parse as form fields; else `json.loads(raw)`; failure or empty →
   400 `{"code": "invalid_json", "message": "Invalid JSON format in request body"}`.
   Query string never contributes fields.
5. **Validation** (collect all errors, `{"errors": [...]}`, 400):
   - top-level must be a dict → `model_type`, `loc: []`,
     `msg: "Input should be a valid dictionary or instance of <Model>"`,
     `input: <body>`; Model = `CreateItemRequest` / `UpdateNodeRequest` /
     `MoveNodeRequest` / `MirrorNodeRequest` (all observed, §6.1).
   - Validate fields in the declaration order of §6.2, ignoring body key order,
     and collect every error.
   - string fields (`name`, `note`, `layoutMode`, `parent_id`): non-null
     non-string → `string_type`. For create, `name` is required and non-null
     (`missing` with `input` = whole body, or `string_type` with `input: null`).
     For mirror, `parent_id` is required and non-null in the same way. For
     create and move, `parent_id` is optional and nullable. For update,
     `null` means "not provided", and `parent_id`/`position` are not fields.
   - `position`: `null` or `"top"`/`"bottom"` (case-sensitive) else `enum` /
     `Input should be 'top' or 'bottom'`.
   - `layoutMode`: any string accepted and stored.
   - unknown keys ignored.
6. **Lookup** of path id: GET accepts exact UUID or 12-hex short id
   (case-insensitive suffix match on dash-stripped UUID); all other endpoints
   exact UUID only. Miss → 404 `{"code": "not_found", "message": "Item not found"}`.
7. **Lookup** of `parent_id` (list/create/move) per §2.2's algorithm; mirror:
   exact UUID only, then redirect a mirror parent to its origin; an orphan
   mirror parent → `Parent item not found` (followup-1). Mirror's `:id` is
   resolved to its origin *before* the parent lookup (orphan `:id` →
   `Item not found`). Misses → `Parent item not found` (uuid-shaped, a URL
   carrying a 12-hex short id, or any mirror miss) or `Location '<raw>' not found`
   (everything else, including a bare short id and a URL with a full UUID).
   For a missing, null, or `""` `parent_id` on create or move, production
   behaviour is unknown (probably the account root). A mock could treat it as
   root, but must not invent a validation error.
8. **Semantic errors**: `DELETE …/mirror` on non-mirror → 400
   `{"code": "invalid_request", "message": "Node is not a mirror"}`; mirror
   under itself → 400 `{"code": "invalid_request", "message": "Cannot mirror a node under itself"}`.
   Move under self → 200, nothing changes except the node's `modifiedAt`
   (bumped). Move under descendant → see §7 (coordinator decision: reject by
   default, faithful cycle creation behind a flag).
9. **Responses**: `application/json`, serialised with Python
   `json.dumps(obj)` defaults; node key order per §9.
10. **Consistency**: strictly read-your-writes; no eventual consistency needed.
11. Optional fidelity: include the static security headers listed in §1;
    no rate-limit or cache headers. (Rate limit behaviour — 429 shape —
    is unknown; if a mock simulates `nodes-export`'s 1/min limit, the body
    shape is a guess.)

---

## 12. Open questions

- Shape of a **429** response (and whether `retry-after` is sent); the
  `nodes-export` 1 req/min limit was not exercised (endpoint is out of scope).
- ~~Model names~~: resolved by followup-2 (§6.1).
- What `parent_id: null`, `""` or omitted *resolve to* on create and move.
  Followup-2 showed they pass validation. Does `""` mean root, or does it give
  `Location '' not found`? Also `"None"`, `inbox`, calendar keys and shortcut
  keys. All are out of scope for prod testing, because a passing request
  would execute outside the scratchpad.
- Whether `GET /nodes` without `parent_id` lists the root or errors.
- Short-id collisions: what happens if two nodes share the last 12 hex chars
  (cannot construct).
- Request size limit above 1.1 MB (nginx `client_max_body_size`?); max name
  length.
- Whether uppercase or dashless UUIDs of an **existing** node are rejected by
  complete/uncomplete/move/mirror/delete (*inferred* yes, the same exact lookup
  as update). This needs `AllowList.contains` in prod_client.py to normalise
  case and dashes so that the guard permits such spellings of sandbox ids. It
  is a coordinator decision, so followup-2 did not do it.
- Does a cyclic (detached) subtree appear in `nodes-export`? Does the web UI
  choke on it? (We repaired all cycles immediately.)
- Tie-break order in list when two siblings share a priority.
- Which "beta"-like hosts are accepted in URL parent_ids besides
  `beta.workflowy.com` (`www.` and `app.` are rejected).

---

## 13. UI scenarios (for a human to perform; we then observe via API)

1. **Copy a node's link in the UI** (right-click bullet → Copy link) and
   confirm its URL form; then pass that exact string as `parent_id` to list.
   (Does the UI ever produce `www.` or `/#/a/b` forms?)
2. **Move a node into its own child via drag-and-drop**: the UI should refuse.
   Compare with the API's acceptance (§7). Also: open the sandbox
   "errors-http: move/mirror cycles (repaired)" and confirm X/X1/X2 look sane
   after the API repair.
3. **Create a mirror of a node inside its own child** in the UI (Alt-drag or
   "mirror" command): does the UI allow it? We saw the API allow it.
4. **Set a custom layout via the UI** on the node in sandbox
   "errors-http: body encodings + field types" whose `layoutMode` is
   `"nonsense"`: how does the UI render a node with an unknown layoutMode,
   and what does the API report after the UI touches it?
5. **Open the node with a 1.1 MB note history** (sandbox "errors-http: misc
   errors…", node A1, note now reset to "size probes done") — check the UI
   handled the large note and nothing is stuck.
6. **Create a keyboard shortcut / jump-to key** (e.g. `zq`) pointing at a
   sandbox node; we then test `parent_id=zq` for list/create/move, and the
   resulting not-found message when the shortcut is deleted.
7. **Delete a node in the UI, then undo**: does the API show the same UUID
   again (resurrection) or a new id? What does GET return in between
   (we saw 404 `Item not found` for API deletes)?
8. **Complete a node in the UI** with "show completed" off: confirm list still
   returns it (we saw completed nodes included for API-completed nodes).

### Sandboxes left in place (all under the scratchpad)

- `sandbox: errors-http: id formats`
- `sandbox: errors-http: short ids on mutating endpoints`
- `sandbox: errors-http: nonexistent + malformed ids`
- `sandbox: errors-http: auth, methods, routes` (two copies; script re-run)
- `sandbox: errors-http: body encodings + field types`
- `sandbox: errors-http: precedence, list, timing` (two copies)
- `sandbox: errors-http: misc errors (cycles, not-found variants, size)`
- `sandbox: errors-http: move/mirror cycles (repaired)`
