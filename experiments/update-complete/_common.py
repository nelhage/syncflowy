"""Shared helpers for update/complete experiments.

Run scripts as:
  cd /home/nelhage/code/syncflowy && SYNCFLOWY_TAG=update-complete \
      direnv exec . uv run python experiments/update-complete/<script>.py
"""

import json
import time

from syncflowy.prod_client import ProdClient

c = ProdClient()


def show(label, resp):
    """Print a request result compactly; return parsed body (or text)."""
    try:
        body = resp.json()
    except Exception:
        body = resp.text
        if len(body) > 300:
            body = body[:120] + f"...<{len(body)} chars total>"
    ct = resp.headers.get("content-type")
    print(f"--- {label}: {resp.status_code} [{ct}]")
    print("    " + json.dumps(body, ensure_ascii=False))
    return body


def node(nid):
    r = c.get(nid)
    if r.status_code != 200:
        show(f"GET {nid}", r)
        return None
    return r.json()["node"]


def brief(n):
    if n is None:
        return None
    keys = ["name", "note", "completed", "completedAt", "createdAt", "modifiedAt", "priority", "parent_id", "data"]
    return {k: n.get(k) for k in keys}


def pnode(label, nid):
    n = node(nid)
    print(f"=== {label}: " + json.dumps(brief(n), ensure_ascii=False))
    return n


def mk(parent, name, **extra):
    r = c.create(parent, name, **extra)
    if r.status_code != 200:
        show("create", r)
        r.raise_for_status()
    return r.json()["item_id"]


def children(parent):
    r = c.list(parent)
    return r.json().get("nodes", [])


def now():
    return time.time()
