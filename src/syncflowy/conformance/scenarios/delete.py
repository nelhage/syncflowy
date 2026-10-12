"""Delete: subtree, 404s afterwards, no renumbering, no bumps (§4.6)."""

from ..registry import scenario
from ._util import FAKE_UUID, node_sandbox, short


@scenario
def delete_subtree(ctx):
    """Delete removes the node and its whole subtree; everything under it 404s (§4.6)."""
    p = node_sandbox(ctx)
    a = ctx.alias(ctx.create(p, "a"), "A")
    c = ctx.alias(ctx.create(a, "c"), "C")
    g = ctx.alias(ctx.create(c, "g"), "G")
    s = ctx.alias(ctx.create(p, "sibling"), "S")
    ctx.sleep()
    ctx.delete(a)
    for n in (a, c, g):
        ctx.get(n)
    ctx.list(a)
    ctx.list(c)
    ctx.list(p)
    ctx.get(p)
    ctx.get(s)


@scenario
def delete_twice(ctx):
    """Deleting twice gives 404 Item not found the second time (§4.6)."""
    a = ctx.alias(ctx.create(ctx.root, "a"), "A")
    ctx.delete(a)
    ctx.delete(a)
    ctx.delete(FAKE_UUID)


@scenario
def delete_by_short_id_is_noop(ctx):
    """DELETE with a short id or uppercase/dashless UUID 404s and does not delete (§3.1)."""
    a = ctx.alias(ctx.create(ctx.root, "a"), "A")
    ctx.req("DELETE", f"/api/v1/nodes/{short(a)}")
    ctx.req("DELETE", f"/api/v1/nodes/{a.upper()}")
    ctx.req("DELETE", f"/api/v1/nodes/{a.replace('-', '')}")
    ctx.get(a)


@scenario
def delete_then_operate(ctx):
    """Every endpoint on a deleted node gives the right 404 (§4.6)."""
    a = ctx.alias(ctx.create(ctx.root, "a"), "A")
    b = ctx.alias(ctx.create(ctx.root, "b"), "B")
    ctx.delete(a)
    ctx.get(a)
    ctx.list(a)
    ctx.update(a, name="x")
    ctx.complete(a)
    ctx.uncomplete(a)
    ctx.move(a, ctx.root)
    ctx.move(b, a)
    ctx.mirror(a, ctx.root)
    ctx.mirror(b, a)
    ctx.delete_mirror(a)
    ctx.try_create(a, "child of deleted")
    ctx.list(ctx.root)


@scenario
def delete_ignores_body(ctx):
    """DELETE ignores its body, even invalid JSON (§4.6)."""
    a = ctx.alias(ctx.create(ctx.root, "a"), "A")
    ctx.req("DELETE", f"/api/v1/nodes/{a}", content="{garbage", headers={"Content-Type": "application/json"})
    ctx.get(a)


@scenario
def delete_no_renumber(ctx):
    """Deleting a child renumbers nothing; the next top insert halves the remaining min (§4.6, §7)."""
    p = node_sandbox(ctx)
    a = ctx.create(p, "a")  # 100
    ctx.create(p, "b", position="top")  # 50
    c = ctx.create(p, "c", position="top")  # 25
    ctx.delete(c)
    ctx.list(p)
    ctx.create(p, "d", position="top")  # 25 again
    ctx.delete(a)
    ctx.create(p, "e", position="bottom")
    ctx.list(p)


@scenario
def delete_does_not_bump(ctx):
    """Delete bumps nobody: not the parent, not siblings (§8)."""
    p = node_sandbox(ctx)
    a = ctx.create(p, "a")
    b = ctx.alias(ctx.create(p, "b"), "B")
    ctx.sleep()
    ctx.delete(a)
    ctx.get(p)
    ctx.get(b)
    ctx.get(ctx.root)


@scenario
def delete_completed_and_multiline(ctx):
    """Deleting a multiline-created node removes its block children (§4.6)."""
    a = ctx.alias(ctx.create(ctx.root, "A\nB\nC"), "A")
    kids = ctx.children(a)
    ctx.complete(a)
    ctx.delete(a)
    for k in kids:
        ctx.get(k["id"])
    ctx.list(ctx.root)


@scenario
def delete_parent_then_create_under(ctx):
    """Creating or moving into a deleted parent's descendant gives Parent item not found (§4.6)."""
    p = node_sandbox(ctx)
    c = ctx.alias(ctx.create(p, "c"), "C")
    x = ctx.alias(ctx.create(ctx.root, "x"), "X")
    ctx.delete(p)
    ctx.try_create(c, "under deleted descendant")
    ctx.move(x, c)
    ctx.list(c)
    ctx.get(x)
