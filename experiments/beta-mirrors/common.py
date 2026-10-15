"""Shared helpers for the beta-mirror probes: ``b`` (beta host) and ``p`` (production, same account)."""

from __future__ import annotations

import json
import sys
import time

from syncflowy.prod_client import BETA_URL, ProdClient

b = ProdClient(base_url=BETA_URL)
p = ProdClient()


def show(label: str, r) -> object:
    try:
        body = r.json()
    except Exception:
        body = r.text
    host = "beta" if "beta." in str(r.request.url) else "prod"
    req = r.request
    print(f"--- {label}")
    print(f"    [{host}] {req.method} {req.url.path}{'?' + req.url.query.decode() if req.url.query else ''}")
    if req.content:
        print(f"    req: {req.content.decode()}")
    print(f"    -> {r.status_code}")
    print("    " + json.dumps(body))
    sys.stdout.flush()
    return body


def ok(label: str, r) -> object:
    body = show(label, r)
    if r.status_code != 200:
        raise SystemExit(f"unexpected {r.status_code} at {label}")
    return body


def tick() -> None:
    time.sleep(1.2)  # timestamps are whole seconds


def section(title: str) -> None:
    print(f"\n===== {title}")


