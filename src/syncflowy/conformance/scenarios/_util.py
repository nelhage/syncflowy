"""Helpers shared by scenario modules.

Fake ids: every fake id's last 12 hex digits start with ten zeros. The
production backend recognises that pattern as "deliberately nonexistent" and
allows probing it (see ProdBackend._is_ours_or_fake). Never use any other
made-up id in a scenario.
"""

from __future__ import annotations

FAKE_UUID = "00000000-0000-4000-8000-000000000001"
FAKE_UUID2 = "00000000-0000-4000-8000-000000000002"
FAKE_SHORT = "000000000003"  # a 12-hex short id that matches nothing

# A garbage parent_id (contains a space so it can never be a shortcut key).
GARBAGE = "zz-conformance no such node"


def short(node_id: str) -> str:
    """12-hex short id (last 12 hex digits)."""
    return node_id.replace("-", "")[-12:]


def upper(node_id: str) -> str:
    return node_id.upper()


def dashless(node_id: str) -> str:
    return node_id.replace("-", "")


def url(node_id: str, host: str = "https://workflowy.com", suffix: str = "") -> str:
    return f"{host}/#/{short(node_id)}{suffix}"


def create_and_show(ctx, parent: str, name, **fields) -> str | None:
    """Create ``name`` under ``parent``; GET it and list its children.

    Used by text scenarios: shows the transformed name, the layoutMode, and
    any children a multi-line create produced. Returns the id (or None if the
    create failed, in which case only the failing create is in the transcript).
    """
    r = ctx.try_create(parent, name, **fields)
    if r.status != 200:
        return None
    nid = r.json["item_id"]
    ctx.get(nid)
    ctx.list(nid)
    return nid


def node_sandbox(ctx, name: str = "P") -> str:
    """Create a fresh container node under the root (aliased ``name``)."""
    return ctx.alias(ctx.create(ctx.root, name), name)
