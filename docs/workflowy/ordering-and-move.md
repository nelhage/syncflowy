# Sibling ordering (`priority`) and `POST /api/v1/nodes/:id/move`

Area tag: `ordering-move`. Experiments: `experiments/ordering-move/` (each script
is re-runnable; every request/response is in `.prod-state/requests.jsonl` with
`"tag": "ordering-move"`). 409 production requests were made, all on
2026-10-12.

Legend: **[obs]** means we observed it directly, sometimes repeatedly.
**[inf]** means we inferred it from observations and it has not been
independently confirmed.

## TL;DR

* `priority` is an **integer sort key that has gaps (sparse)**, not a dense rank, and it is scoped to one parent. The server
  assigns it with a simple deterministic algorithm:
  * **bottom**: `ceil((max(sibling priorities) + 100) / 100) * 100`, i.e. max+100 rounded **up** to a multiple
    of 100 (`100` if there are no other siblings). This equals `max+100` only when max is a multiple of 100.
    Confirmed identically for create, move and mirror (followup-0, §7): max 2 → 200, max 50 → 200, max 3 → 200,
    max 0 → 100; followup-1 added max 150 → 300 and max 250 → 400 (§7.1). An earlier version of this doc said plain `max + 100`; that was wrong for non-multiples.
  * **top** (this is the default): `floor(min(sibling priorities) / 2)` (`100` if there are no other siblings).
    If the min is already `0`, the server **renumbers the whole sibling list**:
    the new node gets `100`, and the existing siblings in priority order get `200, 300, 400, …`.
    Identical for create, move (cross-parent and same-parent) and mirror; mirror shells are renumbered like any
    other sibling (followup-0, §7).
  * "Siblings" never includes the node that is being placed. A move is "remove, then insert". So a same-parent
    move-to-top of the node that is itself the only 0 does **not** renumber: `[self=0, 100, 200]` → self = 50.
* The server behaves as if each insert/move is **serialized per parent (or per account)**: 60 overlapping
  concurrent top/bottom creates, top moves and bottom mirrors never produced a duplicate priority (followup-0, §8).
* Apart from that one renumber case, **no other sibling priority is ever rewritten**: not by create, not by
  move-in or move-out, and not by delete. Renumbering does **not** bump the siblings' `modifiedAt`.
* `GET /api/v1/nodes?parent_id=` returns children **in creation order**, NOT priority order (as the docs warn).
  Moves, renames, completes and renumbers do not change that order.
* `position` is a strict enum: `"top" | "bottom"`. `null` or absent means `"top"`. Any other value gives a 400
  pydantic-style error. **There is no way to put a node in the middle of a sibling list through the API.** The server
  silently ignores `priority`, `index`, `after`, `before`, `after_id` and `sibling_id` on move, `priority` on create,
  and `priority`, `position` and `parent_id` on update.
* Move keeps the node's `id` and `createdAt`. The whole subtree moves with the node, and the descendants keep their
  own priorities. Move bumps `modifiedAt` on the **moved node and the new parent only**. The old parent and other
  ancestors are not bumped.
* Changes are immediately consistent: a GET or list straight after a move already shows the new state.
* **Moving a node into its own descendant SUCCEEDS (200) and creates a detached cycle** (see below). We repaired the
  one we created. Moving a node into itself returns 200 and does nothing except bump `modifiedAt`.
* The `:id` path parameter of `/move` must be a **full UUID**. A 12-char short id gives 404 `Item not found`,
  even though `GET /nodes/<short>` works. `parent_id` accepts a full UUID, a short id or a URL.

## 1. Priority assignment on create

Script: `01_create_priorities.py` (sandbox `0417219b-648d-4d3a-9025-9d94be99bee3`). We performed a sequence of
creates into an empty parent and re-listed after each one **[obs]**:

| op (in order) | priority given to new node | other siblings changed? |
|---|---|---|
| create b1 bottom | 100 | – |
| b2..b5 bottom | 200, 300, 400, 500 | none |
| t1 top | 50 (=floor(100/2)) | none |
| t2..t5 top | 25, 12, 6, 3 | none |
| d1 (no `position`) | 1 | none |
| d2 (no `position`) | 0 | none |
| t6 top (min is 0) | **100** | **all 12 renumbered**: d2→200, d1→300, t5→400, t4→500, t3→600, t2→700, t1→800, b1→900, …, b5→1300 |
| b6 bottom | 1400 | none |
| t7 top | 50 | none |
| b7 bottom | 1500 (=max+100; *not* (count+1)*100, which would be 1600) | none |
| t8 top / b8 bottom | 25 / 1600 | none |

This shows:

* Bottom uses `max + 100` *when max is a multiple of 100* (as in every row here). We checked it against the
  alternative `(count+1)*100`, and `max + 100` fits. The general rule is `ceil((max+100)/100)*100`
  (create.md §2.1, and followup-0 §7 below for move/mirror).
* Top uses `floor(min/2)` with integer floor division: 25→12, 3→1, 1→0.
* The renumber happens exactly when `min == 0`. The new node takes 100 and every other sibling takes
  `100*(rank+2)` in ascending priority order. Renumbered siblings keep their old `modifiedAt` (in this sandbox every
  node still has `modifiedAt == createdAt` after the renumber).
* Create into an empty parent, top or bottom, gives `100` (`06_priority_misc.py`).
* Create with `"priority": 150` is **ignored** and gets the default top placement (50). Create with `"priority": 150,
  "position": "bottom"` gets `max+100` **[obs]**.
* Create with `"position": null` behaves like top **[obs]**.
* Create with `"position": "middle"` returns 400 with the same enum error as move (see section 3).

Priorities seen in production are always integers. The scratchpad itself has priority 28 at the root level, which is
not in the halving sequence 100→50→25→12. Our guess is that the web UI assigns priorities differently (perhaps by
inserting at a midpoint); see the UI scenarios **[inf]**.

## 2. Priority assignment on move

Scripts: `02_move_basics.py`, `06_priority_misc.py`, `07_…`, `08_…`, `09_recompute_vs_noop.py`.

Moves use the same algorithm as create, computed over the **destination siblings excluding the moved node** **[obs]**:

| scenario | before | result |
|---|---|---|
| x3 → P2 top (P2: y1 100, y2 200, y3 300) | | x3 = 50 |
| x2 → P2 bottom (P2 max 300) | | x2 = 400 |
| x1 → P2, no position | min 50 | x1 = 25 (default top) |
| x5 → same parent top (P1: x4 400, x5 500) | | x5 = 200 = floor(400/2) |
| x4 → same parent bottom (P1: x4 400, x5 200) | | x4 = 300 = 200+100 |
| y3 (300) → same parent bottom (max other 400) | | 500 |
| y3 (500) → same parent bottom again (max other 400) | | 500 (self excluded; if self were counted it would be 600) |
| n4 (400, last) → same-parent bottom, max other = 200 | | **300**: recomputed, it can go *down* |
| n0 (50, first) → same-parent top, min other = 200 | | **100**: recomputed, it can go *up* |
| k0 (50, first) → top, min other = 100 (repeated 3x) | | 50 each time |
| move into empty parent, top or bottom | | 100 |
| m1..m8 → B top in turn (B: 100, 200) | | 50, 25, 12, 6, 3, 1, 0, then **m8 = 100 + full renumber** (m7→200, m6→300, …, b2→1000) |

So moving to the position the node already holds still recomputes its priority from the other siblings. It is not a
no-op.

Siblings' priorities are not changed by moves in or out, except for the min==0 renumber **[obs]**, which is the same
as for create.

Delete does **not** rewrite sibling priorities, not even when the first child is deleted (`06_priority_misc.py`) **[obs]**.

### Is there any way to place a node in the middle?

No, as far as we found. Every candidate parameter was tried in `03_move_params_and_errors.py` and `06_priority_misc.py`:

| request | status | effect |
|---|---|---|
| move `{"parent_id":P,"priority":250}` | 200 `{"status": "ok"}` | ignored; placed at top (50) |
| move `{"parent_id":P,"index":1}` | 200 | ignored; top |
| move `{"parent_id":P,"after":<p2 id>}` | 200 | ignored; top |
| move `{"parent_id":P,"before":<p2 id>}` | 200 | ignored; top |
| move `{"parent_id":P,"after_id":…}` / `"sibling_id":…` | 200 | ignored; top |
| move `{"position":"after","sibling_id":…}` | 400 enum error | not moved |
| move `position` = `"middle"`, `"first"`, `"last"`, `"start"`, `"end"`, `"after"`, `"before"`, `""`, `"TOP"`, `"Bottom"`, `0`, `1`, `2`, `"1"`, `-1`, `true`, `250` | 400 enum error (identical shape every time) | not moved |
| move `position: null` | 200 | top |
| update (`POST /nodes/:id`) `{"priority":250}` | 200 `{"status": "ok"}` | **ignored** (priority unchanged) |
| update `{"priority":"bottom"}` | 200 | ignored |
| update `{"position":"bottom"}` | 200 | ignored |
| update `{"parent_id":<other>}` | 200 | ignored (**update cannot reparent**) |
| create `{"priority":150}` | 200 | ignored |

Unknown fields are silently accepted everywhere we tried them **[obs]**.

**What this means for sync:** through the API, a client can only reorder by moving nodes to the top or the bottom.
To get an arbitrary order `[n1..nk]` under one parent, move each node to `bottom` in order: `k` requests, and the
priorities become `max+100, max+200, …`. Alternatively, move them to `top` in reverse order; each move halves the
min, so after about 7 moves from 100 the min reaches 0 and the server renumbers. Inserting one node in the middle
means moving that node plus every sibling after it to the bottom.

## 3. Move endpoint: request/response details

`POST /api/v1/nodes/:id/move`, body `{"parent_id": <str>, "position": "top"|"bottom"|null}`.

### Success
```
HTTP/1.1 200
content-type: application/json
content-length: 16

{"status": "ok"}
```
The body is serialized with a space after the colon, which is why content-length is 16. The response contains no
node data. It is identical for a real move, a same-parent reorder, a move to self, and the cycle-creating move.

### Errors **[obs]**

| case | status | body |
|---|---|---|
| bad `position` (any non-enum value, including a wrong case) | 400 | `{"errors": [{"type": "enum", "loc": ["position"], "msg": "Input should be 'top' or 'bottom'", "input": <echoed value, original JSON type>}]}` |
| `parent_id` not a string (`123`, `["x"]`) | 400 | `{"errors": [{"type": "string_type", "loc": ["parent_id"], "msg": "Input should be a valid string", "input": 123}]}` |
| invalid JSON body | 400 | `{"code": "invalid_json", "message": "Invalid JSON format in request body"}` |
| `parent_id` = random well-formed UUID | 404 | `{"code": "not_found", "message": "Parent item not found"}` |
| `parent_id` = garbage string `zzzz-not-an-id` | 404 | `{"code": "not_found", "message": "Location 'zzzz-not-an-id' not found"}` |
| `parent_id` = a **deleted** node's UUID | 404 | `{"code": "not_found", "message": "Parent item not found"}` |
| `:id` = random UUID | 404 | `{"code": "not_found", "message": "Item not found"}` |
| `:id` = deleted node | 404 | `{"code": "not_found", "message": "Item not found"}` |
| `:id` = valid 12-char **short id** of an existing node | 404 | `{"code": "not_found", "message": "Item not found"}` (reproduced twice; GET with the same short id → 200) |
| `GET /nodes/:id/move` | 405 | empty body, `content-type: text/html; charset=utf-8`, `allow: POST` |

Validation order: body schema errors (400) come before existence checks (404). A random node id **and** a bad
position give the 400 enum error. A random parent and a bad position also give 400.

When a request fails, nothing changes; the node stays where it was **[obs]**.

### Body parsing
* A form-encoded body (`Content-Type: application/x-www-form-urlencoded`, `parent_id=<uuid>`) **works**: 200, and the
  node moves **[obs]**.
* `parent_id` forms accepted: full UUID, 12-char short id, and `https://workflowy.com/#/<short>` URL **[obs]**.
  Special keys (`None`, `inbox`, calendar keys, shortcuts) were **not tested**; our scope guard blocks them.
* **Not tested on purpose:** what a body with no `parent_id`, `parent_id: null`, or `parent_id: ""` *does*. Any of
  these might resolve to the account root and move a node out of the scratchpad. followup-2 established, with a random
  `:id`, that all three **pass validation** (404 `Item not found`, not 400). So `parent_id` is optional in
  `MoveNodeRequest`, which is the model name seen in `model_type` errors. Errors list in the order `parent_id, position`.
* A URL `https://workflowy.com/#/<12 hex>` naming a nonexistent node gives 404 `Parent item not found`. A bare
  nonexistent short id, or a URL that embeds a full UUID, gives `Location '<raw>' not found` (followup-2; see
  errors-and-http.md §2.2).

## 4. Structural semantics **[obs]**

* **Id preserved.** The moved node keeps its `id` and `createdAt`.
* **Descendants move with it.** x3 had children x3a(100), x3b(200) and a grandchild x3a1. After the move they still
  hang under x3 with the same ids and the same priorities.
* **Immediate consistency.** A `GET /nodes/:id` straight after the move shows the new `parent_id` and `priority`. A list
  of the new parent includes the node, and a list of the old parent no longer does, on the very next request.
* **Move to self** (`parent_id == id`): 200 `{"status": "ok"}`. Parent and priority are unchanged (the priority is
  not recomputed, unlike a same-parent move), but the node's `modifiedAt` **is bumped** (1791834655 → 1791834670).
  Result is the same with `position: "bottom"`.
* **Move into own descendant: accepted, and it corrupts the tree.** `03_move_params_and_errors.py`:
  1. `P` (child of the sandbox) has child `p1`, which has child `p1-child`.
  2. `move P → p1` gave 200. P.parent_id became p1, while p1.parent_id is still P: a 2-cycle.
  3. `move P → p1-child` gave 200. Now P → p1 → p1-child → P.
  4. The sandbox's child list no longer contained P. The whole cycle was **detached from the tree** but still
     readable by GET/list (`04_inspect_cycle.py`).
  5. Repaired with `move P → sandbox top` (200, `05_repair_cycle.py`). The structure is back to normal.

  We did not repeat this, because it puts user data in an invalid state. A human should check whether the web UI
  behaved strangely during the minutes the cycle existed (2026-10-12 roughly 19:49–19:52 UTC).

## 5. `modifiedAt` effects **[obs]**

Each step in `02_move_basics.py` was separated by 1.5 s to beat the 1-second timestamp resolution.

| node | cross-parent move | same-parent reorder | move to self | sibling renumber |
|---|---|---|---|---|
| moved node | bumped to now | bumped | bumped | n/a |
| new parent | bumped | bumped (it is also the old parent) | **not bumped** (followup-0 `06`) | n/a |
| old parent | **not bumped** | n/a | n/a | n/a |
| grandparent / other ancestors | not bumped | not bumped | – | – |
| siblings whose priority was renumbered | – | – | – | **not bumped** |
| descendants of moved node | **not bumped** (child and grandchild re-checked in followup-0 `06_move_mtimes.py`) | | not bumped | |

followup-1 (`experiments/followup-1/01_move_timestamps.py`) independently re-confirmed the whole table with a 2.1 s gap
before every action and a re-read of the sandbox, both parents, the moved node X, its child, its grandchild and an
existing child of the destination after every action **[obs]**:
3 clean cross-parent moves (P1→P2→P1→P2, `position: bottom`) bumped exactly {X, destination} each time. The old parent,
the descendants, the destination's existing child and the sandbox were not bumped. 2 move-to-self calls bumped
exactly {X}, not the parent. A same-parent move to top bumped {X, parent}. A move into or out of a **mirror
shell** behaves the same (the shell is bumped as the new parent) and never bumps the mirror's origin (see
delete-and-mirrors.md §3.5).

Related side observation: creating children did not bump the parent's `modifiedAt` either. P1 has `modifiedAt` 448
while its children were created at 449–450. The create/timestamps area should confirm this.

## 6. List order (`GET /api/v1/nodes?parent_id=`)

The docs say "unordered". In practice the order is deterministic: **ascending creation order** **[obs]**.

* `01`: order b1..b5, t1..t5, d1, d2, t6, b6, t7, b7, t8, b8. That is exactly the creation sequence, whatever the
  priorities.
* `07`: z1..z6 were created alternately in H2 and H1 (z2, z4, z6 in H1), then z6, z4 and z2 were moved into H2 in
  reverse order with mixed top/bottom. Listing H2 gives `z1, z2, z3, z4, z5, z6`, which is creation order. Several of
  these share the same `createdAt` second, so the tie-break is creation sequence, probably an internal sequence
  number **[inf]**.
* The order is unchanged by rename, complete, a move out and back, same-parent moves, and renumbering (`06`, `07`).
* followup-0 `05_list_order.py` re-confirmed it deterministically: a node created *before* P's children (under
  another parent) and moved into P at bottom is listed **first**; a node created *after* them and moved in at top is
  listed **last**. Moving both out and back with swapped positions does not change that. Mirror shells sort by the
  shell's own creation time. Across the whole request log (331 multi-node list responses at the time), 100% are
  sorted by `createdAt` and only 165 by priority; for the 277 that contain nodes whose create request is in the log,
  100% are sorted by the time of the create request, including the 213 with same-second `createdAt` ties. So the
  tie-break is creation sequence **[obs]**. For concurrent creates, list order equals the order in which the server
  assigned priorities (followup-0 §8).

Clients must still sort by `priority`. The mock should return creation order so that tests do not accidentally come to
depend on priority order.

## 7. Edge cases for move and mirror (followup-0) **[obs]**

Scripts: `experiments/followup-0/01_bottom_nonmultiple.py`, `02_top_min0.py` (outputs in `*.out` next to them).

### 7.1 Bottom when the destination max is not a multiple of 100

| destination children (live) | op | got | `max+100` would be |
|---|---|---|---|
| 0, 1, 2 (multiline-create children) | move bottom | **200**, then 300 | 102 |
| 25, 50 (the 100 was deleted) | move bottom | **200**, then 300 | 150 |
| 3 (all others deleted) | move bottom | **200**, then 300 | 103 |
| 0 only | move bottom | 100, then 200 | 100 |
| 0, 1, 2 | mirror bottom | **200**, then 300 | 102 |
| 25, 50 | mirror bottom | **200**, then 300 | 150 |
| 3 | mirror bottom | **200**, then 300 | 103 |
| 0 only | mirror bottom | 100, then 200 | 100 |
| same parent: k0=0, k1=1, k2=2; move k2 bottom (others' max 1) | move bottom | **200** | 101 |

So move and mirror use exactly create's rule, `ceil((max+100)/100)*100`.

**followup-1 settled the max > 100 case** (`experiments/followup-1/07_bottom_formula.py`). Non-multiples above 100
*can* be made through the API by halving a larger min. Make children 100, 200, 300, delete the 100 and the 200, and
create at top: you get 150. Then delete the 300. The same trick with 500 gives 250. Each parent had exactly one live
child **[obs]**:

| only live child | create bottom | move bottom | mirror bottom | `max+100` | "next multiple of 100 above max" |
|---|---|---|---|---|---|
| 50 | **200** | **200** | **200** | 150 | 100 |
| 150 | **300** | **300** | **300** | 250 | 200 |
| 250 | **400** | **400** | **400** | 350 | 300 |

So the rule is `ceil((max+100)/100)*100`, which is the same as `100*(ceil(max/100)+1)`: round max **up** to a multiple of
100, then add 100. The alternative "next multiple of 100 strictly above max" is ruled out. The rule is identical for
create, move and mirror.

### 7.2 Top when the destination min is 0

| setup | op | result |
|---|---|---|
| 0, 1, 2 (multiline children) | move mv0 → top | mv0 = 100; k0 200, k1 300, k2 400 (full renumber). The k's `modifiedAt` was not bumped |
| 0, 1, 2 | mirror → top | shell = 100; k0 200, k1 300, k2 400 |
| 0, 1, 2 plus a mirror shell at 200 | create top | new = 100; k0 200, k1 300, k2 400, **shell 500**: the shell is renumbered like an ordinary sibling |
| 100, 50, 25, 12, 6, 3, 1 plus a mirror shell at 0 (the shell is the min) | move top | moved = 100; **shell 200**, then 1→300 … 100→900 |
| 0, 1, 2; move k2 → top (same parent, others min 0) | move top | k2 = 100; k0 200, k1 300 |
| **[self=0, 100, 200]**, move self → top | move top | self = **50**, no renumber (self is excluded; others' min is 100) |
| [a=0, self=1, 100], move self → top | move top | self = 100; a 200, b100 → 300 (renumber, because others' min is 0) |
| [self=0] alone, move self → top | move top | self = **100** (no other siblings → 100) |

## 8. Concurrency (followup-0) **[obs]**

Scripts: `03_concurrency.py` (3 trials × {5 top creates, 5 bottom creates, 5 top moves, 5 bottom mirrors} into a
parent that holds one child at 100) and `04_concurrent_renumber.py` (2 trials × {5 top creates, and a mix of 2 top
creates + 2 top moves + 1 bottom create} into a parent whose min is 1, so the burst straddles the min==0 renumber).
Each request in a burst ran on its own thread with its own `ProdClient` (its own connection). Our cross-process rate
limiter still spaces request *starts* ≥ 111 ms apart. Each request took 0.18–0.45 s, so consecutive requests in a burst
always overlapped: the log shows 2 requests in flight most of the time, and up to 4.

Results across 70 concurrent mutations:

* **No duplicate priorities ever.** Top bursts gave exactly 50, 25, 12, 6, 3; bottom bursts 200 … 600; the
  renumber bursts gave 0, then 100 + full renumber, then 50, 25, 12, which is exactly the serial result.
* The **list order (creation order) of a burst equals the order the server assigned priorities**, and also the order
  in which the requests started. For example, in trial 0 the creates started top1, top0, top3, top4, top2, and got 50,
  25, 12, 6, 3 respectively. The server behaves as if every request is applied atomically and in arrival order.
* One burst (trial 0, top creates) had every request take ~0.44 s, with completions spaced by ~0.1 s. That fits
  (but does not prove) server-side serialization of writes per account **[inf]**.

Limit: we could not get more than ~4 requests in flight without breaking our 10 req/s self-imposed limit, so a race
window narrower than that cannot be ruled out. A sync engine should still tolerate equal priorities (tie-break on
something stable such as list position or id), because the web UI and other clients write by other paths. The order in
which a renumber ranks tied siblings remains unknown.

## Mock server implementation notes

Data: store `priority INTEGER` per node and a monotonically increasing `create_seq INTEGER`; `created_at` alone is not
enough because of same-second ties.

```python
def assign_priority(parent_id, position, exclude_id):
    # Used identically by create, move (incl. same-parent) and mirror.
    # Run the whole read-compute-write inside one transaction / lock: production
    # never handed out duplicate priorities under concurrency (followup-0 §8).
    others = [n for n in children(parent_id) if n.id != exclude_id]  # includes mirror shells
    sibs = [n.priority for n in others]
    if not sibs:
        return 100
    if position == "bottom":
        # max+100 rounded UP to a multiple of 100: max 0 -> 100, 1..100 -> 200,
        # 101..200 -> 300 (max in 101..199 is unreachable via the API; untested).
        return -(-(max(sibs) + 100) // 100) * 100
    # "top", None, or absent
    m = min(sibs)
    if m > 0:                 # observed for m >= 1: 1 -> 0 accepted
        return m // 2
    # m == 0 (or <= 0): renumber everyone, new node first.
    # Do NOT touch siblings' modifiedAt.
    for rank, n in enumerate(sorted(others, key=lambda n: (n.priority, n.create_seq))):
        n.priority = 100 * (rank + 2)
    return 100
```
(The order used to break ties when renumbering siblings with equal priority is unknown. Ties never arise through
the API, not even under concurrent requests (followup-0 §8); only the web UI could conceivably produce them.)

Move handler, in order:
1. Parse the body as JSON. A form-encoded body must also be accepted. Invalid JSON gives 400
   `{"code":"invalid_json","message":"Invalid JSON format in request body"}`.
2. Validate the schema (model `MoveNodeRequest`; collect all errors in the order `parent_id, position`):
   `parent_id` is optional; if present and non-null it must be a string (else 400 `string_type`). `position` must be in
   `{"top","bottom"}` or null/absent (else 400 `enum`, with `input` echoed in its original JSON type). Ignore unknown
   keys.
3. Resolve `:id` **as a full UUID only**. If it is missing or deleted, return 404 `Item not found`.
4. Resolve `parent_id` (UUID, short id, URL, and the special keys the other docs describe). A well-formed UUID that
   does not exist or is deleted gives 404 `Parent item not found`, and so does a `https://workflowy.com/#/<short>`
   URL whose short id does not exist. Any other miss gives 404 `Location '<input>' not found`
   (errors-and-http.md §2.2).
5. If `parent_id == id`: bump the node's `modifiedAt` and return ok. Do not change the priority.
6. **Cycle check: production has none.** For fidelity, a mock could allow cycles behind a flag. Our recommendation is
   to *reject* by default with a mock-specific error and record this as a known divergence, because sync code should
   never send such a move, and a cycle would break tree-walk code in the mock. If a strict-fidelity mode is wanted:
   set the parent anyway; the subtree then becomes unreachable from the root, and GET and list still work.
7. Set `priority = assign_priority(new_parent, position, exclude_id=id)` (this applies to a same-parent move too), set
   `parent_id`, set node `modifiedAt = now`, and set new parent `modifiedAt = now`. Leave the old parent, the
   ancestors and the siblings alone.
8. Respond 200 `{"status": "ok"}` (serialized exactly like that). Every change is visible to the very next request.
9. `GET` (and probably other verbs) on `/move` gives 405, empty body, `Allow: POST`.

Create: use the same `assign_priority` with `exclude_id=None`. Ignore `priority` in the body. A null or missing
`position` means top.

Update: ignore `priority`, `position` and `parent_id`.

List: return children ordered by `create_seq` ascending.

## Open questions

1. Is the `m == 0` boundary exact, or does renumbering also happen at some upper bound for bottom inserts? Bottom is
   `max + 100` without limit; we never saw an overflow or renumber on the bottom side.
2. How does the **web UI** assign priorities (midpoint between neighbours? its own renumber)? The scratchpad has
   priority 28, which suggests a different algorithm. Can the UI produce non-integer or duplicate priorities? How
   does the API then sort and renumber ties?
3. Move with a missing `parent_id`, `parent_id: null` or `parent_id: ""`. All three pass validation (followup-2).
   Probably root; the effect was not tested because of scope.
4. Move into special locations (`inbox`, calendar keys, which "are created on demand"): what priority does the moved
   node get there? Not tested (out of scope).
5. ~~Does move-to-self bump the parent's `modifiedAt`? Do descendants' `modifiedAt` change on a move?~~ Resolved by
   followup-0 `06_move_mtimes.py`: neither is bumped.
10. ~~Bottom rule for a non-multiple max above 100~~ **Resolved (followup-1 §7.1):** reachable through the API by
    halving. 150 → 300 and 250 → 400 for create, move and mirror, so it is `ceil((max+100)/100)*100`.
11. Can a wider race than ~4 concurrent requests produce duplicate priorities? How would a renumber then order ties?
6. ~~Moving a mirror node or an origin node: does it behave the same?~~ Yes. A shell moves like any node, and
   moves into or out of a shell treat it as an ordinary parent. The origin is never bumped (followup-1;
   delete-and-mirrors.md §3.5).
7. Does the server ever repair the cycle we created, or does the web client choke on it? (We repaired it ourselves.)
8. Do other verbs on `/move` (PUT/DELETE) also give 405?
9. Is `nodes-export` ordered the same way (creation order)? (Export area.)

## UI scenarios (for a human to perform; we then observe through the API)

Do each one inside a fresh sandbox under the scratchpad. Before and after, record the priorities, `modifiedAt` and the
list order for all siblings, the old parent and the new parent.

1. **Drag-reorder to the middle**: in a parent with children 100/200/300/400, drag the last child between the first and
   second. Observe: does the UI use a midpoint (150), shift the others, or renumber everything? Whose `modifiedAt`
   changes?
2. **Keyboard reorder** (Ctrl/Cmd+Shift+Up/Down) of one item by one step, repeated many times between the same two
   neighbours, to see what happens when the gap runs out (integer midpoint exhaustion → renumber? fractional values?).
3. **Press Enter at the end of an item** to create a new sibling below it, in the middle of a list. What priority does
   it get? Then press Enter on an empty first line to create one above.
4. **Indent (Tab)** a middle item, so it becomes the last child of its previous sibling. Observe its new priority,
   whether it is appended (`max+100`?), and the `modifiedAt` of the old and new parents.
5. **Outdent (Shift+Tab)** a child, so it becomes a sibling right after its former parent: a middle insertion in the
   grandparent's list. Observe the priority and whether the grandparent's later siblings are rewritten.
6. **Move To… dialog** (UI "Move" command) to another parent. Compare top/bottom placement with the API's.
7. **Drag a node onto another node's bullet** (making it a child): where does it land, and what is its priority?
8. **Drag a node into its own descendant** in the UI. Presumably prevented; this confirms the UI blocks the case that
   the API allows.
9. **Sort command**, if available (e.g. "Sort children" or alphabetical): how are priorities rewritten?
10. **Delete then undo (Ctrl+Z)** of a middle child: does the restored node get its old priority and id?
    Is its `createdAt` or creation-order position in the list preserved?
11. **Cut and paste** a node elsewhere: is it a move (same id) or delete+create (new id)? What priority does it get?
12. **Collapse/expand** a parent: confirm it has no effect on the priority or `modifiedAt` of the children.
13. Look in the UI at the sandbox `ordering-move: move params and errors` (`7f261b8d…`). Check that P, p1 and
    p1-child look normal after the cycle repair, and report any oddity (e.g. a duplicated or missing P).
14. Check whether the UI's displayed order **always** matches ascending `priority` for the sandboxes made here, in
    particular `ordering-move: create priorities top/bottom` (`0417219b…`), where list order and priority order differ
    heavily.
15. **Bottom with a UI-made odd priority** (followup-0): in a fresh sandbox create children 100, 200, 300 through the
    API. In the UI, reorder them (drag the first child between the other two, drag the middle one to the end, press
    Enter at the end of the last item to add a new last sibling) until the API shows a sibling whose priority is not a
    multiple of 100, ideally the **largest** one (e.g. 250 or 350). Then we send an API `move`/`create` with
    `position: bottom` and check whether we get `ceil((max+100)/100)*100` (e.g. 350 → 500, not 450).
16. **Duplicate priorities from the UI** (followup-0): reorder quickly in the UI in two browser tabs, or reorder while
    offline in one tab and online in another, then look for equal `priority` values via the API. If any appear,
    we send one API top-create into a min-0 version of that parent to learn how a renumber orders the ties.

## Sandboxes created (left in place)

| sandbox name | id |
|---|---|
| followup-0: 01 bottom when max not multiple of 100 | c4d3218b-ac84-4300-8cca-5acdc55994a8 |
| followup-0: 02 top into min-0 parent; self-at-0 | 6e1d20bf-de32-43b8-b751-d35525071dc3 |
| followup-0: 03 concurrency | c7c34ec1-ce7e-489e-9507-68e63f0a2a3c |
| followup-0: 04 concurrent inserts across renumber | c2e9731e-965a-4485-88c9-83e9f7adffb9 |
| followup-0: 05 list order | 0d32311c-0278-4f24-bd79-4001189ca8ac |
| followup-0: 06 move modifiedAt of descendants / self-move parent | ce830aae-ca49-4d5c-8913-46b24cb47852 |
| ordering-move: create priorities top/bottom | 0417219b-648d-4d3a-9025-9d94be99bee3 |
| ordering-move: move basics | 6602fd27-72d0-4afa-b148-a96dd8926d89 |
| ordering-move: move params and errors (the cycle experiment, repaired) | 7f261b8d-2e2b-4a1b-a776-207c6f28b6d1 |
| ordering-move: priority misc | 71661a68-5e34-4907-8103-b3162bb13e1f |
| ordering-move: list order + renumber mtime | 2940f280-653c-416c-995f-c549d0719206 |
| ordering-move: top when already top | d37a7329-eb6c-42b4-9d7c-168cf96a26fe |
| ordering-move: recompute vs noop | 987354d2-d326-4250-9677-3af88dfb2d1b |
| ordering-move: move validation (x2: first run crashed on our own guard after 3 requests) | 354ab2ae-baf5-45d5-aab5-3d9d2b988c82 and one earlier |
