"""POST /nodes: defaults, field handling, text pipeline, multiline, golden vectors (§4.3, §6)."""

from __future__ import annotations

import pytest

from syncflowy.mock.text import inline_markdown, transform_create_name

from . import golden
from .conftest import WF

CASES = golden.all_cases()


def tree_of(wf: WF, nid: str) -> list[dict]:
    return [wf.get(nid), *wf.children(nid)]


@pytest.mark.parametrize("case", CASES, ids=[c.id for c in CASES])
def test_golden_create_vectors(wf: WF, case: golden.Case) -> None:
    """Replays every create-path name captured from production (≈500 cases)."""
    sb = wf.sandbox()
    r = wf.create_raw({"parent_id": sb, "name": case.name, "position": "bottom", **case.extra})
    assert r.status_code == case.status, r.text
    if case.status != 200:
        assert wf.list(sb) == []  # nothing was created
        return
    nodes = tree_of(wf, r.json()["item_id"])
    got = [(n["name"], n["data"]["layoutMode"], n["completed"]) for n in nodes]
    want = case.tree
    if not case.children_known:
        got = got[:1]
    assert len(got) == len(want), (got, want)
    for g, w in zip(got, want):
        assert g[0] == w[0]
        if w[1] is not None:
            assert g[1] == w[1]
        if w[2] is not None:
            assert g[2] == w[2]
    if case.note != "__unchecked__":
        assert nodes[0]["note"] == case.note
    # children: priorities 0..n-1, note "", same timestamps
    for i, c in enumerate(nodes[1:]):
        assert c["priority"] == i and c["note"] == ""
        assert c["createdAt"] == c["modifiedAt"] == nodes[0]["createdAt"]
        if c["completed"]:
            assert c["completedAt"] == c["createdAt"]


@pytest.mark.parametrize("label,value,want_name,want_note", list(golden.update_cases()))
def test_golden_update_vectors(wf: WF, label: str, value: str, want_name: str, want_note: str) -> None:
    sb = wf.sandbox()
    nid = wf.create(sb, "u")
    wf.update(nid, name=value, note=value)
    n = wf.get(nid)
    assert n["name"] == want_name and n["note"] == want_note


def test_defaults(wf: WF) -> None:
    sb = wf.sandbox()
    nid = wf.create(sb, "hello world")
    n = wf.get(nid)
    assert n == {
        "id": nid,
        "name": "hello world",
        "note": None,
        "parent_id": sb,
        "priority": 100,
        "completed": False,
        "data": {"layoutMode": "bullets"},
        "createdAt": wf.clock.now,
        "modifiedAt": wf.clock.now,
        "completedAt": None,
    }


def test_note_null_vs_empty_and_verbatim(wf: WF) -> None:
    sb = wf.sandbox()
    assert wf.get(wf.create(sb, "a"))["note"] is None
    assert wf.get(wf.create(sb, "a", note=None))["note"] is None
    assert wf.get(wf.create(sb, "a", note=""))["note"] == ""
    raw = "  **bold** *it* `code` [l](https://x.com) # h [2026-01-15]\n- bullet\r\n&amp; \x01\x0b  "
    assert wf.get(wf.create(sb, "a", note=raw))["note"] == raw


def test_note_with_nul_is_500_and_creates_nothing(wf: WF) -> None:
    sb = wf.sandbox()
    r = wf.create_raw({"parent_id": sb, "name": "a\x00b", "note": "x\x00y"})
    assert r.status_code == 500
    assert wf.list(sb) == []
    # NUL in the name alone is fine (stripped)
    assert wf.get(wf.create(sb, "a\x00b"))["name"] == "ab"


def test_ignored_fields(wf: WF) -> None:
    sb = wf.sandbox()
    nid = wf.create(
        sb, "x", completed=True, completedAt=1700000000, priority=5, id="11111111-2222-3333-4444-555555555555",
        createdAt=1600000000, data={"layoutMode": "h1"}, foo="bar",
    )
    n = wf.get(nid)
    assert nid != "11111111-2222-3333-4444-555555555555"
    assert n["completed"] is False and n["completedAt"] is None
    assert n["priority"] == 100 and n["createdAt"] == wf.clock.now
    assert n["data"] == {"layoutMode": "bullets"}


def test_done_prefix_wins_over_completed_false(wf: WF) -> None:
    sb = wf.sandbox()
    n = wf.get(wf.create(sb, "- [x] x", completed=False))
    assert n["completed"] is True and n["completedAt"] == n["createdAt"]


@pytest.mark.parametrize(
    "layout,expected",
    [("todo", "todo"), ("p", "p"), ("board", "board"), ("H1", "H1"), ("quote-block", "quote-block"), ("", "bullets"), (None, "bullets")],
)
def test_layout_param_any_string(wf: WF, layout, expected: str) -> None:
    sb = wf.sandbox()
    assert wf.get(wf.create(sb, "plain", layoutMode=layout))["data"]["layoutMode"] == expected


def test_layout_param_overrides_prefix_but_prefix_still_consumed(wf: WF) -> None:
    sb = wf.sandbox()
    n = wf.get(wf.create(sb, "- [x] done", layoutMode="h1"))
    assert (n["name"], n["data"]["layoutMode"], n["completed"]) == ("done", "h1", True)
    # param applies to block 0 only
    nid = wf.create(sb, "a\n\nb\n\nc", layoutMode="todo")
    assert wf.get(nid)["data"]["layoutMode"] == "todo"
    assert [(c["name"], c["data"]["layoutMode"]) for c in wf.children(nid)] == [("c", "p")]


def test_empty_names(wf: WF) -> None:
    sb = wf.sandbox()
    for name in ("", "   ", "\n\n\n"):
        nid = wf.create(sb, name)
        assert wf.get(nid)["name"] == ""
        assert wf.list(nid) == []


def test_completion_leak(wf: WF) -> None:
    sb = wf.sandbox()
    nid = wf.create(sb, "p\n\n- [x] a\n\n- [ ] B\n\n- C")
    p = wf.get(nid)
    assert (p["name"], p["completed"]) == ("p", False)
    kids = [(c["name"], c["data"]["layoutMode"], c["completed"]) for c in wf.children(nid)]
    assert kids == [("a", "todo", True), ("B", "todo", True), ("C", "bullets", True)]


def test_multiline_children_flat_and_note_on_block0(wf: WF) -> None:
    sb = wf.sandbox()
    nid = wf.create(sb, "- a\n  - b\n    - c", note="N")
    assert wf.get(nid)["note"] == "N"
    kids = wf.children(nid)
    assert [(c["name"], c["priority"], c["note"]) for c in kids] == [("b", 0, ""), ("c", 1, "")]
    assert all(wf.list(c["id"]) == [] for c in kids)


def test_parent_not_bumped_by_create(wf: WF) -> None:
    sb = wf.sandbox()
    before = wf.get(sb)["modifiedAt"]
    wf.clock.advance(5)
    wf.create(sb, "child")
    assert wf.get(sb)["modifiedAt"] == before


# ----------------------------------------------------------------- pure-function unit tests


@pytest.mark.parametrize(
    "src,out",
    [
        ("***bi***", "<b><i>bi</b></i>"),
        ("**unclosed", "<i></i>unclosed"),
        ("****x****", "<b></b>x<b></b>"),
        ("~~~x~~~", "<s>~x</s>~"),
        ("[w](https://en.wikipedia.org/wiki/Foo_(bar))", '<a href="https://en.wikipedia.org/wiki/Foo_(bar">w</a>)'),
        ("[2026-01-15](https://e.com)", '<time startYear="2026" startMonth="1" startDay="15">Thu, Jan 15, 2026</time>(https://e.com)'),
        ("[2026-01-15 00:00]", '<time startYear="2026" startMonth="1" startDay="15" startHour="0" startMinute="0">Thu, Jan 15, 2026 at 12:00 AM</time>'),
        ("[2025-02-29]", "[2025-02-29]"),
        ("[2026-01-15 24:00]", "[2026-01-15 24:00]"),
        ("\\*not italic\\*", "*not italic*"),
        ("a\\\\b", "a\\\\b"),
        ('<b title="**t**">x</b>', '<b title="<b>t</b>">x</b>'),
    ],
)
def test_inline_markdown_quirks(src: str, out: str) -> None:
    assert inline_markdown(src) == out


def test_transform_never_empty() -> None:
    assert [b.name for b in transform_create_name("")] == [""]
