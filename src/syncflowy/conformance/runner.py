"""Running scenarios, golden-file I/O, and comparison."""

from __future__ import annotations

import contextlib
import datetime
import difflib
import inspect
import json
import os
import traceback
from dataclasses import dataclass
from pathlib import Path
from typing import Iterator

from .backend import Backend, FakeClock, MockBackend, UnsafeRequest
from .context import Ctx
from .normalize import normalize, render
from .registry import Scenario

GOLDEN_DIR = Path(
    os.environ.get(
        "SYNCFLOWY_GOLDEN_DIR",
        Path(__file__).resolve().parents[3] / "tests" / "conformance" / "golden",
    )
)

MOCK_API_KEY = "conformance-key"


class HarnessError(Exception):
    pass


def run_scenario(backend: Backend, sc: Scenario) -> dict:
    """Run one scenario in a fresh sandbox; return the raw transcript.

    Exceptions raised by the scenario body are recorded as an ``error`` step
    (so a mock that fails a precondition shows up in the diff). Safety
    refusals (:class:`UnsafeRequest`) are re-raised: they are bugs in the
    scenario, not behaviour.
    """
    sandbox = backend.setup(sc.name)
    ctx = Ctx(backend, sandbox)
    unsafe: Exception | None = None
    try:
        ctx.start()
        try:
            sc.fn(ctx)
        except UnsafeRequest as e:
            unsafe = e
        except Exception as e:  # noqa: BLE001 - recorded into transcript
            tb = traceback.extract_tb(e.__traceback__)
            where = next(
                (f"{Path(f.filename).name}:{f.lineno}" for f in reversed(tb) if "scenarios" in f.filename),
                "?",
            )
            ctx.error(f"{type(e).__name__} at {where}: {e}")
    finally:
        backend.teardown(sandbox)
    if unsafe is not None:
        raise HarnessError(f"{sc.name}: refused unsafe request: {unsafe}") from unsafe
    return {
        "scenario": sc.name,
        "backend": backend.name,
        "clock_kind": backend.clock_kind,
        "root": sandbox.root,
        "sandbox_parent": sandbox.parent,
        "sandbox_name": sandbox.name,
        "setup_window": list(sandbox.window),
        "steps": ctx.steps,
    }


def has_errors(raw: dict) -> list[str]:
    return [st["message"] for st in raw["steps"] if st.get("type") == "error"]


def ambiguous_aliases(raw: dict) -> list[str]:
    """Aliased nodes whose short id has no letter, as ``"<name>=<id>"`` strings.

    Scenarios send short ids and URLs both as-is and upper-cased, and the
    normaliser labels the upper-cased form ``:UPPER`` by looking at the letters
    in the token. An all-digit short id (about 0.36% of UUIDv4s) is its own
    upper case, so such a transcript cannot be normalised consistently and must
    not become a golden. The mock never generates these ids
    (``syncflowy.mock.ops.new_node_id``); production does, so ``record``
    re-runs the scenario in a fresh sandbox when this list is non-empty.
    """
    out = []
    for st in raw["steps"]:
        if st.get("type") == "alias":
            node_id = str(st["id"])
            if not any(c.isalpha() for c in node_id[-12:]):
                out.append(f"{st['name']}={node_id}")
    return out


# ---------------------------------------------------------------------------
# golden files


def golden_path(name: str, golden_dir: Path | None = None) -> Path:
    return (golden_dir or GOLDEN_DIR) / f"{name}.json"


def write_golden(sc: Scenario, raw: dict, golden_dir: Path | None = None) -> Path:
    p = golden_path(sc.name, golden_dir)
    p.parent.mkdir(parents=True, exist_ok=True)
    doc = {
        "scenario": sc.name,
        "doc": sc.doc,
        "recorded_at": datetime.datetime.now(datetime.UTC).isoformat(timespec="seconds"),
        "normalized": normalize(raw),
        "raw": raw,
    }
    p.write_text(json.dumps(doc, indent=1) + "\n")
    return p


def read_golden(name: str, golden_dir: Path | None = None) -> dict | None:
    p = golden_path(name, golden_dir)
    if not p.exists():
        return None
    return json.loads(p.read_text())


def golden_names(golden_dir: Path | None = None) -> list[str]:
    d = golden_dir or GOLDEN_DIR
    if not d.exists():
        return []
    return sorted(p.stem for p in d.glob("*.json"))


# ---------------------------------------------------------------------------
# mock lifecycle


class MockOptionUnsupported(Exception):
    pass


@contextlib.contextmanager
def mock_backend(options: dict | None = None, api_key: str = MOCK_API_KEY) -> Iterator[MockBackend]:
    """Start an in-process mock (``running_mock``) with a FakeClock."""
    from syncflowy.mock.testing import running_mock  # type: ignore[import-not-found]

    options = dict(options or {})
    if options:
        sig = inspect.signature(running_mock)
        has_kwargs = any(p.kind == p.VAR_KEYWORD for p in sig.parameters.values())
        missing = [k for k in options if k not in sig.parameters]
        if missing and not has_kwargs:
            raise MockOptionUnsupported(
                f"running_mock does not accept {', '.join(missing)}"
            )
    clock = FakeClock()
    with running_mock(db_path=None, api_key=api_key, clock=clock, **options) as url:
        b = MockBackend(url, api_key=api_key, clock=clock)
        try:
            yield b
        finally:
            b.close()


class MockPool:
    """Lazily starts one mock per distinct ``mock_options`` set."""

    def __init__(self, base_url: str | None = None, api_key: str = MOCK_API_KEY):
        self.base_url = base_url
        self.api_key = api_key
        self.stack = contextlib.ExitStack()
        self.backends: dict[str, MockBackend | Exception] = {}

    def get(self, options: dict) -> MockBackend:
        if self.base_url is not None:
            key = "external"
            if key not in self.backends:
                self.backends[key] = MockBackend(self.base_url, self.api_key, clock=None)
            if options:
                raise MockOptionUnsupported(f"external mock: cannot set options {options}")
            return self.backends[key]  # type: ignore[return-value]
        key = json.dumps(options, sort_keys=True)
        if key not in self.backends:
            try:
                self.backends[key] = self.stack.enter_context(mock_backend(options, self.api_key))
            except MockOptionUnsupported as e:
                self.backends[key] = e
        b = self.backends[key]
        if isinstance(b, Exception):
            raise b
        return b

    def close(self) -> None:
        for b in self.backends.values():
            if isinstance(b, MockBackend) and self.base_url is not None:  # external only
                b.close()
        self.stack.close()

    def __enter__(self):
        return self

    def __exit__(self, *exc):
        self.close()


# ---------------------------------------------------------------------------
# comparison


@dataclass
class CheckResult:
    name: str
    status: str  # "pass", "fail", "skip", "error"
    detail: str = ""


def diff_transcripts(expected: list[dict], actual: list[dict], *, context: int = 4) -> str:
    a, b = render(expected), render(actual)
    if a == b:
        return ""
    return "\n".join(
        difflib.unified_diff(a, b, "production (golden)", "mock", n=context, lineterm="")
    )


def check_scenario(sc: Scenario, pool: MockPool, golden_dir: Path | None = None) -> CheckResult:
    g = read_golden(sc.name, golden_dir)
    if g is None:
        return CheckResult(sc.name, "skip", "no golden recording")
    try:
        backend = pool.get(sc.mock_options)
    except MockOptionUnsupported as e:
        return CheckResult(sc.name, "skip", f"mock lacks required option(s) {sc.mock_options}: {e}")
    try:
        raw = run_scenario(backend, sc)
    except Exception as e:  # noqa: BLE001
        return CheckResult(sc.name, "error", f"{type(e).__name__}: {e}\n{traceback.format_exc()}")
    expected = normalize(g["raw"])
    actual = normalize(raw)
    d = diff_transcripts(expected, actual)
    return CheckResult(sc.name, "fail" if d else "pass", d)
