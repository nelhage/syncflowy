"""Shared helpers for ordering/move experiments. All prod access via ProdClient."""
import json
import sys
import time

from syncflowy.prod_client import ProdClient

c = ProdClient()


def out(*a):
    print(*a, flush=True)


def kids(parent):
    """List children; return (raw_order_names, {name: priority}, raw nodes)."""
    r = c.list(parent)
    r.raise_for_status()
    nodes = r.json()["nodes"]
    return nodes


def show(parent, label=""):
    nodes = kids(parent)
    raw = [(n["name"], n["priority"]) for n in nodes]
    srt = sorted(raw, key=lambda x: x[1])
    out(f"--- {label}")
    out("  raw order :", raw)
    out("  sorted?   :", raw == srt)
    return nodes


def pmap(nodes):
    return {n["name"]: n["priority"] for n in nodes}


def diff(before, after):
    ch = {k: (before.get(k), after.get(k)) for k in set(before) | set(after)
          if before.get(k) != after.get(k)}
    return ch


def mk(parent, name, **kw):
    r = c.create(parent, name, **kw)
    if r.status_code >= 400:
        out("CREATE FAIL", r.status_code, r.text)
        r.raise_for_status()
    return r.json()["item_id"]


def dump(r):
    try:
        body = r.json()
    except Exception:
        body = r.text
    return f"{r.status_code} {json.dumps(body) if not isinstance(body, str) else body!r}"
