"""Update, complete and uncomplete (data-model.md §4.4, §4.5, §6.1)."""

from __future__ import annotations

from .conftest import WF


def test_partial_update_and_null_means_unchanged(wf: WF) -> None:
    sb = wf.sandbox()
    nid = wf.create(sb, "orig", note="n0", layoutMode="todo")
    wf.update(nid, name="new")
    n = wf.get(nid)
    assert (n["name"], n["note"], n["data"]["layoutMode"]) == ("new", "n0", "todo")
    wf.update(nid, name=None, note=None, layoutMode=None)
    n = wf.get(nid)
    assert (n["name"], n["note"], n["data"]["layoutMode"]) == ("new", "n0", "todo")


def test_note_cannot_return_to_null(wf: WF) -> None:
    sb = wf.sandbox()
    nid = wf.create(sb, "x")
    wf.update(nid, note="")
    assert wf.get(nid)["note"] == ""
    wf.update(nid, note=None)
    assert wf.get(nid)["note"] == ""


def test_update_is_verbatim_minus_c0(wf: WF) -> None:
    sb = wf.sandbox()
    nid = wf.create(sb, "x")
    raw = "  **b** &amp; <script>x</script>\r\n# h\n- [x] t [2026-01-15] \t"
    wf.update(nid, name=raw, note=raw)
    n = wf.get(nid)
    assert n["name"] == raw and n["note"] == raw
    assert n["completed"] is False and n["data"]["layoutMode"] == "bullets"
    wf.update(nid, name="a\x00\x01\x08\x0b\x0c\x0e\x1fb\t\n\r\x7f\x85", note="c\x0bd")
    n = wf.get(nid)
    assert n["name"] == "ab\t\n\r\x7f\x85" and n["note"] == "cd"


def test_update_layout_any_string(wf: WF) -> None:
    sb = wf.sandbox()
    nid = wf.create(sb, "x")
    for lm in ("nonsense", "p", "H1", ""):
        wf.update(nid, layoutMode=lm)
        assert wf.get(nid)["data"]["layoutMode"] == lm


def test_update_ignores_structural_fields(wf: WF) -> None:
    sb = wf.sandbox()
    other = wf.sandbox("other")
    nid = wf.create(sb, "x")
    before = wf.get(nid)
    wf.update(nid, parent_id=other, priority=5, position="bottom", completed=True, completedAt=1, id="zz", data={"layoutMode": "h1"})
    after = wf.get(nid)
    for k in ("parent_id", "priority", "completed", "completedAt", "id", "data", "createdAt"):
        assert after[k] == before[k]


def test_update_always_bumps_modified(wf: WF) -> None:
    sb = wf.sandbox()
    nid = wf.create(sb, "x")
    t0 = wf.clock.now
    for body in ({}, {"name": None}, {"name": "x"}, {"unknown": 1}):
        wf.clock.advance(2)
        wf.ok(wf.update_raw(nid, body))
        n = wf.get(nid)
        assert n["modifiedAt"] == wf.clock.now and n["createdAt"] == t0


def test_update_fixed_point_on_create_output(wf: WF) -> None:
    sb = wf.sandbox()
    nid = wf.create(sb, "**b** [2026-01-15] &amp;")
    name = wf.get(nid)["name"]
    wf.update(nid, name=name)
    assert wf.get(nid)["name"] == name


def test_complete_uncomplete(wf: WF) -> None:
    sb = wf.sandbox()
    nid = wf.create(sb, "x", layoutMode="h2")
    child = wf.create(nid, "child")
    wf.clock.advance(3)
    wf.complete(nid)
    n = wf.get(nid)
    assert n["completed"] is True and n["completedAt"] == wf.clock.now == n["modifiedAt"]
    assert n["data"]["layoutMode"] == "h2" and n["priority"] == 100
    c = wf.get(child)
    assert c["completed"] is False and c["modifiedAt"] < wf.clock.now  # no cascade
    assert wf.get(sb)["modifiedAt"] < wf.clock.now  # parent untouched
    # re-completing re-stamps completedAt
    wf.clock.advance(3)
    wf.complete(nid)
    assert wf.get(nid)["completedAt"] == wf.clock.now
    wf.clock.advance(3)
    wf.uncomplete(nid)
    n = wf.get(nid)
    assert n["completed"] is False and n["completedAt"] is None and n["modifiedAt"] == wf.clock.now
    # uncomplete on a never-completed node still bumps
    wf.clock.advance(3)
    wf.uncomplete(child)
    assert wf.get(child)["modifiedAt"] == wf.clock.now


def test_completed_nodes_always_listed(wf: WF) -> None:
    sb = wf.sandbox()
    a = wf.create(sb, "a")
    wf.complete(a)
    assert [n["id"] for n in wf.list(sb)] == [a]
