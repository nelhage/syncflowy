# Follow-up 0: priority edge cases for move/mirror, concurrency, list order

Tag `followup-0`. Scripts and their captured output are in `experiments/followup-0/` (`NN_*.py` and `NN_*.out`).
About 410 production requests were made on 2026-10-12, all inside six sandboxes under the scratchpad (listed at the
end). Every request and response is in `.prod-state/requests.jsonl` with `"tag": "followup-0"`.

**[obs]** = observed directly. **[inf]** = inferred.

## Summary of what was established

| # | Question | Answer |
|---|---|---|
| 1 | Bottom when the destination's live max is not a multiple of 100 | **`ceil((max+100)/100)*100`** for create, move and mirror alike. `max+100` was wrong. **[obs]** |
| 2 | Move/mirror to top into a parent whose min is 0 | **Full renumber**, exactly as for create (new node 100, others 200, 300, … in priority order, their `modifiedAt` not bumped). Mirror shells are renumbered like ordinary siblings. **[obs]** |
| 3 | Can concurrent inserts produce duplicate priorities? | **No**, in 70 overlapping mutations (2–4 in flight). The results equal a serial execution in request-arrival order. **[obs]** (a narrower race cannot be ruled out) |
| 4 | List order | **Ascending creation sequence**, whatever the parent, priority or move history. Ties within one `createdAt` second follow creation-request order. **[obs]** |
| 5 | Same-parent move-to-top when self is the only 0 | `[self=0, 100, 200]` → self = **50**, no renumber (self is excluded). **[obs]** |
| extra | Descendants' `modifiedAt` on move; parent's on move-to-self | **Neither is bumped.** **[obs]** |

## 1. Bottom with a non-multiple max (`01_bottom_nonmultiple.py`)

Setups: "multiline" parents (created with name `"X\n\n- k0\n\n- k1\n\n- k2"`, which gives children at 0, 1, 2); a
parent with top-created 100, 50, 25 where the 100 was then deleted; one where only the 3 survives; one where only
the 0 survives. Each received two bottom inserts in a row.

| live children | 1st bottom (move) | 1st bottom (mirror) | 2nd bottom | `max+100` would give |
|---|---|---|---|---|
| 0, 1, 2 | 200 | 200 | 300 | 102 |
| 25, 50 | 200 | 200 | 300 | 150 |
| 3 | 200 | 200 | 300 | 103 |
| 0 | 100 | 100 | 200 | 100 |
| same-parent: move k2 (2) bottom among 0, 1 | 200 | – | – | 101 |

Max 0 → 100 also rules out "smallest multiple of 100 strictly greater than max+…" variants that would give 200. The
formula `ceil((max+100)/100)*100` (Python: `-(-(max+100)//100)*100`) fits every observation, on create too (create.md
§2.1).

Untestable through the API: a max in 101–199 that is not a multiple of 100. The API only produces non-multiples of 100
below 100 (halving and multiline children). Renumbers and bottoms only produce multiples. See UI scenario U1.

## 2. Top into a min-0 parent (`02_top_min0.py`)

| setup | operation | result |
|---|---|---|
| k0=0, k1=1, k2=2 | move mv0 top | mv0 100; k0 200, k1 300, k2 400. The k's `modifiedAt` was unchanged (1791835555) |
| 0, 1, 2 | mirror top | shell 100; 200, 300, 400 |
| 0, 1, 2 + shell at 200 | create top | new 100; 200, 300, 400, **shell 500** |
| 1..100 halving chain + **shell at 0** | move top | moved 100; **shell 200**; 1→300 … 100→900 |
| 0, 1, 2; move k2 top (same parent) | move top | k2 100; k0 200, k1 300 |
| [self=0, 100, 200] | move self top | self **50**; no renumber |
| [a=0, self=1, 100] | move self top | self 100; a 200; 100 → 300 |
| [self=0] alone | move self top | self **100** (no other siblings) |

## 3. Concurrency (`03_concurrency.py`, `04_concurrent_renumber.py`)

Method: N=5 threads, each with its own `ProdClient` (own HTTP connection), released by a barrier. The shared rate
limiter spaces request starts ≥ 111 ms apart (we are not allowed to exceed 10 req/s), and requests took 0.18–0.45 s,
so consecutive requests always overlapped. Overlap in the log: 2 in flight typically, up to 4.

| burst (per trial) | trials | priorities obtained | duplicates |
|---|---|---|---|
| 5 top creates into [100] | 3 | 50, 25, 12, 6, 3 | none |
| 5 bottom creates into [100] | 3 | 200, 300, 400, 500, 600 | none |
| 5 top moves into [100] | 3 | 50, 25, 12, 6, 3 | none |
| 5 bottom mirrors of one origin into [100] | 3 | 200 … 600 | none |
| 5 top creates into [100, 50, 25, 12, 6, 3, 1] | 2 | 0, then 100 + full renumber, then 50, 25, 12 | none |
| 2 top creates + 2 top moves + 1 bottom create into the same | 2 | 0 → renumber → 100/200 for the creates, 50/25 for the moves, 1000 for the bottom | none |

In every burst, the creation order (= list order) matched the request start order, and the priorities matched a
serial execution in that order. For example, trial 0's creates started top1, top0, top3, top4, top2, were listed in
that order, and got 50, 25, 12, 6, 3. The server therefore behaves as if writes are applied atomically, one at a time,
in arrival order **[inf: probably a per-account lock or queue]**.

**Implication for the sync engine:** priority ties do not arise from API concurrency at the concurrency we could
reach. The engine should still tolerate ties defensively (UI or offline clients may produce them; see U2), using a
stable tie-break. **Mock:** do the read-compute-write of `assign_priority` (including a renumber) inside a single
SQLite transaction, and serialize writes. That reproduces the observed behaviour exactly.

## 4. List order (`05_list_order.py`)

EARLY was created under another parent *before* P's children. P then got p0 bottom (100), p1 top (50), p2 bottom (200)
and p3 top (25). LATE was created elsewhere *after* them.

```
P before moves      : p0-bottom 100, p1-top 50, p2-bottom 200, p3-top 25
LATE -> P top       : p0 100, p1 50, p2 200, p3 25, LATE 12
EARLY -> P bottom   : EARLY 300, p0 100, p1 50, p2 200, p3 25, LATE 12
out and back (EARLY top, LATE bottom):
                      EARLY 12, p0 100, p1 50, p2 200, p3 25, LATE 300
createdAt           : EARLY …674, p0..p3 …675 (same second), LATE …676
```

The order is ascending creation sequence and is independent of priority and of the move history. Across the whole
request log: 331 multi-node list responses, **331** sorted by `createdAt`, only 165 sorted by priority. Of the 277 that
contain nodes whose create request is logged, all 277 are sorted by the create-request timestamp, including 213 with
same-second `createdAt` ties. errors-and-http.md §8 had already been corrected by followup-2; this confirms it.

## 5. modifiedAt on move (`06_move_mtimes.py`)

```
before      A 699  B 700  X 700  X1 700  X11 700  S 701  sandbox 699
move X -> B (bottom) at 704:
after       A 699  B 704  X 704  X1 700  X11 700  S 701           # new parent + moved node only
move X -> X at 707:
after       B 704  X 707  X1 700 X11 700                           # self-move bumps only X
```

Descendants are not bumped by a move. A move-to-self bumps only the node itself, not its parent. This agrees with
followup-2's move-to-self result. (Critic item "move-to-self": the mock should bump the node's `modifiedAt`.)

## Docs updated in place

* `ordering-and-move.md`: TL;DR bottom rule corrected to the rounded-up formula; added a serialization bullet and the
  self-exclusion example; §1 caveat; §5 table (descendants and move-to-self parent now observed); §6 list-order
  re-confirmation; new §7 (move/mirror edge cases) and §8 (concurrency); mock `assign_priority` fixed (rounded bottom,
  run in one transaction, mirror shells count as siblings); open questions and UI scenarios; sandbox list.
* `delete-and-mirrors.md` §1.5: "max(existing)+100" corrected. §2.3: followup-0 mirror edge cases added, and a note
  that the printed error keys are sorted (real order `type, loc, msg, input`).
* `create.md` §2.1: the bottom formula is now confirmed for move and mirror too.
* `errors-and-http.md` §8: re-confirmation note.

## Mock server implementation notes

```python
def assign_priority(tx, parent_id, position, exclude_id=None):
    # Single transaction; writes are serialized (BEGIN IMMEDIATE in SQLite).
    others = [n for n in live_children(tx, parent_id) if n.id != exclude_id]  # mirror shells included
    if not others:
        return 100
    if position == "bottom":
        return -(-(max(n.priority for n in others) + 100) // 100) * 100
    m = min(n.priority for n in others)          # top / null / absent
    if m > 0:
        return m // 2
    for rank, n in enumerate(sorted(others, key=lambda n: (n.priority, n.create_seq))):
        n.priority = 100 * (rank + 2)            # do NOT touch modifiedAt
    return 100
```

* Use the same function for create (`exclude_id=None`), move (`exclude_id=id`, also same-parent) and mirror (new
  shell, `exclude_id=None`). Move-to-self (`parent_id == id`) skips it entirely and only bumps the node's `modifiedAt`.
* The tie order in a renumber (`create_seq` above) is a guess; ties cannot be produced through the API.
* List: `ORDER BY create_seq`. A moved node keeps its `create_seq`. A mirror shell gets a fresh one.
* Move bumps `modifiedAt` of the moved node and the new parent only (not descendants, not the old parent).

## Open questions

1. Bottom rule for a non-multiple max in 101–199 (needs a UI-made priority; U1).
2. Can a race wider than ~4 concurrent requests produce duplicate priorities? We could not test that within
   10 req/s. If duplicates exist, how does a renumber order them?
3. Does the web UI write priorities through the same serialized path (so that UI and API edits interleave safely)?

## UI scenarios

* **U1, odd max:** in a fresh sandbox with API-made children 100/200/300, reorder in the UI (drag, Enter at the end of
  the last item) until the API shows a largest priority that is not a multiple of 100 (e.g. 250/350). Then we send an
  API bottom move/create to test `ceil((max+100)/100)*100`.
* **U2, UI-made ties:** reorder the same list quickly from two browser tabs (or one offline tab plus one online tab),
  then look for equal priorities via the API. If any appear, we trigger an API top insert into that parent, with min
  0, to see how ties are renumbered.
* **U3:** view the sandbox `followup-0: 02 top into min-0 parent; self-at-0` in the UI and check that the displayed
  order matches ascending priority, including for the mirror shells (C, C2), after the renumbers.

## Sandboxes (left in place)

| name | id |
|---|---|
| followup-0: 01 bottom when max not multiple of 100 | c4d3218b-ac84-4300-8cca-5acdc55994a8 |
| followup-0: 02 top into min-0 parent; self-at-0 | 6e1d20bf-de32-43b8-b751-d35525071dc3 |
| followup-0: 03 concurrency | c7c34ec1-ce7e-489e-9507-68e63f0a2a3c |
| followup-0: 04 concurrent inserts across renumber | c2e9731e-965a-4485-88c9-83e9f7adffb9 |
| followup-0: 05 list order | 0d32311c-0278-4f24-bd79-4001189ca8ac |
| followup-0: 06 move modifiedAt of descendants / self-move parent | ce830aae-ca49-4d5c-8913-46b24cb47852 |
