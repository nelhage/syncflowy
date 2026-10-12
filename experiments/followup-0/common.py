"""Shared helpers for followup-0 (priority edge cases + concurrency).

All production access goes through ProdClient (rate-limited, scope-guarded).
"""
import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "src"))
from syncflowy.prod_client import ProdClient  # noqa: E402

c = ProdClient()


def out(*a):
    print(*a, flush=True)


def ok(r):
    if r.status_code >= 400:
        out("FAIL", r.request.method, r.request.url.path, r.status_code, r.text)
        r.raise_for_status()
    return r.json()


def mk(parent, name, cl=None, **kw):
    return ok((cl or c).create(parent, name, **kw))["item_id"]


def raw(parent):
    """Children in the order the server returned them."""
    return ok(c.list(parent))["nodes"]


def pri(parent):
    """[(name, priority)] in server (list) order."""
    return [(n["name"], n["priority"]) for n in raw(parent)]


def by_pri(parent):
    return sorted(pri(parent), key=lambda x: x[1])


def show(parent, label):
    nodes = raw(parent)
    lst = [(n["name"] or f"<mirror {n['id'][-6:]}>", n["priority"]) for n in nodes]
    out(f"  {label}: list order={lst}")
    out(f"  {'':{len(label)}}  sorted     ={sorted(lst, key=lambda x: x[1])}")
    return nodes
