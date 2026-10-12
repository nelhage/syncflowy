"""Loaders for production golden vectors captured by the experiments.

Each loader yields :class:`Case` objects: a create-path ``name`` (plus extra
create params) and the tree production produced -- block 0 and its flat
children, each as (name, layoutMode, completed). ``None`` in an expected
field means "not recorded by that experiment; don't compare".
"""

from __future__ import annotations

import json
from collections.abc import Iterator
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[2]
CREATE = ROOT / "experiments" / "create"
TEXT = ROOT / "experiments" / "text-content" / "results"
LOG = ROOT / ".prod-state" / "requests.jsonl"


@dataclass
class Case:
    source: str
    name: str
    extra: dict[str, Any] = field(default_factory=dict)
    # [(name, layoutMode | None, completed | None)], block 0 first
    tree: list[tuple[str, str | None, bool | None]] = field(default_factory=list)
    note: Any = "__unchecked__"
    status: int = 200
    #: False when the experiment only recorded block 0 (children unknown)
    children_known: bool = True

    @property
    def id(self) -> str:
        return f"{self.source}:{self.name[:40]!r}"


def _lm(d: dict[str, Any]) -> str | None:
    if "lm" in d:
        return d["lm"]
    if "data" in d:
        return d["data"]["layoutMode"]
    return d.get("layoutMode")


def _child(c: Any) -> tuple[str, str | None, bool | None]:
    if isinstance(c, str):
        return (c, None, None)
    if isinstance(c, list):
        return (c[0], c[1]["layoutMode"] if isinstance(c[1], dict) else c[1], None)
    return (c["name"], _lm(c), c.get("completed"))


def create_cases() -> Iterator[Case]:
    for f in ("04_markdown_name", "05_markdown_followup"):
        for r in json.loads((CREATE / f"{f}.out.json").read_text()):
            yield Case(
                f,
                r["input"],
                tree=[(r["name"], _lm(r), r["completed"])] + [_child(c) for c in r["children"]],
                note=r["note"],
            )
    for f, key in (
        ("06_multiline", "tree_under_sandbox"),
        ("07_multiline_determinism", "tree"),
        ("08_line_join_rule", "tree"),
        ("09_line_join_rule2", "tree"),
        ("10_code_fences", "tree"),
    ):
        for r in json.loads((CREATE / f"{f}.out.json").read_text()):
            if r.get("status") != 200:
                continue
            root = r[key][0]
            yield Case(
                f,
                r["input"],
                extra=r.get("extra") or {},
                tree=[(root["name"], _lm(root), root.get("completed"))] + [_child(c) for c in root["children"]],
            )
    for r in json.loads((CREATE / "11_layoutmode_param.out.json").read_text()):
        if r["status"] != 200:
            continue
        yield Case(
            "11_layoutmode_param",
            r["input"],
            extra=r["extra"],
            tree=[(r["name"], r["data"]["layoutMode"], r["completed"])],
            children_known=False,
        )


def text_cases() -> Iterator[Case]:
    for f in (
        "01_unicode_whitespace",
        "02_html_tags",
        "03_markdown",
        "04_md_parser_edges",
        "05_escapes_order_dates",
        "09_control_chars",
        "13_trim_and_md_in_html",
    ):
        d = json.loads((TEXT / f"{f}.json").read_text())
        # these batches sent the same string as name and (usually) note
        extra_of = (lambda v: {"note": v}) if "note" in d["fields"] else (lambda v: {})
        for r in d["cases"]:
            if r.get("create_status") == 200:
                n = r["create_node"]
                yield Case(
                    f,
                    r["input"],
                    extra_of(r["input"]),
                    [(n["name"], _lm(n), n["completed"])],
                    note=n["note"],
                    children_known=False,
                )
            elif r.get("create_status") is not None:
                yield Case(f, r["input"], extra_of(r["input"]), status=r["create_status"])
    for f, key in (("06_multiline_blocks", "create_name"), ("07_multiline_deeper", "cases"), ("08_multiline_rules", "cases")):
        for r in json.loads((TEXT / f"{f}.json").read_text())[key]:
            if r.get("status") != 200:
                continue
            kids = sorted(r.get("children") or [], key=lambda c: c["priority"])
            n = r["node"]
            yield Case(f, r["input"], tree=[(n["name"], _lm(n), n["completed"])] + [_child(c) for c in kids])


def update_cases() -> Iterator[tuple[str, str, str | None, str | None]]:
    """(label, input, expected update name, expected update note)."""
    for f in (
        "01_unicode_whitespace",
        "02_html_tags",
        "03_markdown",
        "04_md_parser_edges",
        "05_escapes_order_dates",
        "09_control_chars",
        "13_trim_and_md_in_html",
    ):
        d = json.loads((TEXT / f"{f}.json").read_text())
        for r in d["cases"]:
            if r.get("update_status") == 200 and r.get("update_node"):
                yield (f"{f}:{r['label']}", r["input"], r["update_node"]["name"], r["update_node"]["note"])


def all_cases() -> list[Case]:
    return [*create_cases(), *text_cases()]


def log_records() -> list[dict[str, Any]]:
    if not LOG.exists():
        return []
    return [json.loads(line) for line in LOG.read_text().splitlines() if line.strip()]
