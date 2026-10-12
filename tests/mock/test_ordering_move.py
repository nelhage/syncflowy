"""Priority algorithm, list order, move semantics (data-model.md §4.2, §4.7, §7, §8)."""

from __future__ import annotations

import threading

import pytest

from .conftest import WF


def test_top_halving_then_renumber(wf: WF) -> None:
    sb = wf.sandbox()
    ids = []
    for i in range(9):
        ids.append(wf.create(sb, f"top-{i}"))
    # 100, 50, 25, 12, 6, 3, 1, 0, then renumber
    p = wf.prios(sb)
    assert p["top-8"] == 100
    assert [p[f"top-{i}"] for i in range(8)] == [900, 800, 700, 600, 500, 400, 300, 200]
    wf.create(sb, "top-9")
    assert wf.prios(sb)["top-9"] == 50


def test_halving_sequence(wf: WF) -> None:
    sb = wf.sandbox()
    seen = []
    for i in range(8):
        nid = wf.create(sb, f"t{i}", position="top")
        seen.append(wf.get(nid)["priority"])
    assert seen == [100, 50, 25, 12, 6, 3, 1, 0]


def test_renumber_does_not_bump_siblings(wf: WF) -> None:
    sb = wf.sandbox()
    ids = [wf.create(sb, f"n{i}") for i in range(8)]
    stamps = {i: wf.get(i)["modifiedAt"] for i in ids}
    wf.clock.advance(10)
    wf.create(sb, "renumberer")
    for i in ids:
        assert wf.get(i)["modifiedAt"] == stamps[i]


@pytest.mark.parametrize(
    "existing,expected",
    [([100, 200, 300], 400), ([12, 25, 50, 100], 200), ([50], 200), ([150], 300), ([250], 400)],
)
def test_bottom_formula_via_api(wf: WF, existing: list[int], expected: int) -> None:
    """Construct sibling sets with the API itself, then add at bottom."""
    sb = wf.sandbox()
    # build priorities with top-halving/bottom inserts and deletes
    if existing == [100, 200, 300]:
        for i in range(3):
            wf.create(sb, f"b{i}", position="bottom")
    elif existing == [12, 25, 50, 100]:
        for i in range(4):
            wf.create(sb, f"t{i}")
    elif existing == [50]:
        a = wf.create(sb, "a")
        wf.create(sb, "b")
        wf.delete(a)
    elif existing in ([150], [250]):
        target = existing[0] * 2
        for i in range(target // 100):
            wf.create(sb, f"b{i}", position="bottom")
        keep = wf.create(sb, "keep")  # top -> 50
        for n in wf.list(sb):
            if n["id"] != keep and n["priority"] != target:
                wf.delete(n["id"])
        wf.delete(keep)
        # now one node at `target`; halve via a top insert and delete the original
        half = wf.create(sb, "half")
        for n in wf.list(sb):
            if n["id"] != half:
                wf.delete(n["id"])
    assert sorted(wf.prios(sb).values()) == existing
    nid = wf.create(sb, "new", position="bottom")
    assert wf.get(nid)["priority"] == expected


def test_bottom_after_multiline_children(wf: WF) -> None:
    sb = wf.sandbox()
    nid = wf.create(sb, "p\n\nC1\n\nC2")
    assert [c["priority"] for c in wf.children(nid)] == [0, 1]
    b = wf.create(nid, "bottom", position="bottom")
    assert wf.get(b)["priority"] == 200
    t = wf.create(nid, "top")
    # min 0 -> renumber everyone: new 100, others 200, 300, 400
    assert wf.get(t)["priority"] == 100
    assert sorted(wf.prios(nid).values()) == [100, 200, 300, 400]


def test_empty_parent_gets_100_any_position(wf: WF) -> None:
    for pos in ("top", "bottom", None):
        sb = wf.sandbox()
        assert wf.get(wf.create(sb, "x", position=pos))["priority"] == 100


def test_delete_does_not_renumber(wf: WF) -> None:
    sb = wf.sandbox()
    ids = [wf.create(sb, f"b{i}", position="bottom") for i in range(3)]
    wf.delete(ids[0])
    assert sorted(wf.prios(sb).values()) == [200, 300]


def test_list_is_creation_order_not_priority(wf: WF) -> None:
    sb = wf.sandbox()
    names = []
    for i, pos in enumerate(["bottom", "top", "bottom", "top", "top", "bottom"]):
        wf.create(sb, f"n{i}", position=pos)
        names.append(f"n{i}")
    assert [n["name"] for n in wf.list(sb)] == names
    # rename / complete / same-parent moves do not change list order
    first = wf.list(sb)[0]["id"]
    wf.update(first, name="n0")
    wf.complete(first)
    wf.move(first, sb, "bottom")
    assert [n["name"] for n in wf.list(sb)] == names


def test_list_order_with_moved_in_nodes(wf: WF) -> None:
    a, b = wf.sandbox("A"), wf.sandbox("B")
    early = wf.create(b, "early")  # created before a's children
    wf.create(a, "c1")
    wf.create(a, "c2")
    late = wf.create(b, "late")
    wf.move(early, a, "bottom")
    wf.move(late, a, "top")
    assert [n["name"] for n in wf.list(a)] == ["early", "c1", "c2", "late"]


def test_move_basics(wf: WF) -> None:
    a, b = wf.sandbox("A"), wf.sandbox("B")
    x = wf.create(a, "x")
    x1 = wf.create(x, "x1")
    x1a = wf.create(x1, "x1a")
    created = wf.get(x)["createdAt"]
    wf.create(b, "b1", position="bottom")
    wf.create(b, "b2", position="bottom")
    wf.move(x, b)  # default top
    n = wf.get(x)
    assert n["parent_id"] == b and n["priority"] == 50 and n["createdAt"] == created
    assert wf.get(x1)["parent_id"] == x and wf.get(x1a)["parent_id"] == x1
    assert [c["name"] for c in wf.list(a)] == []
    wf.move(x, b, "bottom")
    assert wf.get(x)["priority"] == 300


def test_same_parent_move_recomputes_excluding_self(wf: WF) -> None:
    sb = wf.sandbox()
    # [self=0, 100, 200]
    wf.create(sb, "a", position="bottom")
    wf.create(sb, "b", position="bottom")
    s = wf.create(sb, "self")  # 50
    for _ in range(5):
        wf.move(s, sb, "top")  # 50 -> ... recomputed excluding itself: 100//2 = 50
    assert wf.get(s)["priority"] == 50
    wf.move(s, sb, "bottom")
    assert wf.get(s)["priority"] == 300


def test_move_ignores_index_fields(wf: WF) -> None:
    a, b = wf.sandbox("A"), wf.sandbox("B")
    for i in range(3):
        wf.create(b, f"b{i}", position="bottom")
    x = wf.create(a, "x")
    wf.ok(wf.move_raw(x, {"parent_id": b, "priority": 150, "index": 1, "after": "q", "before": "z", "sibling_id": "s"}))
    assert wf.get(x)["priority"] == 50


def test_move_to_self(wf: WF) -> None:
    sb = wf.sandbox()
    wf.create(sb, "other", position="bottom")
    y = wf.create(sb, "y", position="bottom")
    before = wf.get(y)
    sb_before = wf.get(sb)["modifiedAt"]
    wf.clock.advance(5)
    wf.move(y, y, "bottom")
    after = wf.get(y)
    assert after["parent_id"] == before["parent_id"] and after["priority"] == before["priority"]
    assert after["modifiedAt"] == wf.clock.now
    assert wf.get(sb)["modifiedAt"] == sb_before


def test_move_into_descendant_rejected_by_default(wf: WF) -> None:
    sb = wf.sandbox()
    p = wf.create(sb, "P")
    p1 = wf.create(p, "p1")
    p1c = wf.create(p1, "p1c")
    for target in (p1, p1c):
        r = wf.move_raw(p, {"parent_id": target})
        assert r.status_code == 400
        assert r.json() == {"code": "invalid_request", "message": "Cannot move a node under its own descendant"}
    assert wf.get(p)["parent_id"] == sb


def test_move_into_descendant_faithful_creates_cycle(wf_faithful: WF) -> None:
    wf = wf_faithful
    sb = wf.sandbox()
    p = wf.create(sb, "P")
    p1 = wf.create(p, "p1")
    wf.move(p, p1)
    assert wf.get(p)["parent_id"] == p1 and wf.get(p1)["parent_id"] == p
    assert [n["id"] for n in wf.list(sb)] == []  # detached
    assert [n["id"] for n in wf.list(p1)] == [p]
    wf.move(p, sb)  # repair
    assert wf.get(p)["parent_id"] == sb and wf.get(p1)["parent_id"] == p
    # deleting inside a cycle terminates
    wf.move(p, p1)
    wf.delete(p1)
    assert wf.get_raw(p).status_code == 404


def test_concurrent_creates_never_duplicate_priorities(wf: WF) -> None:
    sb = wf.sandbox()
    errors = []

    def worker(i: int) -> None:
        try:
            for j in range(10):
                r = wf.create_raw({"parent_id": sb, "name": f"w{i}-{j}", "position": "top" if j % 2 else "bottom"})
                assert r.status_code == 200
        except Exception as e:  # pragma: no cover
            errors.append(e)

    threads = [threading.Thread(target=worker, args=(i,)) for i in range(8)]
    for t in threads:
        t.start()
    for t in threads:
        t.join()
    assert not errors
    prios = [n["priority"] for n in wf.list(sb)]
    assert len(prios) == 80 and len(set(prios)) == 80
