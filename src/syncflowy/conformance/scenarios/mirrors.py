"""Mirrors: shells, identity, redirects, timestamps, deletion, orphans (§10)."""

from ..registry import scenario
from ._util import FAKE_UUID, FAKE_UUID2, GARBAGE, node_sandbox, short, url


def _origin(ctx, name="origin", **kw):
    return ctx.alias(ctx.create(ctx.root, name, **kw), "O")


@scenario
def mirror_basic_shape(ctx):
    """Mirror returns item_id+origin_id; shell has name/note '' and bullets, not completed (§10.1)."""
    o = _origin(ctx, "# origin", note="origin note")
    ctx.complete(o)
    p = node_sandbox(ctx)
    r = ctx.mirror(o, p)
    ctx.get(o)
    if r.ok:
        ctx.alias(r.json["item_id"], "M")
        ctx.get(r.json["item_id"])
        ctx.list(r.json["item_id"])
    ctx.list(p)


@scenario
def mirror_timestamps(ctx):
    """POST /mirror bumps only the origin's modifiedAt (not the destination) (§8, §10.2)."""
    o = _origin(ctx)
    p = node_sandbox(ctx)
    ctx.sleep()
    m = ctx.alias(ctx.make_mirror(o, p), "M")
    ctx.get(o)
    ctx.get(p)
    ctx.get(m)
    ctx.get(ctx.root)


@scenario
def mirror_no_propagation_from_origin(ctx):
    """Edits/completion of the origin do not show on the shell (§10.1)."""
    o = _origin(ctx)
    p = node_sandbox(ctx)
    m = ctx.alias(ctx.make_mirror(o, p), "M")
    ctx.sleep()
    ctx.update(o, name="renamed", note="n", layoutMode="h1")
    ctx.complete(o)
    ctx.create(o, "origin child")
    ctx.get(m)
    ctx.list(m)
    ctx.get(o)


@scenario
def mirror_no_propagation_from_shell(ctx):
    """Edits/completion of the shell do not reach the origin, nor bump it (§10.1, §8)."""
    o = _origin(ctx)
    p = node_sandbox(ctx)
    m = ctx.alias(ctx.make_mirror(o, p), "M")
    ctx.sleep()
    ctx.update(m, name="shell name", note="shell note")
    ctx.complete(m)
    ctx.get(m)
    ctx.get(o)
    ctx.uncomplete(m)
    ctx.get(m)


@scenario
def mirror_positions(ctx):
    """Shell priority uses the shared algorithm: default top, top, bottom (§7, §10.2)."""
    o = _origin(ctx)
    p = node_sandbox(ctx)
    ctx.create(p, "a")
    ctx.create(p, "b", position="bottom")
    ctx.mirror(o, p)
    ctx.mirror(o, p, position="bottom")
    ctx.mirror(o, p, position="top")
    ctx.list(p)


@scenario
def mirror_into_empty_and_twice(ctx):
    """Mirroring twice into one parent creates two shells; first into empty parent gets 100 (§10.2)."""
    o = _origin(ctx)
    p = node_sandbox(ctx)
    ctx.mirror(o, p)
    ctx.mirror(o, p)
    ctx.list(p)


@scenario
def mirror_of_mirror_redirects(ctx):
    """Mirroring a shell mirrors its origin (origin_id = origin); only the origin is bumped (§10.2)."""
    o = _origin(ctx)
    p = node_sandbox(ctx, "P")
    q = node_sandbox(ctx, "Q")
    m = ctx.alias(ctx.make_mirror(o, p), "M")
    ctx.sleep()
    ctx.mirror(m, q)
    ctx.get(o)
    ctx.get(m)
    ctx.list(q)


@scenario
def mirror_into_shell_parent_redirects(ctx):
    """A shell as mirror parent_id redirects to its origin (§10.2 step 3)."""
    o = _origin(ctx)
    x = ctx.alias(ctx.create(ctx.root, "x"), "X")
    p = node_sandbox(ctx)
    m = ctx.alias(ctx.make_mirror(o, p), "M")
    ctx.sleep()
    ctx.mirror(x, m)
    ctx.list(m)
    ctx.list(o)
    ctx.get(o)
    ctx.get(x)
    ctx.get(m)


@scenario
def mirror_under_itself_and_descendants(ctx):
    """Mirror under itself is 400; under its own child/grandchild is allowed; under own shell -> self error (§10.2)."""
    o = _origin(ctx)
    c = ctx.alias(ctx.create(o, "c"), "C")
    g = ctx.alias(ctx.create(c, "g"), "G")
    ctx.mirror(o, o)
    ctx.mirror(o, c)
    ctx.mirror(o, g, position="bottom")
    ctx.list(c)
    ctx.list(g)
    p = node_sandbox(ctx)
    m = ctx.alias(ctx.make_mirror(o, p), "M")
    ctx.mirror(o, m)  # parent redirects to O itself
    ctx.mirror(m, o)  # source redirects to O, parent O


@scenario
def mirror_parent_id_forms(ctx):
    """Mirror parent_id is exact-UUID only: short/URL/upper/dashless/garbage all give Parent item not found (§3.3)."""
    o = _origin(ctx)
    p = node_sandbox(ctx)
    for pid in (short(p), short(p).upper(), url(p), p.upper(), p.replace("-", ""), GARBAGE, "", FAKE_UUID):
        ctx.mirror(o, pid)
    ctx.list(p)


@scenario
def mirror_children_of_shell(ctx):
    """Create/move into a shell put the node under the shell (no redirect); list shows only shell's children (§10.1)."""
    o = _origin(ctx)
    ctx.create(o, "origin child")
    p = node_sandbox(ctx)
    m = ctx.alias(ctx.make_mirror(o, p), "M")
    ctx.sleep()
    c = ctx.alias(ctx.create(m, "shell child"), "SC")
    x = ctx.alias(ctx.create(ctx.root, "x"), "X")
    ctx.move(x, m)
    ctx.list(m)
    ctx.list(o)
    ctx.get(c)
    ctx.get(m)
    ctx.get(o)


@scenario
def mirror_delete_mirror_endpoint(ctx):
    """DELETE /mirror removes the shell (and its subtree), bumps the origin, then 404s (§10.3)."""
    o = _origin(ctx)
    p = node_sandbox(ctx)
    m = ctx.alias(ctx.make_mirror(o, p), "M")
    sc = ctx.alias(ctx.create(m, "shell child"), "SC")
    ctx.sleep()
    ctx.delete_mirror(m)
    ctx.get(m)
    ctx.get(sc)
    ctx.get(o)
    ctx.get(p)
    ctx.list(p)
    ctx.delete_mirror(m)


@scenario
def mirror_delete_mirror_not_a_mirror(ctx):
    """DELETE /mirror on a plain node, an origin, or a shell's plain child: 400 Node is not a mirror (§10.3)."""
    o = _origin(ctx)
    plain = ctx.alias(ctx.create(ctx.root, "plain"), "PLAIN")
    p = node_sandbox(ctx)
    m = ctx.alias(ctx.make_mirror(o, p), "M")
    sc = ctx.alias(ctx.create(m, "shell child"), "SC")
    ctx.delete_mirror(plain)
    ctx.delete_mirror(o)
    ctx.delete_mirror(sc)
    ctx.delete_mirror(FAKE_UUID)
    ctx.req("DELETE", f"/api/v1/nodes/{short(m)}/mirror")
    ctx.get(plain)
    ctx.get(o)
    ctx.get(sc)


@scenario
def mirror_plain_delete_of_shell(ctx):
    """Plain DELETE of a shell removes it without bumping the origin (§8)."""
    o = _origin(ctx)
    p = node_sandbox(ctx)
    m = ctx.alias(ctx.make_mirror(o, p), "M")
    ctx.sleep()
    ctx.delete(m)
    ctx.get(m)
    ctx.get(o)
    ctx.list(p)


@scenario
def mirror_delete_subtree_with_shell(ctx):
    """Deleting a subtree containing a shell does not bump the origin (§4.6)."""
    o = _origin(ctx)
    p = node_sandbox(ctx)
    ctx.make_mirror(o, p)
    ctx.sleep()
    ctx.delete(p)
    ctx.get(o)


@scenario
def mirror_orphan_lifecycle(ctx):
    """Deleting the origin leaves orphan shells usable as plain nodes, but not as mirror source/parent (§10.4)."""
    o = _origin(ctx)
    p = node_sandbox(ctx)
    m = ctx.alias(ctx.make_mirror(o, p), "M")
    x = ctx.alias(ctx.create(ctx.root, "x"), "X")
    ctx.sleep()
    ctx.delete(o)
    ctx.get(m)
    ctx.list(p)
    ctx.update(m, name="orphan renamed")
    ctx.complete(m)
    c = ctx.try_create(m, "orphan child")
    ctx.list(m)
    ctx.mirror(m, p)
    ctx.mirror(m, FAKE_UUID)
    ctx.mirror(m, m)
    ctx.mirror(x, m)
    ctx.move(x, m)
    ctx.get(m)
    if c.ok:
        ctx.get(c.json["item_id"])
    ctx.delete_mirror(m)
    ctx.get(m)
    ctx.get(x)


@scenario
def mirror_origin_ancestor_deleted(ctx):
    """Deleting an ancestor of the origin also orphans its shells (§10.4)."""
    a = ctx.alias(ctx.create(ctx.root, "ancestor"), "ANC")
    o = ctx.alias(ctx.create(a, "origin"), "O")
    p = node_sandbox(ctx)
    m = ctx.alias(ctx.make_mirror(o, p), "M")
    ctx.delete(a)
    ctx.get(o)
    ctx.get(m)
    ctx.mirror(m, p)
    ctx.delete_mirror(m)
    ctx.list(p)


@scenario
def mirror_validation_errors(ctx):
    """Mirror body validation: parent_id required/non-null, enum position, parse errors (§2.5)."""
    o = _origin(ctx)
    path = f"/api/v1/nodes/{o}/mirror"
    ctx.req("POST", path, json={})
    ctx.req("POST", path, json={"position": "top"})
    ctx.req("POST", path, json={"parent_id": None})
    ctx.req("POST", path, json={"parent_id": 5, "position": "x"})
    ctx.req("POST", path, json={"position": False, "foo": 1})
    jh = {"Content-Type": "application/json"}
    for body in ("", "{bad", "[1]", '"s"', "null"):
        ctx.req("POST", path, content=body, headers=jh)
    ctx.list(ctx.root)


@scenario
def mirror_not_found_errors(ctx):
    """Mirror 404s: item before parent; validation before both (§2.2, §10.2)."""
    o = _origin(ctx)
    ctx.mirror(FAKE_UUID, ctx.root)
    ctx.mirror(o, FAKE_UUID)
    ctx.mirror(FAKE_UUID, FAKE_UUID2)
    ctx.mirror(FAKE_UUID, FAKE_UUID2, position="nope")
    ctx.req("POST", f"/api/v1/nodes/{short(o)}/mirror", json={"parent_id": ctx.root})
    ctx.req("POST", f"/api/v1/nodes/{o.upper()}/mirror", json={"parent_id": ctx.root})
    ctx.list(ctx.root)


@scenario
def mirror_move_shell_and_origin(ctx):
    """Shells and origins move like plain nodes; moving the origin into its own shell is allowed (§9.4)."""
    o = _origin(ctx)
    p = node_sandbox(ctx, "P")
    q = node_sandbox(ctx, "Q")
    m = ctx.alias(ctx.make_mirror(o, p), "M")
    ctx.sleep()
    ctx.move(m, q, position="bottom")
    ctx.get(m)
    ctx.get(o)
    ctx.move(o, m)
    ctx.get(o)
    ctx.list(m)
    ctx.get(m)
    ctx.move(o, ctx.root)
    ctx.get(o)


@scenario
def mirror_wrong_methods(ctx):
    """GET/PUT on /mirror give the JSON 405 with allowed_methods [POST, DELETE] (§2.1)."""
    o = _origin(ctx)
    for m in ("GET", "PUT", "PATCH"):
        ctx.req(m, f"/api/v1/nodes/{o}/mirror")
    ctx.req("POST", f"/api/v1/nodes/{o}/mirror/x", json={"parent_id": ctx.root})
    ctx.get(o)


@scenario
def mirror_completed_origin_and_layout(ctx):
    """A shell of a completed todo origin is not completed and is bullets (§10.1)."""
    o = _origin(ctx, "- [x] done todo")
    p = node_sandbox(ctx)
    m = ctx.alias(ctx.make_mirror(o, p), "M")
    ctx.get(m)
    ctx.get(o)
