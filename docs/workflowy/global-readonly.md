# Read-only global probes: root, special keys, targets, export

On 2026-10-12 the user authorised **read-only** requests outside the scratchpad. `ProdClient.request(...,
readonly_global=True)` permits GET only (anything else raises `ScopeError`), bypasses the scope guard, and does
not add the ids it sees to the allow-list, so they never become mutable. Every request went through the shared
rate limiter and is in `.prod-state/requests.jsonl` (tag `global-readonly`). No create, move or other mutation
was sent, so **materialisation** of special locations is still unobserved.

Privacy: the account is the user's real outline. The analysis was done in code. This document only contains
shapes, counts and system-generated names (calendar nodes, target labels). Ids below are truncated.

Account state at the time: 8 top-level nodes, 12 972 nodes in total, no user shortcuts, inbox never
materialised, a calendar created by the UI on 2026-07-05 holding one year, one month and seven days.

## 1. Root and `parent_id` forms (list)

| request | result |
|---|---|
| `GET /nodes` (no `parent_id`) | 200, the 8 top-level nodes, `parent_id: null` |
| `GET /nodes?parent_id=None` | 200, identical to the above |
| `GET /nodes?parent_id=` (empty) | 404 `Location '' not found` |
| `parent_id=none`, `null`, `NONE`, `home` | 404 `Location '<key>' not found` (case-sensitive) |
| `GET /nodes/None` | 404 `Item not found` (there is no root node) |

The top level is listed in creation order like any other list: `createdAt` ascends (2019 … 2026) while the priorities are
`[100, 200, 900, 12, 1200, 44, 1300, 28]`. Four of the eight are completed and are listed anyway. The
scratchpad (priority 28, `parent_id: null`, created 2026-10-12) is an ordinary top-level node. Its priority is the
integer midpoint of its UI neighbours 12 and 44. This is evidence for **UI midpoint insertion** (open question H1).

## 2. Special keys

### 2.1 Calendar nodes

The calendar was made by the UI. Its nodes (names are system-generated):

| node | name | parent | priority | id |
|---|---|---|---|---|
| calendar | `📆 Calendar` | `null` (top level) | 1300, the bottom when created (the max was 1200) | `e4fc6453-bd18-5384-…` |
| year | `2026` | calendar | 100 | `0c59206d-c666-5602-…` |
| month | `07` (two digits, not a month name) | year | 100 | `4c656a8d-b5b1-528c-…` |
| day | `<time startYear="2026" startMonth="7" startDay="5">Sun, Jul 5, 2026</time>` | month | see below | `221ac01b-e9dd-59a0-…` |

- All calendar nodes have `note: null`, `layoutMode: "bullets"` and `completed: false`. All ten ids are
  **UUIDv5** (version nibble `5`): deterministic, presumably derived from account and date. The day nodes' own
  children are ordinary v4 nodes.
- Day names are exactly the `<time>` element that the create-path date conversion produces (data-model §6.2
  step 11), without a time part.
- Day priorities in creation order: 5→100, 6→200, 7→300, 8→400, 9→500, **16→600, 15→550**. The days were
  created a few seconds apart by the UI. The 15th was created after the 16th and was placed between the 9th and
  the 16th at the integer midpoint. So calendar children are kept in **chronological order**: the bottom rule
  after the last sibling and the midpoint between neighbours otherwise [I: UI-created, the API path is unobserved].

### 2.2 Resolution matrix (GET only; nothing is ever materialised)

`GET /nodes/:key` and `GET /nodes?parent_id=:key`:

| key | `GET /nodes/:key` | list |
|---|---|---|
| `calendar`, `2026`, `2026-07`, `2026-07-05` (exist) | 200 that node | 200 its children |
| `9917fef266d1` (the calendar's short id) | 200 | 200 |
| `inbox` (never materialised) | 404 `Item not found` | **200 `{"nodes": []}`** |
| `today`, `tomorrow`, `next_week` (no day node for 10-12, 10-13, 10-18/19) | 404 `Item not found` | 404 `Location 'today' not found` |
| valid date without a node: `2025`, `2026-10`, `2026-07-12`, `1999-01-01`, `0002`, `0001-02`, `0001-01-07`, `9998-12-31`, `2028-02-29`, `1900`, `2100` | 404 `Item not found` | 404 `Location '<key>' not found` |
| impossible date: `2026-13`, `2026-00`, `2026-02-30`, `2026-02-29`, `2026-07-00`, `2026-07-32`, `0000` | 404 `Item not found` | **500** `internal_error` |
| any key in year 9999: `9999`, `9999-01-01`, `9999-11`, `9999-12-31` | 404 `Item not found` | **500** |
| `0001`, `0001-01`, `0001-01-01` … `0001-01-06` | **500** | **500** |
| not key-shaped: `2026-1`, `2026-7`, `2026-07-5`, `10000`, `" 2026"`, `"2026-07-05 "`, `Calendar`, `CALENDAR`, `Today`, `next-week`, `nextweek` | 404 `Item not found` | 404 `Location '<key>' not found` |

Interpretation:

- **GET `:id` accepts special keys** (new finding). After the UUID and short-id lookups fail, an existing system
  node is resolved by key. It never creates one (as upstream says) and answers every miss with `Item not found`.
- Keys are `^\d{4}(-\d{2}(-\d{2})?)?$` plus the fixed words, matched case-sensitively and without trimming.
- The 500 window `0001-01-01`…`0001-01-06` is exactly the days whose **Sunday-started** week begins before
  `0001-01-01` (0001-01-01 is a Monday and 0001-01-07 is a Sunday). The resolver evidently computes the start of
  the date's week using the account's week-start setting, which is presumably Sunday here, and overflows. The
  list-only 500s (impossible dates, year 9999) suggest that the list resolver builds the date (and something one
  year later) without catching errors, while GET catches `ValueError`.
- `inbox` lists as empty although no inbox node exists, unlike the calendar keys. Its target name is `null` (§3).

Not observable read-only: what create and move do with these keys (materialisation, the inbox's name and
position), the timezone used for `today`, and whether mutating endpoints accept keys as the path `:id` (S7).

## 3. `GET /api/v1/targets`

```json
{"targets": [
  {"key": "inbox", "type": "system", "name": null},
  {"key": "today", "type": "system", "name": "Today"},
  {"key": "tomorrow", "type": "system", "name": "Tomorrow"},
  {"key": "next_week", "type": "system", "name": "Next Week"},
  {"key": "calendar", "type": "system", "name": "📆 Calendar"}
]}
```

- This is the complete response for this account, which has no shortcuts. Key order is `key, type, name`. The five system targets come in this order.
- `today`, `tomorrow` and `next_week` have **fixed labels**, even though none of their day nodes exist. This
  contradicts upstream's "`null` only when not yet created" for these keys.
- `calendar`'s name is the calendar node's name. `inbox` is `null` because it was never materialised.
- Shortcut entries (`type: "shortcut"`) and their position relative to the system targets are still unobserved.
- `GET /targets/` (trailing slash) and `GET /targets?x=1` give the same 200. Headers are the usual set (data-model §2.7), with no rate-limit
  headers.

## 4. `GET /api/v1/nodes-export`

Four successful calls in total: at +0 s, +10.5 s, +66 s and +174 s. The last was an unintended
`/nodes-export/` (trailing slash, also 200).

- **Shape**: `{"nodes": [...]}` and nothing else. 12 972 records, about 4.4 MB, served in about 1 s.
- **Per-node fields**: exactly the GET shape and key order (`id, name, note, parent_id, priority, completed, data,
  createdAt, modifiedAt, completedAt`). `data` only ever holds `layoutMode`. **No extra fields**: no
  `mirror_origin_id`, no collapse state.
- Every record compared (the 115 scratchpad children, the 8 top-level nodes, and three nodes with null `modifiedAt`) is
  **identical** to the GET / list record.
- **Mirror shells**: the 55 API-made shells still alive look like ordinary nodes. 51 still have `name ""`/`note ""`,
  and the others were edited in experiments. There is no way to tell them apart in export either.
- **Completed nodes are included** (6 786 of 12 972). Deleted nodes are absent (none of the 293 ids that the log shows
  deleted appear).
- **Integrity**: no duplicate ids, no dangling `parent_id`, and every node reachable from the top level. **No cycles were
  present** (the API-made cycles had been repaired), so whether export includes detached cycles is still unknown.
- **Order**: none of creation order, `createdAt`, priority, id, `modifiedAt` or a tree pre-order. Parents precede
  children only about half the time, and the top-level nodes sit at positions 2174 … 10867 in neither creation nor
  priority order. It looks like storage or hash order. It was **identical in all calls** (the bodies were byte-identical), but
  clients must not rely on it.
- **Freshness**: the first export contained nodes created 17 s earlier by the concurrent session. Nothing that we
  know of changed between the calls, so caching could not be tested.
- **Rate limit: not enforced as documented.** Two calls 10.5 s apart both returned 200. No `Retry-After`,
  `X-RateLimit-*` or similar headers appeared on any response, so **no 429 was observed**. The 429 body is still unknown.
- The first response carried `Set-Cookie` (`_bess`, `did`), which is session noise.

## 5. Other whole-account facts (aggregate only)

- **`modifiedAt` can be `null`**: 2 134 of 12 972 nodes, all created 2019–2023 and none in the scratchpad. GET and
  list return `null` too. The node shape's `modifiedAt` is therefore `int | null`. The mock never produces null, but
  clients must accept it.
- **Ids are not all RFC-4122 v4**: 58 % have a non-RFC variant (legacy random 128-bit ids, mostly pre-2026), v4
  dominates in 2026, and calendar nodes are v5. Every id matches the dashed 8-4-4-4-12 lowercase-hex pattern.
- `priority` is always an int, from 0 to 61 200.
- `note`: `null` 11 592, `""` 1 063, non-empty 317.
- `layoutMode` **outside the scratchpad** (that is, values the UI writes): `bullets` 11 329, `bullet` 32, `cmnt` 3,
  `cmnt-c` 3, `p` 2. `h1`, `todo`, `board`, `code-block` and so on occur only inside the scratchpad, where the API wrote them. This bears on
  H9.
