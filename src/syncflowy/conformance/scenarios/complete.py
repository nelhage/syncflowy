"""Complete / uncomplete (§4.5)."""

from ..registry import scenario
from ._util import node_sandbox


@scenario
def complete_basic(ctx):
    """complete sets completed, completedAt=now, modifiedAt=now; parent not bumped (§4.5)."""
    p = node_sandbox(ctx)
    a = ctx.alias(ctx.create(p, "a"), "A")
    ctx.sleep()
    ctx.complete(a)
    ctx.get(a)
    ctx.get(p)
    ctx.list(p)


@scenario
def complete_twice_restamps(ctx):
    """Re-completing returns ok and re-stamps completedAt and modifiedAt (§4.5)."""
    a = ctx.alias(ctx.create(ctx.root, "a"), "A")
    ctx.complete(a)
    ctx.get(a)
    ctx.sleep()
    ctx.complete(a)
    ctx.get(a)


@scenario
def uncomplete_basic(ctx):
    """uncomplete clears completed and completedAt and bumps modifiedAt (§4.5)."""
    a = ctx.alias(ctx.create(ctx.root, "a"), "A")
    ctx.complete(a)
    ctx.sleep()
    ctx.uncomplete(a)
    ctx.get(a)


@scenario
def uncomplete_never_completed(ctx):
    """uncomplete on a never-completed node still bumps modifiedAt (§4.5)."""
    a = ctx.alias(ctx.create(ctx.root, "a"), "A")
    ctx.sleep()
    ctx.uncomplete(a)
    ctx.get(a)
    ctx.sleep()
    ctx.uncomplete(a)
    ctx.get(a)


@scenario
def complete_no_cascade(ctx):
    """Completion never cascades to descendants or ancestors, and doesn't bump them (§4.5)."""
    p = node_sandbox(ctx)
    a = ctx.alias(ctx.create(p, "a"), "A")
    c = ctx.alias(ctx.create(a, "c"), "C")
    g = ctx.alias(ctx.create(c, "g"), "G")
    ctx.sleep()
    ctx.complete(a)
    for n in (p, a, c, g):
        ctx.get(n)
    ctx.sleep()
    ctx.complete(g)
    ctx.uncomplete(a)
    for n in (p, a, c, g):
        ctx.get(n)


@scenario
def complete_ignores_body(ctx):
    """complete/uncomplete ignore the body entirely, even invalid JSON (§2.4, §4.5)."""
    a = ctx.alias(ctx.create(ctx.root, "a"), "A")
    jh = {"Content-Type": "application/json"}
    ctx.req("POST", f"/api/v1/nodes/{a}/complete", content="{not json", headers=jh)
    ctx.get(a)
    ctx.req("POST", f"/api/v1/nodes/{a}/uncomplete", json={"completed": True, "name": 5})
    ctx.get(a)
    ctx.req("POST", f"/api/v1/nodes/{a}/complete", json={"completed": False})
    ctx.get(a)
    ctx.req("POST", f"/api/v1/nodes/{a}/uncomplete", content="[1, 2", headers=jh)
    ctx.get(a)


@scenario
def complete_keeps_layout_and_priority(ctx):
    """complete works for any layoutMode and changes neither layout nor priority (§4.5)."""
    p = node_sandbox(ctx)
    ids = [ctx.create(p, n, **kw) for n, kw in (("# h", {}), ("- [ ] t", {}), ("b", {"layoutMode": "board"}))]
    for n in ids:
        ctx.complete(n)
    ctx.list(p)


@scenario
def complete_todo_from_create(ctx):
    """A node completed at create via - [x] can be uncompleted and recompleted (§4.5, §6.2)."""
    a = ctx.alias(ctx.create(ctx.root, "- [x] done"), "A")
    ctx.get(a)
    ctx.sleep()
    ctx.uncomplete(a)
    ctx.get(a)
    ctx.sleep()
    ctx.complete(a)
    ctx.get(a)


@scenario
def complete_list_includes_completed(ctx):
    """Lists include completed nodes; there is no filter (§4.2)."""
    p = node_sandbox(ctx)
    a = ctx.create(p, "a")
    ctx.create(p, "b")
    ctx.complete(a)
    ctx.list(p)
    ctx.req("GET", "/api/v1/nodes", params={"parent_id": p, "include_completed": "false", "completed": "false"})


@scenario
def complete_wrong_methods(ctx):
    """GET/PUT/DELETE on /complete and /uncomplete give the HTML 405 with Allow: POST (§2.1)."""
    a = ctx.alias(ctx.create(ctx.root, "a"), "A")
    for m in ("GET", "PUT", "DELETE", "PATCH"):
        ctx.req(m, f"/api/v1/nodes/{a}/complete")
    for m in ("GET", "PUT"):
        ctx.req(m, f"/api/v1/nodes/{a}/uncomplete")
    ctx.get(a)


@scenario
def complete_huge_garbage_body(ctx):
    """complete/uncomplete with a large non-JSON body still succeed (open question P5)."""
    a = ctx.alias(ctx.create(ctx.root, "a"), "A")
    jh = {"Content-Type": "application/json"}
    junk = "{garbage " + "x" * 50_000
    ctx.req("POST", f"/api/v1/nodes/{a}/complete", content=junk, headers=jh)
    ctx.get(a)
    ctx.req("POST", f"/api/v1/nodes/{a}/uncomplete", content=junk, headers=jh)
    ctx.get(a)
