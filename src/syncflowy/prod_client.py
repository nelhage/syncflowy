"""Rate-limited, scope-guarded client for the *production* Workflowy API.

ALL access to https://workflowy.com from this project must go through this
module. It enforces:

* A global rate limit (default 9 req/s, below the 10 req/s ceiling we agreed
  to) that is shared across *all processes* on this machine via an fcntl-locked
  state file, so parallel experiment scripts cannot collectively exceed it.
* A scope guard: we may only touch the scratchpad node (short id
  ``a846cfce4c6d``) and its descendants. Node ids we are allowed to touch are
  tracked in a shared on-disk allow-list which grows as we create / list nodes
  under the scratchpad.
* A request log (JSONL) of every request/response, for documentation and for
  replaying against the mock server later.

Usage::

    from syncflowy.prod_client import ProdClient
    c = ProdClient()
    root = c.scratchpad_id()          # full UUID of the scratchpad
    r = c.request("POST", "/api/v1/nodes", json={"parent_id": root, "name": "x"})
    r.status_code, r.json()

Escape hatches (use sparingly, and document why in your notes):

* ``unscoped=True`` on ``request`` skips the scope guard. Only use it to probe
  ids that cannot possibly exist (e.g. random UUIDs, malformed ids) to observe
  error behaviour. Never use it to read or write real nodes outside the
  scratchpad.
"""

from __future__ import annotations

import fcntl
import json
import os
import re
import time
from pathlib import Path
from typing import Any

import httpx

BASE_URL = "https://workflowy.com"
SCRATCHPAD_SHORT_ID = "a846cfce4c6d"

STATE_DIR = Path(
    os.environ.get(
        "SYNCFLOWY_PROD_STATE",
        Path(__file__).resolve().parents[2] / ".prod-state",
    )
)
RATE_FILE = STATE_DIR / "ratelimit"
ALLOW_FILE = STATE_DIR / "allowed_ids.json"
LOG_FILE = STATE_DIR / "requests.jsonl"

# Seconds between requests, shared across processes. 1/9 s => <= 9 req/s.
MIN_INTERVAL = float(os.environ.get("SYNCFLOWY_PROD_MIN_INTERVAL", 1 / 9))

# Special parent_id keys which resolve outside the scratchpad. Using any of
# these against production is out of scope.
SPECIAL_KEYS_RE = re.compile(
    r"^(None|inbox|calendar|today|tomorrow|next_week|\d{4}(-\d{2}(-\d{2})?)?)$"
)

# Nodes that pre-date our experiments; never mutate these (only create
# children under the scratchpad root).
PROTECTED_IDS = {
    "5e2e76e4-fea4-4f1f-9c5b-a846cfce4c6d",  # scratchpad root
    "019daf85-288f-4404-bbf8-5f8f829dec96",  # Child A (user-made)
    "61e96ea0-585d-49f7-9988-5cda12d78b6a",  # Child B (user-made)
    "6f2eb3f7-7d79-4d19-b6ef-7736c9104d58",  # Child C (user-made)
}

UUID_RE = re.compile(r"^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$")


class ScopeError(Exception):
    pass


def _ensure_state_dir() -> None:
    STATE_DIR.mkdir(parents=True, exist_ok=True)


class RateLimiter:
    """Cross-process minimum-interval limiter using an flock'd file."""

    def __init__(self, path: Path = RATE_FILE, interval: float = MIN_INTERVAL):
        self.path = path
        self.interval = interval

    def acquire(self) -> None:
        _ensure_state_dir()
        with open(self.path, "a+") as f:
            fcntl.flock(f, fcntl.LOCK_EX)
            try:
                f.seek(0)
                raw = f.read().strip()
                next_ok = float(raw) if raw else 0.0
                now = time.time()
                if now < next_ok:
                    time.sleep(next_ok - now)
                    now = time.time()
                f.seek(0)
                f.truncate()
                f.write(repr(now + self.interval))
                f.flush()
            finally:
                fcntl.flock(f, fcntl.LOCK_UN)


class AllowList:
    """Shared on-disk set of node ids known to be inside the scratchpad."""

    def __init__(self, path: Path = ALLOW_FILE):
        self.path = path

    def _locked(self, fn):
        _ensure_state_dir()
        with open(self.path, "a+") as f:
            fcntl.flock(f, fcntl.LOCK_EX)
            try:
                f.seek(0)
                raw = f.read()
                data = json.loads(raw) if raw.strip() else {"ids": []}
                ids = set(data["ids"])
                changed = fn(ids)
                if changed:
                    f.seek(0)
                    f.truncate()
                    json.dump({"ids": sorted(ids)}, f)
                    f.flush()
                return ids
            finally:
                fcntl.flock(f, fcntl.LOCK_UN)

    def ids(self) -> set[str]:
        return self._locked(lambda ids: False)

    def add(self, *new: str) -> None:
        def go(ids: set[str]) -> bool:
            before = len(ids)
            ids.update(n for n in new if n)
            return len(ids) != before

        self._locked(go)

    def contains(self, node_id: str) -> bool:
        ids = self.ids()
        if node_id in ids:
            return True
        # Allow short ids (last 12 hex chars of the UUID, dashes removed).
        if len(node_id) == 12:
            return any(i.replace("-", "")[-12:] == node_id for i in ids)
        # Allow workflowy URLs of allowed nodes.
        m = re.search(r"#/([0-9a-f]{12})$", node_id)
        if m:
            return self.contains(m.group(1))
        return False


class ProdClient:
    def __init__(self, api_key: str | None = None, timeout: float = 30.0):
        self.api_key = api_key or os.environ["WORKFLOWY_API_KEY"]
        self.http = httpx.Client(
            base_url=BASE_URL,
            timeout=timeout,
            headers={"Authorization": f"Bearer {self.api_key}"},
        )
        self.limiter = RateLimiter()
        self.allow = AllowList()
        self._scratch: str | None = None

    # ----- scope -------------------------------------------------------

    def scratchpad_id(self) -> str:
        """Full UUID of the scratchpad root (resolved once and cached)."""
        if self._scratch:
            return self._scratch
        for i in self.allow.ids():
            if i.replace("-", "").endswith(SCRATCHPAD_SHORT_ID):
                self._scratch = i
                return i
        r = self._raw("GET", f"/api/v1/nodes/{SCRATCHPAD_SHORT_ID}")
        r.raise_for_status()
        self._scratch = r.json()["node"]["id"]
        self.allow.add(self._scratch)
        return self._scratch

    def _check_id(self, node_id: str, what: str) -> None:
        if node_id is None or SPECIAL_KEYS_RE.match(str(node_id)):
            raise ScopeError(f"{what}={node_id!r} resolves outside the scratchpad")
        if node_id == SCRATCHPAD_SHORT_ID:
            return
        if not self.allow.contains(str(node_id)):
            raise ScopeError(
                f"{what}={node_id!r} is not a known scratchpad descendant "
                "(create or list it under the scratchpad first)"
            )

    @staticmethod
    def _is_protected(node_id: str) -> bool:
        short = node_id.replace("-", "")[-12:]
        return any(p.replace("-", "")[-12:] == short for p in PROTECTED_IDS)

    def _guard(self, method: str, path: str, params, body) -> None:
        self.scratchpad_id()
        p = path.split("?")[0]
        if p in ("/api/v1/nodes-export", "/api/v1/targets"):
            raise ScopeError(f"{p} reads data outside the scratchpad; not allowed")
        m = re.match(r"^/api/v1/nodes/([^/]+)(/.*)?$", p)
        if m:
            self._check_id(m.group(1), "id")
            if method != "GET" and self._is_protected(m.group(1)):
                raise ScopeError(f"{m.group(1)} is a protected pre-existing node")
        elif p == "/api/v1/nodes":
            if method == "GET":
                self._check_id((params or {}).get("parent_id"), "parent_id")
        if isinstance(body, dict) and "parent_id" in body:
            self._check_id(body["parent_id"], "parent_id")
        elif method == "POST" and p == "/api/v1/nodes":
            # Omitted parent_id defaults to (probably) the root: out of scope.
            raise ScopeError("POST /api/v1/nodes without parent_id is out of scope")

    def _learn(self, method: str, path: str, resp: httpx.Response) -> None:
        """Grow the allow-list from responses about scratchpad nodes."""
        try:
            data = resp.json()
        except Exception:
            return
        if not isinstance(data, dict):
            return
        new = []
        for k in ("item_id", "origin_id"):
            if isinstance(data.get(k), str):
                new.append(data[k])
        if isinstance(data.get("node"), dict):
            new.append(data["node"].get("id"))
        for n in data.get("nodes") or []:
            if isinstance(n, dict):
                new.append(n.get("id"))
        if new:
            self.allow.add(*[n for n in new if isinstance(n, str)])

    # ----- requests ----------------------------------------------------

    def _raw(self, method: str, path: str, **kw) -> httpx.Response:
        attempt = 0
        while True:
            self.limiter.acquire()
            t0 = time.time()
            resp = self.http.request(method, path, **kw)
            self._log(method, path, kw, resp, t0)
            if resp.status_code == 429 and attempt < 5:
                attempt += 1
                retry = resp.headers.get("retry-after")
                time.sleep(float(retry) if retry and retry.isdigit() else 2**attempt)
                continue
            return resp

    def _log(self, method, path, kw, resp: httpx.Response, t0: float) -> None:
        _ensure_state_dir()
        try:
            body = resp.json()
        except Exception:
            body = resp.text[:5000]
        rec = {
            "ts": t0,
            "elapsed": round(time.time() - t0, 4),
            "method": method,
            "path": path,
            "params": kw.get("params"),
            "request_json": kw.get("json"),
            "request_content": (
                kw["content"].decode("utf-8", "replace")
                if isinstance(kw.get("content"), bytes)
                else kw.get("content")
            ),
            "request_headers": {
                k: v for k, v in (kw.get("headers") or {}).items()
                if k.lower() != "authorization"
            },
            "status": resp.status_code,
            "response_headers": dict(resp.headers),
            "response": body,
            "tag": os.environ.get("SYNCFLOWY_TAG"),
        }
        with open(LOG_FILE, "a") as f:
            fcntl.flock(f, fcntl.LOCK_EX)
            f.write(json.dumps(rec) + "\n")
            fcntl.flock(f, fcntl.LOCK_UN)

    def request(
        self,
        method: str,
        path: str,
        *,
        params: dict | None = None,
        json: Any = None,
        content: bytes | str | None = None,
        headers: dict | None = None,
        unscoped: bool = False,
    ) -> httpx.Response:
        """Issue a request. ``json`` and ``content`` are mutually exclusive.

        ``content`` lets you send raw / malformed bodies (e.g. form-encoded,
        invalid JSON) to probe the server's parsing.
        """
        method = method.upper()
        if not unscoped:
            body = json
            if body is None and content is not None:
                try:
                    import json as _j

                    body = _j.loads(content)
                except Exception:
                    body = None
            self._guard(method, path, params, body)
        kw: dict[str, Any] = {}
        if params is not None:
            kw["params"] = params
        if json is not None:
            kw["json"] = json
        if content is not None:
            kw["content"] = content.encode() if isinstance(content, str) else content
        if headers is not None:
            kw["headers"] = headers
        resp = self._raw(method, path, **kw)
        if not unscoped and resp.status_code < 400:
            self._learn(method, path, resp)
        return resp

    # ----- conveniences ------------------------------------------------

    def get(self, node_id: str) -> httpx.Response:
        return self.request("GET", f"/api/v1/nodes/{node_id}")

    def list(self, parent_id: str) -> httpx.Response:
        return self.request("GET", "/api/v1/nodes", params={"parent_id": parent_id})

    def create(self, parent_id: str, name: str, **extra) -> httpx.Response:
        return self.request(
            "POST", "/api/v1/nodes", json={"parent_id": parent_id, "name": name, **extra}
        )

    def update(self, node_id: str, **fields) -> httpx.Response:
        return self.request("POST", f"/api/v1/nodes/{node_id}", json=fields)

    def delete(self, node_id: str) -> httpx.Response:
        return self.request("DELETE", f"/api/v1/nodes/{node_id}")

    def move(self, node_id: str, parent_id: str, **extra) -> httpx.Response:
        return self.request(
            "POST", f"/api/v1/nodes/{node_id}/move", json={"parent_id": parent_id, **extra}
        )

    def mirror(self, node_id: str, parent_id: str, **extra) -> httpx.Response:
        return self.request(
            "POST", f"/api/v1/nodes/{node_id}/mirror", json={"parent_id": parent_id, **extra}
        )

    def complete(self, node_id: str) -> httpx.Response:
        return self.request("POST", f"/api/v1/nodes/{node_id}/complete")

    def uncomplete(self, node_id: str) -> httpx.Response:
        return self.request("POST", f"/api/v1/nodes/{node_id}/uncomplete")

    def make_sandbox(self, label: str) -> str:
        """Create a fresh child of the scratchpad for an experiment; returns its id."""
        r = self.create(self.scratchpad_id(), f"sandbox: {label}", position="bottom")
        r.raise_for_status()
        return r.json()["item_id"]
