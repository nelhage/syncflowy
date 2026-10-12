"""Id resolution: path ids (§3.1), parent_id for list/create/move (§3.2), mirror parent_id (§3.3)."""

from __future__ import annotations

import pytest

from .conftest import WF, rand_uuid

ITEM = {"code": "not_found", "message": "Item not found"}
PARENT = {"code": "not_found", "message": "Parent item not found"}


def loc(raw: str) -> dict[str, str]:
    return {"code": "not_found", "message": f"Location '{raw}' not found"}


# ------------------------------------------------------------------ path ids


def test_get_accepts_uuid_and_short_id_any_case(wf: WF) -> None:
    sb = wf.sandbox()
    short = sb[-12:]
    for form in (sb, short, short.upper()):
        n = wf.get(form)
        assert n["id"] == sb  # canonical lowercase uuid


@pytest.mark.parametrize(
    "form",
    [
        lambda u: u.upper(),
        lambda u: u.replace("-", ""),
        lambda u: u.replace("-", "").upper(),
        lambda u: u[-11:],
        lambda u: u.replace("-", "")[-13:],
        lambda u: u[:12],
        lambda u: u[-8:],
        lambda u: "{" + u + "}",
        lambda u: u + "x",
        lambda u: u[:-1],
        lambda u: "garbage-zq9x",
        lambda u: "zzzzzzzzzzzz",
        lambda u: "nöde",
        lambda u: "a b",
        lambda u: "a" * 300,
    ],
)
def test_get_other_forms_404(wf: WF, form) -> None:
    sb = wf.sandbox()
    r = wf.get_raw(form(sb))
    assert r.status_code == 404 and r.json() == ITEM


def test_mutating_endpoints_reject_short_ids_and_do_nothing(wf: WF) -> None:
    sb = wf.sandbox()
    other = wf.sandbox("other")
    nid = wf.create(sb, "target")
    before = wf.get(nid)
    for form in (nid[-12:], nid[-12:].upper(), nid.upper(), nid.replace("-", "")):
        assert wf.update_raw(form, {"name": "changed"}).json() == ITEM
        assert wf.http.delete(f"/api/v1/nodes/{form}").json() == ITEM
        assert wf.move_raw(form, {"parent_id": other}).json() == ITEM
        assert wf.http.post(f"/api/v1/nodes/{form}/complete").json() == ITEM
        assert wf.http.post(f"/api/v1/nodes/{form}/uncomplete").json() == ITEM
        assert wf.mirror_raw(form, {"parent_id": other}).json() == ITEM
        assert wf.delete_mirror(form).json() == ITEM
    assert wf.get(nid) == before


def test_nonexistent_ids_every_endpoint(wf: WF) -> None:
    sb = wf.sandbox()
    r = rand_uuid()
    assert wf.get_raw(r).json() == ITEM
    assert wf.list_raw(r).json() == PARENT
    assert wf.create_raw({"parent_id": r, "name": "x"}).json() == PARENT
    assert wf.update_raw(r, {"name": "x"}).json() == ITEM
    assert wf.update_raw(r, {}).json() == ITEM
    assert wf.http.delete(f"/api/v1/nodes/{r}").json() == ITEM
    assert wf.move_raw(r, {"parent_id": sb}).json() == ITEM
    assert wf.move_raw(sb, {"parent_id": r}).json() == PARENT
    assert wf.move_raw(r, {"parent_id": rand_uuid()}).json() == ITEM  # item before parent
    assert wf.http.post(f"/api/v1/nodes/{r}/complete").json() == ITEM
    assert wf.http.post(f"/api/v1/nodes/{r}/uncomplete").json() == ITEM
    assert wf.mirror_raw(r, {"parent_id": sb}).json() == ITEM
    assert wf.mirror_raw(sb, {"parent_id": r}).json() == PARENT
    assert wf.mirror_raw(r, {"parent_id": rand_uuid()}).json() == ITEM
    assert wf.delete_mirror(r).json() == ITEM


def test_deleted_node_and_descendants_404(wf: WF) -> None:
    sb = wf.sandbox()
    p = wf.create(sb, "p")
    c = wf.create(p, "c")
    wf.delete(p)
    for x in (p, c):
        assert wf.get_raw(x).json() == ITEM
        assert wf.list_raw(x).json() == PARENT
        assert wf.create_raw({"parent_id": x, "name": "y"}).json() == PARENT
        assert wf.update_raw(x, {}).json() == ITEM
    assert wf.http.delete(f"/api/v1/nodes/{p}").json() == ITEM  # deleting twice


# ---------------------------------------------------------- parent_id resolution


def _forms_ok(sb: str) -> list[str]:
    short = sb[-12:]
    return [
        sb,
        short,
        short.upper(),
        f"https://workflowy.com/#/{short}",
        f"https://workflowy.com/#/{short.upper()}",
        f"http://workflowy.com/#/{short}",
        f"https://WORKFLOWY.COM/#/{short}",
        f"https://beta.workflowy.com/#/{short}",
        f"https://workflowy.com/#/{short}/",
        f"https://workflowy.com/#/{short}?q=x",
        f"https://workflowy.com/#/{short}/0123456789ab",
    ]


def test_parent_forms_resolve_for_list_create_move(wf: WF) -> None:
    sb = wf.sandbox()
    for form in _forms_ok(sb):
        assert wf.list_raw(form).status_code == 200, form
        nid = wf.create(form, f"via {form}")
        assert wf.get(nid)["parent_id"] == sb
        other = wf.sandbox("other")
        wf.move(nid, other)
        wf.move(nid, form)
        assert wf.get(nid)["parent_id"] == sb


def test_url_first_segment_wins(wf: WF) -> None:
    a, b = wf.sandbox("a"), wf.sandbox("b")
    wf.create(a, "in a")
    names = [n["name"] for n in wf.list(f"https://workflowy.com/#/{a[-12:]}/{b[-12:]}")]
    assert names == ["in a"]


@pytest.mark.parametrize(
    "make,expected",
    [
        (lambda sb, r: r, "parent"),  # random uuid
        (lambda sb, r: r.upper(), "parent"),  # random uppercase uuid
        (lambda sb, r: sb.upper(), "parent"),  # UPPERCASE uuid of an existing node: exact lookup misses
        (lambda sb, r: r.replace("-", ""), "loc"),  # dashless
        (lambda sb, r: sb.replace("-", ""), "loc"),  # dashless existing
        (lambda sb, r: "0123456789ab", "loc"),  # bare random short id
        (lambda sb, r: "ffffffffffff", "loc"),
        (lambda sb, r: "garbage-zq9x", "loc"),
        (lambda sb, r: "nöde-zq9x", "loc"),
        (lambda sb, r: "zzzzzzzzzzzz", "loc"),
        (lambda sb, r: f" {sb[-12:]} ", "loc"),  # no trimming
        (lambda sb, r: f"{sb} ", "loc"),
        (lambda sb, r: "https://workflowy.com/#/0123456789ab", "parent"),  # URL with a random short id
        (lambda sb, r: "https://workflowy.com/#/0123456789AB", "parent"),
        (lambda sb, r: f"https://workflowy.com/#/{r}", "loc"),  # URL with a full uuid
        (lambda sb, r: f"https://workflowy.com/#/{sb}", "loc"),  # ... even an existing one
        (lambda sb, r: f"https://workflowy.com/#/{sb.replace('-', '')}", "loc"),
        (lambda sb, r: f"https://www.workflowy.com/#/{sb[-12:]}", "loc"),
        (lambda sb, r: f"https://app.workflowy.com/#/{sb[-12:]}", "loc"),
        (lambda sb, r: f"https://evil.example.com/#/{sb[-12:]}", "loc"),
        (lambda sb, r: f"workflowy.com/#/{sb[-12:]}", "loc"),
        (lambda sb, r: f"#/{sb[-12:]}", "loc"),
        (lambda sb, r: f"https://workflowy.com#/{sb[-12:]}", "loc"),
        (lambda sb, r: f"https://workflowy.com/?q=1#/{sb[-12:]}", "loc"),
        (lambda sb, r: f"https://workflowy.com/#/x/{sb[-12:]}", "loc"),
        (lambda sb, r: sb[-11:], "loc"),
        (lambda sb, r: sb[:12], "loc"),
    ],
)
def test_parent_misses(wf: WF, make, expected: str) -> None:
    sb = wf.sandbox()
    nid = wf.create(sb, "mover")
    raw = make(sb, rand_uuid())
    want = PARENT if expected == "parent" else loc(raw)
    assert wf.list_raw(raw).json() == want
    assert wf.create_raw({"parent_id": raw, "name": "x"}).json() == want
    r = wf.move_raw(nid, {"parent_id": raw})
    assert r.status_code == 404 and r.json() == want
    assert wf.get(nid)["parent_id"] == sb  # unmoved


def test_location_message_is_json_escaped(wf: WF) -> None:
    r = wf.list_raw("nöde-zq9x")
    assert r.content == b'{"code": "not_found", "message": "Location \'n\\u00f6de-zq9x\' not found"}'


def test_non_string_parent_is_validation_error(wf: WF) -> None:
    for v in (123456789, ["x"]):
        r = wf.create_raw({"parent_id": v, "name": "x"})
        assert r.status_code == 400 and r.json()["errors"][0]["type"] == "string_type"


# ---------------------------------------------------------- mirror parent_id


def test_mirror_parent_exact_uuid_only(wf: WF) -> None:
    sb = wf.sandbox()
    dest = wf.create(sb, "dest")
    src = wf.create(sb, "src")
    short = dest[-12:]
    for raw in (
        short,
        short.upper(),
        f"https://workflowy.com/#/{short}",
        dest.upper(),
        dest.replace("-", ""),
        "zzzz-not-an-id",
        rand_uuid(),
        "0123456789ab",
        "None",
        "inbox",
        "",
    ):
        r = wf.mirror_raw(src, {"parent_id": raw})
        assert r.status_code == 404 and r.json() == PARENT, raw
    assert wf.mirror(src, dest)["origin_id"] == src
