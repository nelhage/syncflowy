"""List: depth-1, ordering, ignored params, empty, repeated params (§4.2)."""

from ..registry import scenario
from ._util import node_sandbox


@scenario
def list_direct_children_only(ctx):
    """List returns direct children only (no grandchildren) (§4.2)."""
    p = node_sandbox(ctx)
    a = ctx.create(p, "a")
    ctx.create(a, "grandchild")
    ctx.create(p, "b")
    ctx.list(p)
    ctx.list(a)


@scenario
def list_empty(ctx):
    """An empty parent lists as {"nodes": []} (§4.2)."""
    p = node_sandbox(ctx)
    ctx.list(p)
    ctx.list(ctx.root)


@scenario
def list_ignores_extra_params(ctx):
    """limit/offset/cursor/depth/... are ignored; no pagination (§4.2)."""
    p = node_sandbox(ctx)
    a = ctx.create(p, "a")
    ctx.create(a, "deep")
    for i in range(3):
        ctx.create(p, f"n{i}", position="bottom")
    for extra in (
        {"limit": "1"},
        {"offset": "2", "page": "2", "per_page": "1"},
        {"cursor": "abc"},
        {"depth": "2", "recursive": "true"},
        {"include_deleted": "true"},
    ):
        ctx.req("GET", "/api/v1/nodes", params={"parent_id": p, **extra})


@scenario
def list_repeated_parent_id_last_wins(ctx):
    """With parent_id repeated, the last one wins (§4.2)."""
    p = node_sandbox(ctx, "P")
    q = node_sandbox(ctx, "Q")
    ctx.create(p, "in p")
    ctx.create(q, "in q")
    ctx.req("GET", "/api/v1/nodes", params=[("parent_id", p), ("parent_id", q)])
    ctx.req("GET", "/api/v1/nodes", params=[("parent_id", q), ("parent_id", p)])


@scenario
def list_after_mixed_operations(ctx):
    """List order stays creation order through renames, completes and renumbers (§4.2)."""
    p = ctx.alias(ctx.create(ctx.root, "P\nA\nB"), "P")
    c = ctx.create(p, "C", position="top")  # renumbers A,B
    ctx.create(p, "D", position="bottom")
    kids = ctx.children(p)
    ctx.update(kids[0]["id"], name="A renamed")
    ctx.complete(c)
    ctx.list(p)


@scenario
def list_deleted_and_missing_parent(ctx):
    """List of a deleted parent gives Parent item not found (§4.2)."""
    p = node_sandbox(ctx)
    c = ctx.create(p, "c")
    ctx.delete(p)
    ctx.list(p)
    ctx.list(c)


@scenario
def list_many_children(ctx):
    """A parent with 25 children lists all of them in creation order (§4.2)."""
    p = node_sandbox(ctx)
    for i in range(25):
        ctx.create(p, f"c{i:02d}", position="bottom" if i % 2 else "top")
    ctx.list(p)
