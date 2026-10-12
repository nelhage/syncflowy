"""Backends: where a scenario's requests go.

Both backends expose the same tiny interface used by :class:`~.context.Ctx`:

* ``setup(scenario_name)`` creates a fresh sandbox node and returns a
  :class:`Sandbox` (its id, its parent's id, and the clock window during which
  it was created).
* ``request(method, path, params=, json=, content=, headers=)`` sends one HTTP
  request and returns the ``httpx.Response``. Header values may contain the
  placeholder ``{API_KEY}``, which is replaced by the backend's real key (so
  transcripts never contain a key and scenarios can test auth-header variants).
* ``now()`` / ``sleep(seconds)`` -- the clock the server's timestamps come
  from. On production this is wall-clock time; on the mock it is a
  :class:`FakeClock` that the mock server also reads, so sleeping is instant.
* ``teardown(sandbox)``.

Sandbox layout is the same on both backends, so that the sandbox root is an
ordinary non-top-level node everywhere::

    production:  <scratchpad a846cfce4c6d>  -> "sandbox: conformance <name>"
    mock:        "conformance scratchpad" (top-level, parent "None")
                                            -> "sandbox: conformance <name>"
"""

from __future__ import annotations

import json as _json
import math
import re
import threading
import time
import urllib.parse
from dataclasses import dataclass
from typing import Any

import httpx

API_KEY_PLACEHOLDER = "{API_KEY}"

UUID_RE = re.compile(
    r"^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$", re.I
)
# Id-like hex tokens. A token may directly follow a percent-escape such as
# "%2F" (whose hex digits would otherwise defeat the left boundary check).
HEX_TOKEN_RE = re.compile(
    r"(?:(?<![0-9a-fA-F])|(?<=%[0-9a-fA-F]{2}))("
    r"[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}"
    r"|[0-9a-fA-F]{32}"
    r"|[0-9a-fA-F]{12}"
    r")(?![0-9a-fA-F])"
)


@dataclass
class Sandbox:
    root: str  # full UUID of the sandbox node (the scenario's ctx.root)
    parent: str  # full UUID of the sandbox's parent (scratchpad / mock scratch)
    window: tuple[float, float]  # backend clock before/after creating it
    name: str


class FakeClock:
    """A controllable clock shared between a test and the mock server.

    Calling the instance returns the current fake unix time (float), which is
    the ``clock`` callable contract of ``running_mock``. It starts in the
    middle of the current wall-clock second (so small offsets never cross a
    second boundary) and only moves when :meth:`advance` is called.
    """

    def __init__(self, start: float | None = None):
        if start is None:
            start = math.floor(time.time()) + 0.5
        self._t = float(start)
        self._lock = threading.Lock()

    def __call__(self) -> float:
        with self._lock:
            return self._t

    def advance(self, seconds: float) -> None:
        with self._lock:
            self._t += float(seconds)


class Backend:
    name = "abstract"
    clock_kind = "real"  # "real" or "fake"
    # ctx.sleep() never sleeps less than this. Epoch timestamp ranges are
    # floor(t + skew +- slack) with slack <= 0.9 s, so on a real clock a gap
    # of >= 1 + 2*0.9 s guarantees adjacent epochs cannot share a second.
    min_epoch_gap = 3.0

    def setup(self, scenario_name: str) -> Sandbox:  # pragma: no cover
        raise NotImplementedError

    def teardown(self, sandbox: Sandbox) -> None:  # pragma: no cover
        pass

    def request(
        self,
        method: str,
        path: str,
        *,
        params: Any = None,
        json: Any = None,
        content: str | bytes | None = None,
        headers: dict | None = None,
    ) -> httpx.Response:  # pragma: no cover
        raise NotImplementedError

    def now(self) -> float:
        return time.time()

    def sleep(self, seconds: float) -> None:
        time.sleep(seconds)

    def close(self) -> None:
        pass

    # helpers -------------------------------------------------------------

    @staticmethod
    def _subst_headers(headers: dict | None, key: str) -> dict | None:
        if headers is None:
            return None
        return {
            k: (v.replace(API_KEY_PLACEHOLDER, key) if isinstance(v, str) else v)
            for k, v in headers.items()
        }

    @staticmethod
    def _sandbox_name(scenario_name: str) -> str:
        return f"sandbox: conformance {scenario_name}"


# ---------------------------------------------------------------------------
# Mock


class MockBackend(Backend):
    """Talks plain HTTP to a mock server at ``base_url``.

    ``clock`` should be the same :class:`FakeClock` passed to
    ``running_mock(clock=...)``; then ``sleep`` advances it instead of
    sleeping. With ``clock=None`` (an externally started mock) real time is
    used.
    """

    name = "mock"
    min_epoch_gap = 2.0  # exact with a FakeClock (no skew); overridden below if real

    def __init__(self, base_url: str, api_key: str = "test-key", clock: FakeClock | None = None):
        self.base_url = base_url.rstrip("/")
        self.api_key = api_key
        self.clock = clock
        self.clock_kind = "fake" if clock is not None else "real"
        if clock is None:
            self.min_epoch_gap = 3.0
        self.http = httpx.Client(
            base_url=self.base_url,
            timeout=60.0,
            headers={"Authorization": f"Bearer {api_key}"},
        )
        self._scratch: str | None = None

    def now(self) -> float:
        return self.clock() if self.clock is not None else time.time()

    def sleep(self, seconds: float) -> None:
        if self.clock is not None:
            self.clock.advance(seconds)
        else:
            time.sleep(seconds)

    def request(self, method, path, *, params=None, json=None, content=None, headers=None):
        kw: dict[str, Any] = {}
        if params is not None:
            kw["params"] = params
        if json is not None:
            kw["json"] = json
        if content is not None:
            kw["content"] = content.encode() if isinstance(content, str) else content
        h = self._subst_headers(headers, self.api_key)
        if h is not None:
            kw["headers"] = h
        return self.http.request(method.upper(), path, **kw)

    def _create(self, parent: str, name: str) -> str:
        # Setup bypasses any request hooks of subclasses (e.g. vetting).
        r = MockBackend.request(
            self, "POST", "/api/v1/nodes", json={"parent_id": parent, "name": name, "position": "bottom"}
        )
        if r.status_code != 200:
            raise RuntimeError(f"mock: create under {parent!r} failed: {r.status_code} {r.text[:300]}")
        return r.json()["item_id"]

    def scratch(self) -> str:
        """The mock's stand-in for the production scratchpad (top-level node)."""
        if self._scratch is None:
            self._scratch = self._create("None", "conformance scratchpad")
        return self._scratch

    def setup(self, scenario_name: str) -> Sandbox:
        parent = self.scratch()
        t0 = self.now()
        name = self._sandbox_name(scenario_name)
        root = self._create(parent, name)
        t1 = self.now()
        return Sandbox(root=root, parent=parent, window=(t0, t1), name=name)

    def close(self) -> None:
        self.http.close()


# ---------------------------------------------------------------------------
# Production


class UnsafeRequest(Exception):
    """A scenario asked for a production request we refuse to send."""


# Any 12-hex suffix starting with ten zeros is treated as a deliberately fake
# id (see scenarios/_util.py FAKE_*). A real UUID matches with p = 16**-10.
_FAKE_SUFFIX_RE = re.compile(r"^0{10}[0-9a-f]{2}$")
_SPECIAL_KEYS_RE = re.compile(
    r"^(none|inbox|calendar|today|tomorrow|next_week|\d{4}(-\d{2}(-\d{2})?)?)$", re.I
)
# Characters that cannot plausibly occur in a user shortcut key; a garbage
# parent_id must contain one (or start with "zz-conformance") before we will
# send it unscoped.
_GARBAGE_MARKERS = set(" /:'\"#?%&")


class ProdBackend(Backend):
    """Runs scenarios against production through :class:`ProdClient`.

    All requests go through ``ProdClient.request`` (rate limit + scope guard).
    The guard only knows ids it has seen, so some deliberate probes -- a
    dashless UUID of our own node, a fake never-existing UUID, a garbage
    ``parent_id``, a repeated query parameter -- trip it. For those, and only
    when :meth:`_unscoped_ok` proves every id-like token in the request refers
    either to a node created in *this scenario's sandbox* or to a deliberately
    fake id, the request is re-sent with ``unscoped=True`` (the documented
    escape hatch for probing ids that cannot exist). Requests whose
    ``parent_id`` would resolve to the account root or a special location
    (missing / null / "" on create and move, ``None``, ``inbox``, calendar
    keys) are refused outright with :class:`UnsafeRequest`.
    """

    name = "prod"
    clock_kind = "real"

    def __init__(self, client=None, keep_sandboxes: bool = False):
        from syncflowy.prod_client import ProdClient

        self.c = client or ProdClient()
        self.api_key = self.c.api_key
        self.keep = keep_sandboxes
        self.ours: set[str] = set()  # full lowercase UUIDs created in this sandbox

    # --- sandbox ---------------------------------------------------------

    def setup(self, scenario_name: str) -> Sandbox:
        parent = self.c.scratchpad_id()
        t0 = self.now()
        root = self.c.make_sandbox(f"conformance {scenario_name}")
        t1 = self.now()
        self.ours = {root}
        return Sandbox(root=root, parent=parent, window=(t0, t1), name=self._sandbox_name(scenario_name))

    def teardown(self, sandbox: Sandbox) -> None:
        if self.keep:
            return
        r = self.c.request("DELETE", f"/api/v1/nodes/{sandbox.root}")
        if r.status_code != 200:
            print(f"warning: failed to delete sandbox {sandbox.root}: {r.status_code} {r.text[:200]}")

    # --- safety ----------------------------------------------------------

    def _is_ours_or_fake(self, token: str) -> bool:
        t = token.lower().replace("-", "")
        if _FAKE_SUFFIX_RE.match(t[-12:]):
            return True
        for u in self.ours:
            d = u.replace("-", "")
            if t == d or (len(t) == 12 and d.endswith(t)):
                return True
        return False

    def _check_string(self, s: str, what: str, *, is_parent: bool) -> None:
        if _SPECIAL_KEYS_RE.match(s):
            raise UnsafeRequest(f"{what}={s!r} is a special location key")
        tokens = HEX_TOKEN_RE.findall(s)
        for tok in tokens:
            if not self._is_ours_or_fake(tok):
                raise UnsafeRequest(f"{what}={s!r} references id {tok!r} outside this sandbox")
        if not tokens and is_parent:
            if not (s.startswith("zz-conformance") or (set(s) & _GARBAGE_MARKERS)):
                raise UnsafeRequest(
                    f"{what}={s!r}: garbage parent ids must start with 'zz-conformance' or "
                    "contain one of ' /:\\'\"#?%&' so they cannot be a shortcut key"
                )

    @staticmethod
    def _parse_body(json_body, content, headers) -> tuple[bool, Any]:
        """Return (parsed, body) as the server would see it (best effort)."""
        if json_body is not None:
            return True, json_body
        if content is None:
            return False, None
        text = content.decode("utf-8", "replace") if isinstance(content, bytes) else content
        ctype = ""
        for k, v in (headers or {}).items():
            if k.lower() == "content-type":
                ctype = v.lower()
        if "x-www-form-urlencoded" in ctype:
            return True, {k: v[-1] for k, v in urllib.parse.parse_qs(text, keep_blank_values=True).items()}
        if "multipart/form-data" in ctype:
            # crude: name="field"\r\n\r\nvalue
            fields = dict(re.findall(r'name="([^"]+)"\r?\n\r?\n([^\r\n]*)', text))
            return True, fields
        try:
            return True, _json.loads(text)
        except Exception:
            return False, None

    def _vet(self, method: str, path: str, params, json_body, content, headers) -> None:
        """Refuse anything that could touch nodes outside our sandbox."""
        p, _, query = path.partition("?")
        if p.rstrip("/") in ("/api/v1/nodes-export", "/api/v1/targets"):
            raise UnsafeRequest(f"{p} reads outside the scratchpad")
        m = re.match(r"^/api/v1/nodes/([^/]+)", p)
        if m:
            self._check_string(urllib.parse.unquote(m.group(1)), "path id", is_parent=False)
        qs: list[tuple[str, str]] = urllib.parse.parse_qsl(query, keep_blank_values=True)
        if isinstance(params, dict):
            qs += [(k, str(v)) for k, v in params.items()]
        elif params:
            qs += [(k, str(v)) for k, v in params]
        is_list = method == "GET" and re.match(r"^/api/v1/nodes/?$", p)
        list_parents = [v for k, v in qs if k == "parent_id"]
        if is_list:
            if not list_parents:
                raise UnsafeRequest("list without parent_id would list the account root")
            for v in list_parents:
                if v == "":
                    raise UnsafeRequest("list with empty parent_id")
                self._check_string(v, "parent_id", is_parent=True)
        parsed, body = self._parse_body(json_body, content, headers)
        is_create = method == "POST" and re.match(r"^/api/v1/nodes/?$", p)
        is_move = method == "POST" and re.match(r"^/api/v1/nodes/[^/]+/move/?$", p)
        is_mirror = method == "POST" and re.match(r"^/api/v1/nodes/[^/]+/mirror/?$", p)
        if parsed and isinstance(body, dict):
            pid = body.get("parent_id")
            if isinstance(pid, str) and pid != "":
                self._check_string(pid, "parent_id", is_parent=True)
            elif (is_create or is_move) and not (isinstance(pid, (int, float, bool, list, dict))):
                # missing / null / "" resolve to the account root (untested in prod).
                raise UnsafeRequest(
                    f"{'create' if is_create else 'move'} with parent_id={pid!r} may target the account root"
                )
            elif is_mirror:
                pass  # validation rejects missing/null for mirror; "" is a miss
        # unparseable or non-object bodies are rejected by the server before
        # any lookup, so they are safe.

    # --- requests --------------------------------------------------------

    def request(self, method, path, *, params=None, json=None, content=None, headers=None):
        from syncflowy.prod_client import ScopeError

        method = method.upper()
        self._vet(method, path, params, json, content, headers)
        h = self._subst_headers(headers, self.api_key)
        kw = dict(params=params, json=json, content=content, headers=h)
        try:
            resp = self.c.request(method, path, **kw)
        except (ScopeError, AttributeError):
            # _vet already proved every id-like token is ours or fake.
            resp = self.c.request(method, path, unscoped=True, **kw)
        self._learn(resp)
        return resp

    def _learn(self, resp: httpx.Response) -> None:
        if resp.status_code >= 400:
            return
        try:
            data = resp.json()
        except Exception:
            return
        if not isinstance(data, dict):
            return
        for k in ("item_id", "origin_id"):
            if isinstance(data.get(k), str):
                self.ours.add(data[k])
        if isinstance(data.get("node"), dict) and isinstance(data["node"].get("id"), str):
            self.ours.add(data["node"]["id"])
        for n in data.get("nodes") or []:
            if isinstance(n, dict) and isinstance(n.get("id"), str):
                self.ours.add(n["id"])

    def close(self) -> None:
        self.c.http.close()


class VettingMockBackend(MockBackend):
    """A MockBackend that also applies ProdBackend's safety vetting to every
    request. Running all scenarios through it (see tests/conformance/
    test_harness.py and ``syncflowy-conformance record --dry-run``) proves no
    scenario will be refused -- or would touch anything outside its sandbox --
    before anything is sent to production."""

    def __init__(self, *a, **kw):
        super().__init__(*a, **kw)
        self._vetter = ProdBackend.__new__(ProdBackend)
        self._vetter.ours = set()

    def setup(self, scenario_name: str) -> Sandbox:
        sb = super().setup(scenario_name)
        self._vetter.ours = {sb.root}
        return sb

    def request(self, method, path, *, params=None, json=None, content=None, headers=None):
        self._vetter._vet(method.upper(), path, params, json, content, headers)
        r = super().request(method, path, params=params, json=json, content=content, headers=headers)
        self._vetter._learn(r)
        return r
