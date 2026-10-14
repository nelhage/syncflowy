"""Unit tests for the conformance normaliser and production-safety vetting."""

from __future__ import annotations

import email.utils
import json

import pytest

from syncflowy.conformance.backend import FakeClock, ProdBackend, UnsafeRequest
from syncflowy.conformance.normalize import Clock, Symbols, normalize, render
from syncflowy.conformance.runner import ambiguous_aliases, diff_transcripts

ROOT = "5e2e76e4-fea4-4f1f-9c5b-0123456789ab"
PARENT = "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee"
A = "1a1b1c1d-2222-4333-8444-5e5e5e5e5e5e"
B = "66666666-7777-4888-9999-aaaaaaaaaaaa"


def node(id, parent, created, modified, completed_at=None, priority=100):
    return {
        "id": id,
        "name": "n",
        "note": None,
        "parent_id": parent,
        "priority": priority,
        "completed": completed_at is not None,
        "data": {"layoutMode": "bullets"},
        "createdAt": created,
        "modifiedAt": modified,
        "completedAt": completed_at,
    }


def req(method, path, body, *, epoch, t0, t1=None, json_body=None, status=200, headers=None, date=None):
    text = json.dumps(body) if body is not None else ""
    rh = {"content-type": "application/json", **(headers or {})}
    if date is not None:
        rh["date"] = email.utils.formatdate(date, usegmt=True)
    return {
        "type": "request",
        "epoch": epoch,
        "t0": t0,
        "t1": t0 if t1 is None else t1,
        "method": method,
        "path": path,
        "params": None,
        "json": json_body,
        "content": None,
        "headers": None,
        "status": status,
        "response_headers": rh,
        "body_text": text,
        "body_is_json": body is not None,
    }


def raw(steps, *, clock_kind="fake", setup=(1000.5, 1000.5)):
    return {
        "scenario": "t",
        "backend": "mock",
        "clock_kind": clock_kind,
        "root": ROOT,
        "sandbox_parent": PARENT,
        "setup_window": list(setup),
        "steps": steps,
    }


# ---------------------------------------------------------------------------
# ids


def test_symbols_first_appearance_and_seeds():
    s = Symbols(seed={ROOT: "ROOT"})
    assert s.replace(f"x {B} y {A} {B}") == "x <N1> y <N2> <N1>"
    assert s.replace(ROOT) == "<ROOT>"


def test_symbols_forms():
    s = Symbols()
    assert s.replace(A) == "<N1>"
    assert s.replace(A.upper()) == "<N1:UPPER>"
    assert s.replace(A[:10].upper() + A[10:]) == "<N1:MIXED>"
    assert s.replace(A.replace("-", "")) == "<N1:dashless>"
    short = A.replace("-", "")[-12:]
    assert s.replace(short) == "<N1:short>"
    assert s.replace(short.upper()) == "<N1:short:UPPER>"
    assert s.replace(f"https://workflowy.com/#/{short}/x") == "https://workflowy.com/#/<N1:short>/x"
    assert s.replace(f"Location '{short} ' not found") == "Location '<N1:short> ' not found"


def test_symbols_unknown_short_left_alone_and_digits_only_case():
    s = Symbols()
    assert s.replace("000000000003") == "000000000003"
    # an all-digit uuid has no case to report
    u = "12345678-1234-4123-8123-123456789012"
    assert s.replace(u) == "<N1>"
    assert s.replace(u.upper()) == "<N1>"


def test_symbols_alias_applies_before_alias_step():
    steps = [
        req("POST", "/api/v1/nodes", {"item_id": A}, epoch=1, t0=1002.5, json_body={"parent_id": ROOT, "name": "x"}),
        {"type": "alias", "id": A, "name": "P"},
        req("GET", f"/api/v1/nodes/{B}", {"code": "not_found"}, epoch=1, t0=1002.5),
    ]
    n = normalize(raw(steps))
    assert n[0]["body"] == {"item_id": "<P>"}
    assert n[0]["json"] == {"parent_id": "<ROOT>", "name": "x"}
    assert n[1]["request"] == "GET /api/v1/nodes/<N1>"
    assert len(n) == 2  # alias steps are dropped


def test_ids_in_error_messages_and_inputs():
    short = A.replace("-", "")[-12:]
    steps = [
        req("POST", "/api/v1/nodes", {"item_id": A}, epoch=1, t0=1002.5),
        req(
            "GET",
            "/api/v1/nodes",
            {"code": "not_found", "message": f"Location '{A.replace('-', '')}' not found"},
            epoch=1,
            t0=1002.5,
            status=404,
        ),
        req(
            "POST",
            f"/api/v1/nodes/{A}/mirror",
            {"errors": [{"type": "missing", "loc": ["parent_id"], "msg": "Field required", "input": {"x": short}}]},
            epoch=1,
            t0=1002.5,
            status=400,
        ),
    ]
    n = normalize(raw(steps))
    assert n[1]["body"]["message"] == "Location '<N1:dashless>' not found"
    assert n[2]["body"]["errors"][0]["input"] == {"x": "<N1:short>"}


# ---------------------------------------------------------------------------
# timestamps


def test_fake_clock_epochs():
    steps = [
        req("POST", "/api/v1/nodes", {"item_id": A}, epoch=1, t0=1002.5),
        {"type": "sleep", "seconds": 2.0, "epoch": 2},
        req("GET", f"/api/v1/nodes/{A}", {"node": node(A, ROOT, 1002, 1004, 1004)}, epoch=2, t0=1004.5),
        req("GET", f"/api/v1/nodes/{ROOT}", {"node": node(ROOT, PARENT, 1000, 1000, priority=28)}, epoch=2, t0=1004.5),
    ]
    n = normalize(raw(steps))
    assert n[1] == {"sleep": "-> E2"}
    a = n[2]["body"]["node"]
    assert (a["createdAt"], a["modifiedAt"], a["completedAt"]) == ("<E1>", "<E2>", "<E2>")
    r = n[3]["body"]["node"]
    assert r["priority"] == "<ROOT_PRIORITY>"
    assert r["parent_id"] == "<SANDBOX_PARENT>"
    assert (r["createdAt"], r["modifiedAt"]) == ("<E0>", "<E0>")


def test_timestamps_in_requests_untouched():
    body = {"parent_id": ROOT, "id": A, "createdAt": 5, "modifiedAt": 5, "priority": 1, "name": "x"}
    steps = [req("POST", "/api/v1/nodes", {"item_id": B}, epoch=1, t0=1002.5, json_body=body)]
    n = normalize(raw(steps))
    assert n[0]["json"]["createdAt"] == 5
    assert n[0]["json"]["priority"] == 1


def test_real_clock_tolerates_boundary_crossing_within_epoch():
    # Epoch 1 spans 1002.8 .. 1003.3 local, so the server may stamp 1002 or
    # 1003; both must map to E1. Server clock == local clock (Date agrees).
    steps = [
        req("POST", "/api/v1/nodes", {"item_id": A}, epoch=1, t0=1002.8, t1=1002.9, date=1002.85),
        req("POST", "/api/v1/nodes", {"item_id": B}, epoch=1, t0=1003.2, t1=1003.3, date=1003.25),
        req("GET", f"/api/v1/nodes/{A}", {"node": node(A, ROOT, 1002, 1002)}, epoch=1, t0=1003.3, t1=1003.4, date=1003.35),
        req("GET", f"/api/v1/nodes/{B}", {"node": node(B, ROOT, 1003, 1003)}, epoch=1, t0=1003.4, t1=1003.5, date=1003.45),
        {"type": "sleep", "seconds": 2.0, "epoch": 2},
        req("POST", f"/api/v1/nodes/{A}", {"status": "ok"}, epoch=2, t0=1005.6, t1=1005.7, date=1005.65),
        req("GET", f"/api/v1/nodes/{A}", {"node": node(A, ROOT, 1002, 1005)}, epoch=2, t0=1005.7, t1=1005.8, date=1005.75),
    ]
    n = normalize(raw(steps, clock_kind="real", setup=(1000.0, 1000.3)))
    assert n[2]["body"]["node"]["createdAt"] == "<E1>"
    assert n[3]["body"]["node"]["createdAt"] == "<E1>"
    assert n[6]["body"]["node"]["modifiedAt"] == "<E2>"
    assert n[6]["body"]["node"]["createdAt"] == "<E1>"


def test_real_clock_skew_from_date_header():
    # The server clock runs 0.7 s ahead of ours. Without skew correction the
    # E2 stamp (local 1005.4 -> server 1006.1 -> 1006) would be unmatched.
    skew = 0.7
    steps = []
    for i, t in enumerate([1002.1, 1002.4, 1002.6, 1002.95]):
        steps.append(req("GET", f"/api/v1/nodes/{A}", {"x": 1}, epoch=1, t0=t, t1=t + 0.05, date=t + 0.02 + skew))
    steps.append({"type": "sleep", "seconds": 2.0, "epoch": 2})
    steps.append(
        req("GET", f"/api/v1/nodes/{A}", {"node": node(A, ROOT, 1002, 1006)}, epoch=2, t0=1005.35, t1=1005.4, date=1005.37 + skew)
    )
    c = Clock(raw(steps, clock_kind="real", setup=(1000.0, 1000.1)))
    assert c.skew_interval is not None
    lo, hi = c.skew_interval
    assert lo <= skew <= hi
    n = normalize(raw(steps, clock_kind="real", setup=(1000.0, 1000.1)))
    assert n[-1]["body"]["node"]["modifiedAt"] == "<E2>"


def test_unmatched_timestamp_is_flagged():
    steps = [req("GET", f"/api/v1/nodes/{A}", {"node": node(A, ROOT, 900, 1002)}, epoch=1, t0=1002.5)]
    n = normalize(raw(steps))
    assert n[0]["body"]["node"]["createdAt"].startswith("<E?")
    assert n[0]["body"]["node"]["modifiedAt"] == "<E1>"


# ---------------------------------------------------------------------------
# response metadata / rendering / diff


def test_json_bytes_flag_and_text_bodies():
    good = req("GET", "/x", {"a": "é"}, epoch=1, t0=1002.5)
    bad = dict(good, body_text='{"a":"é"}')
    html = dict(good, body_text="<!DOCTYPE html>", body_is_json=False, status=404,
                response_headers={"content-type": "text/html; charset=utf-8"})
    n = normalize(raw([good, bad, html]))
    assert n[0]["json_bytes"] == "python-default"
    assert n[1]["json_bytes"] == "non-default"
    assert n[2]["text"] == "<HTML page title=None>"
    assert n[2]["content-type"] == "text/html; charset=utf-8"


def test_compared_headers_only():
    st = req("GET", "/x", {"a": 1}, epoch=1, t0=1002.5, headers={"allow": "POST", "server": "nginx", "x-foo": "1"})
    n = normalize(raw([st]))[0]
    assert n["allow"] == "POST"
    assert "server" not in n and "x-foo" not in n


def test_diff_and_render():
    a = normalize(raw([req("GET", f"/api/v1/nodes/{A}", {"node": node(A, ROOT, 1002, 1002)}, epoch=1, t0=1002.5)]))
    b = normalize(raw([req("GET", f"/api/v1/nodes/{B}", {"node": node(B, ROOT, 1002, 1002, priority=50)}, epoch=1, t0=1002.5)]))
    assert diff_transcripts(a, a) == ""
    d = diff_transcripts(a, b)
    assert "-        \"priority\": 100," in d and "+        \"priority\": 50," in d
    assert render(a)[0] == "## [0] GET /api/v1/nodes/<N1>"


def test_fake_clock():
    c = FakeClock(10.5)
    assert c() == 10.5
    c.advance(2)
    assert c() == 12.5
    assert FakeClock()() % 1 == 0.5


# ---------------------------------------------------------------------------
# production safety vetting (no network)


class _StubClient:
    api_key = "k"


@pytest.fixture
def prod():
    b = ProdBackend(client=_StubClient())
    b.ours = {ROOT, A}
    return b


def _short(u):
    return u.replace("-", "")[-12:]


@pytest.mark.parametrize(
    "method,path,kw",
    [
        ("GET", f"/api/v1/nodes/{A}", {}),
        ("GET", f"/api/v1/nodes/{_short(A).upper()}", {}),
        ("GET", "/api/v1/nodes/garbage", {}),
        ("GET", "/api/v1/nodes", {"params": [("parent_id", A), ("parent_id", ROOT)]}),
        ("GET", f"/api/v1/nodes?parent_id={A}", {}),
        ("POST", "/api/v1/nodes", {"json": {"parent_id": f"https://workflowy.com/#/{_short(A)}", "name": "x"}}),
        ("POST", "/api/v1/nodes", {"json": {"parent_id": "00000000-0000-4000-8000-000000000001"}}),
        ("POST", "/api/v1/nodes", {"json": {"parent_id": 1}}),
        ("POST", "/api/v1/nodes", {"content": "{bad"}),
        ("POST", "/api/v1/nodes", {"content": "[1]"}),
        ("POST", "/api/v1/nodes", {"content": f"parent_id={A}&name=x",
                                   "headers": {"Content-Type": "application/x-www-form-urlencoded"}}),
        ("POST", f"/api/v1/nodes/{A}/move", {"json": {"parent_id": "zz-conformance nope"}}),
        ("POST", f"/api/v1/nodes/{A}/mirror", {"json": {}}),
        ("POST", f"/api/v1/nodes/{A}/mirror", {"json": {"parent_id": ""}}),
        ("POST", f"/api/v1/nodes/{A}", {"json": {}}),
    ],
)
def test_vet_allows(prod, method, path, kw):
    prod._vet(method, path, kw.get("params"), kw.get("json"), kw.get("content"), kw.get("headers"))


@pytest.mark.parametrize(
    "method,path,kw",
    [
        ("GET", f"/api/v1/nodes/{B}", {}),  # not ours
        ("GET", "/api/v1/nodes/inbox", {}),
        ("GET", "/api/v1/nodes", {}),  # root listing
        ("GET", "/api/v1/nodes", {"params": {"parent_id": ""}}),
        ("GET", "/api/v1/nodes", {"params": [("parent_id", A), ("parent_id", B)]}),
        ("GET", "/api/v1/nodes-export", {}),
        ("GET", "/api/v1/targets", {}),
        ("POST", "/api/v1/nodes", {"json": {"name": "x"}}),
        ("POST", "/api/v1/nodes", {"json": {"name": "x", "parent_id": None}}),
        ("POST", "/api/v1/nodes", {"json": {"name": "x", "parent_id": ""}}),
        ("POST", "/api/v1/nodes", {"json": {"name": "x", "parent_id": "None"}}),
        ("POST", "/api/v1/nodes", {"json": {"name": "x", "parent_id": "2026-01-15"}}),
        ("POST", "/api/v1/nodes", {"json": {"name": "x", "parent_id": "someshortcut"}}),
        ("POST", "/api/v1/nodes", {"content": "name=x",
                                   "headers": {"Content-Type": "application/x-www-form-urlencoded"}}),
        ("POST", f"/api/v1/nodes/{A}/move", {"json": {}}),
        ("POST", f"/api/v1/nodes/{A}/move", {"json": {"parent_id": None}}),
        ("POST", f"/api/v1/nodes/{A}/move", {"json": {"parent_id": f"https://workflowy.com/#/{_short(B)}"}}),
        ("POST", f"/api/v1/nodes/{B}", {"json": {"name": "x"}}),
    ],
)
def test_vet_refuses(prod, method, path, kw):
    with pytest.raises(UnsafeRequest):
        prod._vet(method, path, kw.get("params"), kw.get("json"), kw.get("content"), kw.get("headers"))


def test_symbols_after_percent_escape():
    s = Symbols()
    s.replace(A)
    short = A.replace("-", "")[-12:]
    assert s.replace(f"/api/v1/nodes/x%2F{short}") == "/api/v1/nodes/x%2F<N1:short>"


def test_html_pages_are_summarized():
    page = (
        '<!DOCTYPE html>\n<html lang="en-US">\n<head>\n  <title>404</title>\n'
        '<script nonce="%s">x</script></head></html>'
    )
    steps = []
    for nonce in ("WqQgOmhonxvvDbvDrfEqHQ==", "mucpldfeQ5lhmMV01XhI1A=="):
        st = req("GET", f"/api/v1/nodes/{A}/children", None, epoch=1, t0=1002.5, status=404,
                 headers={"content-type": "text/html; charset=utf-8"})
        st["body_text"] = page % nonce
        steps.append(st)
    st = req("POST", "/api/v1/nope", None, epoch=1, t0=1002.5, status=404)
    steps.append(st)
    out = normalize(raw(steps))
    assert out[0]["text"] == out[1]["text"] == "<HTML page title='404'>"
    assert out[2]["text"] == ""


# ------------------------------------------------------------------ recorder guard


def test_ambiguous_aliases_flags_all_digit_short_ids() -> None:
    """An all-digit short id is its own upper case, so the normaliser cannot label
    ``short(p).upper()`` consistently: the recorder must re-run such a scenario."""
    digits = "5e2e76e4-fea4-4f1f-9c5b-012345678901"
    raw = {
        "steps": [
            {"type": "alias", "id": A, "name": "A"},
            {"type": "alias", "id": digits, "name": "P"},
            {"type": "request", "method": "GET", "path": "/api/v1/nodes"},
        ]
    }
    assert ambiguous_aliases(raw) == [f"P={digits}"]
    raw["steps"][1]["id"] = B
    assert ambiguous_aliases(raw) == []
    # The ambiguity itself, as the normaliser sees it: the two forms collapse.
    syms = Symbols(aliases={digits: "P"})
    assert syms.replace(digits[-12:]) == syms.replace(digits[-12:].upper()) == "<P:short>"
    syms = Symbols(aliases={A: "A"})
    assert syms.replace(A[-12:]) == "<A:short>" and syms.replace(A[-12:].upper()) == "<A:short:UPPER>"
