"""Shared helpers for followup-1 experiments (timestamps + mirror identity).

All production access goes through ProdClient (rate limited, scope guarded).
`show` prints the raw response body in the server's own key order (no
sort_keys), so transcripts can be copied byte-for-byte.
"""
from __future__ import annotations

import json
import sys
import time

from syncflowy.prod_client import ProdClient

c = ProdClient()
GAP = 2.1  # > 1 s timestamp resolution, so any bump is visible


def show(label: str, r):
    try:
        body = r.json()
    except Exception:
        body = r.text
    req = r.request
    q = "?" + req.url.query.decode() if req.url.query else ""
    print(f"--- {label}  [t={time.time():.2f}]")
    print(f"    {req.method} {req.url.path}{q}")
    if req.content:
        print(f"    req: {req.content.decode()}")
    print(f"    -> {r.status_code} {json.dumps(body)}")
    sys.stdout.flush()
    return body


def create(parent: str, name: str, **kw) -> str:
    r = c.create(parent, name, **kw)
    b = show(f"create {name!r}", r)
    r.raise_for_status()
    return b["item_id"]


def mirror(src: str, parent: str, **kw) -> str:
    r = c.mirror(src, parent, **kw)
    b = show(f"mirror {src[-12:]} -> {parent[-12:]}", r)
    r.raise_for_status()
    return b["item_id"]


def mtimes(ids: dict[str, str]) -> dict[str, int | None]:
    out = {}
    for name, i in ids.items():
        r = c.get(i)
        out[name] = r.json()["node"]["modifiedAt"] if r.status_code == 200 else None
    return out


def wait():
    time.sleep(GAP)


def probe(label: str, ids: dict[str, str], action):
    """Wait, snapshot modifiedAt of `ids`, run `action`, snapshot again, report diff."""
    wait()
    before = mtimes(ids)
    t = time.time()
    res = action()
    after = mtimes(ids)
    bumped = [k for k in ids if before[k] != after[k]]
    print(f"=== {label}: action at {t:.2f}")
    for k in ids:
        flag = "BUMPED" if k in bumped else "same"
        print(f"      {k:14s} {before[k]} -> {after[k]}  {flag}")
    print(f"    SUMMARY {label}: bumped={bumped}")
    sys.stdout.flush()
    return res, bumped
