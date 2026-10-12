"""UNVERIFIED areas (root, special keys, shortcuts, targets, export) and the mock-only admin API.

These pin down the mock's *chosen* behaviour (data-model.md §11,
open-questions S1-S6) so that changes are deliberate; they are not evidence
about production.
"""

from __future__ import annotations

import datetime
from pathlib import Path

import httpx

from syncflowy.mock.testing import running_mock

from .conftest import T0, WF, FakeClock


def test_mock_starts_empty_and_root_semantics(wf: WF) -> None:
    assert wf.list(None) == []
    assert wf.list("None") == []
    assert wf.list("") == []
    a = wf.create("None", "top-level a")
    b = wf.create(None, "top-level b")  # omitted parent_id -> root (UNVERIFIED)
    c = wf.create("", "top-level c")
    for x in (a, b, c):
        assert wf.get(x)["parent_id"] is None
    assert [n["id"] for n in wf.list(None)] == [a, b, c]
    assert sorted(wf.prios("None").values()) == [25, 50, 100]
    # move to root with {}, null, "None"
    child = wf.create(a, "child")
    for body in ({}, {"parent_id": None}, {"parent_id": "None"}, {"parent_id": ""}):
        wf.move(child, a)
        wf.ok(wf.move_raw(child, body))
        assert wf.get(child)["parent_id"] is None


def test_inbox_materialised_on_create(wf: WF) -> None:
    r = wf.list_raw("inbox")
    assert r.status_code == 404 and r.json() == {"code": "not_found", "message": "Location 'inbox' not found"}
    t = wf.targets()
    assert {"key": "inbox", "type": "system", "name": None} in t
    nid = wf.create("inbox", "captured")
    inbox = wf.get(nid)["parent_id"]
    assert wf.get(inbox)["name"] == "Inbox" and wf.get(inbox)["parent_id"] is None
    assert [n["id"] for n in wf.list("inbox")] == [nid]
    assert {"key": "inbox", "type": "system", "name": "Inbox"} in wf.targets()
    # stable: second create goes to the same node
    assert wf.get(wf.create("inbox", "again"))["parent_id"] == inbox


def test_calendar_keys(wf: WF) -> None:
    assert wf.list_raw("2026-01-15").status_code == 404
    nid = wf.create("2026-01-15", "meeting")
    day = wf.get(wf.get(nid)["parent_id"])
    month = wf.get(day["parent_id"])
    year = wf.get(month["parent_id"])
    cal = wf.get(year["parent_id"])
    assert (day["name"], month["name"], year["name"], cal["name"]) == ("Thu, Jan 15, 2026", "January", "2026", "Calendar")
    assert cal["parent_id"] is None
    assert [n["id"] for n in wf.list("2026-01-15")] == [nid]
    assert [n["id"] for n in wf.list("2026-01")] == [day["id"]]
    assert [n["id"] for n in wf.list("2026")] == [month["id"]]
    assert [n["id"] for n in wf.list("calendar")] == [year["id"]]
    # today / tomorrow follow the mock clock (UTC)
    today = datetime.datetime.fromtimestamp(T0, datetime.UTC).date()
    t = wf.create("today", "t")
    assert wf.get(wf.get(t)["parent_id"])["name"].endswith(f"{today.day}, {today.year}")
    tm = wf.create("tomorrow", "tm")
    assert wf.get(tm)["parent_id"] != wf.get(t)["parent_id"]
    # invalid dates fall through to Location not found
    for bad in ("2026-13", "2026-02-30", "0000"):
        r = wf.create_raw({"parent_id": bad, "name": "x"})
        assert r.json() == {"code": "not_found", "message": f"Location '{bad}' not found"}


def test_next_week_monday_start(wf: WF) -> None:
    # T0 is Mon 2026-10-12 UTC -> next week starts Mon 2026-10-19
    nid = wf.create("next_week", "x")
    assert wf.get(wf.get(nid)["parent_id"])["name"] == "Mon, Oct 19, 2026"


def test_shortcuts_admin_and_targets(wf: WF) -> None:
    sb = wf.sandbox("home page")
    r = wf.admin("POST", "shortcuts", json={"key": "home", "node_id": sb})
    assert r.status_code == 200
    assert wf.list("home") == []
    nid = wf.create("home", "via shortcut")
    assert wf.get(nid)["parent_id"] == sb
    targets = wf.targets()
    assert targets[0] == {"key": "home", "type": "shortcut", "name": "home page"}
    assert [t["key"] for t in targets[1:]] == ["inbox", "calendar", "today", "tomorrow", "next_week"]
    wf.delete(sb)
    assert wf.list_raw("home").json() == {"code": "not_found", "message": "Location 'home' not found"}
    assert all(t["key"] != "home" for t in wf.targets())
    assert wf.admin("DELETE", "shortcuts/home").status_code == 200
    assert wf.admin("DELETE", "shortcuts/home").status_code == 404


def test_special_keys_not_accepted_as_path_ids(wf: WF) -> None:
    wf.create("inbox", "x")
    for key in ("inbox", "None", "today"):
        assert wf.get_raw(key).status_code == 404
        assert wf.update_raw(key, {}).status_code == 404


def test_export_and_rate_limit(wf: WF) -> None:
    sb = wf.sandbox()
    m = wf.create(sb, "m")
    shell = wf.mirror(m, sb)["item_id"]
    r = wf.http.get("/api/v1/nodes-export")
    assert r.status_code == 200
    nodes = r.json()["nodes"]
    assert [n["id"] for n in nodes] == [sb, m, shell]  # creation order, no hidden fields
    assert set(nodes[0]) == {"id", "name", "note", "parent_id", "priority", "completed", "data", "createdAt", "modifiedAt", "completedAt"}
    r = wf.http.get("/api/v1/nodes-export")
    assert r.status_code == 429 and r.json()["code"] == "rate_limited"
    wf.clock.advance(61)
    assert wf.http.get("/api/v1/nodes-export").status_code == 200
    wf.admin("POST", "config", json={"export_min_interval": 0})
    assert wf.http.get("/api/v1/nodes-export").status_code == 200


def test_targets_head(wf: WF) -> None:
    r = wf.http.head("/api/v1/targets")
    assert r.status_code == 200 and r.content == b""


def test_admin_dump_reset_clock(wf: WF) -> None:
    sb = wf.sandbox()
    o = wf.create(sb, "o")
    shell = wf.mirror(o, sb)["item_id"]
    d = wf.admin("GET", "dump").json()
    by_id = {n["id"]: n for n in d["nodes"]}
    assert by_id[shell]["_mirror_origin_id"] == o
    assert [n["_create_seq"] for n in d["nodes"]] == [1, 2, 3]
    assert d["config"]["faithful_cycles"] is False
    r = wf.admin("POST", "clock", json={"advance": 100}).json()
    assert int(r["now"]) == wf.clock.now + 100
    n = wf.get(wf.create(sb, "later"))
    assert n["createdAt"] == wf.clock.now + 100
    wf.admin("POST", "clock", json={"offset": 0})
    assert wf.admin("POST", "reset").status_code == 200
    assert wf.list(None) == []
    assert wf.admin("GET", "nope").status_code == 404


def test_persistence_across_restart(tmp_path: Path) -> None:
    db = tmp_path / "persist.sqlite3"
    clock = FakeClock()
    with running_mock(db_path=db, clock=clock) as url:
        c = httpx.Client(base_url=url, headers={"Authorization": "Bearer test-key"})
        a = c.post("/api/v1/nodes", json={"parent_id": "None", "name": "a"}).json()["item_id"]
        o = c.post("/api/v1/nodes", json={"parent_id": a, "name": "o"}).json()["item_id"]
        shell = c.post(f"/api/v1/nodes/{o}/mirror", json={"parent_id": a}).json()["item_id"]
    with running_mock(db_path=db, clock=clock) as url:
        c = httpx.Client(base_url=url, headers={"Authorization": "Bearer test-key"})
        assert c.get(f"/api/v1/nodes/{a}").json()["node"]["name"] == "a"
        # hidden mirror link persisted
        assert c.delete(f"/api/v1/nodes/{shell}/mirror").status_code == 200
        # create_seq keeps increasing: new node lists after old ones
        b = c.post("/api/v1/nodes", json={"parent_id": a, "name": "b"}).json()["item_id"]
        assert [n["id"] for n in c.get("/api/v1/nodes", params={"parent_id": a}).json()["nodes"]] == [o, b]


def test_cli_parse_keys(monkeypatch) -> None:
    from syncflowy.mock.cli import parse_keys

    monkeypatch.delenv("SYNCFLOWY_MOCK_API_KEY", raising=False)
    assert parse_keys(None) == ["test-key"]
    assert parse_keys(["a,b", "c"]) == ["a", "b", "c"]
    monkeypatch.setenv("SYNCFLOWY_MOCK_API_KEY", "x,y")
    assert parse_keys(None) == ["x", "y"]
