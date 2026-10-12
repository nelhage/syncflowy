"""Update: verbatim text, partial semantics, modifiedAt, validation (§4.4, §6.1)."""

from ..registry import scenario
from ._util import FAKE_UUID, node_sandbox


@scenario
def update_name_verbatim(ctx):
    """Update stores name verbatim: no markdown, entities, trimming or newline handling (§6.1)."""
    n = ctx.alias(ctx.create(ctx.root, "orig"), "A")
    for name in ("**x** &amp; <b>y</b>", "a\nb\n- [x] c", "  padded  ", "", "# not a header", "[2026-01-15]"):
        ctx.update(n, name=name)
        ctx.get(n)
    ctx.list(n)


@scenario
def update_note_verbatim(ctx):
    """Update stores note verbatim (CRLF and CR preserved) (§6.1)."""
    n = ctx.alias(ctx.create(ctx.root, "orig"), "A")
    for note in ("**md** &lt;", "l1\r\nl2\rl3\n", "   ", "é\U0001F600"):
        ctx.update(n, note=note)
        ctx.get(n)


@scenario
def update_control_chars_stripped(ctx):
    """Update strips [\\x00-\\x08\\x0b\\x0c\\x0e-\\x1f] from name and note; keeps TAB/LF/CR/DEL (§6.1)."""
    n = ctx.alias(ctx.create(ctx.root, "orig"), "A")
    s = "a\x00b\x01c\x08d\x0be\x0cf\rg\nh\ti\x1fj\x7fk\x85l"
    ctx.update(n, name=s, note=s)
    ctx.get(n)


@scenario
def update_layoutmode_control_chars(ctx):
    """Is layoutMode control-stripped on update? (open question P4)"""
    n = ctx.alias(ctx.create(ctx.root, "orig"), "A")
    ctx.update(n, layoutMode="h\x01\x0b1")
    ctx.get(n)


@scenario
def update_null_fields_are_noops(ctx):
    """null name/note/layoutMode leave fields unchanged but still bump modifiedAt (§4.4)."""
    n = ctx.alias(ctx.create(ctx.root, "keep", note="keep note", layoutMode="h1"), "A")
    ctx.sleep()
    ctx.update(n, name=None, note=None, layoutMode=None)
    ctx.get(n)


@scenario
def update_empty_body_bumps(ctx):
    """{} and unknown-fields-only updates return ok and bump modifiedAt (§4.4, §8)."""
    a = ctx.alias(ctx.create(ctx.root, "a"), "A")
    b = ctx.alias(ctx.create(ctx.root, "b"), "B")
    ctx.sleep()
    ctx.update(a)
    ctx.get(a)
    ctx.get(b)
    ctx.sleep()
    ctx.update(b, foo="bar")
    ctx.get(a)
    ctx.get(b)


@scenario
def update_noop_same_value_bumps(ctx):
    """Setting a field to its current value still bumps modifiedAt (§4.4)."""
    a = ctx.alias(ctx.create(ctx.root, "same"), "A")
    ctx.sleep()
    ctx.update(a, name="same")
    ctx.get(a)


@scenario
def update_ignores_structural_fields(ctx):
    """parent_id/priority/position/completed/id in an update are ignored (no move, reorder, complete) (§4.4)."""
    p = node_sandbox(ctx, "P")
    q = node_sandbox(ctx, "Q")
    a = ctx.alias(ctx.create(p, "a"), "A")
    ctx.update(
        a,
        parent_id=q,
        priority=1,
        position="bottom",
        completed=True,
        completedAt=5,
        id=FAKE_UUID,
        createdAt=5,
        data={"layoutMode": "h1"},
    )
    ctx.get(a)
    ctx.list(p)
    ctx.list(q)


@scenario
def update_note_empty_cannot_return_to_null(ctx):
    """note can be set to "" but not back to null (§6.4)."""
    a = ctx.alias(ctx.create(ctx.root, "a"), "A")
    ctx.get(a)
    ctx.update(a, note="x")
    ctx.update(a, note="")
    ctx.get(a)
    ctx.update(a, note=None)
    ctx.get(a)


@scenario
def update_layoutmode_any_string(ctx):
    """layoutMode on update stores any string, including "" (§4.4)."""
    a = ctx.alias(ctx.create(ctx.root, "a"), "A")
    for lm in ("todo", "nonsense", "H1", ""):
        ctx.update(a, layoutMode=lm)
        ctx.get(a)


@scenario
def update_partial_preserves_other_fields(ctx):
    """Updating one field leaves the others (incl. completion, priority) untouched (§4.4)."""
    a = ctx.alias(ctx.create(ctx.root, "- [x] name", note="note"), "A")
    ctx.get(a)
    ctx.update(a, note="new note")
    ctx.get(a)
    ctx.update(a, name="new name")
    ctx.get(a)
    ctx.update(a, layoutMode="bullets")
    ctx.get(a)


@scenario
def update_text_fixed_point(ctx):
    """Re-submitting a create-produced name via update is a fixed point (§6.1)."""
    a = ctx.alias(ctx.create(ctx.root, "**b** *i* [t](u) [2026-01-15] &amp; `c`"), "A")
    name = ctx.node(a)["name"]
    ctx.update(a, name=name)
    ctx.get(a)


@scenario
def update_does_not_bump_parent(ctx):
    """Updating a child bumps only the child (§8)."""
    p = node_sandbox(ctx)
    c = ctx.alias(ctx.create(p, "c"), "C")
    ctx.sleep()
    ctx.update(c, name="c2")
    ctx.get(c)
    ctx.get(p)


@scenario
def update_validation_errors(ctx):
    """Non-string fields give string_type errors, all collected in declaration order (§2.5)."""
    a = ctx.alias(ctx.create(ctx.root, "a"), "A")
    ctx.update(a, name=1)
    ctx.update(a, note=True)
    ctx.update(a, layoutMode=[])
    ctx.update(a, layoutMode={"x": 1}, note=2.5, name=False)
    ctx.update(a, name=None, note=7)
    ctx.get(a)


@scenario
def update_bad_bodies(ctx):
    """Invalid JSON, empty body, and non-object JSON on update (§2.4)."""
    a = ctx.alias(ctx.create(ctx.root, "a"), "A")
    path = f"/api/v1/nodes/{a}"
    jh = {"Content-Type": "application/json"}
    for body in ('{"note": ', "", "[1]", '"hello"', "null", "42", "true", '[{"note": "array"}]'):
        ctx.req("POST", path, content=body, headers=jh)
    ctx.get(a)


@scenario
def update_body_encodings(ctx):
    """Form-encoded, multipart, text/plain JSON and no-content-type JSON updates are applied (§2.4)."""
    a = ctx.alias(ctx.create(ctx.root, "a"), "A")
    path = f"/api/v1/nodes/{a}"
    ctx.req("POST", path, content="note=form+note&name=n%C3%A9", headers={"Content-Type": "application/x-www-form-urlencoded"})
    ctx.get(a)
    boundary = "XconformanceX"
    mp = (
        f"--{boundary}\r\nContent-Disposition: form-data; name=\"note\"\r\n\r\nmultipart note\r\n"
        f"--{boundary}--\r\n"
    )
    ctx.req("POST", path, content=mp, headers={"Content-Type": f"multipart/form-data; boundary={boundary}"})
    ctx.get(a)
    ctx.req("POST", path, content='{"note": "plain"}', headers={"Content-Type": "text/plain"})
    ctx.get(a)
    ctx.req("POST", path, content='{"note": "no ctype"}')
    ctx.get(a)
    ctx.req("POST", path, content="note=not+form", headers={"Content-Type": "application/json"})
    ctx.get(a)


@scenario
def update_query_string_ignored(ctx):
    """Query parameters never contribute fields to a POST body (§2.4)."""
    a = ctx.alias(ctx.create(ctx.root, "a"), "A")
    ctx.req("POST", f"/api/v1/nodes/{a}", params={"name": "from-query"}, json={"note": "from-body"})
    ctx.get(a)


@scenario
def update_validation_before_lookup(ctx):
    """On a nonexistent id, validation and JSON errors win over 404 (§2.2)."""
    path = f"/api/v1/nodes/{FAKE_UUID}"
    ctx.req("POST", path, json={"name": 123})
    ctx.req("POST", path, content="{bad", headers={"Content-Type": "application/json"})
    ctx.req("POST", path, json={"name": "ok"})
    ctx.req("POST", path, json={})


@scenario
def update_completed_node_text(ctx):
    """Updating a completed node keeps it completed and keeps completedAt (§4.4)."""
    a = ctx.alias(ctx.create(ctx.root, "a"), "A")
    ctx.complete(a)
    ctx.sleep()
    ctx.update(a, name="renamed")
    ctx.get(a)


@scenario
def update_large_note(ctx):
    """A 30 kB note round-trips through update (§2.4)."""
    a = ctx.alias(ctx.create(ctx.root, "a"), "A")
    ctx.update(a, note="0123456789abcdef" * 2000)
    ctx.get(a)


@scenario
def update_nul_layoutmode_and_lone_surrogates(ctx):
    """NUL in update layoutMode; JSON lone surrogates in update name/note/layoutMode: stored or 500? (review)

    Update strips C0 from name/note (so a NUL there is harmless) but stores
    layoutMode verbatim, so a NUL in layoutMode reaches storage, like the
    create-note NUL that gives 500. A lone surrogate (JSON "\\ud800") is valid JSON
    for Python's json.loads but cannot be encoded as UTF-8.
    """
    jh = {"Content-Type": "application/json"}
    a = ctx.alias(ctx.create(ctx.root, "nul layout"), "A")
    b = ctx.alias(ctx.create(ctx.root, "sur name"), "B")
    c = ctx.alias(ctx.create(ctx.root, "sur note"), "C")
    d = ctx.alias(ctx.create(ctx.root, "sur layout"), "D")
    ctx.sleep()
    ctx.update(a, layoutMode="h\x001")
    ctx.get(a)
    ctx.req("POST", f"/api/v1/nodes/{b}", content='{"name": "x\\ud800y"}', headers=jh)
    ctx.get(b)
    ctx.req("POST", f"/api/v1/nodes/{c}", content='{"note": "x\\ud800y"}', headers=jh)
    ctx.get(c)
    ctx.req("POST", f"/api/v1/nodes/{d}", content='{"layoutMode": "x\\udc00"}', headers=jh)
    ctx.get(d)
