"""Shared helpers for the `create` experiments."""
import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "src"))
from syncflowy.prod_client import ProdClient  # noqa: E402

INTERESTING_HEADERS = ("content-type", "x-ratelimit-limit", "x-ratelimit-remaining",
                       "retry-after", "server", "cache-control", "vary")


def show(label, r, headers=False):
    try:
        body = r.json()
    except Exception:
        body = r.text[:2000]
    print(f"--- {label}: {r.request.method} {r.request.url.path} -> {r.status_code}")
    if r.request.content:
        print("    req:", r.request.content.decode(errors="replace")[:500])
    if headers:
        print("    headers:", {k: v for k, v in r.headers.items()})
    print("    resp:", json.dumps(body, ensure_ascii=False)[:3000])
    return body


def children(c, pid):
    r = c.list(pid)
    r.raise_for_status()
    return sorted(r.json()["nodes"], key=lambda n: n["priority"])


def dump_children(c, pid, label="children"):
    kids = children(c, pid)
    print(f"    {label}:")
    for n in kids:
        print("     ", json.dumps(n, ensure_ascii=False))
    return kids
