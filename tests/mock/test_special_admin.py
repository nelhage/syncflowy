"""Root, special keys, shortcuts, targets, export, and the mock-only admin API.

The [O] assertions mirror production behaviour observed with read-only GETs
on 2026-10-12 (docs/workflowy/global-readonly.md). Materialisation via
create / move, the inbox name and user shortcuts are still inferred or
UNVERIFIED: those tests pin the mock's chosen behaviour so that changes are
deliberate; they are not evidence about production.
"""

from __future__ import annotations

import datetime
from pathlib import Path

import httpx
import pytest

from syncflowy.mock.testing import running_mock

from .conftest import T0, WF, FakeClock

NF = {"code": "not_found", "message": "Item not found"}
ISE = {"code": "internal_error", "message": "An internal server error occurred"}
CAL = "\U0001f4c6 Calendar"


def loc(key: str) -> dict[str, str]:
    return {"code": "not_found", "message": f"Location '{key}' not found"}


def test_mock_starts_empty_and_root_semantics(wf: WF) -> None:
    assert wf.list(None) == []
    assert wf.list("None") == []
    # [O] an empty parent_id is not the root
    r = wf.list_raw("")
    assert r.status_code == 404 and r.json() == loc("")
    for k in ("none", "null", "NONE"):
        assert wf.list_raw(k).json() == loc(k)
    assert wf.create_raw({"parent_id": "", "name": "x"}).json() == loc("")  # [I] same resolver
    a = wf.create("None", "top-level a")
    b = wf.create(None, "top-level b")  # omitted parent_id -> root [I]
    for x in (a, b):
        assert wf.get(x)["parent_id"] is None
    assert [n["id"] for n in wf.list(None)] == [a, b]
    assert [n["id"] for n in wf.list("None")] == [a, b]
    # [O] GET /nodes/None is not the root
    assert wf.get_raw("None").json() == NF
    # move to root with {}, null, "None"
    child = wf.create(a, "child")
    for body in ({}, {"parent_id": None}, {"parent_id": "None"}):
        wf.move(child, a)
        wf.ok(wf.move_raw(child, body))
        assert wf.get(child)["parent_id"] is None
    wf.move(child, a)
    assert wf.move_raw(child, {"parent_id": ""}).json() == loc("")


def test_inbox(wf: WF) -> None:
    # [O] never-materialised inbox: list 200 [], GET 404, target name null
    assert wf.list_raw("inbox").status_code == 200 and wf.list("inbox") == []
    assert wf.get_raw("inbox").json() == NF
    assert {"key": "inbox", "type": "system", "name": None} in wf.targets()
    nid = wf.create("inbox", "captured")
    inbox = wf.get(nid)["parent_id"]
    # UNVERIFIED: name "Inbox", top of the outline
    assert wf.get(inbox)["name"] == "Inbox" and wf.get(inbox)["parent_id"] is None
    assert [n["id"] for n in wf.list("inbox")] == [nid]
    assert wf.get("inbox")["id"] == inbox
    assert {"key": "inbox", "type": "system", "name": "Inbox"} in wf.targets()
    # stable: second create goes to the same node
    assert wf.get(wf.create("inbox", "again"))["parent_id"] == inbox


def test_calendar_keys(wf: WF) -> None:
    assert wf.list_raw("2026-07-05").json() == loc("2026-07-05")
    assert wf.get_raw("2026-07-05").json() == NF
    assert wf.get_raw("calendar").json() == NF
    nid = wf.create("2026-07-05", "meeting")
    day = wf.get(wf.get(nid)["parent_id"])
    month = wf.get(day["parent_id"])
    year = wf.get(month["parent_id"])
    cal = wf.get(year["parent_id"])
    # [O] names of production's calendar nodes
    assert (day["name"], month["name"], year["name"], cal["name"]) == (
        '<time startYear="2026" startMonth="7" startDay="5">Sun, Jul 5, 2026</time>',
        "07",
        "2026",
        CAL,
    )
    assert cal["parent_id"] is None
    for n in (day, month, year, cal):
        assert n["note"] is None and n["data"] == {"layoutMode": "bullets"} and n["completed"] is False
        assert n["id"][14] == "5"  # UUIDv5, like production
    assert [n["id"] for n in wf.list("2026-07-05")] == [nid]
    assert [n["id"] for n in wf.list("2026-07")] == [day["id"]]
    assert [n["id"] for n in wf.list("2026")] == [month["id"]]
    assert [n["id"] for n in wf.list("calendar")] == [year["id"]]
    # [O] GET resolves existing calendar keys; missing ones are Item not found
    for key, n in (("calendar", cal), ("2026", year), ("2026-07", month), ("2026-07-05", day)):
        assert wf.get(key) == n
    for key in ("2026-07-06", "2025", "2026-08", "2026-7", "Calendar", "CALENDAR"):
        assert wf.get_raw(key).json() == NF, key
    for key in ("2026-07-06", "2025", "2026-7", "2026-07-5", " 2026", "10000", "Calendar", "next-week"):
        assert wf.list_raw(key).json() == loc(key), key
    # today / tomorrow follow the mock clock (UTC)
    today = datetime.datetime.fromtimestamp(T0, datetime.UTC).date()
    t = wf.create("today", "t")
    assert wf.get(wf.get(t)["parent_id"])["name"].endswith(f"{today.day}, {today.year}</time>")
    tm = wf.create("tomorrow", "tm")
    assert wf.get(tm)["parent_id"] != wf.get(t)["parent_id"]
    assert wf.get("today")["id"] == wf.get(t)["parent_id"]


def test_calendar_chronological_priorities(wf: WF) -> None:
    # [I] fitted to the UI-made calendar: 5..9 -> 100..500, 16 -> 600, 15 -> 550
    for d in (5, 6, 7, 8, 9, 16, 15):
        wf.create(f"2026-07-{d:02d}", "x")
    days = {n["name"].split(">")[1].split("<")[0]: n["priority"] for n in wf.list("2026-07")}
    assert days == {
        "Sun, Jul 5, 2026": 100,
        "Mon, Jul 6, 2026": 200,
        "Tue, Jul 7, 2026": 300,
        "Wed, Jul 8, 2026": 400,
        "Thu, Jul 9, 2026": 500,
        "Thu, Jul 16, 2026": 600,
        "Wed, Jul 15, 2026": 550,
    }
    wf.create("2026-07-01", "x")
    assert wf.list("2026-07")[-1]["priority"] == 50
    # calendar itself goes to the bottom of the top level
    top = wf.list(None)
    assert top[0]["name"] == CAL


def test_date_key_errors(wf: WF) -> None:
    # [O] impossible dates: GET 404, list 500
    for key in ("2026-13", "2026-02-30", "0000", "2026-00", "2026-07-00", "2026-07-32", "2026-02-29"):
        assert wf.get_raw(key).json() == NF, key
        r = wf.list_raw(key)
        assert r.status_code == 500 and r.json() == ISE, key
    # [O] year 9999: GET 404, list 500
    for key in ("9999", "9999-12", "9999-01-01"):
        assert wf.get_raw(key).json() == NF, key
        assert wf.list_raw(key).status_code == 500, key
    # [O] the first week of year 1 (Sunday week start): 500 on both
    for key in ("0001", "0001-01", "0001-01-01", "0001-01-06"):
        assert wf.get_raw(key).status_code == 500, key
        assert wf.list_raw(key).status_code == 500, key
    for key in ("0001-01-07", "0001-02", "0002", "9998-12-31", "2028-02-29"):
        assert wf.get_raw(key).json() == NF, key
        assert wf.list_raw(key).json() == loc(key), key
    # [I] create shares the list resolver
    assert wf.create_raw({"parent_id": "2026-13", "name": "x"}).status_code == 500
    assert wf.list(None) == []  # nothing materialised


def test_week_start_config(wf: WF) -> None:
    # T0 is Mon 2026-10-12 UTC. Default week start Sunday -> Sun 2026-10-18.
    nid = wf.create("next_week", "x")
    assert "Sun, Oct 18, 2026" in wf.get(wf.get(nid)["parent_id"])["name"]
    wf.admin("POST", "config", json={"week_start": 0})
    nid = wf.create("next_week", "y")
    assert "Mon, Oct 19, 2026" in wf.get(wf.get(nid)["parent_id"])["name"]
    # with a Monday week start, 0001-01-01 (a Monday) no longer underflows
    assert wf.get_raw("0001-01-01").json() == NF


def test_targets_system(wf: WF) -> None:
    # [O] shape, order and labels on an account with no shortcuts / inbox
    assert wf.targets() == [
        {"key": "inbox", "type": "system", "name": None},
        {"key": "today", "type": "system", "name": "Today"},
        {"key": "tomorrow", "type": "system", "name": "Tomorrow"},
        {"key": "next_week", "type": "system", "name": "Next Week"},
        {"key": "calendar", "type": "system", "name": None},
    ]
    wf.create("today", "x")
    t = {d["key"]: d["name"] for d in wf.targets()}
    assert t["calendar"] == CAL and t["today"] == "Today"
    assert wf.http.get("/api/v1/targets/").status_code == 200  # trailing slash [O]


def test_shortcuts_admin_and_targets(wf: WF) -> None:
    sb = wf.sandbox("home page")
    r = wf.admin("POST", "shortcuts", json={"key": "home", "node_id": sb})
    assert r.status_code == 200
    assert wf.list("home") == []
    assert wf.get("home")["id"] == sb  # UNVERIFIED
    nid = wf.create("home", "via shortcut")
    assert wf.get(nid)["parent_id"] == sb
    targets = wf.targets()
    assert targets[0] == {"key": "home", "type": "shortcut", "name": "home page"}
    assert [t["key"] for t in targets[1:]] == ["inbox", "today", "tomorrow", "next_week", "calendar"]
    wf.delete(sb)
    assert wf.list_raw("home").json() == loc("home")
    assert all(t["key"] != "home" for t in wf.targets())
    assert wf.admin("DELETE", "shortcuts/home").status_code == 200
    assert wf.admin("DELETE", "shortcuts/home").status_code == 404


@pytest.mark.parametrize("key", ["inbox", "None", "today", "calendar", "2026"])
def test_special_keys_not_accepted_by_mutations(wf: WF, key: str) -> None:
    # UNVERIFIED (S7): mutating endpoints take exact UUIDs only.
    wf.create("inbox", "x")
    wf.create("today", "x")
    assert wf.update_raw(key, {}).json() == NF
    assert wf.http.post(f"/api/v1/nodes/{key}/complete").json() == NF
    assert wf.http.delete(f"/api/v1/nodes/{key}").json() == NF


def test_export(wf: WF) -> None:
    sb = wf.sandbox()
    m = wf.create(sb, "m")
    wf.http.post(f"/api/v1/nodes/{m}/complete")
    shell = wf.mirror(m, sb)["item_id"]
    r = wf.http.get("/api/v1/nodes-export")
    assert r.status_code == 200
    nodes = r.json()["nodes"]
    assert list(r.json()) == ["nodes"]
    assert [n["id"] for n in nodes] == [sb, m, shell]  # mock: creation order (production: unspecified)
    # [O] records are identical to GET's: no hidden mirror metadata, completed included
    for n in nodes:
        assert n == wf.get(n["id"])
    # [O] production did not rate-limit two calls 10 s apart: no limit by default
    assert wf.http.get("/api/v1/nodes-export").status_code == 200
    assert wf.http.get("/api/v1/nodes-export/").status_code == 200


def test_export_optional_rate_limit(wf: WF) -> None:
    wf.admin("POST", "config", json={"export_min_interval": 60})
    assert wf.http.get("/api/v1/nodes-export").status_code == 200
    r = wf.http.get("/api/v1/nodes-export")
    assert r.status_code == 429 and r.json()["code"] == "rate_limited"  # UNVERIFIED body
    wf.clock.advance(61)
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
