# Follow-up 1: timestamp side effects and mirror identity

Tag `followup-1`. 660 production requests on 2026-10-12, server clock about
`1791835510`–`1791835800`. Statuses: 629×200, 20×404, 11×400. Most requests were
`GET`s taking before/after `modifiedAt` snapshots. Scripts and their full transcripts
are in `experiments/followup-1/NN_*.py` and `NN_*.out`. `common.py` prints response
bodies in the server's own key order.

**Method for timestamp questions.** `modifiedAt` has 1-second resolution. Every action
is preceded by a **2.1 s** sleep, then a GET of every node of interest, then the
action, then a second GET of every node. A node counts as "bumped" when its value
changed. Because of the 2.1 s gap, a bump is always visible, and a node touched by
setup can never look bumped by accident.

The docs updated in place are `delete-and-mirrors.md` (TL;DR, §2.2, §2.4, §2.7, new
§3.5, §4.2, new §4.4, §7 mock notes, §8 open questions, §9 U18–U20),
`ordering-and-move.md` (§5, §7.1, TL;DR, open questions), `errors-and-http.md`
(§3 precedence table, mock lookup step 7) and `update-and-complete.md` (the
modifiedAt section and the mock notes).

---

## 1. Results at a glance [all observed]

| question | answer | evidence |
|---|---|---|
| Does a move bump the moved node's descendants? | **No** (child and grandchild, 3 trials) | `01` |
| Does move-to-self bump the parent? | **No.** Only the node itself (2/2). | `01` |
| Is the destination-parent bump on a clean cross-parent move reproducible? | **Yes, 3/3.** Exactly {moved node, new parent} are bumped. Old parent, sandbox and the destination's existing child are not. | `01` |
| Move into or out of a mirror shell: who is bumped? | Move-in bumps {node, **shell**}. Move-out bumps {node, new parent}. The **origin is never bumped.** | `02` |
| Create, update, complete, uncomplete or delete a child under a shell: is the origin bumped? | **Never.** Create bumps nobody, update and complete bump only the child, delete bumps nobody. | `02` |
| Update, complete or move the shell itself: is the origin bumped? | **Never.** | `02` |
| Plain `DELETE /nodes/<shell>` vs `DELETE /nodes/<shell>/mirror`: origin bump? | Plain: **no bump, 3/3**. `/mirror`: **bump, 3/3**. Both return 200 and the shell is 404 afterwards. Deleting an ancestor of a shell does not bump the origin either. | `03` |
| `POST /nodes/M1/mirror` where M1 is a mirror of O: who is bumped? | **O** (the resolved origin), 2/2. M1, M1's parent and the destination are not. | `04` |
| `parent_id` = mirror M1, redirected to origin O: is O bumped as a parent? | **No.** Only the mirrored node X (the new mirror's origin) is bumped. | `04` |
| Mirror creation vs move-in into the same parent | Mirror creation bumps only the origin. Move-in bumps the destination. Create bumps nothing. The asymmetry is re-confirmed. | `04` |
| `POST /mirror` check order | JSON → validation (all errors) → `:id` lookup → origin resolution → parent lookup and redirect → self check | `05` |
| `parent_id` = orphan shell | 404 `{"code": "not_found", "message": "Parent item not found"}` (2 different orphans) | `05`, `06` |
| `parent_id` = garbage string on mirror | 404 `Parent item not found`. Move and create give `Location '…' not found` instead. | `05` |
| `DELETE /:id/mirror` on a plain child of a shell | 400 `{"code": "invalid_request", "message": "Node is not a mirror"}`. The node is untouched. | `05` |
| Orphan shell: delete, move, move-in, create-under, update, complete | All behave **normally** (200, normal bumps). Plain delete removes the subtree. | `06` |
| Mirror *of* a plain child of an orphan | 200, normal | `05` |
| Bottom priority when max is not a multiple of 100 and is above 100 | 150 → **300**, 250 → **400**, 50 → **200**, the same for create, move and mirror. That is `ceil((max+100)/100)*100` and rules out "next multiple above max". | `07` |

---

## 2. Move timestamps (`01_move_timestamps.py`, sandbox `7ca45ac4-…`)

Tree: `S > P1 > X > X1 > X2`, plus `S > P2 > P2child`. Excerpt of the transcript:

```
=== trial0 move X -> P2 bottom
      P2   1791835513 -> 1791835518  BUMPED
      X    1791835513 -> 1791835518  BUMPED
      (S, P1, X1, X2, P2child unchanged)
=== trial1 move X -> P1 bottom      bumped=['P1', 'X']
=== trial2 move X -> P2 bottom      bumped=['P2', 'X']
=== self-move0 X -> X               bumped=['X']
=== self-move1 X -> X               bumped=['X']
=== same-parent move X top          bumped=['P2', 'X']
```

This agrees with followup-0's `06_move_mtimes.py`, so the "descendants not bumped" entry
in ordering-and-move.md is now observed twice, independently.

## 3. Mirror shells as parents (`02_shell_side_effects.py`, sandbox `2d97a9b5-…`)

`S > O > oc`, `S > D > M` (M is a mirror of O), and `S > L`.

| action | bumped |
|---|---|
| move L → M | M, L |
| move L → S (bottom) | S, L |
| move L → M again | M, L |
| create K under M | – |
| update K name | K |
| complete K / uncomplete K | K / K |
| update M name | M |
| complete M | M |
| move M → S / M → D | S, M / D, M |
| delete K | – (only K itself became 404) |
| update O name (control) | O |

O was not bumped by any of these, so the origin bump really is specific to the two mirror
endpoints. Final `list M` returned L only. `list O` returned only `oc`.

## 4. Plain delete vs `DELETE /mirror` (`03_delete_mirror_variants.py`, sandbox `d3b2a5fb-…`)

The script interleaved 3 trials of each variant, each with a fresh origin `O<i>-<variant>` and its mirror in `D`:

```
RESULTS [(0,'plain',[]), (0,'mirror',['O']), (1,'plain',[]), (1,'mirror',['O']),
         (2,'plain',[]), (2,'mirror',['O']), ('H','ancestor',[])]
```

All 6 deletes returned `200 {"status": "ok"}`. In all 6 trials `GET <shell>` afterwards returned
`404 {"code": "not_found", "message": "Item not found"}`. D and S were never bumped.
The upstream doc says the two "have the same effect". That holds for the tree but **not for
the origin's `modifiedAt`**. A mock that wants fidelity must give them separate code paths.

## 5. Mirror-of-mirror and redirected parent (`04_mirror_of_mirror_bumps.py`, sandbox `97c58d49-…`)

```
POST /api/v1/nodes/<M1>/mirror {"parent_id": "<D2>"}      (twice)
-> 200 {"item_id": "...", "origin_id": "f6f2cd16-ec3a-4779-ac76-a25a27078436"}   (= O)
   bumped: O   (M1, D1, D2 unchanged)

POST /api/v1/nodes/<X>/mirror {"parent_id": "<M1>"}
-> 200 {"item_id": "fcfa246e-…", "origin_id": "<X>"}
GET fcfa246e-… -> {"node": {"id": "fcfa246e-…", "name": "", "note": "",
   "parent_id": "f6f2cd16-…" (O), "priority": 100, "completed": false,
   "data": {"layoutMode": "bullets"}, "createdAt": 1791835689, "modifiedAt": 1791835689,
   "completedAt": null}}
   bumped: X   (O, the effective destination, NOT bumped; M1 not bumped)

mirror Y -> D3   bumped: Y
move   Z -> D3   bumped: D3, Z
create under D3  bumped: nothing
```

## 6. `POST /mirror` check order (`05_mirror_check_order.py`, sandbox `468bb6aa-…`)

The full table with exact bodies is in delete-and-mirrors.md §2.7 under "Check order". Pseudocode:

```python
body = parse_json_or_400_invalid_json()
errs = validate(MirrorNodeRequest, body)          # parent_id then position; all errors
if errs: return 400, {"errors": errs}
src = by_full_uuid(id)            or 404 "Item not found"
origin = resolve_origin(src)      or 404 "Item not found"       # orphan source
parent = by_full_uuid(parent_id)  or 404 "Parent item not found"  # garbage too: never "Location"
if parent.mirror_origin_id:
    parent = resolve_origin(parent) or 404 "Parent item not found"  # orphan parent
if parent.id == origin.id: return 400 "Cannot mirror a node under itself"
```

An orphan `:id` with `parent_id` = itself gives `Item not found`, not the self error. That is
consistent with origin resolution coming first.

## 7. Orphans (`06_orphan_ops.py`, sandbox `701c64ae-…`)

Every operation except `POST /mirror` treats an orphan shell as an ordinary node. See
delete-and-mirrors.md §4.4. The orphan N, its child and a moved-in node were deleted at
the end by plain DELETE (the test of plain delete), so the sandbox holds only the empty
holders H and H2 and the deleted-origin bookkeeping. Orphan N (with child nc) and a mirror of
nc in D survive in the `mirror check order` sandbox.

## 8. Bottom formula (`07_bottom_formula.py`, sandbox `845155fe-…`)

Non-multiples above 100 can be built through the API with top-halving. Make 100, 200 and 300,
delete the 100 and the 200, create at top to get 150, then delete the 300. Results (each parent had one live child):

| live max | create bottom | move bottom | mirror bottom |
|---|---|---|---|
| 50 | 200 | 200 | 200 |
| 150 | 300 | 300 | 300 |
| 250 | 400 | 400 | 400 |

`bottom = ceil((max + 100) / 100) * 100 = 100 * (ceil(max / 100) + 1)`, and `100` for an empty parent.

---

## 9. Positions on the critic's contradiction list

These items were in followup-1's scope:

- **Move-to-self**: it bumps the node only. A mock should bump the node and not the parent. This
  agrees with followup-2.
- **Mirror delete equivalence**: settled. The two paths differ in the origin bump (3/3 each).
- **Mirror note heuristic**: corrected in delete-and-mirrors.md §2.2. Multiline-split children also get
  `note: ""`. `name == "" and note == ""` is better but can still be forged. There is no side-effect-free test.
- **Bottom formula**: `ceil` confirmed beyond 100 for create, move and mirror.
- **"Descendants not bumped on move"**: now observed (followup-0 and followup-1).
- **Cycle handling in the mock**: not something experiments can decide. followup-1 adds no new data.
  The other docs already describe the trade-off.
- Not in scope and not touched: list order, line-join rule, entity decode order, model names,
  encoding headers, and update C0 stripping. Some of these were already resolved by followup-0
  and followup-2 in the docs.

## 10. Mock server implementation notes (delta)

1. `modifiedAt` bump table (complete as far as we know):

   | operation | bumps |
   |---|---|
   | create | new node only (`createdAt = modifiedAt = now`) |
   | update / complete / uncomplete | the node only, even on a no-op |
   | move (including same-parent) | node + destination parent |
   | move to self | node only |
   | delete (plain, any node incl. shell or orphan) | nothing |
   | `POST /:id/mirror` | resolved origin only (+ new shell's own timestamps) |
   | `DELETE /:id/mirror` | origin (if it still exists) |
   | renumber on top-insert at min 0 | nothing (renumbered siblings keep `modifiedAt`) |

   No other node is ever bumped. That includes descendants, old parents, ancestors, a redirected
   mirror destination, and a mirror's origin for anything done to or under the shell.
2. Keep `DELETE /nodes/:id` and `DELETE /nodes/:id/mirror` as separate handlers. The only
   difference beyond the "is a mirror" check is the origin bump.
3. Mirror `parent_id` resolution is exact-UUID only and never produces `Location '…' not found`.
   An orphan parent gives `Parent item not found`.
4. Store `mirror_origin_id` and never clear it when the origin is deleted. Use it only in `POST /mirror`
   (source and parent resolution), in `DELETE /mirror` (eligibility, plus the bump if the origin exists),
   and nowhere else.

## 11. Open questions

1. Why are mirror creation and child creation not "touching" the parent while move-in is? This is
   presumably an implementation detail (move rewrites the parent's child list). It cannot be
   established from the API, and the mock just copies the table above.
2. Does a mirror *chain* (a shell whose `mirror_origin_id` points to another shell) ever exist? Through the
   API it cannot be built. The UI might make one (UI scenario U15 in delete-and-mirrors.md).
3. Does `DELETE /mirror` on an orphan bump anything? We cannot see the deleted origin, and nothing
   else was bumped. Untested beyond the 200 response.
4. Are the origin bumps from `POST /mirror` and `DELETE /mirror` visible to the web UI or the
   `nodes-export` endpoint as "recently modified"? This cannot be tested within the scope guard.

## 12. UI scenarios

A human performs each action in the web UI. Afterwards we read `modifiedAt` and `parent_id` through the API,
using the same 2 s gap method:

1. **Add a child to a mirror in the UI.** Is the child's `parent_id` the origin or the shell? Which of
   origin and shell get a `modifiedAt` bump? (Through the API: the shell's id, and neither is bumped.)
2. **Edit the text of a mirror in the UI.** Which record's `name` and `modifiedAt` change?
3. **Delete a mirror in the UI** (Delete key) and, separately, **"Remove mirror"** if the UI offers it.
   Does the origin's `modifiedAt` bump in each case? (API: only `DELETE /mirror` bumps.)
4. **Drag a node into a mirror in the UI.** Does it land under the shell or under the origin? Is the
   origin bumped?
5. **Indent and outdent in the UI.** Confirm that the "destination bumped, source not" rule also
   holds for UI moves, and that descendants are not bumped.
6. **Look at sandbox `followup-1: mirror check order`** in the UI. H contains orphan N (origin
   deleted through the API) with child nc, and D contains a mirror of nc. How does the UI render an
   orphan? Can the UI "repair" or convert it?
7. **Look at sandbox `followup-1: bottom formula`** and check that the UI order matches ascending
   priority for the 150/300 and 250/400 pairs.

## Sandboxes (left in place)

| name | id |
|---|---|
| sandbox: followup-1: move timestamps | 7ca45ac4-2a8d-4bb6-be8d-0a01becdf2a4 |
| sandbox: followup-1: shell side effects | 2d97a9b5-1623-4fe8-970f-4a1bd793d07d |
| sandbox: followup-1: delete vs delete-mirror | d3b2a5fb-93e7-4621-8ff1-7fefa1a66e17 |
| sandbox: followup-1: mirror-of-mirror bumps | 97c58d49-b4a5-484a-964d-776537a98e49 |
| sandbox: followup-1: mirror check order | 468bb6aa-5067-4224-98a2-a26cd59333e6 |
| sandbox: followup-1: orphan ops | 701c64ae-5633-4076-bed3-9340013cf12a |
| sandbox: followup-1: bottom formula | 845155fe-8e21-4026-b1ab-275cf67b3bf1 |
