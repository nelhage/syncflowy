"""Hypothesis: GET /nodes returns children in creation order (not priority,
not move order). Also: does renumbering bump siblings' modifiedAt?
And re-check: move endpoint with a short id in the path."""
import sys, os
sys.path.insert(0, os.path.dirname(__file__))
from common import *

sb = c.make_sandbox("ordering-move: list order + renumber mtime")
out("sandbox", sb)
H1 = mk(sb, "H1", position="bottom")
H2 = mk(sb, "H2", position="bottom")
z = {}
for i in range(1, 7):
    z[i] = mk(H2 if i % 2 else H1, f"z{i}", position="top")
out("H2 raw:", [(n["name"], n["priority"], n["createdAt"]) for n in kids(H2)])
# move H1's nodes into H2 in reverse creation order, mixed top/bottom
c.move(z[6], H2, position="top")
c.move(z[4], H2, position="bottom")
c.move(z[2], H2, position="top")
ns = kids(H2)
out("H2 raw after moves:", [(n["name"], n["priority"], n["createdAt"]) for n in ns])
out("  raw == creation order:", [n["name"] for n in ns] == [f"z{i}" for i in range(1, 7)])
out("  priority order:", [n["name"] for n in sorted(ns, key=lambda n: n["priority"])])

out("\n## renumber: are siblings' modifiedAt bumped?")
# drive min priority down to 0 by repeated top moves of z1 <-> others
time.sleep(1.5)
before = {n["name"]: (n["priority"], n["modifiedAt"]) for n in kids(H2)}
for _ in range(12):
    ns = sorted(kids(H2), key=lambda n: n["priority"])
    last = ns[-1]
    r = c.move(last["id"], H2, position="top")
    cur = {n["name"]: (n["priority"], n["modifiedAt"]) for n in kids(H2)}
    renum = sum(1 for k in cur if k != last["name"] and cur[k][0] != before[k][0])
    out(f"move {last['name']} top -> prio {cur[last['name']][0]}; siblings renumbered: {renum}")
    if renum:
        out("  before:", before)
        out("  after :", cur)
        break
    before = cur

out("\n## move with short id in path (re-check)")
short = z[1].replace("-", "")[-12:]
r = c.request("POST", f"/api/v1/nodes/{short}/move", json={"parent_id": H1}); out("short-id path ->", dump(r))
r = c.request("GET", f"/api/v1/nodes/{short}"); out("GET short-id ->", r.status_code)
r = c.request("POST", f"/api/v1/nodes/{z[1]}/move", json={"parent_id": H1}); out("full-id path ->", dump(r))
