"""Mirror-aware semantics of the beta API (``beta_mirrors``; data-model.md §10.5).

Checked against the beta host on 2026-10-13 (experiments/beta-mirrors/).
"""

from __future__ import annotations

from .conftest import WF

HAS_MIRRORS = {"code": "invalid_request", "message": "Cannot delete a node that has mirrors"}
ITEM = {"code": "not_found", "message": "Item not found"}
PARENT = {"code": "not_found", "message": "Parent item not found"}


def setup(wf: WF) -> tuple[str, str, str, str]:
    """sandbox / O (with child oc) / dest / shell M of O under dest."""
    sb = wf.sandbox()
    o = wf.create(sb, "O origin", note="origin note", layoutMode="todo")
    wf.create(o, "oc")
    dest = wf.create(sb, "dest", position="bottom")
    m = wf.mirror(o, dest)["item_id"]
    return sb, o, dest, m


def test_off_by_default_no_metadata(wf: WF) -> None:
    _, o, _, m = setup(wf)
    assert wf.get(m)["data"] == {"layoutMode": "bullets"}
    assert wf.get(o)["data"] == {"layoutMode": "todo"}
    assert set(wf.list_raw(m).json()) == {"nodes"}


def test_get_combines_shell_location_with_origin_content(wf_beta: WF) -> None:
    wf = wf_beta
    sb, o, dest, m = setup(wf)
    wf.clock.advance(5)
    wf.complete(o)
    origin, shell = wf.get(o), wf.get(m)
    assert list(shell) == ["id", "name", "note", "parent_id", "priority", "completed", "data", "createdAt", "modifiedAt", "completedAt"]
    assert (shell["id"], shell["parent_id"], shell["priority"]) == (m, dest, 100)
    for k in ("name", "note", "completed", "modifiedAt", "completedAt"):
        assert shell[k] == origin[k], k
    assert shell["data"] == {"layoutMode": "todo", "mirror": {"origin_id": o}}
    assert origin["data"] == {"layoutMode": "todo", "mirror": {"mirror_ids": [m]}}
    m2 = wf.mirror(m, sb, "bottom")["item_id"]  # mirroring a mirror mirrors the origin
    assert wf.get(o)["data"]["mirror"] == {"mirror_ids": sorted([m, m2])}  # by id, not creation
    assert wf.get(m[-12:])["data"]["mirror"] == {"origin_id": o}  # short-id GET too


def test_list_under_mirror_root_shows_origin_children(wf_beta: WF) -> None:
    wf = wf_beta
    _, o, dest, m = setup(wf)
    (oc,) = wf.list(o)
    r = wf.list_raw(m).json()
    assert list(r) == ["nodes", "mirror"] and r["mirror"] == {"origin_id": o}
    assert [n["id"] for n in r["nodes"]] == [oc["id"]]
    assert r["nodes"][0] == {**oc, "parent_id": m}
    # the destination lists the shell in its mirror-aware shape
    (shell,) = wf.list(dest)
    assert shell["name"] == "O origin" and shell["data"]["mirror"] == {"origin_id": o}
    # a plain parent's list has no top-level mirror key
    assert list(wf.list_raw(dest).json()) == ["nodes"]


def test_create_under_mirror_root_goes_to_origin(wf_beta: WF) -> None:
    wf = wf_beta
    _, o, _, m = setup(wf)
    c = wf.create(m, "via mirror", position="bottom")
    assert wf.get(c)["parent_id"] == o
    assert [n["name"] for n in wf.children(o)] == ["oc", "via mirror"]
    assert [n["parent_id"] for n in wf.list(m)] == [m, m]
    c2 = wf.create(m[-12:], "via short id")  # any parent_id form naming the shell
    assert wf.get(c2)["parent_id"] == o


def test_move_into_mirror_root_is_persisted_under_the_shell(wf_beta: WF) -> None:
    wf = wf_beta
    sb, o, _, m = setup(wf)
    x = wf.create(sb, "x")
    wf.move(x, m)
    assert wf.get(x)["parent_id"] == m
    assert [n["name"] for n in wf.list(o)] == ["oc"]  # not under the origin
    assert [n["name"] for n in wf.list(m)] == ["oc"]  # and invisible in the shell's list
    # it goes when the shell goes
    wf.delete(m)
    assert wf.get_raw(x).json() == ITEM


def test_move_of_mirror_moves_the_shell_only(wf_beta: WF) -> None:
    wf = wf_beta
    sb, o, _, m = setup(wf)
    wf.move(m, sb, "bottom")
    assert wf.get(m)["parent_id"] == sb
    assert wf.get(o)["parent_id"] == sb and wf.get(o)["priority"] == 100


def test_writes_through_mirror_reach_origin(wf_beta: WF) -> None:
    wf = wf_beta
    _, o, _, m = setup(wf)
    wf.clock.advance(5)
    wf.update(m, name="renamed", note="n2", layoutMode="h1")
    origin = wf.get(o)
    assert (origin["name"], origin["note"], origin["data"]["layoutMode"]) == ("renamed", "n2", "h1")
    assert origin["modifiedAt"] == wf.clock.now
    wf.complete(m)
    assert wf.get(o)["completed"] is True and wf.get(m)["completed"] is True
    wf.uncomplete(m)
    assert wf.get(o)["completed"] is False
    # and the other way round
    wf.update(o, name="again")
    assert wf.get(m)["name"] == "again"
    # the shell row itself is untouched
    row = next(n for n in wf.admin("GET", "dump").json()["nodes"] if n["id"] == m)
    assert (row["name"], row["data"]["layoutMode"], row["completed"]) == ("", "bullets", False)


def test_delete_origin_with_mirrors_fails(wf_beta: WF) -> None:
    wf = wf_beta
    sb, o, dest, m = setup(wf)
    for target in (o, sb):  # the origin itself, or an ancestor of it
        r = wf.http.delete(f"/api/v1/nodes/{target}")
        assert (r.status_code, r.json()) == (400, HAS_MIRRORS)
    assert wf.get(o)["id"] == o
    # deleting the mirror root is fine and detaches it
    wf.delete(m)
    assert "mirror" not in wf.get(o)["data"]
    wf.delete(o)
    assert wf.get_raw(o).json() == ITEM


def test_delete_subtree_with_shell_detaches_it_and_bumps_origin(wf_beta: WF) -> None:
    wf = wf_beta
    _, o, dest, m = setup(wf)
    wf.clock.advance(5)
    wf.delete(dest)
    assert wf.get_raw(m).json() == ITEM
    origin = wf.get(o)
    assert "mirror" not in origin["data"] and origin["modifiedAt"] == wf.clock.now


def test_delete_of_shell_with_mirrored_persisted_child_blocks(wf_beta: WF) -> None:
    wf = wf_beta
    sb, _, _, m = setup(wf)
    y = wf.create(sb, "y")
    wf.move(y, m)
    y2 = wf.mirror(y, sb)["item_id"]
    r = wf.http.delete(f"/api/v1/nodes/{m}")
    assert (r.status_code, r.json()) == (400, HAS_MIRRORS)
    # DELETE /mirror does not check: y goes, and its mirror y2 is orphaned
    wf.ok(wf.delete_mirror(m))
    assert wf.get_raw(y).json() == ITEM
    assert wf.get(y2)["data"]["mirror"] == {"origin_id": None}


def test_mirror_inside_the_subtree_still_blocks(wf_beta: WF) -> None:
    wf = wf_beta
    sb = wf.sandbox()
    o = wf.create(sb, "O")
    oc = wf.create(o, "oc")
    wf.mirror(o, oc)
    r = wf.http.delete(f"/api/v1/nodes/{o}")
    assert (r.status_code, r.json()) == (400, HAS_MIRRORS)


def test_delete_mirror_endpoint_unchanged(wf_beta: WF) -> None:
    wf = wf_beta
    _, o, _, m = setup(wf)
    assert wf.delete_mirror(o).status_code == 400
    wf.ok(wf.delete_mirror(m))
    assert "mirror" not in wf.get(o)["data"]


def test_orphan_falls_back_to_shell_row(wf_beta: WF) -> None:
    # Orphans cannot be made through the beta API any more; make one with
    # production semantics, then switch.
    wf = wf_beta
    wf.admin("POST", "config", json={"beta_mirrors": False})
    _, o, dest, m = setup(wf)
    wf.create(m, "own child")
    wf.delete(o)
    wf.admin("POST", "config", json={"beta_mirrors": True})
    shell = wf.get(m)
    assert (shell["name"], shell["note"]) == ("", "")
    assert shell["data"] == {"layoutMode": "bullets", "mirror": {"origin_id": None}}
    assert wf.list_raw(m).json() == {"nodes": []}  # own child hidden, no mirror key
    assert wf.update_raw(m, {"name": "orphan edit"}).json() == ITEM
    assert wf.http.post(f"/api/v1/nodes/{m}/complete").json() == ITEM
    assert wf.http.post(f"/api/v1/nodes/{m}/uncomplete").json() == ITEM
    assert wf.create_raw({"parent_id": m, "name": "under orphan"}).json() == PARENT
    assert wf.mirror_raw(m, {"parent_id": dest}).json() == ITEM
    x = wf.create(dest, "x")
    assert wf.mirror_raw(x, {"parent_id": m}).json() == PARENT
    wf.move(x, m)  # moves are fine
    wf.move(m, dest)
    wf.delete(m)


def test_export_uses_mirror_aware_shape(wf_beta: WF) -> None:
    wf = wf_beta
    sb, o, _, m = setup(wf)
    hidden = wf.create(sb, "hidden")
    wf.move(hidden, m)
    r = wf.http.get("/api/v1/nodes-export")
    by_id = {n["id"]: n for n in r.json()["nodes"]}
    assert all(by_id[i] == wf.get(i) for i in by_id)  # every record is the GET record
    # the origin's children appear once, under the origin; a shell's hidden
    # persisted child appears under the shell (the only bulk way to see it)
    (oc,) = wf.list(o)
    assert [n["parent_id"] for n in by_id.values() if n["id"] == oc["id"]] == [o]
    assert by_id[hidden]["parent_id"] == m


# ------------------------------------------------------- the mirror registry


def test_prod_host_delete_of_shell_leaves_a_blocking_ghost(wf_beta: WF) -> None:
    wf = wf_beta
    sb, o, _, m = setup(wf)
    wf.admin("POST", "config", json={"beta_mirrors": False})
    wf.delete(m)  # through production
    wf.admin("POST", "config", json={"beta_mirrors": True})
    assert wf.get(o)["data"]["mirror"] == {"mirror_ids": [m]}  # a ghost
    r = wf.http.delete(f"/api/v1/nodes/{o}")
    assert (r.status_code, r.json()) == (400, HAS_MIRRORS)
    assert wf.delete_mirror(m).json() == ITEM  # cannot be cleared on beta
    m2 = wf.mirror(o, sb)["item_id"]
    assert wf.get(o)["data"]["mirror"] == {"mirror_ids": sorted([m, m2])}
    wf.delete(m2)
    assert wf.get(o)["data"]["mirror"] == {"mirror_ids": [m]}
    # only a production delete gets rid of the origin
    wf.admin("POST", "config", json={"beta_mirrors": False})
    wf.delete(o)


def test_prod_host_delete_mirror_cleans_the_registry(wf_beta: WF) -> None:
    wf = wf_beta
    _, o, _, m = setup(wf)
    wf.admin("POST", "config", json={"beta_mirrors": False})
    wf.ok(wf.delete_mirror(m))
    wf.admin("POST", "config", json={"beta_mirrors": True})
    assert "mirror" not in wf.get(o)["data"]
    wf.delete(o)


def test_ui_promotion_leaves_live_shell_unregistered(wf_beta: WF) -> None:
    # ui-findings H6 seen through beta: the promoted origin lists no mirrors,
    # the surviving shell still points at it, and a beta delete orphans it.
    wf = wf_beta
    sb, o, dest, m1 = setup(wf)
    e2 = wf.create(sb, "E2")
    m2 = wf.mirror(o, e2)["item_id"]
    r = wf.admin("POST", "ui/delete", json={"id": o})
    assert r.json()["promoted"]["replaced_shell_id"] == m2
    assert "mirror" not in wf.get(o)["data"]
    assert wf.get(m1)["data"]["mirror"] == {"origin_id": o}
    wf.delete(o)
    assert wf.get(m1)["data"]["mirror"] == {"origin_id": None}
