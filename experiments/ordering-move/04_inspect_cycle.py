"""Inspect the result of moving P into its own descendant (03_...): did it
really create a cycle? Read-only."""
import sys, os
sys.path.insert(0, os.path.dirname(__file__))
from common import *

sb = "7f261b8d-2e2b-4a1b-a776-207c6f28b6d1"
out("sandbox children:", [(n["name"], n["id"]) for n in kids(sb)])
names = {}
for n in json.load(open(".prod-state/allowed_ids.json"))["ids"]:
    pass
# Walk from the ids we know
import re
log = [json.loads(l) for l in open(".prod-state/requests.jsonl") if '"ordering-move"' in l]
ids = {}
for r in log:
    if r["method"] == "POST" and r["path"] == "/api/v1/nodes" and r["status"] == 200 and r["request_json"]["parent_id"] in (sb,) + tuple(ids.values()):
        ids[r["request_json"]["name"]] = r["response"]["item_id"]
out(ids)
for nm in ("P", "p1", "p1-child"):
    n = c.get(ids[nm])
    out(nm, dump(n))
for nm in ("P", "p1", "p1-child"):
    out("children of", nm, [(n["name"], n["parent_id"]) for n in kids(ids[nm])])
