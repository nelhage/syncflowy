# Update, complete and uncomplete: observed behavior

Covers `POST /api/v1/nodes/:id` (update), `POST /api/v1/nodes/:id/complete` and
`POST /api/v1/nodes/:id/uncomplete`, tested against production on 2026-10-12.

- Experiment scripts: `experiments/update-complete/01_*.py` … `10_*.py`. They
  can be re-run, and each one creates its own sandbox under the scratchpad.
- Every request and response is logged in `.prod-state/requests.jsonl` with
  `"tag": "update-complete"` (about 560 requests).
- Unless a statement is marked **(inferred)**, we saw it directly in a
  response.

## TL;DR

1. **Update stores `name` and `note` as sent, except that the C0 control
   characters `[\x00-\x08\x0b\x0c\x0e-\x1f]` are deleted** (observed in
   text-and-content.md §2, e.g. `"a\x01b\x07c\x7f"` is stored as `"abc\x7f"`). It does not parse
   markdown, decode or encode entities, sanitize HTML, check tags, trim
   whitespace or split newlines. This is very different from **create**, which
   parses markdown, decodes `&amp;`, sets layout from prefixes and joins
   single newlines (see the contrast table below).
2. **`layoutMode` is free-form.** Any string is accepted and stored as sent,
   including `"nonsense"`, `"H1"` and `""`. A non-string value gives 400.
   Unlike `name` and `note`, it is **not** control-character stripped:
   `"h\x01\x0b1"` is stored and returned unchanged (added 2026-10-12, conformance
   golden `tests/conformance/golden/update_layoutmode_control_chars.json`).
3. **In the update body, `null` means "leave this field unchanged"**, for
   `name`, `note` and `layoutMode`. There is no way to set `note` back to
   `null` through the API. `""` is the closest you can get.
4. **Unknown or read-only fields are silently ignored** (`parent_id`,
   `priority`, `completed`, `completedAt`, `createdAt`, `modifiedAt`, `id`,
   `position`, `data`, and any other key). You still get 200, and
   **modifiedAt is still bumped**.
5. **Every successful update, complete or uncomplete sets the node's
   `modifiedAt` to the current time**, even when nothing changed (no-op
   update, `{}`, unknown fields only, completing an already-completed node,
   uncompleting a never-completed node). A failed request (400 or 404) does
   not bump it.
6. **Complete is not timestamp-idempotent.** Completing an already-completed
   node sets `completedAt` to the new current time.
7. **Complete and uncomplete affect only the target node.** Children and
   other descendants are untouched, in both directions. Completed nodes still
   show up in list and get responses.
8. **Mutation endpoints accept only the exact full UUID (lowercase, with
   dashes).** A 12-character short id or a URL gives 404 on update, complete
   and uncomplete, while `GET /nodes/<short id>` works. For **update**, an
   uppercase UUID and a dashless UUID were also seen to give 404
   (errors-and-http.md §2.1). For complete and uncomplete the uppercase and
   dashless forms are **inferred** only: they were never sent with an
   existing node's id.
9. Request validation runs **before** the existence check. An invalid body
   sent to a nonexistent id gives 400, not 404.
10. A **mirror** node behaves, through this API, as an independent node. An
    update or complete sent through the mirror changes only the mirror's own
    record, and the origin's changes do not show on the mirror. A new mirror
    reads back as `name: ""` and `note: ""`. This needs UI confirmation.

## Endpoint summary

| Endpoint | Success | Body |
|---|---|---|
| `POST /api/v1/nodes/:id` | 200 `application/json` | `{"status": "ok"}` (16 bytes) |
| `POST /api/v1/nodes/:id/complete` | 200 | `{"status": "ok"}` |
| `POST /api/v1/nodes/:id/uncomplete` | 200 | `{"status": "ok"}` |

The response never echoes the node back. Changes are visible to an immediate
`GET /nodes/:id` and `GET /nodes?parent_id=` (read-your-writes held in every
attempt, including 5 rapid update, complete, list cycles with no delay; see
08). We saw no eventual-consistency lag.

Response headers on success are standard nginx/Django:

```
server: nginx
content-type: application/json
content-length: 16
content-security-policy: script-src * data: blob: 'unsafe-inline' 'unsafe-eval'; ...
referrer-policy: same-origin
service-worker-allowed: /
vary: Accept-Encoding, Cookie, Origin
x-content-type-options: nosniff
x-frame-options: SAMEORIGIN
x-xss-protection: 1; mode=block
strict-transport-security: max-age=63072000; includeSubDomains; preload
```

There are no rate-limit headers and no ETag or Last-Modified.

## Update: `POST /api/v1/nodes/:id`

### Partial update semantics (experiment 01, 02)

Fields that are left out stay unchanged. Starting from `name="orig name",
note="orig note"`:

| Request body | Status | Resulting `name` | Resulting `note` | modifiedAt bumped? |
|---|---|---|---|---|
| `{"name":"new name"}` | 200 | `new name` | `orig note` | yes |
| `{"note":"new note"}` | 200 | unchanged | `new note` | yes |
| `{}` | 200 | unchanged | unchanged | **yes** |
| `{"note":""}` | 200 | unchanged | `""` (empty string, **not** null) | yes |
| `{"note":null}` | 200 | unchanged | **unchanged** (null is ignored) | **yes** |
| `{"name":""}` | 200 | `""` (empty names allowed) | unchanged | yes |
| `{"name":null}` | 200 | **unchanged** | unchanged | **yes** |
| `{"name":"both","note":"both note"}` | 200 | `both` | `both note` | yes |
| `{"name":"subject"}` where name is already `subject` | 200 | same | same | **yes** |

A node created without a note has `note: null`. After `note: ""`, it reads
back as `""`. **(inferred)** Once a node has a note, the API cannot turn it
back into `null`.

### Type validation (experiment 01, 05)

Validation errors use a pydantic-style shape (the model is named
`UpdateNodeRequest`):

```
POST /api/v1/nodes/<id>  {"name": 123}
→ 400 application/json
{"errors": [{"type": "string_type", "loc": ["name"], "msg": "Input should be a valid string", "input": 123}]}
```

| Body | Status | Response |
|---|---|---|
| `{"name":123}` / `{"name":true}` / `{"name":["a"]}` | 400 | `string_type`, `loc:["name"]`, `input` echoes the value |
| `{"note":456}` | 400 | `string_type`, `loc:["note"]` |
| `{"layoutMode":1}` / `true` / `["todo"]` | 400 | `string_type`, `loc:["layoutMode"]` |
| JSON `[1,2]` | 400 | `{"errors":[{"type":"model_type","loc":[],"msg":"Input should be a valid dictionary or instance of UpdateNodeRequest","input":[1,2]}]}` |
| JSON `null` | 400 | same `model_type` error, `"input": null` |
| JSON `"x"` | 400 | same `model_type` error, `"input": "x"` |
| empty body (no bytes) | 400 | `{"code":"invalid_json","message":"Invalid JSON format in request body"}` |
| `{not json` | 400 | `{"code":"invalid_json","message":"Invalid JSON format in request body"}` |

Note that the two error formats differ: `{"errors":[...]}` versus
`{"code":..., "message":...}`. We did not test whether strict pydantic mode
rejects numeric strings such as `"123"` for name (that would be fine
anyway).

**Multiple errors are all listed, in field-declaration order** (followup-2,
`experiments/followup-2/01_validation_and_parent_resolution.py`, repeated
twice). Both `{"name":1,"note":2,"layoutMode":3}` and
`{"layoutMode":3,"note":2,"name":1}` give, byte for byte:

```
{"errors": [{"type": "string_type", "loc": ["name"], "msg": "Input should be a valid string", "input": 1}, {"type": "string_type", "loc": ["note"], "msg": "Input should be a valid string", "input": 2}, {"type": "string_type", "loc": ["layoutMode"], "msg": "Input should be a valid string", "input": 3}]}
```

So the order is `name, note, layoutMode` and does not depend on the order of
keys in the body. Other keys are **not type-checked** on update.
`{"parent_id":1,"position":"q","completed":"x","priority":"z"}` sent to a
random id gives 404 `Item not found`, so it passed validation.
`{"name":1,"position":"q"}` reports only the `name` error. `UpdateNodeRequest`
has no `position` or `parent_id` field.

A failed (400) update **does not** bump modifiedAt and changes nothing.

### Read-only and unknown fields (experiment 05)

Each of these bodies returned `200 {"status":"ok"}` and **changed nothing
except modifiedAt**:

```
{"parent_id": "<another valid node id>"}   # does NOT move the node
{"priority": 5}  {"priority": "x"}          # ignored, no type check
{"completed": true}  {"completed": "yes"}   # does NOT complete
{"completed": false}  (on a completed node) # does NOT uncomplete
{"completedAt": 1000}  {"createdAt": 1000}  {"modifiedAt": 1000}
{"id": "00000000-0000-0000-0000-000000000000"}
{"position": "bottom"}
{"foo": 1}
{"name": "with extra", "bogus": {"nested": [1]}}   # name applied, bogus ignored
{"name": "Name-case", "Name": "capital"}           # keys are case-sensitive; "Name" ignored
{"layout_mode": "h2"}  {"layoutmode": "todo"}       # wrong spellings ignored
{"data": {"layoutMode": "h3"}}                      # nested form is NOT accepted
```

Because unknown fields are not type-checked, `{"priority":"x"}` still
returns 200.

### `layoutMode` (experiment 05)

Every string we tried was accepted and is returned as sent in
`data.layoutMode`:

| Input | Status | Stored `data.layoutMode` |
|---|---|---|
| `todo`, `h1`, `h2`, `h3`, `code-block`, `quote-block`, `bullets` | 200 | as given |
| `board`, `p`, `table`, `dashboard` (not in docs; possibly real UI modes) | 200 | as given |
| `H1`, `Todo` (wrong case) | 200 | `H1`, `Todo` (no normalization) |
| `""` | 200 | `""` |
| `nonsense` | 200 | `nonsense` |
| `null` | 200 | unchanged |
| `1`, `true`, `["todo"]` | 400 | unchanged |

`data` contained only `layoutMode` in every response we observed. Changing
`layoutMode` does not affect completion: a completed `todo` changed to
`bullets` or `h1` stays `completed: true` with the same `completedAt`.

### Content handling of `name` and `note` on update (experiment 03)

For every case below, **for both `name` and `note`**, the output of
`GET /nodes/:id` was byte-for-byte identical to the input. Layout was not
changed, completion was not changed, and no children were created.

| Category | Inputs tested (all stored verbatim) |
|---|---|
| Markdown inline | `**bold md**`, `*italic md*`, `~~strike md~~`, `` `code md` ``, `[link md](https://example.com)`, `[2026-01-15]` |
| Markdown block prefixes | `# heading md`, `- [ ] todo md`, `- [x] done md` (does **not** complete), `> quote md`, ```` ```code block``` ```` |
| Supported HTML | `<b>bold</b>`, `<i>it</i> <s>s</s> <code>c</code>`, `<a href="https://example.com">link</a>` |
| Unsupported HTML | `<u>…</u>`, `<strong>`, `<em>`, `<span class="colored c-red">`, `<div>`, `<script>alert(1)</script>`, `<br>` |
| Malformed HTML | `<b>unclosed`, `</b>stray close`, `<B>upper</B>`, `<b class="x">attr</b>`, `<a href='…'>sq</a>`, `<a href="javascript:alert(1)">js</a>` |
| Bare specials | `a < b`, `a > b`, `a & b`, `it's "quoted"` |
| Entities | `a &amp; b`, `a &lt; b &gt; c`, `&nbsp;x&#39;y&quot;z` (not decoded) |
| Whitespace | `line1\nline2`, `para1\n\npara2` (no children made), `trailing space   `, `   leading space`, `tab\there` |
| Unicode | `emoji 😀 é` |
| Tags/mentions | `#tag @person` |
| Size | 5,000 / 10,000 / 100,000 / 1,000,000 chars of `y`: all accepted and stored in full (experiment 04) |

### Contrast: CREATE does transform the same inputs (experiment 04)

| Input to `POST /api/v1/nodes` | Stored name | layoutMode | completed |
|---|---|---|---|
| `**bold md**` | `<b>bold md</b>` | bullets | false |
| `a < b & c > d` | `a < b & c > d` | bullets | false |
| `a &amp; b` | **`a & b`** (entity decoded) | bullets | false |
| `<u>underline</u>` | `<u>underline</u>` | bullets | false |
| `<b>bold</b>` | `<b>bold</b>` | bullets | false |
| `# heading md` | `heading md` | **h1** | false |
| `- [x] done md` | `done md` | **todo** | **true** |
| `line1\nline2` | `line1 line2` | bullets | false |

So a sync engine that writes names should **always use update** for
round-trip fidelity, or treat create as lossy. One option is to create with a
placeholder and then update the name. **(inferred)**

### Other request encodings (experiment 01, 08)

| Request | Status | Effect |
|---|---|---|
| `Content-Type: application/x-www-form-urlencoded`, `name=formname` | 200 | name set to `formname` |
| form `name=form%20name&note=form+note&layoutMode=h2` | 200 | name `form name`, note `form note`, layout `h2` (standard form decoding) |
| JSON body with **no** Content-Type | 200 | parsed as JSON |
| JSON body with `Content-Type: text/plain` | 200 | parsed as JSON |
| duplicate keys `{"name":"first","name":"second"}` | 200 | `second` (last wins) |
| `?name=from-query` with body `{}` | 200 | query string ignored; name unchanged (modifiedAt bumped) |
| trailing slash `POST /api/v1/nodes/<id>/` | 200 | works like the version without the slash |
| `PATCH /api/v1/nodes/<id>` | 405 JSON | `{"error":"Method not allowed","allowed_methods":["GET","POST","DELETE"]}` |
| `PUT /api/v1/nodes/<id>` | 405 JSON | same |

**(inferred)** The server seems to dispatch on Content-Type: form-encoded is
parsed as a form, and anything else is tried as JSON.

### Id forms and missing nodes (experiment 07)

| Id form in path | GET | update | complete | uncomplete |
|---|---|---|---|---|
| full lowercase UUID | 200 | 200 | 200 | 200 |
| 12-hex short id | **200** | **404** JSON | **404** JSON | **404** JSON |
| uppercase UUID | n/t | 404 JSON | 404 JSON | 404 JSON |
| UUID without dashes | n/t | 404 JSON | 404 JSON | 404 JSON |
| URL `https://workflowy.com/#/<short>` (URL-encoded into path) | n/t | 404 **text/html, empty body** | 404 text/html (site 404 page) | 404 text/html |
| random UUID (cannot exist) | — | 404 | 404 | 404 |
| garbage `not-a-real-id` | — | 404 | 404 | 404 |
| random 12-hex | — | 404 | 404 | 404 |
| a node that was deleted | 404 | 404 | 404 | 404 |
| child of a deleted node | 404 | 404 | 404 | 404 |

The 404 JSON body is always:

```json
{"code": "not_found", "message": "Item not found"}
```

The short-id 404 was reproduced on a node 3 seconds old (to rule out
freshness) and on several nodes across experiments 06 and 07.

An update sent with an **invalid body to a nonexistent id gives 400** (the
`string_type` error), so body validation runs before the lookup.

Deleted nodes were 404 immediately after `DELETE` and still 404 3 seconds
later. Their descendants were also 404 at once.

## Complete / uncomplete (experiment 02, 06)

### Basics

- `POST /api/v1/nodes/:id/complete` → `200 {"status":"ok"}`. It sets
  `completed: true` and `completedAt = now` (Unix **seconds**, an integer,
  which matched the wall-clock floor in every check, for example wall
  1791834685.709 → completedAt 1791834685). It also sets `modifiedAt = now`.
- `POST /api/v1/nodes/:id/uncomplete` → `200`. It sets `completed: false`,
  `completedAt: null` and `modifiedAt = now`.
- It works the same for any layoutMode. `bullets`, `todo` and `h1` can all be
  completed. Completing does **not** change layoutMode.
- The request body is **ignored entirely**. `{"completed": false}` sent to
  `/complete` still completes the node. A body of `{garbage` (invalid JSON)
  sent to `/uncomplete` returns 200 and still uncompletes it. This differs
  from update, which rejects invalid JSON.
- Priority and sibling order are unchanged by completion. Siblings had
  priorities 100/200/300 before and after.

### Idempotency and timestamps

| Sequence | completedAt | modifiedAt |
|---|---|---|
| complete at t0 | t0 | t0 |
| complete again at t1 | **t1** (moved) | t1 |
| uncomplete at t2 | null | t2 |
| uncomplete again at t3 | null | **t3** (moved) |
| uncomplete a node never completed (t4) | null | **t4** |
| update name of a completed node (t5) | unchanged (t1) | t5 |

We reproduced this twice (experiments 02 and 06).

### Descendants (experiment 06)

The tree was: parent P (bullets) → child1, child2 (child2 completed
beforehand at 683) → grandchild under child1.

| Step | P | child1 | child2 | grandchild |
|---|---|---|---|---|
| complete P (685) | completed, 685 | **not completed** | completed, 683 (unchanged) | **not completed** |
| uncomplete P (689) | not completed | not completed | **still completed**, 683 | not completed |

Children's `modifiedAt` did not change either. List under the completed
parent returns both children (completed and not completed). List under the
sandbox returns the completed parent. **Completed nodes are never filtered
from the API.**

**(inferred / needs UI check)** The web UI probably shows the descendants of
a completed node as completed (greyed out or hidden) without storing that.
The API exposes only each node's own flag.

### Alternative routes and methods

| Request | Status | Body |
|---|---|---|
| `GET /api/v1/nodes/:id/complete` | 405 | **text/html, empty body** (not JSON, unlike `/nodes/:id`) |
| `DELETE /api/v1/nodes/:id/complete` | 405 | text/html, empty |
| `POST /api/v1/nodes/:id/completed` (typo) | 404 | text/html, Workflowy site 404 page (~10.9 KB) |
| `POST /api/v1/nodes/:id/toggle` | 404 | text/html site 404 page |
| update body `{"completed": true}` | 200 | ignored, does not complete |
| create with name `- [x] text` | 200 | creates `todo`, completed=true, completedAt=creation time; uncomplete works on it as normal |

## modifiedAt semantics (experiment 02)

Steps were spaced 2.2 s apart. Wall-clock time is shown alongside each step.

| Action on subject S | S.modifiedAt |
|---|---|
| no-op update (same name) | bumped |
| update `{}` | bumped |
| update `{"note":null}` / `{"name":null}` (ignored fields) | bumped |
| failed update `{"name":1}` (400) | **not** bumped |
| update `{"foo":1}` (unknown field only) | bumped |
| update to same layoutMode | bumped |
| complete / complete again / uncomplete / uncomplete again | bumped each time |
| **create a child under S** | **not** bumped |
| update a child's name | not bumped |
| complete a child | not bumped |
| move a child **out of** S | not bumped |
| move a child **into** S | **bumped** |
| delete a child of S | not bumped |
| move S to another parent | S bumped; **new parent bumped** too |

**(inferred)** modifiedAt is the server write time of the node's own record.
Moving a node into a parent touches the parent record, probably to update
its child list or order, while create and delete apparently do not. This
asymmetry is surprising and should be checked again by the move/create
investigators.

**followup-1 re-checked the asymmetry** with a 2.1 s gap before each action. It
held every time [observed]. A clean cross-parent move bumped exactly {moved node,
destination} in 3 of 3 trials. Move-out never bumped the old parent. Create
under a parent and mirror creation into a parent bumped nothing on that
parent, while a move-in to the same parent bumped it. A move-to-self bumped
only the node (2/2). Descendants of a moved node were never bumped. When the
parent is a **mirror shell**, the same rules apply to the shell, and the
mirror's origin is never bumped by updating, completing, uncompleting,
creating, deleting or moving children under the shell, or by updating,
completing or moving the shell itself (delete-and-mirrors.md §3.5).

`createdAt` never changed under any update or complete.

## Mirrors (experiment 08, 09; incidental, see the mirror doc for more)

Mirror M of origin A was created under C with
`POST /nodes/A/mirror {"parent_id": C}`, which returned
`{"item_id": M, "origin_id": A}`.

- Right after creation, `GET M` returned `name: ""`, `note: ""`,
  `completed: false` and `layoutMode: "bullets"`. A at that point was
  `name: "A"`, `note: null`, `completed: true`. **The mirror does not reflect
  the origin's content in the API.**
- `uncomplete M` set M to not completed. A stayed completed.
- `update M {name, note}` changed only M's own fields.
- `complete A` did not change M.
- `update M {layoutMode: h2}` did not change A.
- 20 seconds later the same divergence remained, so this is not eventual
  consistency.

Whether the **UI** shows M with A's content (and ignores M's own fields) is a
UI scenario, listed below.

## Mock server implementation notes

Pseudocode for the three endpoints:

```
POST /api/v1/nodes/{id}            (also accept trailing slash)
  body = parse_body(request):
      if content-type is application/x-www-form-urlencoded: form dict
      else: try json.loads(raw)   # regardless of content-type
            on failure (incl. empty body) -> 400 {"code":"invalid_json","message":"Invalid JSON format in request body"}
  if body is not a dict -> 400 {"errors":[{"type":"model_type","loc":[],
        "msg":"Input should be a valid dictionary or instance of UpdateNodeRequest","input":body}]}
  errors = []
  for field in ("name","note","layoutMode"):     # this order; collect ALL failures (followup-2)
      v = body.get(field)
      if v is not None and not isinstance(v, str):
          errors.append({"type":"string_type","loc":[field],
                         "msg":"Input should be a valid string","input":v})
  if errors -> 400 {"errors": errors}
  # no other key is type-checked (parent_id, position, completed, ... are ignored)
  node = lookup_exact_full_uuid(id)   # NO short ids, NO case folding, NO urls
  if node is None or node.deleted -> 404 {"code":"not_found","message":"Item not found"}
  for field in ("name","note","layoutMode"):
      if body.get(field) is not None:
          node[field] = re.sub(r'[\x00-\x08\x0b\x0c\x0e-\x1f]', '', body[field])
          # otherwise verbatim (text-and-content.md §2). Presumably the same for layoutMode (unverified).
  # ignore all other keys silently (parent_id, priority, completed, ...)
  node.modifiedAt = int(time.time())       # ALWAYS, even if nothing changed
  -> 200 {"status":"ok"}

POST /api/v1/nodes/{id}/complete
  ignore body completely (do not even parse it)
  node = lookup_exact_full_uuid(id) else 404 not_found JSON
  node.completed = True; node.completedAt = now; node.modifiedAt = now   # even if already completed
  -> 200 {"status":"ok"}

POST /api/v1/nodes/{id}/uncomplete
  same; completed=False; completedAt=None; modifiedAt=now (always)
```

Other details:

- Timestamps are integer Unix seconds (`int(time.time())`, which truncates).
  To make tests deterministic, let the mock take an injectable clock.
- `note` stays null until a string is written. `""` is stored as `""`. There
  is no way to go back to null.
- `name` may be `""`. There is no length limit (1 MB accepted).
- `layoutMode`: store any string. Do not validate against the enum. Store it
  in `data.layoutMode`. `data` contains only `layoutMode`.
- Completion is per-node. Never cascade to descendants. Never filter
  completed nodes out of list or get.
- Do not touch the parent's modifiedAt on update or complete of a child, or
  on create or delete of a child. **Do** bump the destination parent's
  modifiedAt (and the moved node's) on move. (Cross-checked by followup-0 and
  followup-1: old parent, descendants and other ancestors are never bumped;
  move-to-self bumps only the node. Updates and completes of a mirror shell
  or its children never bump the mirror's origin.)
- Unsupported methods on `/api/v1/nodes/{id}`: 405 JSON
  `{"error":"Method not allowed","allowed_methods":["GET","POST","DELETE"]}`.
  Unsupported methods on `/complete` and `/uncomplete`: 405 with an empty
  `text/html; charset=utf-8` body. Unknown sub-paths: 404 `text/html` (any
  HTML body; a mock can return a short HTML stub).
- A path id containing `%2F` or other URL-encoded URL characters: 404
  `text/html` with an empty body for update. **(inferred)** These do not
  match the route at all.
- Mirrors (pending confirmation from the mirror investigation): treat a
  mirror as its own record for update and complete. A new mirror starts with
  `name=""`, `note=""`, `completed=false`, `layoutMode="bullets"`.
- Duplicate JSON keys: last wins (Python `json.loads` default). Query-string
  parameters are ignored.

## Open questions

1. ~~Does a multiple-field validation failure list every error, or only the
   first?~~ **Resolved (followup-2):** every error is listed, in the order `name, note, layoutMode`.
2. Is `layoutMode` with a value outside the documented set (e.g. `board`,
   `table`, `p`, `dashboard`) a real UI mode? What does the UI do with
   `nonsense`, `""` or `H1`? See the UI scenarios.
3. When the UI edits a node, how does it encode `<`, `>`, `&` in `name`? It
   may store `&lt;` / `&amp;`, which would mean the API's verbatim storage
   creates names the UI would never produce.
4. Why does create bump nothing on the parent while move-in does? Does
   delete or move-out bump the parent's modifiedAt after some delay? (We
   checked about 2 s later: no. followup-1 re-confirmed the asymmetry with
   repeated, clean trials. The "why" is still open.)
5. Mirrors: does the API ever expose origin content through the mirror id?
   Should an update via the mirror id be considered "wrong" usage?
6. Is there server-side handling of `completedAt` for todo vs bullets in the
   UI (e.g. a completed bullet is hidden while a completed todo is shown with
   a check)? The API treats them identically.
7. Do update, complete or uncomplete work on the special keys (`inbox`,
   `today`, calendar dates) or shortcut keys as the path id? Untested (out of
   scope). Given that short ids fail, probably not.
8. Is the short-id 404 on mutation endpoints intentional? The upstream docs
   for update and complete only say "identifier", while the GET docs
   explicitly list short ids. Matching the observed behavior is the safe
   choice.
9. Does `uncomplete` with a malformed JSON body really skip parsing, or
   would a huge body fail? Minor.

## UI scenarios (for a human to perform; we then read via the API)

The fixtures are in sandbox **`sandbox: update-complete: 10 UI inspection
fixtures`** (id `df1de294-3283-493b-a85d-29da5138c2d4`, short
`29da5138c2d4`) unless noted otherwise.

1. **Rendering of raw specials (U1, U2).** Open U1 (`a < b & c > d`) and U2
   (`a &amp; b &lt;tag&gt;`) in the UI. Do they render literally or as
   decoded entities? Then make a trivial edit in the UI (add and remove a
   character) on each, and read the name via the API. Does the UI rewrite
   them to `&lt;`, `&amp;` and so on?
2. **Typing specials in the UI.** In a new node, type `a < b & c > d`, then
   `5 > 3`, then `AT&T`. Read via the API: are they stored as `&lt;`,
   `&amp;` or raw?
3. **Raw markdown via update (U3).** Does the UI render
   `**md bold** [md link](…)` literally? After a UI edit, does it convert
   them?
4. **Unsupported tags (U4).** How does the UI show `<u>`, `<strong>`,
   `<span class="colored c-red">` and `<script>`? After a UI edit, are any of
   them stripped or rewritten? Also: apply underline and a text colour/
   highlight in the UI to a fresh node and read back the HTML the API
   reports (`<u>`? `<span class="colored c-…">`?).
5. **Malformed HTML (U5 `<b>unclosed bold`).** What does the UI show? Does a
   UI edit normalize it to `<b>unclosed bold</b>`?
6. **Embedded newline (U6).** How does the UI display `line1\nline2` in a
   name? After a UI edit, is the `\n` kept? In the UI, type Shift+Enter in a
   note to make a multi-line note, then read the note to check how newlines
   are encoded.
7. **Formatting via UI.** Bold, italic, strikethrough, inline code and a
   link applied with keyboard shortcuts on a new node. Report the exact HTML
   the API returns (attribute order, quoting, `<a>` extras such as
   `target`).
8. **Layout modes (L1–L7).** For each node, what does the UI show
   (`board`, `p`, `table`, `dashboard`, `nonsense`, `""`, `H1`)? Then use the
   UI to set a node to each layout the UI offers (bullets, todo, h1/h2/h3,
   paragraph, board, code block, quote block, table?, dashboard?) and read
   back `data.layoutMode`. This gives the true enum.
9. **Completed parent with uncompleted children (C1).** Does the UI show the
   child as completed or hidden? Then, in the UI, complete a parent whose
   children are not completed and read back the children's `completed`
   flags. Does the UI cascade where the API does not?
10. **Complete in UI.** Complete a bullet (Ctrl/Cmd+Enter) and complete a
    todo (click the checkbox). Read back `completed`, `completedAt`,
    `modifiedAt` and `layoutMode`. Then uncomplete both and read again.
11. **Show/hide completed.** Toggle "show completed" in the UI and confirm it
    has no effect on the API (expected: no change).
12. **Re-complete in UI.** Complete an already-completed item (uncomplete and
    re-complete quickly). Check whether `completedAt` updates (the API
    re-stamps it).
13. **Mirror content (sandbox `update-complete: 08 misc`, short
    `1726462dd495`).** Node "A" (completed, origin) and its mirror under the
    node named "trailing slash". Through the API, the mirror has its own name
    "A renamed via mirror", note "mirror note", layout h2, and is not
    completed. What does the UI display for the mirror: A's content or the
    mirror's own fields? Then edit the origin's name in the UI and read both
    nodes via the API, and edit the mirror's name in the UI and read both
    again.
14. **Note editing.** Delete all text of an existing note in the UI. Does
    the API then report `note: null` or `""`?
15. **Empty name.** Clear a node's name entirely in the UI (leave the
    bullet). Does the API report `""`? Compare with the 01 sandbox node,
    which was set to `""` via the API and then renamed.
16. **modifiedAt from UI actions.** In the UI: (a) add a child to node X,
    (b) edit a child's text, (c) drag a child out of X, (d) drag a child into
    X, (e) collapse or expand X, (f) add a tag. After each step, read X's
    `modifiedAt` to compare with the API-driven results above (create-child
    does not bump; move-into does). Collapse/expand is especially
    interesting: does it bump modifiedAt even though it is not exposed?

## Sandboxes left in place

| Experiment | Sandbox id |
|---|---|
| 01 partial update semantics | `04de221a-9e07-44cc-aed6-5bc349887319` |
| 02 modifiedAt semantics | `da468ad9-0454-4d09-a2fa-1bd2576779ce` |
| 03 name content transforms | `4c175c12-8cb3-4a7e-aa8c-ad82d9021e5f` (subject left with a 5000-char name) |
| 03 note content transforms | `08a16e26-6fec-4bce-a766-a6de632c2f05` |
| 04 create-vs-update contrast, size | `972307fe-dfc9-4ca6-9af5-fcb10e337250` |
| 05 layoutMode + read-only fields | `cbef1b92-10aa-4b50-b247-f3ec925e5e10` |
| 06 complete/uncomplete semantics | `c1fd7f65-5920-470f-b3c9-e2ca5bb16567` |
| 07 id forms + missing/deleted | `44728fa7-b95a-4a36-a904-15c80b99489f` |
| 08 misc (consistency, encodings, mirrors) | `4e154634-c6b8-4814-b119-1726462dd495` |
| 10 UI inspection fixtures | `df1de294-3283-493b-a85d-29da5138c2d4` |

Aside, not in this area: in experiment 02, a node created with the default
`position` (top) under a parent whose existing child had priority 100 got
**priority 50**. The create/priority doc should cover this.
