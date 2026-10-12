# Follow-up 2: validation shapes and id/parent resolution

Tag `followup-2`, 88 production requests, run on 2026-10-12. All requests went
through `ProdClient`.

- Scripts:
  - `experiments/followup-2/01_validation_and_parent_resolution.py` (output in
    `01_validation_and_parent_resolution.out.json`)
  - `experiments/followup-2/02_move_to_self_mtime.py` (output in
    `02_move_to_self_mtime.out.json`)
- Sandboxes left in place under the scratchpad:
  - `sandbox: followup-2: parent_id resolution misses` holds one child,
    "mover (should never actually move)". We confirmed it never moved.
  - `sandbox: followup-2: move-to-self modifiedAt` holds Y0 and Y1.

Safety method: every probe did at least one of three things.
1. It targeted a random-UUID path id.
2. It failed body validation (for example `name: 1`), so nothing executed.
3. It used a `parent_id` that cannot exist. In that case the path id was our
   own sandbox node.

`unscoped=True` was needed only because the guard rejects parent ids that are
not on its allow-list. In each such request the *probed* id could not exist,
with one exception: a read-only list of
`https://workflowy.com/#/<our sandbox UUID>`.

Every statement below is **observed** unless it is marked *inferred*.

The existing docs were updated in place: errors-and-http.md (most changes),
create.md, update-and-complete.md, ordering-and-move.md and
delete-and-mirrors.md.

---

## 1. Results

### 1.1 Update reports every validation error

`POST /nodes/<random>` with `{"name":1,"note":2,"layoutMode":3}`, and the
same fields in reversed key order, gave identical results in 3 runs:

```
400 {"errors": [{"type": "string_type", "loc": ["name"], "msg": "Input should be a valid string", "input": 1}, {"type": "string_type", "loc": ["note"], "msg": "Input should be a valid string", "input": 2}, {"type": "string_type", "loc": ["layoutMode"], "msg": "Input should be a valid string", "input": 3}]}
```

- Errors are collected, not short-circuited.
- They appear in declaration order (`name, note, layoutMode`). The order of
  keys in the body does not matter.
- `{"parent_id":1,"position":"q","completed":"x","priority":"z"}` gives 404
  `Item not found`, so none of those keys is type-checked on update.
- `{"name":1,"position":"q"}` reports only the name error.

The update-and-complete.md mock pseudocode reported only the first error. It
now collects all of them.

### 1.2 Model names (from `model_type` errors)

We sent bodies `[1]`, `"s"` and `null` to each endpoint.

| Endpoint | Model |
|---|---|
| create | `CreateItemRequest` |
| update | `UpdateNodeRequest` |
| move | `MoveNodeRequest` |
| mirror | `MirrorNodeRequest` |

Example:

```
POST /nodes/<random>/move   body: [1]
400 {"errors": [{"type": "model_type", "loc": [], "msg": "Input should be a valid dictionary or instance of MoveNodeRequest", "input": [1]}]}
```

An empty body on move or mirror gives 400 `invalid_json`. errors-and-http.md
had guessed `CreateNodeRequest`. That was wrong.

### 1.3 `parent_id`: required or optional, accepted types, field order

| Body | create (`POST /nodes`) | move (random `:id`) | mirror (random `:id`) |
|---|---|---|---|
| `{}` | 400 `[missing name]` only | **404 Item not found** (passes) | 400 `[missing parent_id, input {}]` |
| `parent_id: null` | (with `name:1`) only the name error | **404** (passes) | 400 `string_type`, `input: null` |
| `parent_id: ""` | (with `name:1`) only the name error | **404** (passes) | **404** (passes) |
| `parent_id: 1` | `string_type` | `string_type` | `string_type` |
| `parent_id: ["x"]` | — | `string_type` | `string_type` |
| `{"position":"q"}` | — | `[enum position]` | `[missing parent_id, enum position]` |

Declaration (error) orders, independent of body key order:

| Model | Order |
|---|---|
| `CreateItemRequest` | `name, parent_id, note, layoutMode, position` |
| `UpdateNodeRequest` | `name, note, layoutMode` |
| `MoveNodeRequest` | `parent_id, position` |
| `MirrorNodeRequest` | `parent_id, position` |

create.md had inferred that `parent_id` came first. It comes second.

Other notes on requiredness:
- A `missing` error and a `string_type` error can appear together:
  `{"parent_id":1}` on create gives
  `[{"type":"missing","loc":["name"],...,"input":{"parent_id":1}}, {"type":"string_type","loc":["parent_id"],...}]`.
- **`MoveNodeRequest.parent_id` is optional and nullable.** A move with no
  `parent_id` would *execute* against a real node. Its effect is untested and
  is probably a move to the account root. A sync engine must always send
  `parent_id`.

### 1.4 Not-found messages for `parent_id`

`:id` for move and mirror was a real sandbox node. Every parent was random and
could not exist. The rows for the URL with a random short id and for the bare
short id were run twice with different random values. The other rows were run
once.

| `parent_id` | list | create | move | mirror |
|---|---|---|---|---|
| `https://workflowy.com/#/<rand12>` (lower or UPPER) | Parent item not found | Parent item not found | Parent item not found | Parent item not found |
| `<rand12>` (bare) | `Location '<x>' not found` | Location | Location | Parent item not found |
| `https://workflowy.com/#/<random UUID>` | `Location '<url>' not found` | Location | Location | Parent item not found |
| `https://workflowy.com/#/<random UUID without dashes>` | Location | Location | Location | Parent item not found |
| `<RANDOM UUID UPPER>` | Parent item not found | Parent item not found | Parent item not found | Parent item not found |
| `<random UUID without dashes>` | Location | Location | Location | Parent item not found |
| `https://workflowy.com/#/<existing sandbox UUID>` (list only) | **Location** (URLs never carry full UUIDs) | — | — | — |

What this settles:

- **create.md was right** that a URL wrapping a nonexistent 12-hex short id
  gives `Parent item not found`.
- The errors-and-http.md resolver predicted the `Location` message for that
  case and was wrong. The resolver was rewritten (errors-and-http.md §2.2): a
  URL that matches `^https?://(beta\.)?workflowy\.com/#/<12 hex>` (case
  insensitive) is looked up as a short id, and a miss gives
  `Parent item not found`.
- A *bare* short id that misses still gives `Location`.
- *Inferred*: the URL regex requires a boundary after the 12 hex characters.
  A 32-hex dashless UUID in the URL is not treated as a URL.

### 1.5 Move-to-self bumps `modifiedAt`

We created Y, waited 2.5 s, then sent `POST /nodes/Y/move {"parent_id": Y, "position": "bottom"}`.

- Response: 200 `{"status": "ok"}`.
- `parent_id` and `priority` were unchanged, and `modifiedAt` went
  1791835582 → 1791835585.
- The repeat run showed the same (1791835585 → 1791835588).

errors-and-http.md called this a pure no-op. That was wrong, and the doc has
been corrected.

### 1.6 List order (offline, from the request log)

We found no requests needed for this. Across every 200 multi-node list
response in `.prod-state/requests.jsonl`:

- 306 responses in total
- 149 not sorted by priority
- 0 not sorted by `createdAt`

errors-and-http.md §8 ("priority order") was wrong and has been corrected to
creation order, which agrees with ordering-and-move.md §6.

---

## 2. Not done

**Uppercase or dashless UUIDs of *existing* nodes** on delete, move, complete,
uncomplete and mirror were not tested.

- This needs `AllowList.contains` in `src/syncflowy/prod_client.py` to
  normalise case and dashes, for example comparing
  `id.lower().replace("-","")` against the same normalisation of the allowed
  ids.
- The task marked this as a coordinator decision, so the guard was not worked
  around.
- If that is approved, the test needs about 10 requests: create 5 nodes in a
  sandbox, then send each endpoint the uppercase and the dashless spelling of
  a node's UUID.
- Today this is *inferred* 404, because update (observed) uses the same exact
  lookup.

---

## 3. Mock server implementation notes (delta)

1. **Validation.** Collect every error and emit it in the declaration order
   of §1.3, whatever the order of the body keys. Error item keys are
   `type, loc, msg, input`. Top-level non-dict → `model_type` with the
   per-endpoint model name of §1.2.
2. **`parent_id` schema.** Optional and nullable on create and move. Required
   and non-null on mirror. Not a field on update, where it is ignored without
   any type check. `""` passes validation everywhere.
3. **Missing, null or `""` `parent_id`** on create or move: production
   behaviour is unknown. Root is the best guess. Do **not** invent a
   validation error.
4. **Parent resolution misses:**
   - uuid-shaped (any case) → `Parent item not found`
   - URL with a 12-hex short id → `Parent item not found`
   - anything else (bare short id, URL with a full UUID, dashless UUID,
     garbage) → `Location '<raw>' not found`
   - mirror: every miss → `Parent item not found`
5. **Move to self:** 200. Bump the node's `modifiedAt` and change nothing else.
6. **List:** return children in creation order.
7. **Cycles (move into a descendant):** the coordinator must decide. The
   recommendation is to reject by default, with a mock-specific error that is
   clearly labelled as not production behaviour, and to offer faithful cycle
   creation behind a flag.

## 4. Open questions

- What does `""`, null or an omitted `parent_id` resolve to on create and
  move: root, or `Location '' not found`? This can't be tested inside the
  scope rules.
- Uppercase or dashless UUIDs of existing nodes on the non-update mutating
  endpoints (see §2).
- Whether the URL regex accepts other characters right after the 12 hex
  digits. `/` and `?` were known to be accepted. We did not test `-`, `#` or
  `.`.

## 5. UI scenarios

- In the web UI, copy the link of a node that is *zoomed into a deeper path*.
  Does the UI ever produce `#/<short>/<short>` URLs, or URLs carrying a full
  UUID, that a client might paste as `parent_id`? We now know that a URL
  carrying a full UUID is rejected (`Location … not found`).
- After the sandbox "followup-2: move-to-self modifiedAt" exists, open it in
  the UI and check that Y0 and Y1 look normal. The API only bumped their
  `modifiedAt`.
