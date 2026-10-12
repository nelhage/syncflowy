"""Id forms: path ids, parent_id resolution for list/create/move (§3)."""

from ..registry import scenario
from ._util import FAKE_SHORT, FAKE_UUID, dashless, node_sandbox, short, url


@scenario
def ids_get_path_forms(ctx):
    """GET accepts the exact UUID or a 12-hex short id (any case); upper/dashless UUIDs 404 (§3.1)."""
    a = ctx.alias(ctx.create(ctx.root, "a"), "A")
    for form in (a, short(a), short(a).upper(), a.upper(), dashless(a), dashless(a).upper(), a + "x", "x" + short(a), FAKE_SHORT, "garbage"):
        ctx.req("GET", f"/api/v1/nodes/{form}")


@scenario
def ids_mutations_need_exact_uuid(ctx):
    """update/complete/uncomplete/move/mirror/delete-mirror via short, upper or dashless id: 404, no effect (§3.1, open S8)."""
    a = ctx.alias(ctx.create(ctx.root, "a"), "A")
    p = node_sandbox(ctx)
    for form in (short(a), a.upper(), dashless(a)):
        ctx.req("POST", f"/api/v1/nodes/{form}", json={"name": "changed"})
        ctx.req("POST", f"/api/v1/nodes/{form}/complete")
        ctx.req("POST", f"/api/v1/nodes/{form}/uncomplete")
        ctx.req("POST", f"/api/v1/nodes/{form}/move", json={"parent_id": p})
        ctx.req("POST", f"/api/v1/nodes/{form}/mirror", json={"parent_id": p})
        ctx.req("DELETE", f"/api/v1/nodes/{form}/mirror")
    ctx.get(a)
    ctx.list(p)


@scenario
def ids_list_parent_forms_ok(ctx):
    """List parent_id accepts UUID, short (any case) and workflowy.com URL forms (§3.2)."""
    p = node_sandbox(ctx)
    ctx.create(p, "child")
    for pid in (
        p,
        short(p),
        short(p).upper(),
        url(p),
        url(p, host="http://workflowy.com"),
        url(p, host="https://beta.workflowy.com"),
        url(p, host="HTTPS://WORKFLOWY.COM"),
        url(p).upper(),
        url(p, suffix="/"),
        url(p, suffix="?q=x"),
        url(p, suffix=f"/{short(ctx.root)}"),
    ):
        ctx.list(pid)


@scenario
def ids_list_parent_forms_errors(ctx):
    """List parent_id misses: Parent item not found vs Location '…' not found (§3.2)."""
    p = node_sandbox(ctx)
    for pid in (
        p.upper(),
        FAKE_UUID,
        dashless(p),
        FAKE_SHORT,
        f"https://www.workflowy.com/#/{short(p)}",
        f"https://app.workflowy.com/#/{short(p)}",
        f"workflowy.com/#/{short(p)}",
        f"https://workflowy.com/#/x/{short(p)}",
        f"https://workflowy.com#/{short(p)}",
        f"https://workflowy.com/#/{p}",
        f"https://workflowy.com/#/{FAKE_SHORT}",
        f" {p}",
        f"{short(p)} ",
        "zz-conformance 'quoted' \"name\"",
    ):
        ctx.list(pid)


@scenario
def ids_create_parent_forms(ctx):
    """Create resolves parent_id like list (short, URL ok; Location/Parent errors) (§3.2)."""
    p = node_sandbox(ctx)
    for pid in (short(p), url(p), short(p).upper()):
        ctx.try_create(pid, f"via {pid}")
    for pid in (p.upper(), dashless(p), FAKE_SHORT, f"https://www.workflowy.com/#/{short(p)}", url(FAKE_UUID)):
        ctx.try_create(pid, "should fail")
    ctx.list(p)


@scenario
def ids_move_parent_forms(ctx):
    """Move resolves parent_id like list (short, URL ok; Location/Parent errors) (§3.2)."""
    p = node_sandbox(ctx, "P")
    q = node_sandbox(ctx, "Q")
    a = ctx.alias(ctx.create(p, "a"), "A")
    ctx.move(a, short(q))
    ctx.list(q)
    ctx.move(a, url(p))
    ctx.list(p)
    for pid in (q.upper(), dashless(q), FAKE_SHORT, f"https://www.workflowy.com/#/{short(q)}", url(FAKE_UUID)):
        ctx.move(a, pid)
    ctx.get(a)


@scenario
def ids_nonexistent_everywhere(ctx):
    """A never-existing UUID on every endpoint (§3, errors-and-http §3)."""
    ctx.get(FAKE_UUID)
    ctx.list(FAKE_UUID)
    ctx.try_create(FAKE_UUID, "x")
    ctx.update(FAKE_UUID, name="x")
    ctx.update(FAKE_UUID)
    ctx.delete(FAKE_UUID)
    ctx.complete(FAKE_UUID)
    ctx.uncomplete(FAKE_UUID)
    ctx.move(FAKE_UUID, ctx.root)
    ctx.move(ctx.root, FAKE_UUID)
    ctx.mirror(FAKE_UUID, ctx.root)
    ctx.delete_mirror(FAKE_UUID)


@scenario
def ids_trailing_slash(ctx):
    """All routes accept a trailing slash without redirect (§2.1)."""
    p = node_sandbox(ctx)
    r = ctx.req("POST", "/api/v1/nodes/", json={"parent_id": p, "name": "slash"})
    ctx.req("GET", "/api/v1/nodes/", params={"parent_id": p})
    if r.ok:
        a = ctx.alias(r.json["item_id"], "A")
        ctx.req("GET", f"/api/v1/nodes/{a}/")
        ctx.req("POST", f"/api/v1/nodes/{a}/", json={"note": "slash"})
        ctx.req("POST", f"/api/v1/nodes/{a}/complete/")
        ctx.req("POST", f"/api/v1/nodes/{a}/uncomplete/")
        ctx.req("POST", f"/api/v1/nodes/{a}/move/", json={"parent_id": p, "position": "bottom"})
        r2 = ctx.req("POST", f"/api/v1/nodes/{a}/mirror/", json={"parent_id": ctx.root})
        if r2.ok:
            ctx.req("DELETE", f"/api/v1/nodes/{r2.json['item_id']}/mirror/")
        ctx.get(a)
        ctx.req("DELETE", f"/api/v1/nodes/{a}/")
        ctx.get(a)


@scenario
def ids_url_encoded_path(ctx):
    """A %2F in the path id decodes before routing -> unknown route (HTML 404) (§2.1)."""
    a = ctx.alias(ctx.create(ctx.root, "a"), "A")
    ctx.req("GET", f"/api/v1/nodes/https:%2F%2Fworkflowy.com%2F%23%2F{short(a)}")
    ctx.req("POST", f"/api/v1/nodes/x%2F{short(a)}", json={"name": "n"})
    ctx.get(a)


@scenario
def ids_url_parent_boundaries(ctx):
    """Which characters may follow the 12 hex digits of a workflowy.com URL parent_id? (open question P6)

    Known: "/" and "?" are fine, a 13th hex digit (dashless UUID) is rejected.
    """
    p = node_sandbox(ctx)
    ctx.create(p, "child")
    for sfx in ("-x", "#y", ".", "g", "_", " ", "&x=1"):
        ctx.list(url(p, suffix=sfx))
    # A hex continuation, shown on the fake short id (a 13-hex token of our
    # own node would not normalise): boundary enforced -> Location '…' not
    # found; not enforced -> Parent item not found.
    for sfx in ("0", "a", "-x"):
        ctx.list(f"https://workflowy.com/#/{FAKE_SHORT}{sfx}")


@scenario
def ids_parent_id_lone_surrogate(ctx):
    """A JSON lone surrogate in parent_id on create/move/mirror: Location miss, Parent miss or 500? (review)"""
    jh = {"Content-Type": "application/json"}
    p = node_sandbox(ctx)
    a = ctx.alias(ctx.create(p, "a"), "A")
    pid = "zz-conformance x\\ud800"
    ctx.req("POST", "/api/v1/nodes", content='{"parent_id": "%s", "name": "n"}' % pid, headers=jh)
    ctx.req("POST", f"/api/v1/nodes/{a}/move", content='{"parent_id": "%s"}' % pid, headers=jh)
    ctx.req("POST", f"/api/v1/nodes/{a}/mirror", content='{"parent_id": "%s"}' % pid, headers=jh)
    ctx.list(p)
