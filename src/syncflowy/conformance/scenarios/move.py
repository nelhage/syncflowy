"""Move: priorities, same-parent, self, cycles, timestamps, validation (§4.7, §7)."""

from ..registry import scenario
from ._util import FAKE_UUID, FAKE_UUID2, GARBAGE, node_sandbox


def _two_parents(ctx):
    p = node_sandbox(ctx, "P")
    q = node_sandbox(ctx, "Q")
    return p, q


@scenario
def move_cross_parent_top(ctx):
    """Move to another parent at top (explicit and default): floor(min/2) (§4.7, §7)."""
    p, q = _two_parents(ctx)
    a = ctx.alias(ctx.create(p, "a"), "A")
    b = ctx.alias(ctx.create(p, "b", position="bottom"), "B")
    ctx.create(q, "x")  # 100
    ctx.create(q, "y", position="bottom")  # 200
    ctx.move(a, q, position="top")
    ctx.move(b, q)
    ctx.list(p)
    ctx.list(q)


@scenario
def move_cross_parent_bottom(ctx):
    """Move to another parent at bottom: ceil((max+100)/100)*100 (§7)."""
    p, q = _two_parents(ctx)
    a = ctx.alias(ctx.create(p, "a"), "A")
    ctx.create(q, "x")
    ctx.create(q, "y", position="top")
    ctx.move(a, q, position="bottom")
    ctx.list(q)


@scenario
def move_into_empty_parent(ctx):
    """Moving into an empty parent gives priority 100 regardless of position (§7)."""
    p, q = _two_parents(ctx)
    a = ctx.create(p, "a")
    b = ctx.create(p, "b")
    e1 = ctx.alias(ctx.create(q, "e1"), "E1")
    e2 = ctx.alias(ctx.create(q, "e2"), "E2")
    ctx.move(a, e1, position="bottom")
    ctx.move(b, e2, position="top")
    ctx.list(e1)
    ctx.list(e2)


@scenario
def move_same_parent_top(ctx):
    """Same-parent move to top recomputes priority excluding self (may move *up* in value) (§4.7.6)."""
    p = node_sandbox(ctx)
    a = ctx.alias(ctx.create(p, "a"), "A")  # 100
    ctx.alias(ctx.create(p, "b", position="bottom"), "B")  # 200
    c = ctx.alias(ctx.create(p, "c", position="bottom"), "C")  # 300
    ctx.move(c, p, position="top")  # 50
    ctx.list(p)
    ctx.move(c, p, position="top")  # others {100,200} -> 50 again
    ctx.list(p)
    ctx.move(a, p)  # others {50,200} -> 25
    ctx.list(p)


@scenario
def move_same_parent_bottom(ctx):
    """Same-parent move to bottom excludes self from max (§4.7.6)."""
    p = node_sandbox(ctx)
    a = ctx.alias(ctx.create(p, "a"), "A")  # 100
    b = ctx.alias(ctx.create(p, "b", position="bottom"), "B")  # 200
    ctx.move(b, p, position="bottom")  # others {100} -> 200
    ctx.list(p)
    ctx.move(a, p, position="bottom")  # others {200} -> 300
    ctx.list(p)


@scenario
def move_only_child_same_parent(ctx):
    """Moving an only child within its parent: others empty -> 100 (§7)."""
    p = node_sandbox(ctx)
    a = ctx.alias(ctx.create(p, "a"), "A")
    ctx.create(p, "b", position="top")  # 50
    ctx.delete(a)
    b = ctx.children(p)[0]["id"]
    ctx.move(b, p, position="bottom")
    ctx.list(p)


@scenario
def move_top_into_min0_renumbers(ctx):
    """Moving to top under a parent whose min priority is 0 renumbers siblings without bumping them (§7)."""
    p = ctx.alias(ctx.create(ctx.root, "P\nA\nB\nC"), "P")
    x = ctx.alias(ctx.create(ctx.root, "x"), "X")
    ctx.list(p)
    ctx.sleep()
    ctx.move(x, p, position="top")
    ctx.list(p)
    ctx.get(p)


@scenario
def move_self_top_in_min0_parent(ctx):
    """Moving a priority-0 child to top among [0,1,2] excludes itself (min 1 -> 0) (§7)."""
    p = ctx.alias(ctx.create(ctx.root, "P\nA\nB\nC"), "P")
    kids = ctx.children(p)
    ctx.move(kids[0]["id"], p, position="top")
    ctx.list(p)
    ctx.move(kids[2]["id"], p, position="top")
    ctx.list(p)


@scenario
def move_timestamps(ctx):
    """Cross-parent move bumps only the node and the destination parent (§8)."""
    p, q = _two_parents(ctx)
    a = ctx.alias(ctx.create(p, "a"), "A")
    s = ctx.alias(ctx.create(p, "sibling"), "S")
    c = ctx.alias(ctx.create(a, "child"), "C")
    ctx.create(q, "existing")
    ctx.sleep()
    ctx.move(a, q, position="bottom")
    for n in (ctx.root, p, q, a, s, c):
        ctx.get(n)


@scenario
def move_same_parent_timestamps(ctx):
    """Same-parent move bumps the node and the (same) parent (§8)."""
    p = node_sandbox(ctx)
    a = ctx.alias(ctx.create(p, "a"), "A")
    b = ctx.alias(ctx.create(p, "b"), "B")
    ctx.sleep()
    ctx.move(a, p, position="top")
    for n in (p, a, b):
        ctx.get(n)


@scenario
def move_to_self(ctx):
    """Move under itself: 200, parent and priority unchanged, only the node's modifiedAt bumped (§4.7.4)."""
    p = node_sandbox(ctx)
    a = ctx.alias(ctx.create(p, "a"), "A")
    ctx.create(p, "b", position="bottom")
    ctx.get(a)
    ctx.sleep()
    ctx.move(a, a, position="bottom")
    ctx.get(a)
    ctx.get(p)
    ctx.list(p)
    ctx.move(a, a)
    ctx.get(a)


@scenario
def move_ignores_index_fields(ctx):
    """priority/index/after/before/sibling_id on move are ignored (no middle insertion) (§4.7.8)."""
    p, q = _two_parents(ctx)
    x = ctx.alias(ctx.create(q, "x"), "X")
    ctx.create(q, "y", position="bottom")
    a = ctx.create(p, "a")
    b = ctx.create(p, "b")
    ctx.move(a, q, priority=150, index=1, after=x, before=x, sibling_id=x)
    ctx.move(b, q, position="bottom", priority=1, index=0)
    ctx.list(q)


@scenario
def move_preserves_subtree(ctx):
    """Descendants move with the node, keeping ids, priorities and createdAt (§4.7.7)."""
    p, q = _two_parents(ctx)
    a = ctx.alias(ctx.create(p, "A\nX\nY"), "A")
    g = ctx.create(ctx.children(a)[0]["id"], "grandchild")
    ctx.sleep()
    ctx.move(a, q)
    ctx.get(a)
    ctx.list(a)
    ctx.get(g)
    ctx.list(p)


@scenario
def move_list_order_follows_creation(ctx):
    """After a move, list order is still creation order (older moved node lists first) (§4.2)."""
    x = ctx.alias(ctx.create(ctx.root, "x (oldest)"), "X")
    p = node_sandbox(ctx)
    ctx.create(p, "c1")
    ctx.create(p, "c2")
    ctx.move(x, p, position="bottom")
    ctx.list(p)
    y = ctx.alias(ctx.create(ctx.root, "y (newest)"), "Y")
    ctx.move(y, p, position="top")
    ctx.list(p)


@scenario
def move_back_and_forth(ctx):
    """Moving away and back recomputes priority each time; createdAt never changes (§4.7)."""
    p, q = _two_parents(ctx)
    a = ctx.alias(ctx.create(p, "a"), "A")
    ctx.create(p, "b", position="bottom")
    ctx.move(a, q)
    ctx.move(a, p, position="bottom")
    ctx.get(a)
    ctx.list(p)


@scenario
def move_completed_node(ctx):
    """Moving a completed node keeps completion and completedAt (§4.7)."""
    p, q = _two_parents(ctx)
    a = ctx.alias(ctx.create(p, "a"), "A")
    ctx.complete(a)
    ctx.sleep()
    ctx.move(a, q)
    ctx.get(a)


@scenario(mock_options={"faithful_cycles": True})
def move_into_child_creates_cycle(ctx):
    """Production accepts moving a node under its own child and forms a parent cycle; repaired afterwards (§4.7.5)."""
    x = ctx.alias(ctx.create(ctx.root, "X"), "X")
    x1 = ctx.alias(ctx.create(x, "X1"), "X1")
    ctx.move(x, x1)
    ctx.get(x)
    ctx.get(x1)
    ctx.list(ctx.root)
    ctx.list(x)
    ctx.list(x1)
    ctx.move(x, ctx.root)  # repair
    ctx.get(x)
    ctx.get(x1)
    ctx.list(ctx.root)


@scenario(mock_options={"faithful_cycles": True})
def move_into_grandchild_creates_cycle(ctx):
    """Move under a grandchild also forms a cycle; repaired afterwards (§4.7.5)."""
    x = ctx.alias(ctx.create(ctx.root, "X"), "X")
    c = ctx.alias(ctx.create(x, "C"), "C")
    g = ctx.alias(ctx.create(c, "G"), "G")
    ctx.move(x, g, position="bottom")
    for n in (x, c, g):
        ctx.get(n)
    ctx.list(ctx.root)
    ctx.move(x, ctx.root)  # repair
    for n in (x, c, g):
        ctx.get(n)


@scenario
def move_validation_errors(ctx):
    """Move body validation: string_type, enum, all errors collected; parse errors (§2.5)."""
    a = ctx.alias(ctx.create(ctx.root, "a"), "A")
    path = f"/api/v1/nodes/{a}/move"
    ctx.req("POST", path, json={"parent_id": 1})
    ctx.req("POST", path, json={"parent_id": ctx.root, "position": "middle"})
    ctx.req("POST", path, json={"position": 3, "parent_id": True})
    ctx.req("POST", path, json={"parent_id": ["x"], "position": None})
    jh = {"Content-Type": "application/json"}
    for body in ("", "{bad", "[1]", '"s"', "null"):
        ctx.req("POST", path, content=body, headers=jh)
    ctx.get(a)


@scenario
def move_not_found_errors(ctx):
    """Move 404s: missing item, missing parent (UUID vs Location forms), item checked first (§2.2, §3.2)."""
    a = ctx.alias(ctx.create(ctx.root, "a"), "A")
    ctx.move(FAKE_UUID, ctx.root)
    ctx.move(a, FAKE_UUID)
    ctx.move(FAKE_UUID, FAKE_UUID2)
    ctx.move(a, GARBAGE)
    ctx.move(FAKE_UUID, ctx.root, position="sideways")
    ctx.get(a)


@scenario
def move_wrong_methods(ctx):
    """GET/PUT/DELETE on /move give the HTML-empty 405 with Allow: POST (§2.1)."""
    a = ctx.alias(ctx.create(ctx.root, "a"), "A")
    for m in ("GET", "PUT", "DELETE", "PATCH"):
        ctx.req(m, f"/api/v1/nodes/{a}/move")
    ctx.get(a)
