# UI scenario findings (human-in-the-loop)

Results of the scenarios in [ui-scenarios.md](ui-scenarios.md), performed by a human in the Workflowy web UI and
observed via the API with `experiments/ui/ui.py` (snapshots under `experiments/ui/snaps/<scenario>/`).

## A1. Drag-reorder into the middle

Setup `a, b, c, d` = 100/200/300/400 (API bottom creates).

- Step 1 — drag `d` between `a` and `b`: `d.priority` 400 → **150** (midpoint of neighbours). No other sibling's
  priority changed. `modifiedAt` bumped on **`d` and the parent** (sandbox); siblings untouched. List order
  (creation order) unchanged.
- Step 2 — drag `a` to the end (after `c`=300): `a.priority` 100 → **400** (consistent with API bottom
  `ceil((max+100)/100)*100`). Again only the moved node and the parent were bumped; no renumbering.

## A2. Enter to create a sibling mid-list

Setup `a, b, c, d` = 100/200/300/400.

- Step 1 — Enter at end of `b`, type `new-below-b`: new node priority **250** (midpoint), note `null`,
  layout `bullets`, `createdAt` 1 s before `modifiedAt` (typing after creation). **Parent `modifiedAt` NOT bumped**
  (same as API create). No sibling changed. Appended at the end of list order (creation order).
- Step 2 — Enter at the *start* of `c` (creates an empty item above `c`), type `new-above-c`: a **new** node is
  created at priority **275** (midpoint of 250 and 300); `c` keeps its id and is entirely unchanged (not even
  `modifiedAt`). Parent not bumped.
- Step 3 — Enter at end of last item `d` (400), type `new-last`: priority **500** (= API bottom rule). Parent not
  bumped.
