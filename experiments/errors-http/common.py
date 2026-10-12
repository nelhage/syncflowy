"""Shared helpers for errors-http experiments.

Run scripts from the repo root:

    SYNCFLOWY_TAG=errors-http direnv exec . uv run python experiments/errors-http/<script>.py

All production traffic goes through syncflowy.prod_client.ProdClient.
"""

from __future__ import annotations

import json
import re
import sys
import uuid
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "src"))

from syncflowy.prod_client import ProdClient  # noqa: E402

# Response headers that are boring / always-present infrastructure noise.
BORING = {
    "date",
    "transfer-encoding",
    "connection",
    "content-security-policy",
    "referrer-policy",
    "service-worker-allowed",
    "vary",
    "x-content-type-options",
    "x-frame-options",
    "x-xss-protection",
    "strict-transport-security",
    "content-encoding",
}

c = ProdClient()


def fake_uuid() -> str:
    """A random UUID that cannot exist in the account."""
    return str(uuid.uuid4())


def show(label: str, r, full_headers: bool = False) -> dict:
    hdrs = {
        k: v
        for k, v in r.headers.items()
        if full_headers or k.lower() not in BORING
    }
    body = r.text
    out = {
        "label": label,
        "req": f"{r.request.method} {r.request.url}",
        "status": r.status_code,
        "headers": hdrs,
        "body": body if len(body) < 1500 else body[:1500] + "...<truncated>",
        "elapsed_ms": round(r.elapsed.total_seconds() * 1000),
    }
    print(json.dumps(out, ensure_ascii=False))
    return out


def norm_to_uuid(variant: str, known: set[str]) -> str | None:
    """Client-side check: does `variant` (some spelling of an id) refer to one
    of our known sandbox UUIDs? Used before an unscoped request so that we only
    ever send id-variants of nodes we own inside the scratchpad."""
    v = variant
    m = re.search(r"#/([0-9A-Fa-f]{12})/?$", v)
    if m:
        v = m.group(1)
    v = v.replace("-", "").lower()
    for k in known:
        kk = k.replace("-", "")
        if v == kk or (len(v) == 12 and kk.endswith(v)):
            return k
    return None


def variant_request(method, path, known, variant_ids, **kw):
    """Issue an unscoped request whose id-variants have been verified to refer
    to scratchpad sandbox nodes we created (see norm_to_uuid)."""
    for v in variant_ids:
        if norm_to_uuid(v, known) is None:
            raise RuntimeError(f"refusing: {v!r} is not a variant of a known sandbox id")
    return c.request(method, path, unscoped=True, **kw)
