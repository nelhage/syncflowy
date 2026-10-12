# Differential conformance suite

The conformance suite runs the **same scenarios** against production Workflowy and against the
mock server (`src/syncflowy/mock/`), normalises both transcripts, and diffs them. Production
runs are recorded once as **golden files**. After that, the mock is checked against the goldens
offline, in a few seconds.

```
            scenario fn(ctx)                     (src/syncflowy/conformance/scenarios/*.py)
                  │
     ┌────────────┴────────────┐
 ProdBackend               MockBackend          (backend.py)
 (ProdClient, scratchpad   (HTTP to running_mock,
  sandbox, real clock)      FakeClock)
     │                         │
 raw transcript            raw transcript       (context.py: Ctx records every request)
     │                         │
 normalize()               normalize()          (normalize.py)
     │                         │
 tests/conformance/golden/<name>.json ──diff──► readable unified diff   (runner.py)
```

Code lives in `src/syncflowy/conformance/`:

| module | role |
|---|---|
| `registry.py` | `@scenario` decorator, selection by glob or `@area` |
| `context.py` | `Ctx`, the recording client that scenarios receive; epochs and `ctx.sleep()` |
| `backend.py` | `ProdBackend`, `MockBackend`, `VettingMockBackend`, `FakeClock`, production safety vetting |
| `normalize.py` | raw transcript → comparable transcript (ids, timestamps, headers) |
| `runner.py` | running scenarios, golden I/O, `MockPool`, `check_scenario`, diffing |
| `cli.py` | the `syncflowy-conformance` command |
| `scenarios/*.py` | the scenarios, one module per area (`create`, `text`, `update`, `complete`, `move`, `delete`, `mirrors`, `ids`, `http`, `listing`) |

Tests are in `tests/conformance/`:

| file | what it checks |
|---|---|
| `test_conformance.py` | one test per scenario: replays it on the mock and diffs against its golden. It is skipped when there is no golden. |
| `test_normalize.py` | unit tests for the normaliser (id forms, epochs, clock skew, flags) and the production safety vetting |
| `test_harness.py` | runs every scenario on two independent mocks with different ids and clocks, and requires identical normalised transcripts. The first mock also applies production vetting, so a scenario that production would refuse fails here, offline. |

## Quick start

```sh
cd /home/nelhage/code/syncflowy
direnv exec . uv run syncflowy-conformance list                # all scenarios, golden status
direnv exec . uv run syncflowy-conformance record --dry-run    # offline: would prod accept everything?
direnv exec . uv run syncflowy-conformance record 'move_*' --yes   # PRODUCTION: write goldens
direnv exec . uv run syncflowy-conformance check               # mock vs goldens
direnv exec . uv run syncflowy-conformance check -v '@mirrors' # one area, also print PASS/SKIP
direnv exec . uv run syncflowy-conformance show move_to_self   # print a golden, normalised
direnv exec . uv run syncflowy-conformance run move_to_self    # run on a fresh mock, print the transcript
direnv exec . uv run pytest tests/conformance                  # the same checks under pytest
```

Selectors are `fnmatch` globs on the scenario name, or `@area` (the module name, e.g. `@text`).
No selector means all scenarios.

### Commands

- **`record [SEL…]`** runs scenarios against **production** through `ProdClient` and writes
  `tests/conformance/golden/<scenario>.json`. It skips scenarios that already have a golden
  unless you pass `--force`. More than 3 scenarios require `--yes`. Each scenario runs in a fresh sandbox
  (`ProdClient.make_sandbox("conformance <name>")`), which is deleted afterwards unless you pass
  `--keep-sandboxes`. The request log tag (`SYNCFLOWY_TAG`) is `conformance:<name>`. A golden is *not*
  written when:
  - the scenario raised (for example a helper's create failed). `--allow-errors` overrides this.
  - timestamps could not be attributed to epochs (see below). Re-run the scenario.
  - the safety vetting refused a request (`REFUSE`). That is a scenario bug, so fix the scenario.

  `--dry-run` sends nothing to production. It runs the selection on an in-process mock with the
  production safety vetting applied (`VettingMockBackend`), so it shows whether anything would be refused.
- **`check [SEL…]`** starts an in-process mock (`running_mock(clock=FakeClock())`), runs every selected
  scenario that has a golden, and prints a unified diff for each mismatch (`production (golden)` against `mock`).
  It exits with status 1 when there are failures. Scenarios with `mock_options` (such as `faithful_cycles`) get their own mock instance
  with those options. If `running_mock` does not accept an option, the scenario is skipped with
  an explanation. `--base-url URL [--api-key K]` checks an already-running mock instead (real sleeps,
  and no `mock_options`).
- **`run [SEL…] [--backend mock|prod] [--raw]`** runs scenarios and prints the transcripts without
  comparing or saving. `--backend prod` is limited to 3 scenarios without `--yes`.
- **`show NAME`** prints a golden, re-normalised with the current normaliser.
- **`renormalize [SEL…]`** rewrites the `normalized` field of goldens from their `raw` field. This is only
  cosmetic. `check` always re-normalises the raw part, see below.

## Backends and sandboxes

Each scenario runs in a **fresh sandbox node**. `ctx.root` is that node's UUID. The sandbox
layout is the same on both backends, so the sandbox root is an ordinary non-top-level node everywhere:

```
production:  <scratchpad a846cfce4c6d>             -> "sandbox: conformance <name>"  (position bottom)
mock:        "conformance scratchpad" (top level,  -> "sandbox: conformance <name>"  (position bottom)
              created once per mock with parent_id "None")
```

The sandbox root has the same name on both backends. Its priority (wherever it landed under the scratchpad) is normalised away.

### Production safety

All production traffic goes through `syncflowy.prod_client.ProdClient` (global rate limit below 10 req/s, scope
guard, request log). The guard only knows ids it has seen, so deliberate probes trip it. Examples are a dashless or uppercase
form of our own node, a fake nonexistent UUID, a garbage `parent_id`, and a repeated query parameter. For these
cases `ProdBackend` first **vets** the request itself (`ProdBackend._vet`):

- Every id-like token (dashed UUID, 32-hex, 12-hex short, including inside URLs and `%2F`-escaped paths) in
  the path id and in every `parent_id` (query, JSON, form) must be one of:
  - a node created or listed **in this scenario's sandbox**, or
  - a deliberately **fake id**, whose last 12 hex digits start with ten zeros (`scenarios/_util.py`:
    `FAKE_UUID`, `FAKE_UUID2`, `FAKE_SHORT`).
- A `parent_id` with no id token must look like garbage that cannot be a shortcut key. It must start with `zz-conformance`
  or contain one of ``' /:'"#?%&'``.
- The following are **refused outright**:
  - special keys (`None`, `inbox`, calendar keys)
  - create or move whose `parent_id` is missing, `null` or `""` (these may resolve to the account root and are untested in production)
  - a list without `parent_id`
  - `nodes-export` and `targets`

Only after vetting does the backend re-send a guard-refused request with `unscoped=True`, the documented escape
hatch. A refusal raises `UnsafeRequest`, which aborts that scenario's recording (`REFUSE`). The
guard and the limiter themselves are untouched. `test_harness.py` and `record --dry-run` run every scenario
through the same vetting offline.

Cycle scenarios (`move_into_*_creates_cycle`) repair the cycle (by moving the node back under the
sandbox) inside the scenario, so sandbox deletion removes everything.

## Scenario format

A scenario is a function that takes `ctx`, registered with `@scenario`. The function name is the scenario
name and the docstring is its description. The first docstring line appears in `list` and in pytest
failures.

```python
from ..registry import scenario
from ._util import FAKE_UUID, node_sandbox, short

@scenario
def move_to_self(ctx):
    """Move under itself: 200, parent and priority unchanged, only the node's modifiedAt bumped (§4.7.4)."""
    p = node_sandbox(ctx)                         # creates "P" under ctx.root and aliases it <P>
    a = ctx.alias(ctx.create(p, "a"), "A")        # create returns the id; alias names it <A>
    ctx.get(a)
    ctx.sleep()                                   # new timestamp epoch
    ctx.move(a, a, position="bottom")
    ctx.get(a)
    ctx.get(p)

@scenario(mock_options={"faithful_cycles": True})
def move_into_child_creates_cycle(ctx): ...
```

### `ctx` API

| call | meaning |
|---|---|
| `ctx.root` | the sandbox root's UUID |
| `ctx.req(method, path, json=, params=, content=, headers=)` | raw request. It returns `Resp(status, headers, text, json)`. `json=None` means no body; send a literal `null` with `content="null"`. `params` may be a dict or a list of pairs. Header values may contain `{API_KEY}`, which the backend replaces with its key. |
| `ctx.create(parent, name, **fields) -> id` | create, and raise `ScenarioError` unless it returns 200 |
| `ctx.try_create(parent, name, **fields) -> Resp` | create, with failure allowed |
| `ctx.get / list / update / complete / uncomplete / delete / move / mirror / delete_mirror` | thin wrappers around `req` |
| `ctx.node(id) -> dict`, `ctx.children(parent) -> list` | GET or list, returning parsed data (raise on failure) |
| `ctx.make_mirror(id, parent) -> shell id` | mirror, and raise unless it returns 200 |
| `ctx.sleep(seconds=2)` | start a new timestamp **epoch**. See below. It is instant on the mock. |
| `ctx.alias(id, "NAME")` | the node renders as `<NAME>` instead of `<N3>` in normalised transcripts |
| `ctx.note(text)` | free-text annotation that is recorded and compared |

`scenarios/_util.py` provides id-form helpers (`short`, `upper`, `dashless`, `url`), fake ids,
`GARBAGE`, `node_sandbox(ctx, name)` and `create_and_show(ctx, parent, name)` (create, GET, list children).

**Exceptions** raised inside a scenario are recorded as an `error` step. On the mock this shows up
in the diff. For example, if a create that worked on production fails on the mock, the transcript ends in
`error: ScenarioError at create.py:42: …`. When recording, a scenario that raises is not saved.

### Guidelines for writing scenarios

- Keep each scenario small (up to about 40 requests) and about one behaviour, and cite the `data-model.md` section in the
  docstring.
- **Only reference nodes created in the scenario, or `FAKE_*` ids.** Never use special keys (`None`, `inbox`,
  calendar keys), and never send a create or move without a non-empty `parent_id`. The vetting enforces this, and
  `pytest tests/conformance/test_harness.py` checks it offline.
- Scenarios must be **deterministic**. Don't branch on response contents in a way that could differ between
  backends, except to avoid crashing. For example, `if r.ok: ctx.get(r.json["item_id"])` is fine, because a difference
  shows up in the transcript anyway.
- Put `ctx.sleep()` between two steps whenever the question is "did step B bump a timestamp?".
  Without a sleep, the two steps are in the same epoch and are indistinguishable by design.
- Show state through responses. A scenario has no assertions. The transcript (including every GET and
  list) *is* the observation, so GET or list whatever you want compared.
- Adding a scenario: write it, run `syncflowy-conformance run NAME` (on the mock) to eyeball the
  transcript, run `pytest tests/conformance/test_harness.py -k NAME`, then `record NAME` against
  production, inspect the golden with `show NAME`, and commit the golden.

## Normalisation

`normalize.normalize(raw)` is a **pure function of the raw transcript**. Goldens store the raw
transcript (real ids, real timestamps, local clock windows, all response headers), and `check`
re-normalises the golden's raw part every time. So improvements to the normaliser apply to existing goldens
without re-recording production. The stored `normalized` field is only for human reading and review.

### Ids

Every UUID-looking token in the request path, params, body, raw content, headers and response body is
replaced with a symbol assigned **by order of first appearance**: `<N1>`, `<N2>`, and so on. Tokens in error messages,
`Location '…' not found` echoes, validation `input` echoes, `item_id`/`origin_id` and URLs are covered too.
The sandbox root is `<ROOT>` and its parent is `<SANDBOX_PARENT>`. `ctx.alias()` names nodes explicitly.
The *form* of each reference is kept, because forms matter for id resolution:

| appearance | rendering |
|---|---|
| lowercase UUID | `<N1>` |
| uppercase / mixed-case UUID | `<N1:UPPER>` / `<N1:MIXED>` |
| 32-hex dashless | `<N1:dashless>` (plus a case suffix) |
| 12-hex short id | `<N1:short>`, `<N1:short:UPPER>`, `<N1:short:MIXED>` |

A 12-hex token is rewritten only if it is the suffix of an id already seen. Otherwise it is literal content,
such as a fake short id. Tokens directly after a percent escape (`%2F<short>`) are recognised.

### Timestamps: epochs

`createdAt`, `modifiedAt` and `completedAt` have 1-second resolution and come from the server's clock. Requests 0.2 s
apart on production may or may not share a second, depending on where the boundary falls. So the suite compares
timestamps at the granularity of **epochs**:

- Epoch 0 is sandbox creation. `Ctx.start()` sleeps once, so the scenario body starts in epoch 1.
- Each `ctx.sleep()` starts a new epoch. It always waits at least the backend's `min_epoch_gap`: **3 s on
  production**, and 2 s of *fake* time on the mock, which is instant.
- Each epoch's possible server timestamps are `floor(t + skew ± slack)` over the backend-clock window
  `[first request start, last request end]` of that epoch. On production, the client/server clock **skew is
  estimated from the `Date` response headers**: each response bounds the skew to `[D − t_end, D + 1 − t_start)`, and
  the bounds are intersected. `slack` is `0.3 + half the skew uncertainty`, capped at 0.9 s. With a 3 s gap,
  adjacent epochs therefore can never share a second. On the mock with a `FakeClock`, skew is 0 and the window is a
  single instant.
- Every timestamp in a response node object becomes `<E0>`, `<E1>`, … A value outside every window renders as
  `<E?+N>` (N seconds after sandbox creation), and a value inside two windows renders as `<E1|E2>`. Both mean the
  harness could not attribute it. `record` refuses to save such a golden, and in a diff they point to a clock problem
  or a server bug.

This makes the important questions exactly comparable. Did this update bump `modifiedAt`? Did the renumber
leave siblings alone? Is `completedAt` restamped on re-complete? Is the parent's `modifiedAt` untouched by a create?
It also tolerates arbitrary second-boundary crossings *within* an epoch. What is deliberately **not** comparable is the
ordering of two timestamps inside one epoch (for example `createdAt == modifiedAt` for a node created and touched in the same
epoch). Put a `ctx.sleep()` between the steps when that matters.

The `FakeClock` starts at the middle of a wall-clock second (`x.5`) and only moves on `advance()`, so the
mock's `int(clock())` is stable within an epoch. Timestamps that appear in *request* bodies (for example a client sending
`createdAt: 5`) are never rewritten.

### Other fields

- The sandbox root's `priority` becomes `<ROOT_PRIORITY>`. Every other priority is compared exactly.
- **List order is kept.** Production lists children in creation order deterministically (data-model §4.2,
  more than 330 observations), so order is behaviour, not noise.
- Response headers: only `content-type`, `allow` and `www-authenticate` are compared. `date`, `server`,
  `content-length` and the security headers are dropped (data-model §2.7 lists them as optional fidelity).
- `json_bytes` is `"python-default"` when the raw JSON body is byte-identical to Python
  `json.dumps(parsed)` (`", "`/`": "` separators, `ensure_ascii`), and `"non-default"` otherwise. Production is always
  `python-default` (§1.3). This catches serialisation differences that parsed-JSON comparison cannot see.
- Non-JSON bodies (empty 405 and non-GET 404 bodies) are compared as text, with ids normalised. A full
  HTML document (production's GET 404 page) is collapsed to `<HTML page title='404'>`: production embeds a
  fresh CSP `nonce` in every copy of that page and deploy-specific hashed asset URLs, so its bytes are not
  reproducible even between two requests in the same scenario. Only "an HTML document titled 404" is compared.

### Diffs

`check` renders each normalised step as a header line (`## [7] POST /api/v1/nodes/<A>/move`) followed by
indented JSON, one field per line, in server key order, so key-order differences show up. It then prints a
`difflib` unified diff against the golden. For example:

```
FAIL  move_to_self
--- production (golden)
+++ mock
@@ -61,7 +61,7 @@
         "createdAt": "<E1>",
-        "modifiedAt": "<E2>",
+        "modifiedAt": "<E1>",
         "completedAt": null
```

## Golden file format

`tests/conformance/golden/<scenario>.json`:

```json
{
 "scenario": "move_to_self",
 "doc": "…docstring…",
 "recorded_at": "2026-10-12T21:03:00+00:00",
 "normalized": [ …normalised steps, for reading/review… ],
 "raw": {
  "scenario": "move_to_self", "backend": "prod", "clock_kind": "real",
  "root": "<uuid>", "sandbox_parent": "<scratchpad uuid>", "sandbox_name": "sandbox: conformance move_to_self",
  "setup_window": [t0, t1],
  "steps": [
   {"type": "request", "epoch": 1, "t0": …, "t1": …, "method": "POST", "path": "…", "params": …, "json": …,
    "content": …, "headers": …, "status": 200, "response_headers": {…}, "body_text": "…", "body_is_json": true},
   {"type": "sleep", "seconds": 3.0, "epoch": 2},
   {"type": "alias", "id": "<uuid>", "name": "A"},
   {"type": "note", "text": "…"},
   {"type": "error", "message": "…"}
  ]
 }
}
```

The API key never appears in a golden. Header templates keep the literal `{API_KEY}` placeholder, and
`ProdClient`'s own log strips `Authorization`.

## Recording status

The original 154 scenarios were recorded against production on 2026-10-12 (about 1,550 requests, roughly 15 minutes;
no scenario raised, was refused, or had unattributable timestamps; every sandbox was deleted). Determinism was
checked by re-recording 8 scenarios (`create_top_halving_then_renumber`, `mirror_timestamps`, `move_timestamps`,
`text_multiline_split_and_join`, `http_unknown_routes`, `ids_list_parent_forms_errors`,
`update_noop_same_value_bumps`, `auth_bad_key_shapes`) into a scratch directory
(`SYNCFLOWY_GOLDEN_DIR=… record --yes …`): all normalised transcripts were identical to the goldens. The only
nondeterminism found was the per-response CSP nonce in production's HTML 404 page, which the normaliser now
collapses (see *Other fields*). Estimated clock slack across all goldens was 0.38–0.73 s, well inside the 3 s
epoch gap.

An independent review on 2026-10-12 added 7 scenarios for spec rules that had no golden (open questions P1, P2, P5, P6,
NUL in `layoutMode`, and JSON lone surrogates) and recorded them (about 110 production requests):
`text_dates_edges`, `text_entities_structural`, `ids_url_parent_boundaries`, `ids_parent_id_lone_surrogate`,
`create_layoutmode_controls_nul_and_surrogates`, `update_nul_layoutmode_and_lone_surrogates` and
`complete_huge_garbage_body`. Five of them exposed mock bugs, all fixed (see the "Settled by the 2026-10-12 review"
table in [mock-server.md](mock-server.md)): the URL `parent_id` boundary, date minutes and Unicode digits, a NUL in
update `layoutMode` (production 500s), lone surrogates in names and notes (production stores `?`), and a lone surrogate in
`parent_id` (production answers `Location '…' not found`, except mirror, which 500s). Total: 161 scenarios.

The first recording found three places where the documented spec (and so the mock) was wrong; the docs under
`docs/workflowy/` are corrected and cite the goldens:
`auth_header_variants` (a third token in `Authorization` gives 401), `text_multiline_code_fences` (an unclosed
fence is not a join-free region), and `update_layoutmode_control_chars` (update does not strip control characters
from `layoutMode`).

## Known divergences and mock options

- **Cycles** (§4.7.5): production accepts a move into a node's own descendant and forms a parent cycle. By default the mock
  rejects it with a mock-specific 400. The two `move_into_*_creates_cycle` scenarios declare
  `mock_options={"faithful_cycles": True}`, so `check` runs them on a separate mock started with
  `running_mock(..., faithful_cycles=True)`. If the mock does not support an option, those scenarios are skipped,
  not failed.
- Behaviour that the conformance suite cannot cover because of the scope rules: special keys, the account root, `nodes-export`,
  `targets`, an omitted or empty `parent_id` on create/move, and the absence of an `Authorization` header (`ProdClient` always sends one).
  See [workflowy/open-questions.md](workflowy/open-questions.md) §1.

## Scenario inventory

Run `syncflowy-conformance list` for the current list with descriptions. The areas are:

| area | count | covers |
|---|---|---|
| `create` | 23 | defaults, first-child 100, top halving and renumber (min 0), bottom formula (non-multiple max, after multiline children), null/omitted position, invalid positions, the layoutMode param (and overriding prefixes), note null/""/verbatim/controls/NUL→500, layoutMode controls/NUL and lone surrogates in every field, ignored fields, blank names, no parent bump, form-encoded and any-content-type bodies, duplicate keys, list order, completed parent |
| `text` | 25 | create-path pipeline: bold/italic, code/strike, links, escapes, entities, raw HTML, dates (valid and invalid), header/list/todo prefixes, Unicode, control characters, whitespace stripping, the multiline join regex and chains, line separators, prefixed children, the completion leak, code fences, note/layout on block 0, flat children, a 20k name, structural entities (`&#10;`, `&#35; `, inside fences, charref edge cases), date regex edges (1-digit minutes, Unicode digits, year 0/1) |
| `update` | 21 | verbatim name/note, control stripping (and layoutMode, P4), null no-ops, `{}` and same-value bumps, ignored structural fields, note `""`-not-null, any layoutMode string, partial updates, create→update fixed point, no parent bump, validation, bad bodies, encodings (form, multipart, text/plain), query string ignored, validation before lookup, completed nodes, large notes, NUL in layoutMode (500) and lone surrogates (`?` in name/note, 500 in layoutMode) |
| `complete` | 11 | basic, restamp on re-complete, uncomplete, uncomplete-never-completed, no cascade, body ignored, layout/priority unchanged, `- [x]`-created todos, lists include completed, wrong methods, a 50 KB garbage body |
| `move` | 21 | cross-parent top/bottom, empty parent, same-parent top/bottom (excluding self), only child, min-0 renumber, self-to-top in a min-0 parent, timestamps (cross-parent and same-parent), move to self, ignored index fields, subtree travel, list order after a move, back and forth, completed nodes, cycles (child and grandchild, with `faithful_cycles`), validation, 404 ordering, wrong methods |
| `delete` | 9 | subtree, delete twice, short/upper/dashless ids that don't delete, every endpoint after a delete, body ignored, no renumber, no bumps, multiline children, a deleted parent's descendants |
| `mirrors` | 22 | shape, timestamps, no propagation either way, positions, two shells, mirror-of-mirror redirect, shell-as-parent redirect, under self/descendants/own shell, parent_id forms (exact only), children of shells, DELETE /mirror, not-a-mirror, plain delete of a shell, subtree delete, orphan lifecycle, deleted ancestor of the origin, validation, 404 ordering, moving shells and origins, wrong methods, completed origin |
| `ids` | 11 | GET path forms, exact UUID required on mutations (S8), list parent_id forms that resolve and that fail (Parent vs Location), create/move parent forms, nonexistent ids on every endpoint, trailing slashes, `%2F` in a path, URL `parent_id` boundary characters (P6), a lone surrogate in `parent_id` |
| `http` | 11 | 401 shapes by method, auth header variants, check precedence, JSON vs HTML 405 routes, unknown routes, bad create bodies, create validation (all errors, missing echo), validation before lookup, JSON serialisation bytes, Accept ignored |
| `listing` | 7 | depth 1, empty, ignored params, repeated `parent_id` (last wins), order through mixed operations, deleted parent, 25 children |
