"""HTTP layer: auth, 405 styles, unknown routes, body parsing, validation, serialisation (§2)."""

from ..registry import scenario
from ._util import FAKE_UUID, node_sandbox

BAD = {"Authorization": "Bearer not-a-real-key-0000"}


@scenario
def auth_bad_key_shapes(ctx):
    """401 body depends on method: GET -> {"errors": ...}; POST/DELETE -> {code, message} (§2.3)."""
    a = ctx.alias(ctx.create(ctx.root, "a"), "A")
    ctx.req("GET", f"/api/v1/nodes/{a}", headers=BAD)
    ctx.req("GET", "/api/v1/nodes", params={"parent_id": ctx.root}, headers=BAD)
    ctx.req("POST", "/api/v1/nodes", json={"parent_id": ctx.root, "name": "x"}, headers=BAD)
    ctx.req("POST", f"/api/v1/nodes/{a}", json={"name": "x"}, headers=BAD)
    ctx.req("DELETE", f"/api/v1/nodes/{a}", headers=BAD)
    ctx.req("POST", f"/api/v1/nodes/{a}/move", json={"parent_id": ctx.root}, headers=BAD)
    ctx.req("POST", f"/api/v1/nodes/{a}/complete", headers=BAD)
    ctx.req("POST", f"/api/v1/nodes/{a}/uncomplete", headers=BAD)
    ctx.req("POST", f"/api/v1/nodes/{a}/mirror", json={"parent_id": ctx.root}, headers=BAD)
    ctx.req("DELETE", f"/api/v1/nodes/{a}/mirror", headers=BAD)
    ctx.get(a)
    ctx.list(ctx.root)


@scenario
def auth_header_variants(ctx):
    """The scheme word is ignored; the second whitespace token is the key (§2.3)."""
    a = ctx.alias(ctx.create(ctx.root, "a"), "A")
    for h in (
        "Bearer {API_KEY}",
        "bearer {API_KEY}",
        "Token {API_KEY}",
        "Bearer  {API_KEY}",
        "{API_KEY}",
        "Basic dXNlcjpwYXNz",
        "Bearer",
        "Bearer {API_KEY} extra",
        "BEARER {API_KEY}",
    ):
        ctx.req("GET", f"/api/v1/nodes/{a}", headers={"Authorization": h})


@scenario
def auth_precedence(ctx):
    """Routing and method checks precede auth; auth precedes body parsing and lookup (§2.2)."""
    a = ctx.alias(ctx.create(ctx.root, "a"), "A")
    ctx.req("GET", "/api/v1/nope", headers=BAD)
    ctx.req("PUT", f"/api/v1/nodes/{a}", headers=BAD)
    ctx.req("GET", f"/api/v1/nodes/{a}/move", headers=BAD)
    ctx.req("POST", f"/api/v1/nodes/{a}", content="{bad", headers={**BAD, "Content-Type": "application/json"})
    ctx.req("GET", f"/api/v1/nodes/{FAKE_UUID}", headers=BAD)
    ctx.req("POST", f"/api/v1/nodes/{FAKE_UUID}", json={"name": 1}, headers=BAD)


@scenario
def http_405_json_routes(ctx):
    """JSON 405s with allowed_methods (no Allow header) on /nodes, /nodes/:id, /nodes/:id/mirror (§2.1)."""
    a = ctx.alias(ctx.create(ctx.root, "a"), "A")
    for m in ("PUT", "PATCH", "OPTIONS", "HEAD"):
        ctx.req(m, f"/api/v1/nodes/{a}")
    for m in ("PUT", "PATCH", "DELETE", "HEAD"):
        ctx.req(m, "/api/v1/nodes", params={"parent_id": ctx.root})
    ctx.req("GET", f"/api/v1/nodes/{a}/mirror")
    ctx.req("HEAD", f"/api/v1/nodes/{a}/mirror")
    ctx.get(a)


@scenario
def http_405_html_routes(ctx):
    """HTML-empty 405s with Allow: POST on /move, /complete, /uncomplete (§2.1)."""
    a = ctx.alias(ctx.create(ctx.root, "a"), "A")
    for suffix in ("move", "complete", "uncomplete"):
        for m in ("GET", "DELETE", "HEAD", "OPTIONS"):
            ctx.req(m, f"/api/v1/nodes/{a}/{suffix}")
    ctx.get(a)


@scenario
def http_unknown_routes(ctx):
    """Unknown routes: GET gets an HTML 404 page, other methods an empty HTML 404 (§2.1)."""
    a = ctx.alias(ctx.create(ctx.root, "a"), "A")
    for path in (
        "/api/v1/foo",
        "/api/v1/",
        "/api/v1",
        "/api/v2/nodes",
        "/api/v1/me",
        "/api/v1/trash",
        f"/api/v1/nodes/{a}/children",
        f"/api/v1/nodes/{a}/restore",
        f"/api/v1/nodes/{a}/completed",
    ):
        ctx.req("GET", path)
    ctx.req("POST", "/api/v1/foo", json={})
    ctx.req("POST", f"/api/v1/nodes/{a}/restore")
    ctx.req("DELETE", f"/api/v1/nodes/{a}/children")


@scenario
def create_bad_bodies(ctx):
    """Create with invalid JSON / empty / non-object bodies (§2.4)."""
    jh = {"Content-Type": "application/json"}
    for body in ("{bad", "", "[1]", '"s"', "null", "3", '[{"name": "x"}]'):
        ctx.req("POST", "/api/v1/nodes", content=body, headers=jh)
    ctx.list(ctx.root)


@scenario
def create_validation_errors(ctx):
    """Create validation: all errors in declaration order; missing echoes the whole body (§2.5)."""
    p = node_sandbox(ctx)
    ctx.req("POST", "/api/v1/nodes", json={"parent_id": p, "name": 1, "note": 2, "position": 3})
    ctx.req("POST", "/api/v1/nodes", json={"position": "q", "layoutMode": 3, "note": 2, "name": 1, "parent_id": 1})
    ctx.req("POST", "/api/v1/nodes", json={"parent_id": 1})
    ctx.req("POST", "/api/v1/nodes", json={"parent_id": p})
    ctx.req("POST", "/api/v1/nodes", json={"parent_id": p, "name": None})
    ctx.req("POST", "/api/v1/nodes", json={"parent_id": p, "note": "n", "extra": [1, {"a": None}]})
    ctx.req("POST", "/api/v1/nodes", json={"parent_id": p, "name": ["x"], "layoutMode": {"a": 1}})
    ctx.req("POST", "/api/v1/nodes", json={"parent_id": p, "name": 1.5, "note": None, "position": None})
    ctx.list(p)


@scenario
def create_validation_before_lookup(ctx):
    """Create with a nonexistent parent: validation errors win over 404 (§2.2)."""
    ctx.req("POST", "/api/v1/nodes", json={"parent_id": FAKE_UUID})
    ctx.req("POST", "/api/v1/nodes", json={"parent_id": FAKE_UUID, "name": "x", "position": "middle"})
    ctx.req("POST", "/api/v1/nodes", json={"parent_id": FAKE_UUID, "name": "x"})
    ctx.req("POST", "/api/v1/nodes", content="{bad", headers={"Content-Type": "application/json"})


@scenario
def http_json_serialisation(ctx):
    """Responses are Python json.dumps defaults (', ' separators, ensure_ascii) (§1.3)."""
    p = node_sandbox(ctx)
    a = ctx.alias(ctx.create(p, "café \U0001F600 ☃ / \\ \" '", note="</script> é"), "A")
    ctx.get(a)
    ctx.list(p)
    ctx.req("POST", "/api/v1/nodes", json={"parent_id": p, "name": "é", "position": "é"})


@scenario
def http_accept_header_ignored(ctx):
    """The Accept header is ignored (§2.7)."""
    a = ctx.alias(ctx.create(ctx.root, "a"), "A")
    ctx.req("GET", f"/api/v1/nodes/{a}", headers={"Accept": "text/html"})
    ctx.req("GET", f"/api/v1/nodes/{FAKE_UUID}", headers={"Accept": "application/xml"})
