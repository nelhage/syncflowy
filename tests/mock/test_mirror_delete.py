"""Delete and mirrors (data-model.md §4.6, §9, §10) and the modifiedAt table (§8)."""

from __future__ import annotations

from .conftest import WF, rand_uuid

ITEM = {"code": "not_found", "message": "Item not found"}
PARENT = {"code": "not_found", "message": "Parent item not found"}
SELF = {"code": "invalid_request", "message": "Cannot mirror a node under itself"}
NOT_MIRROR = {"code": "invalid_request", "message": "Node is not a mirror"}


def stamps(wf: WF, *ids: str) -> dict[str, int | None]:
    out = {}
    for i in ids:
        r = wf.get_raw(i)
        out[i] = r.json()["node"]["modifiedAt"] if r.status_code == 200 else None
    return out


def bumped(wf: WF, before: dict[str, int | None]) -> set[str]:
    after = stamps(wf, *before)
    return {i for i in before if after[i] is not None and after[i] != before[i]}


# ------------------------------------------------------------------ delete


def test_delete_is_hard_recursive_and_bumps_nothing(wf: WF) -> None:
    sb = wf.sandbox()
    a = wf.create(sb, "a", position="bottom")
    b = wf.create(sb, "b", position="bottom")
    a1 = wf.create(a, "a1")
    a1x = wf.create(a1, "a1x")
    before = stamps(wf, sb, b)
    wf.clock.advance(5)
    wf.delete(a)
    for x in (a, a1, a1x):
        assert wf.get_raw(x).json() == ITEM
    assert bumped(wf, before) == set()
    assert wf.prios(sb) == {"b": 200}


# ------------------------------------------------------------------ mirrors


def test_mirror_shell_shape_and_independence(wf: WF) -> None:
    sb = wf.sandbox()
    o = wf.create(sb, "O origin **b**", note="origin note", layoutMode="todo")
    wf.create(o, "oc1")
    wf.complete(o)
    dest = wf.create(sb, "dest")
    wf.create(dest, "x1", position="bottom")
    wf.create(dest, "x2", position="bottom")
    res = wf.mirror(o, dest)
    assert list(res) == ["item_id", "origin_id"] and res["origin_id"] == o
    m = wf.get(res["item_id"])
    assert (m["name"], m["note"], m["data"]["layoutMode"], m["completed"], m["completedAt"]) == ("", "", "bullets", False, None)
    assert m["parent_id"] == dest and m["priority"] == 50
    assert wf.list(m["id"]) == []  # origin's children are not visible
    assert wf.get(m["id"][-12:])["id"] == m["id"]
    # nothing propagates either way
    wf.update(o, name="O renamed")
    assert wf.get(m["id"])["name"] == ""
    wf.update(m["id"], name="M renamed", layoutMode="h1")
    assert wf.get(o)["name"] == "O renamed" and wf.get(o)["data"]["layoutMode"] == "todo"
    # mirroring twice makes two shells; bottom placement
    m2 = wf.mirror(o, dest, "bottom")
    assert wf.get(m2["item_id"])["priority"] == 300


def test_mirror_bumps_origin_only(wf: WF) -> None:
    sb = wf.sandbox()
    b = wf.create(sb, "B")
    o = wf.create(b, "O")
    oc = wf.create(o, "oc")
    a = wf.create(sb, "A")
    before = stamps(wf, sb, a, b, o, oc)
    wf.clock.advance(5)
    shell = wf.mirror(o, a)["item_id"]
    assert bumped(wf, before) == {o}
    s = wf.get(shell)
    assert s["createdAt"] == s["modifiedAt"] == wf.clock.now


def test_mirror_of_mirror_resolves_origin_and_bumps_it(wf: WF) -> None:
    sb = wf.sandbox()
    o = wf.create(sb, "O")
    d1, d2 = wf.create(sb, "D1"), wf.create(sb, "D2")
    m1 = wf.mirror(o, d1)["item_id"]
    before = stamps(wf, o, m1, d1, d2)
    wf.clock.advance(5)
    res = wf.mirror(m1, d2)
    assert res["origin_id"] == o
    assert bumped(wf, before) == {o}


def test_mirror_parent_shell_redirects_to_origin(wf: WF) -> None:
    sb = wf.sandbox()
    o = wf.create(sb, "O")
    d = wf.create(sb, "D")
    m1 = wf.mirror(o, d)["item_id"]
    x = wf.create(sb, "X")
    before = stamps(wf, o, m1, x)
    wf.clock.advance(5)
    shell = wf.mirror(x, m1)["item_id"]
    assert wf.get(shell)["parent_id"] == o
    assert wf.list(m1) == []
    assert bumped(wf, before) == {x}
    # create and move into a shell are NOT redirected
    c = wf.create(m1, "child of shell")
    assert wf.get(c)["parent_id"] == m1
    loose = wf.create(sb, "loose")
    wf.move(loose, m1)
    assert wf.get(loose)["parent_id"] == m1
    assert {n["id"] for n in wf.list(m1)} == {c, loose}


def test_mirror_self_and_descendants(wf: WF) -> None:
    sb = wf.sandbox()
    o = wf.create(sb, "O")
    oc1 = wf.create(o, "oc1")
    occ = wf.create(oc1, "occ")
    m1 = wf.mirror(o, sb)["item_id"]
    assert wf.mirror_raw(o, {"parent_id": o}).json() == SELF
    assert wf.mirror_raw(o, {"parent_id": m1}).json() == SELF  # resolves to O
    assert wf.mirror_raw(m1, {"parent_id": m1}).json() == SELF
    assert wf.mirror_raw(o, {"parent_id": o}).status_code == 400
    # descendants allowed
    wf.mirror(o, oc1)
    wf.mirror(o, occ)
    wf.mirror(oc1, o)


def test_moving_origin_into_its_own_shell_allowed(wf: WF) -> None:
    sb = wf.sandbox()
    o = wf.create(sb, "O")
    d = wf.create(sb, "D")
    m = wf.mirror(o, d)["item_id"]
    wf.move(o, m)
    assert wf.get(o)["parent_id"] == m


def test_delete_mirror(wf: WF) -> None:
    sb = wf.sandbox()
    o = wf.create(sb, "O")
    oc = wf.create(o, "oc")
    d = wf.create(sb, "D")
    m = wf.mirror(o, d)["item_id"]
    mc = wf.create(m, "child under shell")
    assert wf.delete_mirror(o).json() == NOT_MIRROR
    assert wf.delete_mirror(oc).json() == NOT_MIRROR
    assert wf.delete_mirror(mc).json() == NOT_MIRROR  # plain child of a shell
    before = stamps(wf, o, oc, d, sb)
    wf.clock.advance(5)
    wf.ok(wf.delete_mirror(m))
    assert bumped(wf, before) == {o}
    assert wf.get_raw(m).json() == ITEM and wf.get_raw(mc).json() == ITEM
    assert wf.delete_mirror(m).json() == ITEM


def test_plain_delete_of_shell_does_not_bump_origin(wf: WF) -> None:
    sb = wf.sandbox()
    o = wf.create(sb, "O")
    h = wf.create(sb, "H")
    m = wf.mirror(o, h)["item_id"]
    m2 = wf.mirror(o, sb)["item_id"]
    before = stamps(wf, o, sb, h)
    wf.clock.advance(5)
    wf.delete(m2)
    wf.delete(h)  # ancestor of a shell
    assert wf.get_raw(m).status_code == 404
    assert bumped(wf, before) == set()


def test_orphans(wf: WF) -> None:
    sb = wf.sandbox()
    o = wf.create(sb, "O")
    o2 = wf.create(sb, "O2")
    d = wf.create(sb, "D")
    n = wf.mirror(o, d)["item_id"]
    nc = wf.create(n, "nc")
    n2 = wf.mirror(o2, d)["item_id"]
    n_before = wf.get(n)
    wf.clock.advance(5)
    wf.delete(o)
    wf.delete(o2)
    assert wf.get(n) == n_before  # survives untouched
    assert wf.get(nc)["parent_id"] == n
    # as mirror source: Item not found (origin resolution before parent lookup)
    assert wf.mirror_raw(n, {"parent_id": d}).json() == ITEM
    assert wf.mirror_raw(n, {"parent_id": rand_uuid()}).json() == ITEM
    assert wf.mirror_raw(n, {"parent_id": n}).json() == ITEM
    # as mirror parent: Parent item not found
    x = wf.create(sb, "X")
    assert wf.mirror_raw(x, {"parent_id": n2}).json() == PARENT
    # a plain child of an orphan is mirrorable
    wf.mirror(nc, d)
    # ordinary node otherwise
    wf.update(n, name="orphan renamed", note="n")
    wf.complete(n)
    h2 = wf.create(sb, "H2")
    wf.move(n, h2, "bottom")
    assert wf.get(n)["parent_id"] == h2 and wf.get(nc)["parent_id"] == n
    k = wf.create(n, "k")
    assert wf.get(k)["parent_id"] == n
    # DELETE /mirror still treats it as a mirror and succeeds
    wf.ok(wf.delete_mirror(n2))
    wf.delete(n)
    assert wf.get_raw(nc).status_code == 404


def test_shell_priority_participates_in_renumber(wf: WF) -> None:
    sb = wf.sandbox()
    o = wf.create(sb, "O")
    # children with priorities 0, 1 (multiline create); a top mirror renumbers them
    gp = wf.create(sb, "multi\n\nM1\n\nM2")
    shell = wf.mirror(o, gp)["item_id"]
    assert sorted(wf.prios(gp).values()) == [100, 200, 300]
    assert wf.get(shell)["priority"] == 100


# ------------------------------------------------------------------ modifiedAt table (§8)


def test_modified_at_table(wf: WF) -> None:
    sb = wf.sandbox()
    p1, p2 = wf.create(sb, "P1"), wf.create(sb, "P2")
    x = wf.create(p1, "X")
    xc = wf.create(x, "xc")
    xcc = wf.create(xc, "xcc")
    existing = wf.create(p2, "existing")
    everyone = (sb, p1, p2, x, xc, xcc, existing)

    def step(action, expected: set[str]) -> None:
        before = stamps(wf, *everyone)
        wf.clock.advance(3)
        action()
        assert bumped(wf, before) == expected

    step(lambda: wf.move(x, p2, "bottom"), {x, p2})  # cross-parent: node + destination only
    step(lambda: wf.move(x, p2, "top"), {x, p2})  # same-parent
    step(lambda: wf.move(x, x), {x})  # move to self
    step(lambda: wf.create(p2, "new child"), set())  # create never bumps the parent
    step(lambda: wf.update(xc, name="renamed"), {xc})
    step(lambda: wf.complete(xc), {xc})
    step(lambda: wf.uncomplete(xc), {xc})
    step(lambda: wf.delete(xcc), set())
    step(lambda: wf.move_raw(x, {"parent_id": rand_uuid()}), set())  # failed request
