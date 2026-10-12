# Workflowy API: Delete and Mirrors

Reverse-engineering notes for `DELETE /api/v1/nodes/:id`,
`POST /api/v1/nodes/:id/mirror`, and `DELETE /api/v1/nodes/:id/mirror`, plus how
deleted and mirrored nodes look through the rest of the API.

- Area tag in `.prod-state/requests.jsonl`: `delete-mirror` (about 290 requests in total)
- Experiment scripts and their captured output: `experiments/delete-mirror/NN_*.py` and `NN_*.out`
- All experiments ran on 2026-10-12 (server clock around `1791834400`–`1791834690`) inside
  sandboxes under the scratchpad (`a846cfce4c6d`).

Labels used below: **[observed]** means seen directly in a response (re-run where
noted). **[inferred]** means a reasonable reading of what was observed but not
directly proven.

---

## TL;DR

1. **Delete is a hard, recursive delete.** The node and all of its descendants
   immediately return 404 `{"code":"not_found","message":"Item not found"}`.
   The API has no trash or restore. Deleting again returns 404. Siblings are **not**
   renumbered. The parent's `modifiedAt` is **not** changed. [observed]
2. **`DELETE /api/v1/nodes/:id` does not accept short ids.** It returns 404, while `GET` with the same
   short id works. `POST /:id/mirror` and the mirror `parent_id` also reject
   short ids and URLs with 404. [observed]
3. **A mirror is an ordinary, separate node: an empty "shell".** The API does
   *not* resolve mirror content:
   - A new mirror has `name: ""`, `note: ""` (empty strings, **not** `null`),
     `data.layoutMode: "bullets"`, `completed: false`, its own new UUID, and its own
     `createdAt`, `modifiedAt` and `priority`. It does not copy the origin's layoutMode or
     completion. [observed]
   - No mirror metadata is exposed in GET or list: no `originId`, no extra keys. [observed]
   - Listing the children of a mirror returns **only the shell's own children**. A new
     mirror returns `[]`, even when the origin has children. [observed]
   - Edits **do not propagate** either way through the API. Renaming, re-noting,
     changing layout or completing the origin leaves the mirror unchanged, and the
     reverse is also true. This was re-checked several minutes later with the same
     result, so it is not eventual consistency. [observed, repeated in exp 03 and 05]
   - Children created under, or moved into, a mirror get `parent_id = <mirror id>`
     and do **not** appear under the origin. [observed]
   - This contradicts the upstream doc's "editing the origin node or any of its
     mirrors updates the same underlying content everywhere". That is
     presumably true *in the UI*, which the UI scenarios below need to confirm.
4. **The server does track the mirror→origin link internally.** It shows through in
   these places:
   - Mirroring a mirror resolves to the true origin (`origin_id` = the original).
   - `parent_id` = a mirror in `POST /mirror` puts the new mirror under the
     **origin** (`parent_id` of the result = origin id).
   - `DELETE /:id/mirror` succeeds only on mirror shells (400 `Node is not a mirror`
     otherwise).
   - Mirroring a node under one of its own mirrors fails with
     400 `Cannot mirror a node under itself`.
   - `POST /mirror` and `DELETE /mirror` bump the **origin's** `modifiedAt`. These are the
     **only** operations that do so. Plain `DELETE /nodes/<shell>` does not (3/3 trials, followup-1).
     Nothing done to or under a shell bumps the origin: moving nodes in or out, creating,
     updating, completing or deleting children of the shell, and renaming, completing or moving the
     shell itself (followup-1). For mirror-of-mirror, the bump goes to the **resolved origin**, not to the
     mirror named in the path (followup-1, 2/2). [observed]
5. **Deleting an origin leaves its mirrors in place** as orphan shells, together with any
   children that were added under them. An orphan still counts as a mirror:
   `DELETE /mirror` on it returns 200. Mirroring it returns 404 `Item not found`. Using it as
   the `parent_id` of `POST /mirror` returns 404 `Parent item not found` (it is not redirected). For
   everything else (GET, list, move, move-in, create under it, update, complete, plain delete) an orphan
   behaves exactly like an ordinary node (followup-1). [observed]
6. **Move has no cycle check for descendants.** `POST /nodes/O/move`
   with `parent_id` set to O's own child **succeeds (200) and creates a parent cycle**
   (O.parent = oc and oc.parent = O). The whole subtree drops out of the tree.
   Moving a node onto itself returns 200 but is a no-op apart from bumping the node's own `modifiedAt`.
   The parent is not bumped (followup-1, 2/2).
   This was seen by accident, then repaired (see exp 05–07). **Other agents and the sync engine
   should never do this. The mock must decide whether to replicate it; see the notes below.**

---

## 1. Delete

### 1.1 Basic delete of a subtree (exp 01)

Setup: `P` with children `c1`, `c2`, `c3` (priorities 100/200/300, created with
`position: bottom`); `c2 → g → gg`.

```
DELETE /api/v1/nodes/4f341b45-2f0e-475e-bf76-7927a3bc8c81        (c2)
-> 200 {"status": "ok"}
```

Immediately afterwards, with no delay:

| Request | Status | Body |
|---|---|---|
| `GET /nodes/c2` | 404 | `{"code": "not_found", "message": "Item not found"}` |
| `GET /nodes/g` (child of c2) | 404 | `{"code": "not_found", "message": "Item not found"}` |
| `GET /nodes/gg` (grandchild) | 404 | `{"code": "not_found", "message": "Item not found"}` |
| `GET /nodes?parent_id=c2` | 404 | `{"code": "not_found", "message": "Parent item not found"}` |
| `GET /nodes?parent_id=P` | 200 | c1 (priority 100), c3 (priority 300); c2 is gone and **c3 keeps priority 300** |
| `GET /nodes/P` | 200 | `modifiedAt` unchanged (1791834412 before and after; delete issued at 1791834418) |
| `GET /nodes/<sandbox>` (grandparent) | 200 | `modifiedAt` unchanged |

All error responses are `content-type: application/json`.

### 1.2 Deleting things that do not exist

| Request | Status | Body |
|---|---|---|
| `DELETE` c2 a second time | 404 | `{"code": "not_found", "message": "Item not found"}` |
| `DELETE` g (already gone because its ancestor was deleted) | 404 | same |
| `DELETE` a random UUID | 404 | same |
| `DELETE /nodes/not-a-real-id` | 404 | same |
| `DELETE /nodes/0123456789ab` (random short id) | 404 | same |
| `DELETE /nodes/<short id of an existing node>` | **404** | same. **The node is NOT deleted** (a later GET by full id returned 200) |
| `DELETE /api/v1/nodes` (no id) | 405 | `{"allowed_methods": ["GET", "POST"], "error": "Method not allowed"}` |

So `DELETE` is not idempotent at the HTTP level: the second call returns 404.

### 1.3 Operations on a deleted node (exp 01)

| Request | Status | Body |
|---|---|---|
| `POST /nodes/<deleted>` (update `{"name":"zombie"}`) | 404 | `{"code":"not_found","message":"Item not found"}` |
| `POST /nodes/<deleted>/complete` | 404 | `Item not found` |
| `POST /nodes/<deleted>/move` | 404 | `Item not found` |
| `POST /nodes/<deleted>/mirror` | 404 | `Item not found` |
| `POST /nodes` with `parent_id: <deleted>` | 404 | `{"code":"not_found","message":"Parent item not found"}` |
| `GET /nodes/<deleted>?include_deleted=true` | 404 | `Item not found` (the parameter is ignored) |

### 1.4 Trash or restore

The API exposes nothing for this. [observed]
- `GET /api/v1/trash` → 404 **HTML** page (`content-type: text/html; charset=utf-8`, `<title>404</title>`).
  This is the generic site 404, not the API's JSON 404.
- `POST /api/v1/nodes/:id/restore` and `/undelete` → 404 HTML, as above.
- The upstream doc says "Permanently deletes a node. This cannot be undone." Whether the
  web UI's trash or undo can restore an API-deleted node is still open (see UI scenarios).

### 1.5 Priorities after a delete (exp 01, exp 05)

- Remaining siblings keep their priorities; nothing is renumbered. [observed]
- A later `position: bottom` create uses the max of the **live** siblings. After deleting c2
  (200) out of 100/200/300, the next bottom create got 400, not 300. [observed] The general bottom rule is
  `ceil((max+100)/100)*100` (max+100 rounded up to a multiple of 100), not plain `max+100`. The two agree in every
  case in this doc because the max was always a multiple of 100. With live siblings 25, 50 (the 100 deleted), bottom
  gives **200**, not 150, for create, move and mirror alike (create.md §2.1; followup-0, ordering-and-move.md §7).
- Exp 05: with siblings `p1=100, p2=200, p3=300`, deleting `p1` and then creating at top
  gave the new node priority **100**, the same value p1 had. Deleting `p3` and then creating at
  bottom gave **300**, the same value p3 had. This fits "top = min/2" and "bottom = max+100", computed over the **live**
  siblings only (min 200 → top 100; max 200 → bottom 300; see above for the rounding of a non-multiple max). Deleted nodes leave no residue in
  the priority calculation. The exact top rule is covered by the create/priority docs. Observed values elsewhere in this
  area: an empty parent gives 100; top with min=100 gives 50; top with min=50 gives 25.

### 1.6 Timing

The delete is visible to the immediately following GET, list and DELETE (about 200 ms later through the
rate limiter). No eventual-consistency window was seen. [observed]

---

## 2. Mirrors: creation

### 2.1 Response shape (exp 02)

```
POST /api/v1/nodes/de8f2b83-e578-4c33-9ecd-83d7c7fee7de/mirror
{"parent_id":"b4e9a9a4-1592-4890-b09d-a960731f384b"}
-> 200 application/json
{"item_id": "52d397bd-bca3-4fbe-9dd7-75f4161f4d4b",
 "origin_id": "de8f2b83-e578-4c33-9ecd-83d7c7fee7de"}
```

The response has no node body and no special headers. The headers are the normal nginx, CSP and HSTS set.

### 2.2 How the mirror looks (exp 02)

The origin was `name: "O origin <b>bold</b>"`, `note: "origin note"`, `layoutMode: "todo"`,
and had two children. The mirror, read back immediately:

```
GET /api/v1/nodes/52d397bd-bca3-4fbe-9dd7-75f4161f4d4b
-> 200
{"node": {"id": "52d397bd-bca3-4fbe-9dd7-75f4161f4d4b",
          "parent_id": "b4e9a9a4-1592-4890-b09d-a960731f384b",
          "name": "", "note": "",
          "priority": 50, "completed": false,
          "data": {"layoutMode": "bullets"},
          "createdAt": 1791834445, "modifiedAt": 1791834445, "completedAt": null}}
```

- `note` is `""`. A normal node created without a note has `note: null`. Together with `name: ""`
  this is a **weak heuristic** for a fresh mirror. It is **not** reliable:
  - an edited mirror loses it once the name or note is set;
  - a normal node can be given `""` explicitly;
  - the children that a multiline `name` on create splits off also get `note: ""`
    (create.md, text-and-content.md). Those usually have a non-empty `name`, so `name == "" and note == ""`
    is the tighter test, but even that can be faked by `update {"name":"","note":""}`.

  The API gives no definitive way to tell that a node is a mirror without side effects. `DELETE /:id/mirror`
  answering 400 `Node is not a mirror` is a definitive *negative* test, but on a mirror it deletes it. [observed]
- The mirror appears in `GET /nodes?parent_id=<dest>` with exactly the same object. [observed]
- `GET /nodes/<short id of mirror>` works. [observed]
- `GET /nodes?parent_id=<mirror>` → `{"nodes": []}`, even though the origin has two
  children. [observed]
- A mirror of a completed todo node is `completed:false, layoutMode:"bullets"`. [observed, exp 05]

### 2.3 Positioning

`position` works the same way as for create ([observed], exp 02). Destination A held x1=100, x2=200:

| call | resulting priority |
|---|---|
| mirror, default (top) | 50 |
| mirror, `position: bottom` | 300 |
| mirror, `position: top` | 25 |
| mirror of oc1, bottom | 400 |
| mirror into an empty parent | 100 |
| `position: "middle"` | 400 `{"errors":[{"input":"middle","loc":["position"],"msg":"Input should be 'top' or 'bottom'","type":"enum"}]}` (keys printed sorted here; the real order is `type, loc, msg, input`) |

followup-0 (ordering-and-move.md §7) checked the edge cases [observed]: mirror bottom into a parent whose live
children are 0,1,2 / 25,50 / 3 gave **200** each time (rounded-up rule, not max+100), and into a parent whose only
child is 0 gave 100. Mirror top into 0,1,2 triggered the same full renumber as create (shell 100, others 200, 300,
400). An existing mirror shell is renumbered like any ordinary sibling, both when it is the min (0 → 200) and when it
is not (200 → 500).

### 2.4 Timestamps touched by mirror creation (exp 05)

| node | modifiedAt before | after `POST /O/mirror` (at ~1791834591.8) |
|---|---|---|
| destination parent A | 1791834586 | 1791834586 (unchanged) |
| origin's parent B | 1791834587 | unchanged |
| **origin O** | 1791834587 | **1791834591** (bumped) |
| origin's child oc | 1791834587 | unchanged |
| new mirror | – | createdAt = modifiedAt = 1791834591 |

The same bump appeared in exp 02 (O: 1791834442 → 1791834445). [observed, repeated]

followup-1 (`experiments/followup-1/04_mirror_of_mirror_bumps.py`; every action preceded by a 2.1 s gap,
then every node re-read) [observed]:

| action | bumped | not bumped |
|---|---|---|
| `POST /nodes/M1/mirror {"parent_id": D2}`, where M1 is a mirror of O (2 trials) | **O** (the resolved origin) | M1 (the node named in the path), D2 (destination), D1 (M1's parent) |
| `POST /nodes/X/mirror {"parent_id": M1}`, which is redirected so that the new shell lands under **O** | **X** (origin) | O (the *effective* destination parent), M1 (the *requested* parent) |
| `POST /nodes/Y/mirror {"parent_id": D3}` (re-check of the asymmetry) | Y | **D3** (destination parent not bumped) |
| `POST /nodes/Z/move {"parent_id": D3}` (same D3, move-in) | Z, **D3** | – |
| `POST /nodes {"parent_id": D3}` (create child) | nothing | D3 |

So a destination parent is bumped by **move-in only**, never by mirror creation or child creation, and this
holds even when the destination is the redirect target of a mirror `parent_id`.

### 2.5 Mirroring a mirror, and using a mirror as `parent_id` (exp 04)

```
POST /nodes/M1/mirror  {"parent_id": C}          # M1 is a mirror of O
-> 200 {"item_id": "b4583754-…", "origin_id": "66590f9d-…"}   # = O, not M1
```

```
POST /nodes/X/mirror   {"parent_id": M1}         # destination is a mirror of O
-> 200 {"item_id": "ff827bec-…", "origin_id": "<X>"}
GET  /nodes/ff827bec-…  -> parent_id = "66590f9d-…" (O!), priority 50
GET  /nodes?parent_id=M1 -> {"nodes": []}
GET  /nodes?parent_id=O  -> includes the new mirror
```

Note the inconsistency: **`POST /mirror` redirects a mirror `parent_id` to the
origin, but `POST /nodes` (create) and `POST /move` do not.** Those place the node
under the shell itself (§3.4). [observed]

### 2.6 Self, ancestor and descendant destinations (exp 04)

| call | result |
|---|---|
| mirror O → O | 400 `{"code":"invalid_request","message":"Cannot mirror a node under itself"}` |
| mirror O → M1 (M1 is a mirror of O, so it resolves to O) | 400 same |
| mirror M1 → M1 | 400 same (M1 resolves to O and the parent resolves to O) |
| mirror O → oc1 (**child** of O) | **200**: allowed, creates a mirror of O inside O's own subtree |
| mirror O → occ (**grandchild** of O) | **200**: allowed |
| mirror oc1 → O (into the origin's own parent) | 200 |

The only check is `resolve(parent) == resolve(origin)`. Descendants are not checked.
The upstream doc's sentence "The destination cannot resolve to the origin node being
mirrored" describes this exactly. [observed]

### 2.7 Validation and errors for `POST /:id/mirror` (exp 04)

| request | status | body |
|---|---|---|
| body `{}` | 400 | `{"errors":[{"input":{},"loc":["parent_id"],"msg":"Field required","type":"missing"}]}` |
| no body at all | 400 | `{"code":"invalid_json","message":"Invalid JSON format in request body"}` |
| `:id` = random UUID | 404 | `{"code":"not_found","message":"Item not found"}` |
| `:id` = **short id of an existing node** | 404 | `Item not found` |
| `:id` = deleted node | 404 | `Item not found` |
| `:id` = orphan mirror (its origin was deleted) | 404 | `Item not found` |
| `parent_id` = random UUID | 404 | `{"code":"not_found","message":"Parent item not found"}` |
| `parent_id` = short id of an existing node | 404 | `Parent item not found` |
| `parent_id` = `https://workflowy.com/#/<short>` | 404 | `Parent item not found` |
| body `{"parent_id": null}` (random `:id`) | 400 | `{"errors": [{"type": "string_type", "loc": ["parent_id"], "msg": "Input should be a valid string", "input": null}]}` (followup-2) |
| body `{"parent_id": ""}` (random `:id`) | 404 | `Item not found`, so `""` passes validation (followup-2) |
| body `{"position": "q"}` (random `:id`) | 400 | two errors, `missing parent_id` then `enum position` (followup-2) |
| body `[1]` / `"s"` / `null` (random `:id`) | 400 | `{"errors": [{"type": "model_type", "loc": [], "msg": "Input should be a valid dictionary or instance of MirrorNodeRequest", "input": [1]}]}` (followup-2) |
| `parent_id` = URL embedding a full UUID, uppercase random UUID, dashless random UUID, bare random short | 404 | `Parent item not found` in every case (followup-2) |
| `parent_id` = garbage string `zzzz-not-an-id` | 404 | `Parent item not found` (followup-1). **Differs from move and create**, which say `Location 'zzzz-not-an-id' not found`. Mirror's `parent_id` never goes through the location resolver. |
| `parent_id` = an **orphan** mirror shell | 404 | `Parent item not found` (followup-1, 2 separate orphans). It is not redirected and not created under the shell. |
| invalid `position` | 400 | pydantic-style `errors` array (see §2.3) |
| `GET /nodes/:id/mirror` | 405 | `{"error":"Method not allowed","allowed_methods":["POST","DELETE"]}` |

(Key order: the real error items are `{"type", "loc", "msg", "input"}`, as in
the followup-2 rows above, and the 405 body is `{"error", "allowed_methods"}`.
The older rows in this doc were printed with keys sorted alphabetically. See
`.prod-state/requests.jsonl` for the exact bytes.)

#### Check order (followup-1, `05_mirror_check_order.py`) [observed]

Exact response bodies (real key order):

| `:id` | `parent_id` | `position` | result |
|---|---|---|---|
| random | real sandbox node | `"middle"` | 400 `{"errors": [{"type": "enum", "loc": ["position"], "msg": "Input should be 'top' or 'bottom'", "input": "middle"}]}` |
| random | random | – | 404 `{"code": "not_found", "message": "Item not found"}` |
| random | random | `"middle"` | 400 enum (as above) |
| random | (missing, body `{}`) | – | 400 `{"errors": [{"type": "missing", "loc": ["parent_id"], "msg": "Field required", "input": {}}]}` |
| random | body `{nope` (invalid JSON) | – | 400 `{"code": "invalid_json", "message": "Invalid JSON format in request body"}` |
| random | `5` | – | 400 `{"errors": [{"type": "string_type", "loc": ["parent_id"], "msg": "Input should be a valid string", "input": 5}]}` |
| real X | random | `"middle"` | 400 enum |
| real X | random | – | 404 `{"code": "not_found", "message": "Parent item not found"}` |
| O | O (self) | `"middle"` | 400 enum |
| O | O (self) | – | 400 `{"code": "invalid_request", "message": "Cannot mirror a node under itself"}` |
| orphan N | real D | – | 404 `Item not found` |
| orphan N | real D | `"middle"` | 400 enum |
| orphan N | random | – | 404 `Item not found`. **Origin resolution happens before the parent lookup.** |
| orphan N | N | – | 404 `Item not found` (not "Cannot mirror a node under itself") |
| plain child of orphan N | real D | – | 200. A plain node under an orphan is mirrorable as usual. |

Order: **JSON parse → schema validation (all field errors) → `:id` lookup → origin resolution (orphan → `Item
not found`) → `parent_id` lookup and mirror redirect (orphan parent → `Parent item not found`) → self check.**

The upstream doc says `parent_id` "Must be the full `id`". This is confirmed. Special keys were
not tested because the scope guard blocks them.

---

## 3. Mirrors: editing, completing, moving, children (exp 03, exp 05)

### 3.1 Edit propagation: none, through the API

| step | origin O afterwards | mirror M afterwards |
|---|---|---|
| `POST /O {"name":"O renamed via origin","note":"note v2"}` | name and note changed, modifiedAt bumped | `name:""`, `note:""`, modifiedAt **unchanged** |
| `POST /M {"name":"M renamed via mirror"}` | **unchanged**, modifiedAt unchanged | `name:"M renamed via mirror"`, modifiedAt bumped |
| `POST /M {"note":"mirror note"}` | unchanged | note set |
| `POST /M {"layoutMode":"h1"}` | still `bullets` | `h1` |
| `POST /M/complete` | `completed:false` | `completed:true`, completedAt and modifiedAt set |
| `POST /O/complete` | `completed:true` | `completed:false`, modifiedAt unchanged (exp 05) |

The state was re-read about 2 minutes later (exp 05 start) and was unchanged: M still
`"M renamed via mirror"`/`h1`, and O still `"O renamed via origin"`. **The API treats the
mirror shell's fields as completely independent storage.** [observed, repeated]

[inferred] Internally, Workflowy probably stores a mirror as a node carrying a hidden
`mirror → originalId` property, and the UI renders the origin's content in place of the
shell's. Writing a name or note to the shell through the API may then be invisible in
the UI, or may show up in a strange way. See UI scenario U4.

### 3.2 Creating children under a mirror

```
POST /nodes {"parent_id": M, "name": "child created under mirror M"}
-> 200 {"item_id": "c078119e-…"}
GET /nodes/c078119e-… -> parent_id = M, priority 100
GET /nodes?parent_id=M -> [that child]
GET /nodes?parent_id=O -> only O's own children (child NOT here)
```

A child added under O afterwards does **not** appear in `list M`. [observed]

### 3.3 Moving a node into a mirror

`POST /nodes/loose/move {"parent_id": M}` → 200. `loose.parent_id = M`, priority 50 (top),
and it does not appear under O. [observed]

### 3.4 Moving the mirror, and moving the origin

- `POST /nodes/M/move {"parent_id": D}` → 200. M.parent_id = D. The old parent's list no
  longer contains M. The children of the shell move with it (re-checked later: `list M` still
  returned its three children). The origin's modifiedAt is unchanged. [observed]
- Moving the origin O → D: O moves and M is untouched. [observed]
- Moving the origin **into its own mirror** (`move O → M5`): **200**, and O.parent_id = M5. Moving
  the origin's parent B into M5 is also 200. Workflowy does not prevent these. [observed]

### 3.5 Timestamps for operations on and under a shell (followup-1)

`experiments/followup-1/02_shell_side_effects.py`. O is the origin (with child oc), and M is a mirror of O
placed in D. There is a 2.1 s gap before every action, and S, O, oc, D, M, L and K are all re-read after
it. [observed]

| action | bumped | **origin O bumped?** |
|---|---|---|
| move L → M (into shell), done twice | L, M | no |
| move L → S (out of shell) | L, S | no (M is not bumped either: move-out never bumps the old parent) |
| create child K under M | nothing | no |
| update K's name | K | no |
| complete K / uncomplete K | K | no |
| delete K | (K gone) | no. M is not bumped either. |
| update M's name | M | no |
| complete M | M | no |
| move M → S, and M → D back | M plus the new parent | no |
| update O's name (control) | O | – |

A shell acts exactly like an ordinary node as a parent: move-in bumps it, and create, delete and move-out of
children do not. **Nothing done through or under a shell touches the origin's `modifiedAt`.** The only origin
bumps are `POST /mirror` and `DELETE /mirror`.

---

## 4. Mirrors: deletion (exp 04, 05, 08)

### 4.1 `DELETE /api/v1/nodes/:id/mirror`

| target | status | body |
|---|---|---|
| a mirror shell (with a child added under it) | 200 | `{"status":"ok"}`. The shell **and its child** then return 404. Origin and origin's children are intact |
| the same shell again | 404 | `{"code":"not_found","message":"Item not found"}` |
| an origin node (one that has mirrors) | 400 | `{"code":"invalid_request","message":"Node is not a mirror"}` |
| a plain node | 400 | `{"code":"invalid_request","message":"Node is not a mirror"}` |
| a random UUID | 404 | `{"code":"not_found","message":"Item not found"}` |
| an **orphan** mirror (its origin was deleted) | 200 | `{"status":"ok"}` |

`DELETE /mirror` **bumps the origin's modifiedAt** (exp 05: O 1791834598 → 1791834603;
the delete was issued at 1791834603.0). It does not touch the destination parent, the
origin's parent, or the origin's children. [observed]

### 4.2 `DELETE /api/v1/nodes/:id` on a mirror

- 200 `{"status":"ok"}`. The shell returns 404 and the origin is intact. [observed]
- **It does NOT bump the origin's modifiedAt** (exp 08: O stayed at 1791834675; the delete was at
  1791834678.5). This differs from `DELETE /mirror`. [observed] **Confirmed by followup-1**
  (`03_delete_mirror_variants.py`). The script ran 3 trials of each variant, interleaved, each with a fresh origin and
  a 2.1 s gap before the delete. Plain `DELETE /nodes/<shell>` bumped nothing in 3 of 3 trials.
  `DELETE /nodes/<shell>/mirror` bumped the origin in 3 of 3 trials. Both returned `200 {"status": "ok"}`, and the shell was
  `404 {"code": "not_found", "message": "Item not found"}` afterwards in all 6. Neither bumped the destination
  parent D or the sandbox.
  **So the upstream reference's claim that plain delete "has the same effect" as `DELETE /mirror` is true for the
  tree, but not for the origin's `modifiedAt`.**
- Deleting an ancestor that contains a mirror (exp 05, `H`) deletes the shell. The origin and
  its children survive. [observed] It does **not** bump the origin's modifiedAt (followup-1, 1 trial).

### 4.3 Deleting an origin that has mirrors

```
DELETE /nodes/O2                 -> 200
GET /nodes/O2                    -> 404 Item not found
GET /nodes/N1   (mirror of O2)   -> 200, unchanged shell (name "", note "")
GET /nodes/n1c  (child of N1)    -> 200, still parent_id = N1
DELETE /nodes/N2/mirror          -> 200 {"status":"ok"}   (still recognised as a mirror)
POST /nodes/N1/mirror {...}      -> 404 {"code":"not_found","message":"Item not found"}
```

- The mirrors' `modifiedAt` does **not** change when the origin is deleted (exp 08). [observed]
- Deleting the origin's *ancestor* behaves the same way. The mirror survives as an orphan. Renaming it
  still works (200, and the name is stored). [observed, exp 05 `H2`]

#### 4.4 Life of an orphan shell (followup-1, `06_orphan_ops.py`) [observed]

| operation on orphan N (origin O deleted; N has child nc) | result |
|---|---|
| `POST /nodes/N/move {"parent_id": H2, "position": "bottom"}` | 200. N.parent_id = H2, priority 100. N and H2 are bumped; old parent H, nc and others are not. N's child nc moves with it. |
| move L → N (into the orphan) | 200. L and N are bumped. |
| create a child under N | 200, priority 25 (top under 50/100). N is not bumped. |
| `update N {"name":"orphan renamed","note":"n"}` | 200. Stored, and N is bumped. |
| complete N | 200. `completed: true`, completedAt set, N bumped. |
| list N | Its own children, in creation order (L, which was created before nc and moved in later, lists first). |
| `POST /nodes/X/mirror {"parent_id": N2}` (N2 is another orphan) | **404 `Parent item not found`** |
| `DELETE /nodes/N2/mirror` | 200 `{"status": "ok"}` |
| `DELETE /nodes/N` (plain) | 200. N, nc and the moved-in L are all 404 afterwards. Old and new parents are not bumped. |

So an orphan is an ordinary node everywhere, except in `POST /mirror`: as the source it gives `Item not found`, and as
the parent it gives `Parent item not found`. `DELETE /mirror` still treats it as a mirror.

---

## 5. Identity model of mirrors (summary)

```
Node (one id space; every API object is one of these)
 ├─ id, parent_id, priority, name, note, layoutMode, completed, timestamps
 └─ hidden: mirror_origin_id (null for normal nodes)
```

- A mirror is a **distinct node with its own UUID**. In the API it is a peer of normal
  nodes. Its visible fields belong to the shell and start as `name:""`, `note:""`, `bullets`,
  not completed.
- `origin_id` is always a **non-mirror** node: mirroring follows the `mirror_origin_id` chain.
  [observed for one level. Chains cannot form because a mirror is always created
  with a resolved origin.]
- The children of a mirror (as seen by the API) are the nodes whose `parent_id` is the shell.
  They are not the origin's children. The origin's children have exactly one parent, the origin.
- In the API only, the mirror link affects: `POST /mirror` origin resolution, `POST /mirror`
  parent resolution, the "under itself" check, `DELETE /mirror` eligibility, and origin
  `modifiedAt` bumps.
- Deleting the origin does not delete the mirrors or clear their link. They become orphans. Mirroring an orphan
  fails with 404 because the origin cannot be resolved.
- The API offers no way to find a node's mirrors or a mirror's origin, except
  `POST /mirror` itself, which returns `origin_id` but creates a node as a side effect.

---

## 6. Parent cycles through move (found by accident; relevant to the mock)

Exp 05 ran `POST /nodes/O/move {"parent_id": oc}`, where `oc` is O's direct child:

```
-> 200 {"status":"ok"}
GET /nodes/O  -> parent_id = oc
GET /nodes/oc -> parent_id = O
GET /nodes?parent_id=oc -> [O, M5]
```

This is a real cycle, and the subtree became unreachable from the sandbox. It was repaired in
`06_repair_cycle.py` by moving O back under the sandbox (200), which restored a proper tree.
Self-move (exp 07, `move Q → Q`) returns 200 but leaves `parent_id` unchanged
(modifiedAt bumped 1791834653 → 1791834654). In exp 06, `move O → O` also returned 200.
The relevant structure in the "followups" sandbox is now `S > O > oc > M5 (mirror of O) > B > …`.

**Recommendation:** the mock should probably **reject** descendant moves rather than corrupt
its tree. If exact fidelity matters, it could reproduce them behind a flag. The coordinator and the
move-area agent should know that production accepts these moves.

---

## 7. Mock server implementation notes

Storage: give every node a nullable `mirror_origin_id` column, never serialised. Use
hard delete: actually remove rows, or mark them deleted and treat them as nonexistent everywhere.

### `DELETE /api/v1/nodes/:id`
1. Look `:id` up **by full UUID only**. If it is not found, including short ids, garbage, or
   already deleted (or under a deleted ancestor), return 404 `{"code":"not_found","message":"Item not found"}`.
2. Remove the node and its entire subtree. Do **not** renumber siblings or touch anyone's `modifiedAt`
   (not the parent, not the origin if the node is a mirror, not mirrors if it is an origin).
3. Mirrors whose origin is in the deleted subtree stay, with their `mirror_origin_id`
   left dangling.
4. Return 200 `{"status":"ok"}`.

### `POST /api/v1/nodes/:id/mirror`
1. Parse the body. If it is empty or invalid JSON, return 400 `{"code":"invalid_json","message":"Invalid JSON format in request body"}`.
   Use pydantic-style validation. A missing `parent_id` gives 400
   `{"errors":[{"input":<body>,"loc":["parent_id"],"msg":"Field required","type":"missing"}]}`. A bad position gives
   `{"errors":[{"input":"<value>","loc":["position"],"msg":"Input should be 'top' or 'bottom'","type":"enum"}]}`.
   (followup-2: validation runs **before** the 404 checks, as on every other endpoint. The model name is
   `MirrorNodeRequest`, and fields are validated in the order `parent_id, position`, collecting all errors.
   `parent_id: null` gives `string_type` with `input: null`. `""` passes validation. Real key order in each
   error item is `type, loc, msg, input`.)
2. `src` = lookup(`:id`), full UUID only. If it is missing, return 404 `Item not found`.
3. `origin` = `src`; while `origin.mirror_origin_id` is set, `origin` = lookup(that). If the
   lookup fails (orphan), return 404 `{"code":"not_found","message":"Item not found"}`.
4. `parent` = lookup(`parent_id`), full UUID only. Any miss gives 404
   `{"code":"not_found","message":"Parent item not found"}`. That covers short ids, URLs, uppercase or dashless UUIDs,
   **and garbage strings**: mirror never produces `Location '…' not found`, unlike create and move (followup-1). If
   `parent` is a mirror, replace it with its resolved origin. If that resolution fails (an **orphan** parent),
   return 404 `Parent item not found` (observed, followup-1).
   (Steps 2–4 run in exactly this order: a random `:id` with a random parent gives `Item not found`, and an orphan
   `:id` with a random parent gives `Item not found` (followup-1).)
5. If `parent.id == origin.id`, return 400 `{"code":"invalid_request","message":"Cannot mirror a node under itself"}`.
   Do **not** reject descendants of the origin.
6. Create a node: new UUID v4, `parent_id=parent.id`, `name=""`, `note=""`,
   `layoutMode="bullets"`, `completed=false`, `completedAt=null`, `createdAt=modifiedAt=now`,
   priority from the standard top/bottom rule (top is the default), and `mirror_origin_id=origin.id`.
7. Set `origin.modifiedAt = now`. This is the *resolved* origin, even when `:id` was a mirror. Do **not** bump
   `src` (if it differs), the destination parent (even when it was reached by redirect), or anyone else.
8. Return 200 `{"item_id": <new>, "origin_id": <origin.id>}`.

### `DELETE /api/v1/nodes/:id/mirror`
1. Look up by full UUID. If it is missing, return 404 `Item not found`.
2. If `mirror_origin_id` is null, return 400 `{"code":"invalid_request","message":"Node is not a mirror"}`.
3. Delete the shell and its subtree. If the origin still exists, set its `modifiedAt = now`.
4. Return 200 `{"status":"ok"}`. Orphans succeed too.
5. Any other method on `/nodes/:id/mirror` returns 405 `{"allowed_methods":["POST","DELETE"],"error":"Method not allowed"}`.

### Every other endpoint
Treat mirror shells as ordinary nodes: GET, list, update, complete, move, and create with
`parent_id=<mirror>` all act on the shell's own fields and children. No propagation.
Moves into or out of mirrors are not redirected. **Never** bump the origin from these paths. The shell gets the
normal parent treatment (move-in bumps it, and create, delete and move-out of children do not). Orphans are the
same (followup-1 §3.5, §4.4).

### Deleted nodes in other endpoints
- GET, update, complete, uncomplete, move or mirror of a deleted id: 404 `{"code":"not_found","message":"Item not found"}`.
- List or create with a deleted `parent_id`: 404 `{"code":"not_found","message":"Parent item not found"}`.

### Unknown routes
Unknown routes under `/api/v1` (e.g. `/api/v1/trash`, `/nodes/:id/restore`) return 404 with an
**HTML** body. A mock can return `text/html` with `<title>404</title>`.

---

## 8. Open questions

1. Is the observed edit independence API-only, or does the UI also show the shell's own `name`?
   (UI scenarios U1–U4.) If the UI renders origin content, how do API writes to a shell affect
   what the UI shows?
2. Do children added under a mirror shell through the API show up anywhere in the UI? Are they
   merged into the origin's children, hidden, or shown?
3. When a mirror is created in the UI, does the API see the same shape (`name:""`, `note:""`)? Does
   the UI write the origin's text into the shell?
4. Does deleting an origin in the **UI** behave differently, for example by promoting one mirror to
   be the new origin? The API leaves orphans.
5. Can the UI trash or undo restore API-deleted nodes? If so, do they keep their ids, priorities
   and timestamps?
6. Can `nodes-export` (blocked by our scope guard) reveal mirror metadata?
7. ~~`DELETE /nodes/:mirror` vs `DELETE /mirror` origin bump~~ **Resolved (followup-1):** 3/3 each. Only
   `/mirror` bumps the origin.
8. ~~Validation order in `POST /mirror`~~ **Resolved (followup-1, §2.7 "Check order").**
9. ~~Mirror into an orphan `parent_id`~~ **Resolved (followup-1):** 404 `Parent item not found`.
9a. Why does an orphan still answer `DELETE /mirror` with 200, while mirroring it gives 404? [inferred] The shell
   keeps its `mirror_origin_id`, and only the code paths that *dereference* the origin fail.
10. Is `move` of a node onto its own descendant always accepted, including deeper descendants? Does the
    UI break on such cycles? (Ours was repaired within about 30 s.)
11. What ordering does the list endpoint use? Results looked like createdAt / insertion order,
    not priority order. That belongs to the list area.

---

## 9. UI scenarios (for a human to perform; then we observe through the API)

Sandboxes left in place under the scratchpad:
- `sandbox: delete-mirror: delete basics`
- `sandbox: delete-mirror: mirror basics`: three mirrors of "O origin **bold**" plus a mirror of oc1, in A
- `sandbox: delete-mirror: mirror edits`: mirror M in D renamed via the API to "M renamed via mirror", with
  note "mirror note", layout h1, and 3 children added under the shell
- `sandbox: delete-mirror: mirror identity + deletion`: mirror-of-mirror, mirrors of O inside O's
  own descendants (oc1, occ), orphan mirror N1 (its origin O2 was deleted) with a child
- `sandbox: delete-mirror: followups`: O > oc > M5 (mirror of O inside O's own subtree) > B;
  orphan mirror "orphan mirror renamed" in A
- `sandbox: delete-mirror: delete timestamps`, `sandbox: delete-mirror: self-move (cycle check)`

| # | UI action | What to observe through the API |
|---|---|---|
| U1 | Open "mirror basics" and look at the mirrors in A | Do they render as live mirrors of O, with O's name and children? (UI-only observation. Then re-GET the shells to confirm they still have name `""`) |
| U2 | In the UI, create a mirror of a node (Alt-drag or the "Mirror" menu) into another sandbox node | GET the new node: `name` and `note` (`""`? origin's text?), layoutMode, priority, and whether the origin's `modifiedAt` bumped |
| U3 | In the UI, edit the **text** of a mirror (not the origin) | Which node's `name` and `modifiedAt` change in the API: origin, shell, or both? |
| U4 | Open the "mirror edits" sandbox: view mirror M in D | What the UI shows for a shell whose API name, note and layout were set directly, and whether the 3 shell children are visible |
| U5 | In the UI, add a child under a mirror | Is the new child's `parent_id` the origin or the shell? Does it list under both? |
| U6 | In the UI, complete a mirror, then complete an origin | `completed` and `completedAt` on the origin and the shell |
| U7 | In the UI, delete the **origin** of a node that has mirrors | Do the mirror shells survive? Does one get promoted (e.g. its name filled in, or the id reused)? |
| U8 | In the UI, delete a mirror | The shell returns 404, the origin is intact, and whether the origin's modifiedAt changed |
| U9 | Look at the orphan mirrors (N1 in "mirror identity + deletion", "orphan mirror renamed" in "followups") | How the UI renders a mirror whose origin was deleted through the API |
| U10 | Look at the mirrors of O placed inside O's own descendants ("mirror identity + deletion", "followups") | Does the UI render the recursion safely? Does it warn or flag them? |
| U11 | In the UI, delete a normal node with children, then undo (Ctrl+Z) | Does the API see the same ids again, with the same priorities and timestamps? |
| U12 | In the UI, delete a node, then restore it from the trash or "recently deleted" (if available) | As U11, and whether children come back |
| U13 | In the UI, delete the middle of three siblings | Whether the remaining siblings' priorities are renumbered (the API does not renumber) |
| U14 | In the UI, use "convert mirror to copy" or "unmirror" (if it exists) | Does the shell keep its id? Does it get name and children copies with new ids? |
| U15 | In the UI, mirror a node that is itself a mirror | The API `name` of the new shell and its relation to the origin |
| U16 | In the UI, drag a node into one of its own mirrors, or into a mirror of its ancestor | Whether the UI allows it, and the resulting `parent_id` |
| U17 | In the UI, move a mirror by drag | `parent_id` and priority of the shell. Whether the origin's modifiedAt changes |
| U18 | (followup-1) In the UI, add a child under a mirror, rename a child of a mirror, and drag a node into a mirror | Whether the **origin's** `modifiedAt` changes (through the API it never does for these). Is the new child's `parent_id` the origin or the shell? |
| U19 | (followup-1) Open sandbox `followup-1: mirror check order`: H contains orphan N (origin O2 deleted via API) with child nc; D contains a mirror of nc | How the UI renders orphan N, and the mirror of a child of an orphan |
| U20 | (followup-1) In the UI, use "remove mirror" or delete on a mirror, once each | Whether the origin's `modifiedAt` bumps in each case. The API bumps it only for `DELETE /mirror`. |
