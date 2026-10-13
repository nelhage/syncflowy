"""Soft delete, the /_mock/ui/* UI-simulation routes, and xml_check.

Scenarios mirror docs/workflowy/ui-findings.md (A1–A4, B1/B2/B4, C1, D1/D2,
E1/E3, G2, H2–H4, H6). Assertions on [O] behaviour reproduce observed
production data; the rest pin UNVERIFIED choices of the mock.
"""

from __future__ import annotations

from pathlib import Path
from typing import Any

import httpx

from syncflowy.mock.testing import running_mock

from .conftest import WF, FakeClock

NF = {"code": "not_found", "message": "Item not found"}


def ui(wf: WF, action: str, expect: int = 200, **body: Any) -> dict[str, Any]:
    r = wf.admin("POST", f"ui/{action}", json=body)
    assert r.status_code == expect, r.text
    return r.json()


def mod(wf: WF, node_id: str) -> int:
    return wf.get(node_id)["modifiedAt"]


def tick(wf: WF) -> int:
    wf.clock.advance(5)
    return wf.clock.now


def kids(wf: WF, parent: str) -> list[tuple[str, int]]:
    return [(n["name"], n["priority"]) for n in wf.children(parent)]


# ------------------------------------------------------------------ soft delete


def test_api_delete_is_soft_and_restorable(wf: WF) -> None:
    """B2: an API-deleted subtree comes back from the Trash with the same ids."""
    s = wf.sandbox()
    a = wf.create(s, "a", position="bottom")
    b = wf.create(s, "b", position="bottom")
    b1 = wf.create(b, "b1")
    c = wf.create(s, "c", position="bottom")
    before = {i: wf.get(i) for i in (b, b1)}
    s_mod = mod(wf, s)
    t = tick(wf)
    wf.delete(b)
    # Through the API it is exactly a hard delete.
    for i in (b, b1):
        assert wf.get_raw(i).json() == NF
        assert wf.get_raw(i.replace("-", "")[-12:]).json() == NF
    assert [n["id"] for n in wf.list(s)] == [a, c]
    assert b not in {n["id"] for n in wf.http.get("/api/v1/nodes-export").json()["nodes"]}
    assert wf.http.post(f"/api/v1/nodes/{b}", json={"name": "x"}).json() == NF
    trash = wf.admin("GET", "ui/trash").json()["trash"]
    assert [(e["id"], e["count"], e["deleted_at"]) for e in trash] == [(b, 2, t)]
    t2 = tick(wf)
    out = ui(wf, "restore", id=b)
    assert out["restored"] == [b, b1]
    for i in (b, b1):
        after = wf.get(i)
        assert after["modifiedAt"] == t2  # every restored node bumped [O]
        assert {k: v for k, v in after.items() if k != "modifiedAt"} == {
            k: v for k, v in before[i].items() if k != "modifiedAt"
        }
    assert mod(wf, s) == s_mod  # parent not bumped [O]
    assert [n["id"] for n in wf.list(s)] == [a, b, c]  # creation-order position [O]
    assert wf.admin("GET", "ui/trash").json()["trash"] == []


def test_undo_last_and_restore_errors(wf: WF) -> None:
    s = wf.sandbox()
    a = wf.create(s, "a")
    b = wf.create(s, "b")
    a1 = wf.create(a, "a1")
    ui(wf, "delete", id=a)
    ui(wf, "delete", id=b)
    assert ui(wf, "restore", last=True)["restored"] == [b]
    assert ui(wf, "restore", 400, id=a1)["code"] == "bad_ui_request"  # not a delete root
    assert ui(wf, "restore", 404, id=b)["code"] == "not_found"  # no longer in trash
    assert ui(wf, "restore", last=True)["restored"] == [a, a1]
    assert ui(wf, "restore", 404, last=True)["code"] == "not_found"


def test_restore_origin_reattaches_orphan_shells(wf: WF) -> None:
    s = wf.sandbox()
    o = wf.create(s, "origin")
    d = wf.create(s, "d")
    shell = wf.mirror(o, d)["item_id"]
    wf.delete(o)  # API delete: the shell becomes an orphan (§10.4)
    assert wf.mirror_raw(shell, {"parent_id": s}).json() == NF  # orphan source
    ui(wf, "restore", id=o)
    r = wf.mirror(shell, s)
    assert r["origin_id"] == o


def test_delete_mirror_is_soft(wf: WF) -> None:
    s = wf.sandbox()
    o = wf.create(s, "o")
    shell = wf.mirror(o, s)["item_id"]
    wf.ok(wf.delete_mirror(shell))
    assert wf.get_raw(shell).json() == NF
    ui(wf, "restore", id=shell)
    assert wf.get(shell)["name"] == ""


def test_restore_when_parent_gone_goes_top_level(wf: WF) -> None:
    s = wf.sandbox()
    p = wf.create(s, "p")
    c = wf.create(p, "c")
    wf.delete(c)
    wf.delete(p)
    ui(wf, "restore", id=c)  # UNVERIFIED choice
    assert wf.get(c)["parent_id"] is None


def test_reset_clears_trash_and_dump_shows_it(wf: WF) -> None:
    s = wf.sandbox()
    a = wf.create(s, "a")
    wf.delete(a)
    d = wf.admin("GET", "dump").json()
    assert [(n["id"], n["_op_root"]) for n in d["trash"]] == [(a, True)]
    wf.admin("POST", "reset")
    assert wf.admin("GET", "dump").json()["trash"] == []


def test_v1_database_is_migrated(tmp_path: Path, clock: FakeClock) -> None:
    import sqlite3

    db = tmp_path / "old.sqlite3"
    with running_mock(db_path=db, clock=clock) as url:
        h = {"Authorization": "Bearer test-key"}
        nid = httpx.post(f"{url}/api/v1/nodes", json={"parent_id": "None", "name": "keep"}, headers=h).json()[
            "item_id"
        ]
    con = sqlite3.connect(db)
    con.execute("DROP TABLE trash")
    con.execute("UPDATE meta SET value = '1' WHERE key = 'schema_version'")
    con.execute("DELETE FROM meta WHERE key = 'next_delete_op'")
    con.commit()
    con.close()
    with running_mock(db_path=db, clock=clock) as url:
        h = {"Authorization": "Bearer test-key"}
        assert httpx.get(f"{url}/api/v1/nodes/{nid}", headers=h).json()["node"]["name"] == "keep"
        assert httpx.delete(f"{url}/api/v1/nodes/{nid}", headers=h).status_code == 200
        assert httpx.post(f"{url}/_mock/ui/restore", json={"id": nid}).json()["restored"] == [nid]
    con = sqlite3.connect(db)
    assert con.execute("SELECT value FROM meta WHERE key = 'schema_version'").fetchone() == ("2",)


# -------------------------------------------------------------------- placement


def test_a1_drag_midpoint_and_end(wf: WF) -> None:
    s = wf.sandbox()
    ids = {n: wf.create(s, n, position="bottom") for n in "abcd"}  # 100..400
    s0 = mod(wf, s)
    t = tick(wf)
    out = ui(wf, "move", id=ids["d"], after_id=ids["a"])
    assert out["priority"] == 150
    assert kids(wf, s) == [("a", 100), ("d", 150), ("b", 200), ("c", 300)]
    assert mod(wf, ids["d"]) == t and mod(wf, s) == t  # node + parent [O]
    assert all(mod(wf, ids[n]) < t for n in "abc")
    assert s0 < t
    tick(wf)
    ui(wf, "move", id=ids["a"], parent_id=s, position="bottom")
    assert kids(wf, s) == [("d", 150), ("b", 200), ("c", 300), ("a", 400)]


def test_a2_enter_creates_midpoint_without_parent_bump(wf: WF) -> None:
    s = wf.sandbox()
    ids = {n: wf.create(s, n, position="bottom") for n in "abcd"}
    s0 = mod(wf, s)
    tick(wf)
    ui(wf, "create", after_id=ids["b"], text="new-below-b")
    ui(wf, "create", before_id=ids["c"], text="new-above-c")
    new_last = ui(wf, "create", after_id=ids["d"], text="new-last")["id"]
    assert kids(wf, s) == [
        ("a", 100), ("b", 200), ("new-below-b", 250), ("new-above-c", 275), ("c", 300), ("d", 400), ("new-last", 500)
    ]
    assert mod(wf, s) == s0
    n = wf.get(new_last)
    assert n["note"] is None and n["data"] == {"layoutMode": "bullets"} and n["createdAt"] == n["modifiedAt"]


def test_a3_a4_indent_outdent(wf: WF) -> None:
    s = wf.sandbox()
    a = wf.create(s, "a", position="bottom")
    a1 = wf.create(a, "a1", position="bottom")
    a2 = wf.create(a, "a2", position="bottom")
    b = wf.create(s, "b", position="bottom")
    wf.create(s, "c", position="bottom")
    s0 = mod(wf, s)
    t = tick(wf)
    ui(wf, "move", id=b, parent_id=a)  # Tab: last child of a
    assert wf.get(b)["priority"] == 300 and mod(wf, a) == t and mod(wf, s) == s0
    t = tick(wf)
    ui(wf, "move", id=a2, after_id=a)  # Shift+Tab: between a (100) and c (300)
    assert wf.get(a2)["priority"] == 200 and wf.get(a2)["parent_id"] == s
    assert mod(wf, s) == t and mod(wf, a) < t
    ui(wf, "move", id=a1, after_id=a)  # Shift+Tab of the first child [O: A4 step 2]
    assert wf.get(a1)["priority"] == 150
    ui(wf, "move", id=a2, index=0, parent_id=s)
    assert wf.get(a2)["priority"] == 50  # midpoint(0, 100)


def test_no_integer_gap_renumbers(wf: WF) -> None:
    s = wf.sandbox()
    x = wf.create(s, "x", position="bottom")
    y = wf.create(s, "y", position="bottom")
    ui(wf, "move", id=y, before_id=x)  # 50
    z = ui(wf, "create", before_id=y, text="z")["id"]  # 25
    for _ in range(5):
        z = ui(wf, "create", before_id=z, text="z")["id"]
    # 12, 6, 3, 1, then no gap -> renumber (UNVERIFIED)
    prios = [n["priority"] for n in wf.children(s)]
    assert prios == sorted(prios) and len(set(prios)) == len(prios)
    assert [n["name"] for n in wf.children(s)][-2:] == ["y", "x"]


def test_placement_errors(wf: WF) -> None:
    s = wf.sandbox()
    a = wf.create(s, "a")
    other = wf.sandbox("other")
    assert ui(wf, "create", 400, parent_id=other, after_id=a, text="x")["code"] == "bad_ui_request"
    assert ui(wf, "create", 400, text="no parent")["code"] == "bad_ui_request"
    assert ui(wf, "create", 400, parent_id=s, index=5, text="x")["code"] == "bad_ui_request"
    assert ui(wf, "create", 404, parent_id="00000000-0000-4000-8000-000000000000", text="x")["code"] == "not_found"
    assert ui(wf, "move", 400, id=s, parent_id=a)["code"] == "bad_ui_request"  # into own descendant
    assert wf.admin("POST", "ui/nope", json={}).status_code == 404
    assert wf.admin("POST", "ui/create", json=[1]).status_code == 400


def test_top_level_placement(wf: WF) -> None:
    a = ui(wf, "create", parent_id=None, text="first")["id"]
    assert wf.get(a)["priority"] == 100 and wf.get(a)["parent_id"] is None
    b = ui(wf, "create", parent_id="None", position="top", text="top")["id"]
    assert wf.get(b)["priority"] == 50


# ------------------------------------------------------------------ cut / copy


def test_b4_cut_paste_moves_and_bumps_descendants(wf: WF) -> None:
    s = wf.sandbox()
    p1 = wf.create(s, "P1", position="bottom")
    x = wf.create(p1, "x")
    x1 = wf.create(x, "x1")
    p2 = wf.create(s, "P2", position="bottom")
    t = tick(wf)
    ui(wf, "move", id=x, parent_id=p2, method="cut_paste")
    assert wf.get(x)["parent_id"] == p2 and wf.get(x1)["parent_id"] == x
    assert mod(wf, x) == t and mod(wf, x1) == t and mod(wf, p2) == t and mod(wf, p1) < t
    # drag does not bump descendants
    t2 = tick(wf)
    ui(wf, "move", id=x, parent_id=p1)
    assert mod(wf, x1) == t and mod(wf, x) == t2


def test_b4_copy_paste_signature(wf: WF) -> None:
    s = wf.sandbox()
    p1 = wf.create(s, "P1", position="bottom")
    x = wf.create(p1, "x")
    x1 = wf.create(x, "x1", position="bottom")
    x2 = wf.create(x, "x2", position="bottom", note="kept")
    p2 = wf.create(s, "P2", position="bottom")
    p2_mod = mod(wf, p2)
    t = tick(wf)
    out = ui(wf, "copy", id=x, parent_id=p2)
    nx = out["id"]
    assert set(out["ids"]) == {x, x1, x2} and nx not in (x, x1, x2)
    root = wf.get(nx)
    assert root["note"] == "" and root["priority"] == 100 and root["createdAt"] == root["modifiedAt"] == t
    copies = wf.children(nx)
    assert [(n["name"], n["priority"], n["note"]) for n in copies] == [("x1", 0, ""), ("x2", 1, "kept")]
    assert mod(wf, p2) == p2_mod  # a paste is a create
    assert [n["id"] for n in wf.list(x)] == [x1, x2]  # originals untouched


# ------------------------------------------------------------------- text edits


def test_e1_text_is_escaped_and_html_canonicalised(wf: WF) -> None:
    s = wf.sandbox()
    a = ui(wf, "create", parent_id=s, text="a < b > c & d \"q\" 'q'")["id"]
    assert wf.get(a)["name"] == "a &lt; b &gt; c &amp; d &quot;q&quot; 'q'"
    b = ui(wf, "create", parent_id=s, name="plain <strong>WORD</strong> <mark>m</mark>")["id"]
    assert wf.get(b)["name"] == "plain <b>WORD</b> m"
    assert ui(wf, "create", 400, parent_id=s, name="x", text="y")["code"] == "bad_ui_request"


def test_f_edit_canonicalises_and_destroys_malformed(wf: WF) -> None:
    s = wf.sandbox()
    good = wf.create(s, "g")
    wf.update(good, name="F09 <B>upper</B>")
    badn = wf.create(s, "b")
    wf.update(badn, name="F13 a < b & c > d", note="n")
    t = tick(wf)
    ui(wf, "edit", id=good, append_text=" z")
    assert wf.get(good)["name"] == "F09 <b>upper z</b>" and mod(wf, good) == t
    ui(wf, "edit", id=badn, append_text=" z")
    assert wf.get(badn)["name"] == " z"
    assert wf.get(badn)["note"] == "n"  # note untouched by a name edit
    ui(wf, "edit", id=badn, note_text="a & b")
    assert wf.get(badn)["note"] == "a &amp; b"
    ui(wf, "edit", id=badn, note=None)
    assert wf.get(badn)["note"] == ""
    assert ui(wf, "edit", 400, id=badn)["code"] == "bad_ui_request"


# ----------------------------------------------------------- completion, layout


def test_d1_d2_complete_no_cascade(wf: WF) -> None:
    s = wf.sandbox()
    p = wf.create(s, "P")
    c = wf.create(p, "c")
    t = tick(wf)
    ui(wf, "complete", id=p)
    n = wf.get(p)
    assert n["completed"] is True and n["completedAt"] == n["modifiedAt"] == t
    assert wf.get(c)["completed"] is False and mod(wf, c) < t and mod(wf, s) < t
    t2 = tick(wf)
    ui(wf, "complete", id=p)
    assert wf.get(p)["completedAt"] == t2  # re-stamped [O: D1/D3]
    ui(wf, "uncomplete", id=p)
    n = wf.get(p)
    assert n["completed"] is False and n["completedAt"] is None


def test_g2_layouts_and_side_effects(wf: WF) -> None:
    s = wf.sandbox()
    nodes = {k: wf.create(s, k, position="bottom") for k in ("h1", "numbered", "board", "table", "divider")}
    for k, i in nodes.items():
        out = ui(wf, "set_layout", id=i, layoutMode=k)
        assert wf.get(i)["data"] == {"layoutMode": k}
        if k not in ("board", "table"):
            assert out["created"] == []
    board_kids = wf.list(nodes["board"])
    assert [(n["name"], n["priority"], n["note"]) for n in board_kids] == [("", 100, None)]
    table = wf.children(nodes["table"])
    assert [(n["name"], n["priority"]) for n in table] == [("Headers", 100), ("Row", 200)]
    for row in table:
        assert [(n["name"], n["priority"], n["note"]) for n in wf.children(row["id"])] == [("", 0, None), ("", 1, None)]
    assert wf.get(nodes["divider"])["note"] == ""
    # Board on a node with children: no new child (UNVERIFIED)
    assert ui(wf, "set_layout", id=nodes["table"], layoutMode="board")["created"] == []


# ---------------------------------------------------------------------- mirrors


def test_h2_h3_h4_ui_mirror_semantics(wf: WF) -> None:
    s = wf.sandbox()
    o = wf.create(s, "O", note="O note", layoutMode="todo", position="bottom")
    o1 = wf.create(o, "o1", position="bottom")
    wf.create(o, "o2", position="bottom")
    d = wf.create(s, "D", position="bottom")
    loose = wf.create(s, "L", position="bottom")
    d_mod = mod(wf, d)
    t = tick(wf)
    out = ui(wf, "mirror", id=o, parent_id=d)
    shell = wf.get(out["id"])
    assert out["origin_id"] == o
    assert (shell["name"], shell["note"], shell["data"]["layoutMode"], shell["priority"]) == ("", None, "bullets", 100)
    assert mod(wf, o) == t and mod(wf, d) == d_mod  # [O: H2]
    # H3: edits through the mirror go to the origin
    tick(wf)
    assert ui(wf, "edit", id=shell["id"], append_text=" name-in-mirror")["id"] == o
    assert wf.get(o)["name"] == "O name-in-mirror" and wf.get(shell["id"])["name"] == ""
    # H4: create / drag into the mirror land under the origin
    n = ui(wf, "create", parent_id=shell["id"], text="new-via-mirror")["id"]
    assert wf.get(n)["parent_id"] == o and wf.get(n)["priority"] == 300
    t = tick(wf)
    ui(wf, "move", id=loose, parent_id=shell["id"])
    assert wf.get(loose)["parent_id"] == o and wf.get(loose)["priority"] == 400 and mod(wf, o) == t
    assert wf.list(shell["id"]) == []
    ui(wf, "edit", id=o1, text="o1-renamed")
    assert wf.get(o1)["name"] == "o1-renamed"


def test_h6_delete_origin_promotes_newest_shell(wf: WF) -> None:
    s = wf.sandbox()
    o2 = wf.create(s, "O2 origin", position="bottom")
    o2c = wf.create(o2, "o2 child")
    e1 = wf.create(s, "E1", position="bottom")
    e2 = wf.create(s, "E2", position="bottom")
    m1 = wf.mirror(o2, e1)["item_id"]
    m2 = wf.mirror(o2, e2)["item_id"]
    m2child = wf.create(m2, "API child of shell M2")
    e1_mod = mod(wf, e1)
    t = tick(wf)
    out = ui(wf, "delete", id=o2)
    assert out["promoted"] == {"origin_id": o2, "replaced_shell_id": m2, "parent_id": e2}
    n = wf.get(o2)
    assert n["parent_id"] == e2 and n["priority"] == 100 and n["modifiedAt"] == t
    assert [c["id"] for c in wf.list(o2)] == [o2c]
    for gone in (m2, m2child):
        assert wf.get_raw(gone).json() == NF
    assert wf.get(m1)["parent_id"] == e1 and mod(wf, e1) == e1_mod
    assert mod(wf, e2) == t and mod(wf, s) == t  # new and old parent bumped [O]
    assert wf.mirror(m1, s)["origin_id"] == o2  # M1 still mirrors O2
    # Deleting a node without shells is a plain soft delete.
    out = ui(wf, "delete", id=e1)
    assert out["promoted"] is None and set(out["deleted"]) == {e1, m1}


def test_ui_delete_bumps_nothing(wf: WF) -> None:
    s = wf.sandbox()
    a = wf.create(s, "a")
    b = wf.create(s, "b")
    s0 = mod(wf, s)
    tick(wf)
    ui(wf, "delete", id=b)
    assert mod(wf, s) == s0 and wf.get_raw(b).json() == NF and wf.get(a)


# -------------------------------------------------------------------- xml_check


def test_xml_check_warn_records_and_responses_unchanged(wf: WF) -> None:
    s = wf.sandbox()
    r = wf.create_raw({"parent_id": s, "name": "AT&amp;T"})  # create decodes -> stored "AT&T"
    assert r.status_code == 200 and set(r.json()) == {"item_id"}
    nid = r.json()["item_id"]
    assert wf.get(nid)["name"] == "AT&T"
    wf.ok(wf.update_raw(nid, {"name": "a<br>b", "note": "<b>ok</b>"}))
    wf.ok(wf.update_raw(nid, {"name": "fine &amp; dandy"}))
    w = wf.admin("GET", "warnings").json()["warnings"]
    assert [(x["op"], x["node_id"], x["field"], x["value"]) for x in w] == [
        ("create", nid, "name", "AT&T"),
        ("update", nid, "name", "a<br>b"),
    ]
    wf.admin("DELETE", "warnings")
    assert wf.admin("GET", "warnings").json()["warnings"] == []
    wf.create(s, "x & y")
    wf.admin("POST", "reset")
    assert wf.admin("GET", "warnings").json()["warnings"] == []


def test_xml_check_reject_and_off(wf: WF) -> None:
    s = wf.sandbox()
    assert wf.admin("POST", "config", json={"xml_check": "reject"}).json()["xml_check"] == "reject"
    r = wf.create_raw({"parent_id": s, "name": "fine", "note": "a < b"})
    assert r.status_code == 400 and r.json()["code"] == "mock_malformed_xml"
    assert wf.list(s) == []  # nothing created
    nid = wf.create(s, "ok")
    before = wf.get(nid)
    tick(wf)
    r = wf.update_raw(nid, {"name": "<b>x"})
    assert r.status_code == 400 and wf.get(nid) == before  # not even modifiedAt
    # Production checks still come first: validation and 404 win over xml_check.
    assert wf.update_raw("00000000-0000-4000-8000-000000000000", {"name": "<b>"}).json() == NF
    assert wf.create_raw({"parent_id": s, "name": 5}).status_code == 400
    assert "errors" in wf.create_raw({"parent_id": s, "name": 5}).json()
    wf.admin("POST", "config", json={"xml_check": "off"})
    wf.update(nid, name="<b>x")
    assert wf.admin("GET", "warnings").json()["warnings"] == []
    assert wf.admin("POST", "config", json={"xml_check": "loud"}).status_code == 400
