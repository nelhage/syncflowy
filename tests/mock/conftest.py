"""Shared fixtures for the mock-server tests.

Every test gets a fresh server (fresh temp SQLite file) and a controllable
clock. Tests talk to the server over real HTTP via httpx, exactly like a
client of production would.
"""

from __future__ import annotations

import uuid
from collections.abc import Iterator
from typing import Any

import httpx
import pytest

from syncflowy.mock.testing import running_mock

API_KEY = "test-key"
T0 = 1_791_834_396.25  # a production-ish timestamp


class FakeClock:
    def __init__(self, t: float = T0):
        self.t = t

    def __call__(self) -> float:
        return self.t

    def advance(self, dt: float = 1.0) -> None:
        self.t += dt

    @property
    def now(self) -> int:
        return int(self.t)


class WF:
    """Thin HTTP client for the mock API with convenience helpers."""

    def __init__(self, base_url: str, clock: FakeClock):
        self.base_url = base_url
        self.clock = clock
        self.http = httpx.Client(base_url=base_url, headers={"Authorization": f"Bearer {API_KEY}"}, timeout=30)
        self.anon = httpx.Client(base_url=base_url, timeout=30)

    # raw calls ------------------------------------------------------------
    def request(self, method: str, path: str, **kw: Any) -> httpx.Response:
        return self.http.request(method, path, **kw)

    def create_raw(self, body: Any = None, **kw: Any) -> httpx.Response:
        if body is not None:
            kw["json"] = body
        return self.http.post("/api/v1/nodes", **kw)

    def update_raw(self, node_id: str, body: Any = None, **kw: Any) -> httpx.Response:
        if body is not None:
            kw["json"] = body
        return self.http.post(f"/api/v1/nodes/{node_id}", **kw)

    def move_raw(self, node_id: str, body: Any = None, **kw: Any) -> httpx.Response:
        if body is not None:
            kw["json"] = body
        return self.http.post(f"/api/v1/nodes/{node_id}/move", **kw)

    def mirror_raw(self, node_id: str, body: Any = None, **kw: Any) -> httpx.Response:
        if body is not None:
            kw["json"] = body
        return self.http.post(f"/api/v1/nodes/{node_id}/mirror", **kw)

    def get_raw(self, node_id: str) -> httpx.Response:
        return self.http.get(f"/api/v1/nodes/{node_id}")

    def list_raw(self, parent_id: str | None = None) -> httpx.Response:
        params = {} if parent_id is None else {"parent_id": parent_id}
        return self.http.get("/api/v1/nodes", params=params)

    # checked helpers ------------------------------------------------------
    def create(self, parent_id: str | None, name: str = "x", **fields: Any) -> str:
        body = {"name": name, **fields}
        if parent_id is not None:
            body["parent_id"] = parent_id
        r = self.create_raw(body)
        assert r.status_code == 200, r.text
        return r.json()["item_id"]

    def get(self, node_id: str) -> dict[str, Any]:
        r = self.get_raw(node_id)
        assert r.status_code == 200, r.text
        return r.json()["node"]

    def list(self, parent_id: str | None) -> list[dict[str, Any]]:
        r = self.list_raw(parent_id)
        assert r.status_code == 200, r.text
        return r.json()["nodes"]

    def children(self, parent_id: str) -> list[dict[str, Any]]:
        """Children sorted by priority (display order)."""
        return sorted(self.list(parent_id), key=lambda n: n["priority"])

    def prios(self, parent_id: str) -> dict[str, int]:
        return {n["name"]: n["priority"] for n in self.list(parent_id)}

    def ok(self, r: httpx.Response) -> None:
        assert r.status_code == 200, r.text
        assert r.content == b'{"status": "ok"}'

    def update(self, node_id: str, **fields: Any) -> None:
        self.ok(self.update_raw(node_id, fields))

    def move(self, node_id: str, parent_id: str, position: str | None = None) -> None:
        body: dict[str, Any] = {"parent_id": parent_id}
        if position is not None:
            body["position"] = position
        self.ok(self.move_raw(node_id, body))

    def mirror(self, node_id: str, parent_id: str, position: str | None = None) -> dict[str, str]:
        body: dict[str, Any] = {"parent_id": parent_id}
        if position is not None:
            body["position"] = position
        r = self.mirror_raw(node_id, body)
        assert r.status_code == 200, r.text
        return r.json()

    def complete(self, node_id: str) -> None:
        self.ok(self.http.post(f"/api/v1/nodes/{node_id}/complete"))

    def uncomplete(self, node_id: str) -> None:
        self.ok(self.http.post(f"/api/v1/nodes/{node_id}/uncomplete"))

    def delete(self, node_id: str) -> None:
        self.ok(self.http.delete(f"/api/v1/nodes/{node_id}"))

    def delete_mirror(self, node_id: str) -> httpx.Response:
        return self.http.delete(f"/api/v1/nodes/{node_id}/mirror")

    def targets(self) -> list[dict[str, Any]]:
        r = self.http.get("/api/v1/targets")
        assert r.status_code == 200, r.text
        return r.json()["targets"]

    def sandbox(self, name: str = "sandbox") -> str:
        """A fresh top-level node to work in."""
        return self.create("None", name, position="bottom")

    def admin(self, method: str, path: str, **kw: Any) -> httpx.Response:
        return self.anon.request(method, f"/_mock/{path}", **kw)

    def close(self) -> None:
        self.http.close()
        self.anon.close()


def rand_uuid() -> str:
    return str(uuid.uuid4())


@pytest.fixture(scope="session")
def _session_clock() -> FakeClock:
    return FakeClock()


@pytest.fixture(scope="session")
def _server(_session_clock: FakeClock) -> Iterator[str]:
    """One server per test session; each test resets it via the admin API."""
    with running_mock(clock=_session_clock) as url:
        yield url


@pytest.fixture
def clock(_session_clock: FakeClock) -> FakeClock:
    _session_clock.t = T0
    return _session_clock


@pytest.fixture
def wf(_server: str, clock: FakeClock) -> Iterator[WF]:
    client = WF(_server, clock)
    assert client.admin("POST", "reset").status_code == 200
    assert client.admin("POST", "clock", json={"offset": 0}).status_code == 200
    r = client.admin("POST", "config", json={"faithful_cycles": False, "export_min_interval": 60, "week_start": 0})
    assert r.status_code == 200
    try:
        yield client
    finally:
        client.close()


@pytest.fixture
def wf_faithful(wf: WF) -> WF:
    assert wf.admin("POST", "config", json={"faithful_cycles": True}).status_code == 200
    return wf
