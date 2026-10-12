"""Create: defaults, positions, priorities, fields (data-model §4.3, §7)."""

from ..registry import scenario
from ._util import create_and_show, node_sandbox


@scenario
def create_defaults(ctx):
    """A plain create: response shape, GET defaults, list, parent not bumped (§4.3, §1.2)."""
    a = ctx.alias(ctx.create(ctx.root, "hello"), "A")
    ctx.get(a)
    ctx.list(ctx.root)
    ctx.get(ctx.root)  # root modifiedAt must still be <E0>


@scenario
def create_first_child_priority(ctx):
    """First child of an empty parent gets priority 100, for every position (§7)."""
    for pos in ("top", "bottom", None):
        p = ctx.create(ctx.root, f"parent-{pos}")
        fields = {} if pos is None else {"position": pos}
        ctx.create(p, "first", **fields)
        ctx.list(p)


@scenario
def create_top_halving_then_renumber(ctx):
    """Repeated top inserts halve the min (100,50,...,0); at min 0 all siblings renumber without mtime bumps (§7)."""
    p = node_sandbox(ctx)
    for i in range(8):
        ctx.create(p, f"t{i}", position="top")
    ctx.list(p)
    ctx.sleep()
    ctx.create(p, "renumber", position="top")
    ctx.list(p)
    ctx.get(p)


@scenario
def create_bottom_formula(ctx):
    """bottom = ceil((max+100)/100)*100, mixed with top inserts (§7)."""
    p = node_sandbox(ctx)
    ctx.create(p, "a")  # 100
    ctx.create(p, "b", position="bottom")  # 200
    ctx.create(p, "c", position="top")  # 50
    ctx.create(p, "d", position="top")  # 25
    ctx.create(p, "e", position="bottom")  # 300
    ctx.list(p)


@scenario
def create_bottom_nonmultiple_max(ctx):
    """Bottom after the max is a non-multiple of 100: {25, 50} -> 200; {50} -> 200 (§7)."""
    p = node_sandbox(ctx)
    a = ctx.create(p, "a")  # 100
    ctx.create(p, "b", position="top")  # 50
    ctx.create(p, "c", position="top")  # 25
    ctx.delete(a)
    ctx.create(p, "d", position="bottom")
    ctx.list(p)
    q = ctx.create(ctx.root, "Q")
    x = ctx.create(q, "x")  # 100
    ctx.create(q, "y", position="top")  # 50
    ctx.delete(x)
    ctx.create(q, "z", position="bottom")  # ceil(150/100)*100 = 200
    ctx.list(q)


@scenario
def create_bottom_after_multiline_children(ctx):
    """Multiline children have priorities 0,1,2; bottom after them gives 200 (§4.3, §7)."""
    p = ctx.alias(ctx.create(ctx.root, "P\nA\nB"), "P")
    ctx.list(p)
    ctx.create(p, "bottom", position="bottom")
    ctx.list(p)


@scenario
def create_top_into_min0_renumbers(ctx):
    """Top insert under multiline children (min 0) renumbers them 200,300,... without bumping them (§7)."""
    p = ctx.alias(ctx.create(ctx.root, "P\nA\nB\nC"), "P")
    ctx.list(p)
    ctx.sleep()
    ctx.create(p, "new top", position="top")
    ctx.list(p)


@scenario
def create_position_null_and_omitted_are_top(ctx):
    """position null / omitted behave as top (§4.3)."""
    p = node_sandbox(ctx)
    ctx.create(p, "a")
    ctx.create(p, "b", position=None)
    ctx.create(p, "c")
    ctx.create(p, "d", position="top")
    ctx.list(p)


@scenario
def create_invalid_positions(ctx):
    """Invalid position values give 400 enum errors and create nothing (§2.5)."""
    p = node_sandbox(ctx)
    for pos in ("middle", "TOP", "Bottom", "", 7, True, 0, ["top"]):
        ctx.try_create(p, "x", position=pos)
    ctx.list(p)


@scenario
def create_layoutmode_param(ctx):
    """layoutMode param: any string stored verbatim; "" and null give bullets (§4.3)."""
    p = node_sandbox(ctx)
    for lm in ("todo", "h1", "p", "board", "quote-block", "nonsense", "", None):
        n = ctx.create(p, f"lm {lm!r}", layoutMode=lm)
        ctx.get(n)


@scenario
def create_layoutmode_overrides_prefix(ctx):
    """An explicit layoutMode overrides the prefix layout; the prefix is still stripped and - [x] still completes (§4.3)."""
    p = node_sandbox(ctx)
    for name, lm in (("# head", "todo"), ("- [x] done", "bullets"), ("- [ ] todo", "h2"), ("- item", "p")):
        n = ctx.create(p, name, layoutMode=lm)
        ctx.get(n)


@scenario
def create_note_variants(ctx):
    """note: absent/null -> null, "" stays "", other strings verbatim (no markdown, no strip) (§6.3, §6.4)."""
    p = node_sandbox(ctx)
    for kw in (
        {},
        {"note": None},
        {"note": ""},
        {"note": "  spaced \n\n note  "},
        {"note": "**md** [l](u) &amp; <b>x</b> [2026-01-15]"},
        {"note": "line1\r\nline2\rline3"},
    ):
        n = ctx.create(p, "n", **kw)
        ctx.get(n)


@scenario
def create_note_control_chars_verbatim(ctx):
    """Create note keeps C0 controls verbatim (unlike update) (§6.3)."""
    p = node_sandbox(ctx)
    n = ctx.create(p, "n", note="a\x01b\x0bc\x0cd\x1fe\x7ff")
    ctx.get(n)


@scenario
def create_note_nul_500(ctx):
    """A NUL in the create note gives 500 internal_error and creates nothing (§6.3)."""
    p = node_sandbox(ctx)
    ctx.try_create(p, "with nul note", note="a\x00b")
    ctx.list(p)


@scenario
def create_ignored_fields(ctx):
    """id/priority/completed/completedAt/createdAt/modifiedAt/data/unknown fields are ignored (§2.5)."""
    p = node_sandbox(ctx)
    n = ctx.create(
        p,
        "ignored",
        id="11111111-1111-4111-8111-000000000004",
        priority=5,
        completed=True,
        completedAt=12345,
        createdAt=12345,
        modifiedAt=12345,
        data={"layoutMode": "h2"},
        foo="bar",
        parentId="x",
    )
    ctx.get(n)
    ctx.list(p)


@scenario
def create_empty_and_blank_names(ctx):
    """'', whitespace-only and newline-only names give one node named '' with no children (§6.2 step 7)."""
    p = node_sandbox(ctx)
    for name in ("", "   ", "\n\n\n", "\t", " \n \n "):
        n = ctx.create(p, name)
        ctx.get(n)
        ctx.list(n)


@scenario
def create_does_not_bump_parent(ctx):
    """Creating children (plain or multiline) does not bump the parent's modifiedAt (§8)."""
    p = node_sandbox(ctx)
    ctx.sleep()
    ctx.create(p, "child")
    ctx.create(p, "Multi\nLine", position="bottom")
    ctx.get(p)
    ctx.get(ctx.root)


@scenario
def create_form_encoded(ctx):
    """Form-encoded create bodies are parsed as fields (§2.4)."""
    p = node_sandbox(ctx)
    r = ctx.req(
        "POST",
        "/api/v1/nodes",
        content=f"parent_id={p}&name=form+created&note=n%26b&position=bottom",
        headers={"Content-Type": "application/x-www-form-urlencoded"},
    )
    if r.ok:
        ctx.get(r.json["item_id"])
    ctx.list(p)


@scenario
def create_json_any_content_type(ctx):
    """JSON bodies are parsed whatever the Content-Type (text/plain, none, charset) (§2.4)."""
    import json

    p = node_sandbox(ctx)
    for ct in ("text/plain", "application/json; charset=utf-8", "application/octet-stream", None):
        body = json.dumps({"parent_id": p, "name": f"ct {ct}"})
        r = ctx.req("POST", "/api/v1/nodes", content=body, headers={"Content-Type": ct} if ct else None)
        if r.ok:
            ctx.get(r.json["item_id"])
    ctx.list(p)


@scenario
def create_duplicate_keys_last_wins(ctx):
    """Duplicate JSON keys: the last one wins (§2.4)."""
    p = node_sandbox(ctx)
    r = ctx.req(
        "POST",
        "/api/v1/nodes",
        content='{"parent_id": "%s", "name": "first", "name": "second", "note": null, "note": "n"}' % p,
        headers={"Content-Type": "application/json"},
    )
    if r.ok:
        ctx.get(r.json["item_id"])


@scenario
def create_list_order_is_creation_order(ctx):
    """List returns children in creation order, not priority order (§4.2)."""
    p = node_sandbox(ctx)
    ctx.create(p, "first")
    ctx.create(p, "second", position="bottom")
    ctx.create(p, "third", position="top")
    ctx.create(p, "fourth", position="bottom")
    ctx.list(p)


@scenario
def create_under_completed_parent(ctx):
    """Creating under a completed parent: the child is not completed and the parent is untouched (§4.3)."""
    p = node_sandbox(ctx)
    ctx.complete(p)
    ctx.sleep()
    c = ctx.create(p, "child")
    ctx.get(c)
    ctx.get(p)


@scenario
def create_layoutmode_controls_nul_and_surrogates(ctx):
    """Create layoutMode with C0 controls / NUL; lone surrogates in name, note, layoutMode (review)

    Create's note is verbatim and a NUL in it gives 500; is layoutMode treated
    the same? And what does a JSON lone surrogate do on each create field?
    """
    jh = {"Content-Type": "application/json"}
    p = node_sandbox(ctx)
    create_and_show(ctx, p, "ctl layout", layoutMode="h\x01\x0b1")
    ctx.try_create(p, "nul layout", layoutMode="h\x001")
    ctx.list(p)
    for field, val in (("name", "x\\ud800y"), ("note", "x\\ud800y"), ("layoutMode", "x\\udc00")):
        body = '{"parent_id": "%s", "name": "sur %s", "%s": "%s"}' % (p, field, field, val)
        if field == "name":
            body = '{"parent_id": "%s", "name": "%s"}' % (p, val)
        r = ctx.req("POST", "/api/v1/nodes", content=body, headers=jh)
        if r.status == 200:
            ctx.get(r.json["item_id"])
    ctx.list(p)
