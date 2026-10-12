"""Q4: list order. Create early (under another parent), then in P create
bottom, top, bottom, top, then create late (under another parent). Move the
early node and the late node into P (top and bottom variants). Is list order
ascending creation sequence regardless of parent/priority? Also check the
whole log (.prod-state/requests.jsonl) for every multi-node list response.
"""
import json
from pathlib import Path

from common import c, mk, ok, out, show

sb = c.make_sandbox("followup-0: 05 list order")
other = mk(sb, "other parent", position="bottom")
early = mk(other, "EARLY (created first, elsewhere)", position="bottom")
p = mk(sb, "P", position="bottom")
for i, pos in enumerate(["bottom", "top", "bottom", "top"]):
    mk(p, f"p{i}-{pos}", position=pos)
late = mk(other, "LATE (created last, elsewhere)", position="bottom")
show(p, "P before moves")
ok(c.move(late, p, position="top"))
show(p, "after LATE->top")
ok(c.move(early, p, position="bottom"))
nodes = show(p, "after EARLY->bottom")
out("  createdAt:", [(n["name"][:5], n["createdAt"]) for n in nodes])
# Move them out and back with reversed positions
ok(c.move(early, other)); ok(c.move(late, other))
ok(c.move(early, p, position="top")); ok(c.move(late, p, position="bottom"))
show(p, "after out-and-back (EARLY top, LATE bottom)")

# Log-wide check
log = Path(__file__).resolve().parents[2] / ".prod-state/requests.jsonl"
tot = pri_sorted = ca_sorted = 0
bad_ca = []
for line in open(log):
    r = json.loads(line)
    if r["method"] != "GET" or r["path"] != "/api/v1/nodes" or r["status"] != 200:
        continue
    ns = r["response"].get("nodes") if isinstance(r["response"], dict) else None
    if not ns or len(ns) < 2:
        continue
    tot += 1
    pr = [n["priority"] for n in ns]
    ca = [n["createdAt"] for n in ns]
    pri_sorted += pr == sorted(pr)
    if ca == sorted(ca):
        ca_sorted += 1
    else:
        bad_ca.append(r["ts"])
out(f"log: {tot} multi-node list responses; priority-sorted {pri_sorted}; "
    f"createdAt-sorted {ca_sorted}; not createdAt-sorted at ts={bad_ca[:5]}")
