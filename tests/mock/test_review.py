"""Behaviours added or corrected by the 2026-10-12 independent review.

Each production fact here is backed by a conformance golden (named in the
test docstring); these tests pin the same behaviour at the unit level, plus
the test-double features (full reset, seeded ids) that have no production
counterpart.
"""

from __future__ import annotations

import json

import httpx
import pytest

from syncflowy.mock.testing import running_mock
from syncflowy.mock.text import transform_create_name

from .conftest import WF

JH = {"Content-Type": "application/json"}


# --- §3.2 URL parent_id boundary (golden ids_url_parent_boundaries) -------------


@pytest.mark.parametrize("suffix", ["", "/", "?q=x", "/0123456789ab"])
def test_url_parent_accepted_suffixes(wf: WF, suffix: str) -> None:
    sb = wf.sandbox()
    wf.create(sb, "child")
    r = wf.list_raw(f"https://workflowy.com/#/{sb[-12:]}{suffix}")
    assert r.status_code == 200 and [n["name"] for n in r.json()["nodes"]] == ["child"]


@pytest.mark.parametrize("suffix", ["-x", "#y", ".", "g", "_", " ", "&x=1", "0", "a", "A"])
def test_url_parent_rejected_suffixes(wf: WF, suffix: str) -> None:
    sb = wf.sandbox()
    raw = f"https://workflowy.com/#/{sb[-12:]}{suffix}"
    r = wf.list_raw(raw)
    assert r.status_code == 404
    assert r.json() == {"code": "not_found", "message": f"Location '{raw}' not found"}


def test_url_parent_rejected_suffix_on_missing_short_is_location(wf: WF) -> None:
    raw = "https://workflowy.com/#/000000000003-x"
    assert wf.list_raw(raw).json()["message"] == f"Location '{raw}' not found"


# --- §6.2 step 11 dates (golden text_dates_edges) -------------------------------


@pytest.mark.parametrize(
    "raw, expected",
    [
        (
            "[2026-01-15 9:5]",
            '<time startYear="2026" startMonth="1" startDay="15" startHour="9" startMinute="5">'
            "Thu, Jan 15, 2026 at 9:05 AM</time>",
        ),
        ("[0001-01-01]", '<time startYear="1" startMonth="1" startDay="1">Mon, Jan 1, 1</time>'),
        ("[0000-01-01]", "[0000-01-01]"),
        ("[999-01-15]", "[999-01-15]"),
        ("[2026-01-15 014:30]", "[2026-01-15 014:30]"),
        (
            "[٢٠٢٦-01-15]",
            '<time startYear="2026" startMonth="1" startDay="15">Thu, Jan 15, 2026</time>',
        ),
        (
            "[２０２６-01-15]",
            '<time startYear="2026" startMonth="1" startDay="15">Thu, Jan 15, 2026</time>',
        ),
    ],
)
def test_date_edges(raw: str, expected: str) -> None:
    assert transform_create_name(raw)[0].name == expected


# --- NUL / lone surrogates (goldens create_layoutmode_controls_nul_and_surrogates,
# --- update_nul_layoutmode_and_lone_surrogates) --------------------------------


def _raw_post(wf: WF, path: str, body: str) -> httpx.Response:
    return wf.request("POST", path, content=body.encode("ascii"), headers=JH)


def test_update_layoutmode_nul_is_500_and_changes_nothing(wf: WF) -> None:
    sb = wf.sandbox()
    n = wf.create(sb, "a")
    before = wf.get(n)
    wf.clock.advance(5)
    r = wf.update_raw(n, {"layoutMode": "h\x001", "name": "changed"})
    assert r.status_code == 500 and r.json()["code"] == "internal_error"
    assert wf.get(n) == before  # name and modifiedAt untouched


def test_update_lone_surrogates(wf: WF) -> None:
    sb = wf.sandbox()
    n = wf.create(sb, "a")
    assert _raw_post(wf, f"/api/v1/nodes/{n}", '{"name": "x\\ud800y", "note": "\\udc00z"}').status_code == 200
    got = wf.get(n)
    assert (got["name"], got["note"]) == ("x?y", "?z")
    r = _raw_post(wf, f"/api/v1/nodes/{n}", '{"layoutMode": "x\\udc00"}')
    assert r.status_code == 500
    assert wf.get(n)["data"]["layoutMode"] == "bullets"


def test_create_lone_surrogates(wf: WF) -> None:
    sb = wf.sandbox()
    r = _raw_post(wf, "/api/v1/nodes", json.dumps({"parent_id": sb, "name": "x\ud800y"}))
    assert r.status_code == 200
    assert wf.get(r.json()["item_id"])["name"] == "x?y"
    for field in ("note", "layoutMode"):
        r = _raw_post(wf, "/api/v1/nodes", json.dumps({"parent_id": sb, "name": "n", field: "x\ud800"}))
        assert r.status_code == 500
    assert [c["name"] for c in wf.list(sb)] == ["x?y"]


def test_create_layoutmode_controls_verbatim_nul_500(wf: WF) -> None:
    sb = wf.sandbox()
    n = wf.create(sb, "a", layoutMode="h\x01\x0b1")
    assert wf.get(n)["data"]["layoutMode"] == "h\x01\x0b1"
    assert wf.create_raw({"parent_id": sb, "name": "b", "layoutMode": "h\x001"}).status_code == 500


# --- test-double features -------------------------------------------------------


def test_full_reset_restores_config_and_clock(wf: WF) -> None:
    wf.admin("POST", "config", json={"faithful_cycles": True, "export_min_interval": 0})
    wf.admin("POST", "clock", json={"advance": 1000})
    wf.sandbox()
    assert wf.admin("POST", "reset").status_code == 200  # plain reset keeps config
    assert wf.admin("GET", "config").json()["faithful_cycles"] is True
    assert wf.admin("POST", "reset", json={"all": True}).status_code == 200
    assert wf.admin("GET", "config").json() == {"faithful_cycles": False, "export_min_interval": 60.0, "week_start": 0}
    assert wf.admin("GET", "clock").json()["offset"] == 0
    assert wf.admin("GET", "dump").json()["nodes"] == []


def test_seeded_ids_are_reproducible_and_restart_on_reset() -> None:
    def run(url: str) -> list[str]:
        with httpx.Client(base_url=url, headers={"Authorization": "Bearer test-key"}) as c:
            a = c.post("/api/v1/nodes", json={"parent_id": "None", "name": "a\nB"}).json()["item_id"]
            kids = [n["id"] for n in c.get("/api/v1/nodes", params={"parent_id": a}).json()["nodes"]]
            return [a, *kids]

    with running_mock(id_seed=7) as u1, running_mock(id_seed=7) as u2:
        first = run(u1)
        assert first == run(u2)
        assert all(i[14] == "4" and i == i.lower() for i in first)  # well-formed UUIDv4
        httpx.post(f"{u1}/_mock/reset")
        assert run(u1) == first


def test_parent_id_lone_surrogate(wf: WF) -> None:
    """Golden ids_parent_id_lone_surrogate: create/move -> Location miss; mirror -> 500."""
    sb = wf.sandbox()
    a = wf.create(sb, "a")
    pid = "zz x\\ud800"
    r = _raw_post(wf, "/api/v1/nodes", '{"parent_id": "%s", "name": "n"}' % pid)
    assert r.status_code == 404 and r.json() == {"code": "not_found", "message": "Location 'zz x\ud800' not found"}
    r = _raw_post(wf, f"/api/v1/nodes/{a}/move", '{"parent_id": "%s"}' % pid)
    assert r.status_code == 404 and r.json()["message"] == "Location 'zz x\ud800' not found"
    assert _raw_post(wf, f"/api/v1/nodes/{a}/mirror", '{"parent_id": "%s"}' % pid).status_code == 500
    assert [n["name"] for n in wf.list(sb)] == ["a"]
