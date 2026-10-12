"""Shared helpers for delete/mirror experiments.

Every call goes through ProdClient (rate limited + scope guarded). `show`
prints a compact record of the request/response so the script output itself
serves as an experiment transcript.
"""

from __future__ import annotations

import json
import sys
import time

from syncflowy.prod_client import ProdClient

c = ProdClient()


def show(label: str, r) -> dict | str:
    try:
        body = r.json()
    except Exception:
        body = r.text
    req = r.request
    print(f"--- {label}")
    print(f"    {req.method} {req.url.path}{'?' + req.url.query.decode() if req.url.query else ''}")
    if req.content:
        print(f"    req: {req.content.decode()}")
    print(f"    -> {r.status_code} content-type={r.headers.get('content-type')}")
    print("    " + json.dumps(body, indent=None, sort_keys=True))
    sys.stdout.flush()
    return body


def children(parent_id: str, label: str = "list") -> list[dict]:
    r = c.list(parent_id)
    body = show(label, r)
    if r.status_code != 200:
        return []
    return sorted(body["nodes"], key=lambda n: n["priority"])


def get(node_id: str, label: str = "get") -> dict | None:
    r = c.get(node_id)
    body = show(label, r)
    return body.get("node") if r.status_code == 200 and isinstance(body, dict) else None


def create(parent_id: str, name: str, label: str | None = None, **kw) -> str:
    r = c.create(parent_id, name, **kw)
    body = show(label or f"create {name!r}", r)
    r.raise_for_status()
    return body["item_id"]


def now() -> int:
    return int(time.time())
