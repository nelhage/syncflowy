"""The scenario context ``ctx``: a backend-agnostic, recording API client.

Every request a scenario makes goes through :meth:`Ctx.req`, which records the
request, the response, the backend-clock window around it and the current
*epoch* into ``ctx.steps``. The raw transcript is later normalised by
:mod:`.normalize`.

Epochs
------
Workflowy timestamps have 1-second resolution and the production server
clock is not ours, so two requests 0.2 s apart may or may not get the same
``modifiedAt``. Timestamps are therefore only compared at *epoch*
granularity: an epoch is a run of requests between two ``ctx.sleep()`` calls.
``ctx.sleep()`` always waits at least the backend's ``min_epoch_gap`` (3 s on
production, 2 s of fake time on the mock), so
timestamps from different epochs are guaranteed to differ and to be
attributable. Epoch 0 is the sandbox creation; ``Ctx.start()`` sleeps once
before the scenario body, so the scenario begins in epoch 1 and the sandbox
root's own timestamps are distinguishable from anything the scenario does.

On the mock the backend clock is a ``FakeClock``: sleeping just advances it,
so the whole suite runs in milliseconds.
"""

from __future__ import annotations

import json as _json
from dataclasses import dataclass
from typing import Any

from .backend import Backend, Sandbox

MIN_EPOCH_GAP = 2.0


class ScenarioError(Exception):
    """A helper's precondition failed (e.g. a create that should succeed didn't)."""


@dataclass
class Resp:
    """What a scenario sees of a response."""

    status: int
    headers: dict[str, str]
    text: str
    json: Any  # parsed body or None

    @property
    def ok(self) -> bool:
        return self.status == 200


class Ctx:
    def __init__(self, backend: Backend, sandbox: Sandbox):
        self.backend = backend
        self.sandbox = sandbox
        self.root = sandbox.root
        self.epoch = 0
        self.steps: list[dict] = []

    # ------------------------------------------------------------------
    # recording primitives

    def start(self) -> None:
        """Called by the runner before the scenario body: move to epoch 1."""
        self.backend.sleep(max(MIN_EPOCH_GAP, self.backend.min_epoch_gap))
        self.epoch = 1

    def note(self, text: str) -> None:
        """Add a free-text annotation to the transcript (compared too)."""
        self.steps.append({"type": "note", "text": text})

    def alias(self, node_id: str, name: str) -> str:
        """Give a node a readable symbolic name in normalised transcripts.

        Must be called identically on every backend (it is part of the
        transcript). Names should be short identifiers like ``"P"`` or
        ``"child1"``; they render as ``<P>``.
        """
        self.steps.append({"type": "alias", "id": node_id, "name": name})
        return node_id

    def sleep(self, seconds: float = MIN_EPOCH_GAP) -> None:
        """Start a new timestamp epoch.

        Sleeps at least ``MIN_EPOCH_GAP`` and at least the backend's
        ``min_epoch_gap`` (3 s on production, where clock skew must be
        tolerated). On the fake-clock mock this is instant.
        """
        seconds = max(float(seconds), MIN_EPOCH_GAP, self.backend.min_epoch_gap)
        self.backend.sleep(seconds)
        self.epoch += 1
        self.steps.append({"type": "sleep", "seconds": seconds, "epoch": self.epoch})

    def req(
        self,
        method: str,
        path: str,
        *,
        json: Any = None,
        params: Any = None,
        content: str | bytes | None = None,
        headers: dict | None = None,
    ) -> Resp:
        """Send one request and record it.

        ``json=None`` means "no JSON body"; send a literal JSON ``null`` with
        ``content="null"``. ``params`` may be a dict or a list of pairs.
        Header values may use ``{API_KEY}``.
        """
        if json is not None and content is not None:
            raise ValueError("json and content are mutually exclusive")
        t0 = self.backend.now()
        r = self.backend.request(
            method, path, params=params, json=json, content=content, headers=headers
        )
        t1 = self.backend.now()
        text = r.text
        try:
            parsed = _json.loads(text) if text else None
            is_json = bool(text)
        except ValueError:
            parsed, is_json = None, False
        step = {
            "type": "request",
            "epoch": self.epoch,
            "t0": t0,
            "t1": t1,
            "method": method.upper(),
            "path": path,
            "params": [list(p) for p in params] if isinstance(params, list) else params,
            "json": json,
            "content": content.decode("utf-8", "replace") if isinstance(content, bytes) else content,
            "headers": headers,
            "status": r.status_code,
            "response_headers": dict(r.headers),
            "body_text": text,
            "body_is_json": is_json,
        }
        self.steps.append(step)
        return Resp(status=r.status_code, headers=dict(r.headers), text=text, json=parsed)

    def error(self, message: str) -> None:
        self.steps.append({"type": "error", "message": message})

    # ------------------------------------------------------------------
    # convenience wrappers (all go through req)

    def create(self, parent: str, name: str = "x", **fields) -> str:
        """Create a node and return its id; raises ScenarioError on failure."""
        body = {"parent_id": parent, "name": name, **fields}
        r = self.req("POST", "/api/v1/nodes", json=body)
        if r.status != 200 or not isinstance(r.json, dict) or "item_id" not in r.json:
            raise ScenarioError(f"create {name!r} failed: {r.status} {r.text[:200]}")
        return r.json["item_id"]

    def try_create(self, parent: str, name: Any = "x", **fields) -> Resp:
        body = {"parent_id": parent, "name": name, **fields}
        return self.req("POST", "/api/v1/nodes", json=body)

    def get(self, node_id: str) -> Resp:
        return self.req("GET", f"/api/v1/nodes/{node_id}")

    def node(self, node_id: str) -> dict:
        r = self.get(node_id)
        if r.status != 200:
            raise ScenarioError(f"get {node_id} failed: {r.status}")
        return r.json["node"]

    def list(self, parent: str) -> Resp:
        return self.req("GET", "/api/v1/nodes", params={"parent_id": parent})

    def children(self, parent: str) -> list[dict]:
        r = self.list(parent)
        if r.status != 200:
            raise ScenarioError(f"list {parent} failed: {r.status}")
        return r.json["nodes"]

    def update(self, node_id: str, **fields) -> Resp:
        return self.req("POST", f"/api/v1/nodes/{node_id}", json=fields)

    def complete(self, node_id: str) -> Resp:
        return self.req("POST", f"/api/v1/nodes/{node_id}/complete")

    def uncomplete(self, node_id: str) -> Resp:
        return self.req("POST", f"/api/v1/nodes/{node_id}/uncomplete")

    def delete(self, node_id: str) -> Resp:
        return self.req("DELETE", f"/api/v1/nodes/{node_id}")

    def move(self, node_id: str, parent: str, **fields) -> Resp:
        return self.req("POST", f"/api/v1/nodes/{node_id}/move", json={"parent_id": parent, **fields})

    def mirror(self, node_id: str, parent: str, **fields) -> Resp:
        return self.req("POST", f"/api/v1/nodes/{node_id}/mirror", json={"parent_id": parent, **fields})

    def make_mirror(self, node_id: str, parent: str, **fields) -> str:
        r = self.mirror(node_id, parent, **fields)
        if r.status != 200:
            raise ScenarioError(f"mirror failed: {r.status} {r.text[:200]}")
        return r.json["item_id"]

    def delete_mirror(self, node_id: str) -> Resp:
        return self.req("DELETE", f"/api/v1/nodes/{node_id}/mirror")

    def show(self, node_id: str) -> Resp:
        """GET a node and list its children (two requests)."""
        r = self.get(node_id)
        self.list(node_id)
        return r
