# Workflowy API: reverse-engineering notes

These docs record what we learned by black-box testing of the Workflowy REST API (`https://workflowy.com/api/v1`) on
2026-10-12. The goal is a high-fidelity mock server and, later, a sync engine. All production traffic went
through `syncflowy.prod_client.ProdClient` (rate-limited to under 10 req/s and confined to the scratchpad node `a846cfce4c6d`).
About 4,700 requests were made (plus about 220 on 2026-10-13 probing the beta host, tagged `beta-mirrors`; records carry a `host` field since then), and every one is logged in `.prod-state/requests.jsonl`, tagged by area. The experiment scripts that
established each fact are under `experiments/<area>/`. They are re-runnable, and their captured outputs sit next to them.

## Start here

| doc | what it is |
|---|---|
| [data-model.md](data-model.md) | **The authoritative spec** for the mock: storage model, every endpoint, validation, error bodies, id resolution, the priority algorithm, `modifiedAt` rules, text processing and mirrors. Each rule is tagged OBSERVED or INFERRED, with pointers. |
| [ui-scenarios.md](ui-scenarios.md) | A numbered, prioritised list of actions for a human to perform in the web UI while we observe through the API. |
| [open-questions.md](open-questions.md) | What is still unknown, including everything blocked by the scope restriction. |
| [upstream-api-reference.md](upstream-api-reference.md) | The vendored official API reference (production, `workflowy.com`). |
| [upstream-api-reference-beta.md](upstream-api-reference-beta.md) | The vendored **beta** API reference (`beta.workflowy.com`, fetched 2026-10-13). It differs only in mirror-aware semantics (data-model §10.5). |

## Per-area investigation docs (detail and evidence)

| doc | area | experiments |
|---|---|---|
| [create.md](create.md) | `POST /nodes`: defaults, priorities, markdown and multiline name parsing, `parent_id` forms | `experiments/create/` |
| [update-and-complete.md](update-and-complete.md) | update, complete and uncomplete; `modifiedAt` semantics | `experiments/update-complete/` |
| [ordering-and-move.md](ordering-and-move.md) | the priority algorithm, move, list order, cycles | `experiments/ordering-move/` |
| [delete-and-mirrors.md](delete-and-mirrors.md) | hard delete; the mirror shell model, orphans, mirror errors | `experiments/delete-mirror/` |
| [errors-and-http.md](errors-and-http.md) | routing, 401/404/405 shapes, check order, id resolution, body parsing, serialisation | `experiments/errors-http/` |
| [text-and-content.md](text-and-content.md) | name and note fidelity: control characters, entities, HTML, markdown regexes, dates, lengths | `experiments/text-content/` |
| [followup-0.md](followup-0.md) | bottom formula for move and mirror, the top-renumber edge cases, concurrency, list order | `experiments/followup-0/` |
| [followup-1.md](followup-1.md) | `modifiedAt` on move and mirror operations, mirror check order, orphans, the bottom formula above 100 | `experiments/followup-1/` |
| [followup-2.md](followup-2.md) | validation model names, field orders, `parent_id` miss messages, move-to-self | `experiments/followup-2/` |
| [data-model.md §10.5](data-model.md#105-beta-api-mirror-aware-semantics) | the beta host's mirror-aware semantics, probed through beta and production side by side | `experiments/beta-mirrors/` |
| [global-readonly.md](global-readonly.md) | read-only GETs outside the scratchpad: root, special keys, calendar nodes, `targets`, `nodes-export`, whole-account aggregates | ad hoc (`ProdClient(readonly_global=True)`), log tag `global-readonly` |

When a per-area doc and data-model.md disagree, **data-model.md wins**. Its §13 lists every reconciled conflict.

## Overview in one page

- **Node**: `{id, name, note, parent_id, priority, completed, data: {layoutMode}, createdAt, modifiedAt, completedAt}`
  in exactly that key order. The serialisation matches Python `json.dumps` defaults (`ensure_ascii`). Timestamps are integer seconds.
- **Consistency**: strictly read-your-writes. Concurrent writes behave as if serialized.
- **Ordering**: `priority` is a sparse integer per parent. API placement is only `top` (the default, `floor(min/2)`, with a full renumber
  to 100, 200, … when min is 0) or `bottom` (`ceil((max+100)/100)*100`). There is no API way to insert in the middle. **List returns
  creation order, not priority order.**
- **Ids**: `GET /nodes/:id` accepts a UUID or a 12-hex short id. Every mutating endpoint accepts only the exact lowercase UUID (a short id
  gives 404 and does nothing). `parent_id` for list, create and move also accepts short ids and `workflowy.com/#/<short>` URLs. Mirror's `parent_id`
  accepts only an exact UUID.
- **Text**: update stores name and note verbatim (minus C0 control characters). Create's note is verbatim. Create's **name** is
  transformed: entities are decoded, lines are soft-joined by a regex, the result is split into flat child nodes (priorities 0, 1, 2, layout `p`,
  note `""`), prefixes are detected (`#`, `- `, `- [ ]`, `- [x]`, fences; `>` is **not** recognised), and naive inline-markdown regexes
  and `<time>` dates are applied. There is no HTML sanitisation and no length limit.
- **modifiedAt**: bumped on every successful update, complete or uncomplete (even a no-op). A move bumps the node and the
  destination parent. Create, delete and child edits never bump the parent. Mirror create and mirror delete bump the origin.
- **Completion**: per node only, with no cascade. Re-completing re-stamps `completedAt`. Completed nodes are always listed.
- **Delete**: hard and recursive. There is no trash in the API, and siblings are not renumbered.
- **Mirrors**: through the production API a mirror is an independent, empty shell (`name ""`, `note ""`). Nothing propagates. A hidden
  origin link drives only `POST/DELETE /mirror`. Deleting an origin leaves orphan shells. **The beta API (`beta.workflowy.com`) fixes
  this** (announced 2026-07-22, probed 2026-10-13 in `experiments/beta-mirrors/`, data-model §10.5). Beta exposes
  `data.mirror.origin_id` and `data.mirror.mirror_ids` (sorted by id; a registry that can go stale). A shell shows its origin's
  content and children (its own persisted children are hidden). Create, update and complete through a shell reach the origin, and
  orphans refuse writes. An origin that still has mirrors cannot be deleted ("Cannot delete a node that has mirrors"), but
  `DELETE /mirror` can still make orphans. The mock has it behind `beta_mirrors`.
- **Hazards**: moving a node into its own descendant returns 200 and **creates a cycle** in production. A move without `parent_id`
  passes validation (its effect is unknown, probably the root). A NUL in the create note gives a 500.
- **Root and special keys** (read-only, [global-readonly.md](global-readonly.md)): an omitted `parent_id` or `None` lists the top level, and `""` is a Location miss.
  `GET /nodes/:key` resolves existing calendar and inbox nodes. Calendar nodes are `📆 Calendar` → `2026` → `07` → `<time …>` day, with UUIDv5 ids. `nodes-export` mirrors
  GET's shape with no extra metadata, and its 1/min limit was not enforced. `modifiedAt` may be `null` on old nodes.
- **Untested (scope)**: materialising special locations via create or move, shortcut keys, and an empty or null `parent_id` on create or move.
