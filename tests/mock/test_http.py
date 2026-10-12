"""HTTP layer: routing, 404/405 styles, auth, check order, body parsing, serialisation.

Spec: data-model.md §1.3, §2.1-2.4, §2.6-2.7; errors-and-http.md §1, §4-6.
"""

from __future__ import annotations

import json

import httpx
import pytest

from .conftest import WF, rand_uuid

NOT_FOUND_ITEM = b'{"code": "not_found", "message": "Item not found"}'
AUTH_GET = b'{"errors": "Invalid Credentials, try again."}'
AUTH_POST = b'{"code": "invalid_credentials", "message": "Invalid Credentials, try again."}'
INVALID_JSON = b'{"code": "invalid_json", "message": "Invalid JSON format in request body"}'


# ---------------------------------------------------------------- serialisation


def test_node_key_order_and_bytes(wf: WF) -> None:
    sb = wf.sandbox()
    nid = wf.create(sb, "hello world")
    r = wf.get_raw(nid)
    assert r.headers["content-type"] == "application/json"
    expected = (
        '{"node": {"id": "%s", "name": "hello world", "note": null, "parent_id": "%s", "priority": 100, '
        '"completed": false, "data": {"layoutMode": "bullets"}, "createdAt": %d, "modifiedAt": %d, '
        '"completedAt": null}}' % (nid, sb, wf.clock.now, wf.clock.now)
    )
    assert r.text == expected


def test_create_response_shape(wf: WF) -> None:
    r = wf.create_raw({"parent_id": "None", "name": "x"})
    assert r.status_code == 200
    body = r.json()
    assert list(body) == ["item_id"]
    assert r.content == f'{{"item_id": "{body["item_id"]}"}}'.encode()
    # lowercase hyphenated uuid4
    assert len(body["item_id"]) == 36 and body["item_id"] == body["item_id"].lower()


def test_status_ok_is_16_bytes(wf: WF) -> None:
    sb = wf.sandbox()
    r = wf.update_raw(sb, {})
    assert r.status_code == 200
    assert r.content == b'{"status": "ok"}'
    assert r.headers["content-length"] == "16"


def test_ensure_ascii_serialisation(wf: WF) -> None:
    sb = wf.sandbox()
    nid = wf.create(sb, "x")
    wf.update(nid, name='nöde ☃ "quoted" </script> 😀')
    r = wf.get_raw(nid)
    assert '"name": "n\\u00f6de \\u2603 \\"quoted\\" </script> \\ud83d\\ude00"' in r.text


def test_fidelity_headers(wf: WF) -> None:
    r = wf.list_raw()
    assert r.headers["x-frame-options"] == "SAMEORIGIN"
    assert r.headers["server"] == "nginx"
    for h in ("retry-after", "etag", "last-modified", "access-control-allow-origin", "www-authenticate"):
        assert h not in r.headers


def test_accept_header_ignored(wf: WF) -> None:
    r = wf.http.get("/api/v1/nodes", headers={"Accept": "text/html"})
    assert r.status_code == 200 and r.headers["content-type"] == "application/json"


# ---------------------------------------------------------------- routing


@pytest.mark.parametrize(
    "path",
    ["/api/v1/foo", "/api/v1/", "/api/v1", "/api/v2/nodes", "/api/v1/me", "/api/v1/trash", "/",
     "/api/v1/nodes/abc/children", "/api/v1/nodes/abc/restore", "/api/v1/nodes/abc/mirror/x"],
)
def test_unknown_route_get_is_html_page(wf: WF, path: str) -> None:
    r = wf.http.get(path)
    assert r.status_code == 404
    assert r.headers["content-type"] == "text/html; charset=utf-8"
    assert r.text.startswith("<!DOCTYPE html>") and "<title>404</title>" in r.text


@pytest.mark.parametrize("method", ["POST", "DELETE", "PUT"])
def test_unknown_route_with_body_is_empty(wf: WF, method: str) -> None:
    r = wf.http.request(method, "/api/v1/foo", json={"x": 1})
    assert r.status_code == 404
    assert r.headers["content-type"] == "text/html; charset=utf-8"
    assert r.content == b""


@pytest.mark.parametrize("method", ["POST", "DELETE"])
def test_unknown_route_without_body_is_page(wf: WF, method: str) -> None:
    # e.g. production: bodiless POST /nodes/<id>/restore -> full 404 page
    r = wf.http.request(method, "/api/v1/nodes/abc/restore")
    assert r.status_code == 404 and "<title>404</title>" in r.text


def test_unknown_route_beats_auth(wf: WF) -> None:
    r = wf.anon.get("/api/v1/foo")
    assert r.status_code == 404 and r.headers["content-type"].startswith("text/html")


def test_percent_encoded_slash_in_id_is_unknown_route(wf: WF) -> None:
    sb = wf.sandbox()
    url = "https%3A%2F%2Fworkflowy.com%2F%23%2F" + sb[-12:]
    r = wf.http.get(f"/api/v1/nodes/{url}")
    assert r.status_code == 404 and r.headers["content-type"].startswith("text/html")
    r = wf.http.post(f"/api/v1/nodes/{url}", json={})
    assert r.status_code == 404 and r.content == b""


def test_trailing_slash_accepted(wf: WF) -> None:
    sb = wf.sandbox()
    assert wf.http.get("/api/v1/nodes/", params={"parent_id": sb}).status_code == 200
    r = wf.http.post("/api/v1/nodes/", json={"parent_id": sb, "name": "via slash"})
    assert r.status_code == 200
    nid = r.json()["item_id"]
    assert wf.http.get(f"/api/v1/nodes/{nid}/").status_code == 200
    wf.ok(wf.http.post(f"/api/v1/nodes/{nid}/", json={"note": "n"}))
    wf.ok(wf.http.post(f"/api/v1/nodes/{nid}/complete/"))
    assert wf.get(nid)["completed"] is True


@pytest.mark.parametrize(
    "method,path,allowed",
    [
        ("PUT", "/api/v1/nodes", ["GET", "POST"]),
        ("PATCH", "/api/v1/nodes", ["GET", "POST"]),
        ("DELETE", "/api/v1/nodes", ["GET", "POST"]),
        ("OPTIONS", "/api/v1/nodes", ["GET", "POST"]),
        ("PUT", "/api/v1/nodes/X", ["GET", "POST", "DELETE"]),
        ("PATCH", "/api/v1/nodes/X", ["GET", "POST", "DELETE"]),
        ("OPTIONS", "/api/v1/nodes/X", ["GET", "POST", "DELETE"]),
        ("GET", "/api/v1/nodes/X/mirror", ["POST", "DELETE"]),
        ("PUT", "/api/v1/nodes/X/mirror", ["POST", "DELETE"]),
    ],
)
def test_json_405(wf: WF, method: str, path: str, allowed: list[str]) -> None:
    r = wf.anon.request(method, path)  # method check precedes auth
    assert r.status_code == 405
    assert r.headers["content-type"] == "application/json"
    assert "allow" not in r.headers
    assert r.content == json.dumps({"error": "Method not allowed", "allowed_methods": allowed}).encode()


def test_json_405_head_has_no_body(wf: WF) -> None:
    r = wf.anon.head("/api/v1/nodes/X")
    assert r.status_code == 405
    assert r.content == b""
    assert r.headers["content-length"] == "77"
    r = wf.anon.head("/api/v1/nodes")
    assert r.headers["content-length"] == "67"


@pytest.mark.parametrize(
    "method,path,allow",
    [
        ("GET", "/api/v1/nodes/X/move", "POST"),
        ("DELETE", "/api/v1/nodes/X/move", "POST"),
        ("PUT", "/api/v1/nodes/X/move", "POST"),
        ("GET", "/api/v1/nodes/X/complete", "POST"),
        ("PUT", "/api/v1/nodes/X/complete", "POST"),
        ("GET", "/api/v1/nodes/X/uncomplete", "POST"),
        ("POST", "/api/v1/targets", "GET, HEAD"),
    ],
)
def test_html_405(wf: WF, method: str, path: str, allow: str) -> None:
    r = wf.anon.request(method, path)
    assert r.status_code == 405
    assert r.headers["content-type"] == "text/html; charset=utf-8"
    assert r.headers["allow"] == allow
    assert r.content == b"" and r.headers["content-length"] == "0"


# ---------------------------------------------------------------- auth


@pytest.mark.parametrize(
    "header",
    [None, "Bearer not-a-real-key", "Bearer", "test-key", "Basic dXNlcjpwYXNz", "Bearer test-key-x"],
)
def test_auth_rejected(wf: WF, header: str | None) -> None:
    headers = {} if header is None else {"Authorization": header}
    r = wf.anon.get(f"/api/v1/nodes/{rand_uuid()}", headers=headers)
    assert r.status_code == 401 and r.content == AUTH_GET
    assert "www-authenticate" not in r.headers
    r = wf.anon.post("/api/v1/nodes", json={"name": "x"}, headers=headers)
    assert r.status_code == 401 and r.content == AUTH_POST
    r = wf.anon.delete(f"/api/v1/nodes/{rand_uuid()}", headers=headers)
    assert r.status_code == 401 and r.content == AUTH_POST


@pytest.mark.parametrize("header", ["Bearer test-key", "bearer test-key", "Token test-key", "Bearer  test-key"])
def test_auth_accepted_any_scheme(wf: WF, header: str) -> None:
    r = wf.anon.get(f"/api/v1/nodes/{rand_uuid()}", headers={"Authorization": header})
    assert r.status_code == 404 and r.content == NOT_FOUND_ITEM


def test_auth_401_for_list_export_targets_is_get_shape(wf: WF) -> None:
    for path in ("/api/v1/nodes", "/api/v1/nodes-export", "/api/v1/targets"):
        r = wf.anon.get(path)
        assert r.status_code == 401 and r.content == AUTH_GET
        assert r.headers["content-length"] == "45"


def test_check_order_auth_before_body_and_lookup(wf: WF) -> None:
    r = wf.anon.post(f"/api/v1/nodes/{rand_uuid()}", content=b"{bad")
    assert r.status_code == 401 and r.content == AUTH_POST


def test_check_order_body_before_lookup(wf: WF) -> None:
    r = wf.update_raw(rand_uuid(), content=b"{bad")
    assert r.status_code == 400 and r.content == INVALID_JSON
    r = wf.update_raw(rand_uuid(), {"name": 123})
    assert r.status_code == 400 and r.json()["errors"][0]["type"] == "string_type"


def test_extra_api_keys() -> None:
    from syncflowy.mock.testing import running_mock

    with running_mock(api_key="a", extra_api_keys=("b",)) as url:
        for k in ("a", "b"):
            assert httpx.get(f"{url}/api/v1/nodes", headers={"Authorization": f"Bearer {k}"}).status_code == 200
        assert httpx.get(f"{url}/api/v1/nodes", headers={"Authorization": "Bearer c"}).status_code == 401


# ---------------------------------------------------------------- body parsing


@pytest.mark.parametrize("ctype", ["application/json", "application/json; charset=utf-8", "text/plain", None])
def test_json_parsed_regardless_of_content_type(wf: WF, ctype: str | None) -> None:
    sb = wf.sandbox()
    headers = {} if ctype is None else {"Content-Type": ctype}
    r = wf.update_raw(sb, content=json.dumps({"note": "n"}).encode(), headers=headers)
    wf.ok(r)
    assert wf.get(sb)["note"] == "n"


def test_form_urlencoded_body(wf: WF) -> None:
    sb = wf.sandbox()
    r = wf.create_raw(data={"parent_id": sb, "name": "form-created", "position": "bottom"})
    assert r.status_code == 200
    nid = r.json()["item_id"]
    assert wf.get(nid)["name"] == "form-created"
    wf.ok(wf.update_raw(nid, data={"note": "form-encoded-note"}))
    assert wf.get(nid)["note"] == "form-encoded-note"
    other = wf.sandbox("other")
    wf.ok(wf.move_raw(nid, data={"parent_id": other}))
    assert wf.get(nid)["parent_id"] == other


def test_multipart_body(wf: WF) -> None:
    sb = wf.sandbox()
    r = wf.update_raw(sb, files={"note": (None, "multipart note")})
    wf.ok(r)
    assert wf.get(sb)["note"] == "multipart note"


@pytest.mark.parametrize("body", [b"", b"{", b'{"note": ', b"nope", b"\xff\xfe\x00garbage"])
def test_invalid_json(wf: WF, body: bytes) -> None:
    sb = wf.sandbox()
    for path in (f"/api/v1/nodes/{sb}", "/api/v1/nodes", f"/api/v1/nodes/{sb}/move", f"/api/v1/nodes/{sb}/mirror"):
        r = wf.http.post(path, content=body, headers={"Content-Type": "application/json"})
        assert r.status_code == 400 and r.content == INVALID_JSON, path


def test_duplicate_keys_last_wins(wf: WF) -> None:
    sb = wf.sandbox()
    wf.ok(wf.update_raw(sb, content=b'{"note": "first", "note": "second"}'))
    assert wf.get(sb)["note"] == "second"


def test_query_string_never_contributes_to_post(wf: WF) -> None:
    sb = wf.sandbox()
    wf.ok(wf.http.post(f"/api/v1/nodes/{sb}", params={"note": "x"}, json={}))
    assert wf.get(sb)["note"] is None
    r = wf.http.post("/api/v1/nodes", params={"name": "q"}, json={"parent_id": sb, "name": "b"})
    assert wf.get(r.json()["item_id"])["name"] == "b"


def test_complete_uncomplete_delete_ignore_body(wf: WF) -> None:
    sb = wf.sandbox()
    nid = wf.create(sb)
    wf.ok(wf.http.post(f"/api/v1/nodes/{nid}/complete", content=b"{garbage"))
    wf.ok(wf.http.post(f"/api/v1/nodes/{nid}/uncomplete", content=b"[1,2"))
    wf.ok(wf.http.request("DELETE", f"/api/v1/nodes/{nid}", content=b"{garbage"))


def test_large_body_accepted(wf: WF) -> None:
    sb = wf.sandbox()
    big = "x" * 1_000_000
    wf.update(sb, name=big, note=big)
    n = wf.get(sb)
    assert n["name"] == big and n["note"] == big


def test_list_repeated_param_last_wins_and_others_ignored(wf: WF) -> None:
    a, b = wf.sandbox("a"), wf.sandbox("b")
    wf.create(b, "in b")
    r = wf.http.get("/api/v1/nodes", params=[("parent_id", a), ("parent_id", b), ("limit", "0"), ("depth", "5")])
    assert [n["name"] for n in r.json()["nodes"]] == ["in b"]


def test_internal_error_shape(wf: WF) -> None:
    sb = wf.sandbox()
    r = wf.create_raw({"parent_id": sb, "name": "x", "note": "a\x00b"})
    assert r.status_code == 500
    assert r.content == b'{"code": "internal_error", "message": "An internal server error occurred"}'
