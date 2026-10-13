"""Ghost ``mirror_ids`` entries (shells deleted through *production*, which
leaves them in the origin's registry): does one block a beta delete?

Finds scratchpad origins in the last beta export logged in
.prod-state/requests.jsonl (04_export_and_delete) whose registry lists only
non-live ids, then tries a beta DELETE on the first one.

    SYNCFLOWY_TAG=beta-mirrors direnv exec . uv run python experiments/beta-mirrors/05_ghost_registry.py
"""

from __future__ import annotations

import json

from syncflowy.prod_client import BETA_URL, LOG_FILE, ProdClient

b = ProdClient(base_url=BETA_URL)
recs = [json.loads(line) for line in open(LOG_FILE)]
exp = [r for r in recs if r["path"].startswith("/api/v1/nodes-export") and r.get("host") == BETA_URL][-1]
idx = {n["id"]: n for n in exp["response"]["nodes"]}
scratch = b.scratchpad_id()


def in_scratchpad(i):
    seen = set()
    while i and i not in seen:
        if i == scratch:
            return True
        seen.add(i)
        i = (idx.get(i) or {}).get("parent_id")
    return False


ghost_only = [
    o for o, n in idx.items()
    if in_scratchpad(o) and (ids := n["data"].get("mirror", {}).get("mirror_ids")) and not any(m in idx for m in ids)
]
print("scratchpad origins whose registry holds only ghosts:", len(ghost_only))
o = ghost_only[0]
r = b.get(o)
print("beta GET", r.status_code, r.text)
r = b.delete(o)
print("beta DELETE", r.status_code, r.text)
r = b.get(o)
print("beta GET after", r.status_code, r.text)
