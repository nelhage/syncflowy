# syncflowy

Groundwork for a two-way sync between [Workflowy](https://workflowy.com) and Markdown files, so that an
Obsidian vault can act as a plain-text frontend to a Workflowy tree, without the collaborators using Workflowy
noticing.

The sync engine is written in Rust under [`engine/`](engine/README.md) (build, usage, safety behaviours and known limitations are there). The repository also holds:

- reverse-engineered documentation of the Workflowy REST API and web UI,
- a high-fidelity mock of the API, plus a differential suite that checks it against production,
- the design the engine implements (see [docs/sync-design.md](docs/sync-design.md)).

## Setup

The development environment is a Nix flake (Python 3.14 and `uv`), loaded by direnv:

```sh
direnv allow
direnv exec . uv run pytest        # the offline test suite (mock, conformance goldens, wfhtml)
```

The engine is a Cargo crate; `cargo` and `rustc` come from the dev shell:

```sh
cd engine
cargo build --release              # target/release/syncflowy
cargo test                         # needs uv: the end-to-end tests spawn the Python mock
cargo clippy --all-targets -- -D warnings
```

Anything that talks to production reads `WORKFLOWY_API_KEY` from `.envrc.local`, which is git-ignored.

CI ([`.github/workflows/ci.yml`](.github/workflows/ci.yml)) runs the Python suite and the engine's rustfmt, clippy and tests on every push to `main` and every pull request.

## Documents

| path | what it is |
|---|---|
| [docs/sync-design.md](docs/sync-design.md) | **Design of the sync engine**: the Markdown dialect, three-way merge, applying changes through the API, ordering and safety, deployment as one daemon behind Obsidian Sync, mirrors (synced as their origins; the engine uses the beta API only), and the choice of Rust. |
| [docs/workflowy/README.md](docs/workflowy/README.md) | Index and one-page summary of the API investigation. Start here for anything about Workflowy's behaviour. |
| [docs/workflowy/data-model.md](docs/workflowy/data-model.md) | **The authoritative API spec**: storage model, every endpoint, validation, ids, priorities, `modifiedAt`, text processing, mirrors (production, and the beta API's mirror-aware semantics in §10.5). |
| [docs/workflowy/ui-findings.md](docs/workflowy/ui-findings.md) | What the web UI actually does (reordering, deletes, text format, layouts, mirrors), observed through the API while a human used the UI. |
| [docs/workflowy/ui-scenarios.md](docs/workflowy/ui-scenarios.md) | The list of human-in-the-loop UI scenarios, run and still to run. |
| [docs/workflowy/open-questions.md](docs/workflowy/open-questions.md) | What is still unknown about the API. |
| [docs/workflowy/upstream-api-reference.md](docs/workflowy/upstream-api-reference.md) | The official API reference, vendored. |
| [docs/workflowy/upstream-api-reference-beta.md](docs/workflowy/upstream-api-reference-beta.md) | The official **beta** API reference (`beta.workflowy.com`), vendored. It adds mirror-aware semantics. |
| `docs/workflowy/*.md` (others) | Per-area investigation notes with evidence: create, update and complete, ordering and move, delete and mirrors, errors and HTTP, text and content, three follow-up rounds, and global read-only probes. Indexed in [docs/workflowy/README.md](docs/workflowy/README.md). |
| [docs/mock-server.md](docs/mock-server.md) | How to run and use the mock server, its mock-only admin and UI-simulation routes, and its known deviations from production. |
| [docs/conformance.md](docs/conformance.md) | How the differential conformance suite works, and how to record goldens and check the mock. |

## Engine (`engine/`)

| path | what it is |
|---|---|
| [`engine/README.md`](engine/README.md) | How to build, configure and use the `syncflowy` binary (`render`, `sync`, `daemon`, `status`), the Markdown dialect, safety behaviours, state directory, limitations and the module map. |
| [`engine/src/`](engine/src/) | The Rust sync engine: a pure core (`tree`, `wfhtml`, `mdinline`, `mdblock`, `merge`, `plan`) and the I/O layers (`wfclient`, `apply`, `state`, `sync`, `daemon`). `lib.rs` is the module map and contract. |
| [`engine/tests/`](engine/tests/) | Unit, property, snapshot, differential and end-to-end tests; the end-to-end ones run against the mock server. |

## Code (`src/syncflowy/`)

| path | what it is |
|---|---|
| [`prod_client.py`](src/syncflowy/prod_client.py) | Client for the **production** API (or, with `base_url=BETA_URL`, the beta host): rate-limited, confined to a scratchpad node unless told otherwise, and logging every request to `.prod-state/requests.jsonl`. |
| [`wfhtml.py`](src/syncflowy/wfhtml.py) | Workflowy's rich-text format for names and notes ("WF-HTML"): escaping, canonicalisation and well-formedness checks. The Rust engine's `wfhtml` is a port of this. |
| [`mock/`](src/syncflowy/mock/) | The mock API server (`syncflowy-mock`), backed by SQLite. `app.py` is the HTTP layer, `ops.py` the endpoint semantics, `store.py` storage, `text.py` the create-name text pipeline, `models.py` request validation, `ui.py` the test-only `/_mock/ui/*` routes that simulate web-UI edits, and `testing.py` runs the server in a thread for tests. |
| [`conformance/`](src/syncflowy/conformance/) | The differential conformance suite (`syncflowy-conformance`). `scenarios/` holds the scenarios by area; the rest is the recording client, backends, normaliser and runner. |
| `__init__.py` | Placeholder `syncflowy` entry point. |

## Tests (`tests/`)

| path | what it is |
|---|---|
| [`tests/mock/`](tests/mock/) | Unit tests for the mock server, by area, including a replay of the production request log. |
| [`tests/conformance/`](tests/conformance/) | Replays every recorded production scenario against the mock and diffs the transcripts. `golden/` holds the recorded production transcripts, one JSON file per scenario. |
| [`tests/test_wfhtml.py`](tests/test_wfhtml.py) | Tests for `wfhtml`, from the tables in `ui-findings.md`. |

## Experiments (`experiments/`)

Re-runnable scripts that established the facts in `docs/workflowy/`, each next to its captured output. They
run against production, inside the scratchpad node.

| directory | area (documented in) |
|---|---|
| `create/` | `POST /nodes` ([create.md](docs/workflowy/create.md)) |
| `update-complete/` | update, complete, uncomplete ([update-and-complete.md](docs/workflowy/update-and-complete.md)) |
| `ordering-move/` | priorities, move, cycles ([ordering-and-move.md](docs/workflowy/ordering-and-move.md)) |
| `delete-mirror/` | delete and mirrors ([delete-and-mirrors.md](docs/workflowy/delete-and-mirrors.md)) |
| `errors-http/` | routing, errors, ids, bodies ([errors-and-http.md](docs/workflowy/errors-and-http.md)) |
| `text-content/` | name and note fidelity ([text-and-content.md](docs/workflowy/text-and-content.md)) |
| `followup-0/`, `followup-1/`, `followup-2/` | follow-up rounds ([followup-0.md](docs/workflowy/followup-0.md), [-1](docs/workflowy/followup-1.md), [-2](docs/workflowy/followup-2.md)) |
| `beta-mirrors/` | the beta host's mirror-aware semantics, read side by side with production ([data-model.md §10.5](docs/workflowy/data-model.md#105-beta-api-mirror-aware-semantics)) |
| `ui/` | tooling for the UI scenarios: `ui.py` builds fixtures and snapshots and diffs them, `step.sh` snapshots one step; `state/` and `snaps/` hold each scenario's ids and snapshots ([ui-findings.md](docs/workflowy/ui-findings.md)) |

## Other files

| path | what it is |
|---|---|
| `flake.nix`, `flake.lock` | Nix dev shell (Python 3.14, `uv`, the Rust toolchain). |
| `.envrc` | direnv: loads the flake, sets up the `uv` virtualenv, and sources `.envrc.local`. |
| `pyproject.toml`, `uv.lock`, `.python-version` | Python project and locked dependencies. |
| `.prod-state/` (git-ignored) | `prod_client` state: the log of every production request (`requests.jsonl`), the ids it may touch, and its rate-limit state. |
