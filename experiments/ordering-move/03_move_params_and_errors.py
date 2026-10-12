"""Move: undocumented position values / parameter names, and error cases
(self, descendant, nonexistent parent/node, deleted parent/node).
"""
import sys, os, uuid
sys.path.insert(0, os.path.dirname(__file__))
from common import *

sb = c.make_sandbox("ordering-move: move params and errors")
out("sandbox", sb)
P = mk(sb, "P", position="bottom")
Q = mk(sb, "Q", position="bottom")
for i in range(1, 5):
    mk(P, f"p{i}", position="bottom")
M = mk(Q, "mover", position="bottom")
anchor = [n for n in kids(P) if n["name"] == "p2"][0]["id"]


def try_move(label, nid, body, unscoped=False):
    r = c.request("POST", f"/api/v1/nodes/{nid}/move", json=body, unscoped=unscoped)
    g = c.get(M).json()["node"]
    where = "P" if g["parent_id"] == P else "Q" if g["parent_id"] == Q else g["parent_id"]
    out(f"{label:45s} body={json.dumps(body)[:90]:90s} -> {dump(r)}")
    out(f"{'':45s}   mover now in {where} prio={g['priority']}; P={sorted((n['priority'], n['name']) for n in kids(P))}")
    # put mover back in Q for next test
    if g["parent_id"] != Q:
        c.move(M, Q)
    return r


positions = ["top", "bottom", "TOP", "Bottom", "middle", "", "first", "last",
             "start", "end", "after", "before", 0, 1, 2, "1", -1, None, True, 250]
for p in positions:
    try_move(f"position={p!r}", M, {"parent_id": P, "position": p})

extras = [
    {"priority": 250},
    {"index": 1},
    {"after": anchor},
    {"before": anchor},
    {"after_id": anchor},
    {"sibling_id": anchor},
    {"position": "after", "sibling_id": anchor},
    {"position": "after", "after": anchor},
]
for e in extras:
    try_move(f"extra {list(e)}", M, {"parent_id": P, **e})

out("\n### error cases")
# self
r = c.move(P, P); out("move P into itself ->", dump(r)); out("  P parent now:", c.get(P).json()["node"]["parent_id"] == sb)
# descendant
p1 = [n for n in kids(P) if n["name"] == "p1"][0]["id"]
g1 = mk(p1, "p1-child")
r = c.move(P, p1); out("move P into its child p1 ->", dump(r)); out("  P parent now sb?:", c.get(P).json()["node"]["parent_id"] == sb)
r = c.move(P, g1); out("move P into its grandchild ->", dump(r)); out("  P parent now sb?:", c.get(P).json()["node"]["parent_id"] == sb)
# nonexistent parent (random uuid) -- unscoped only to bypass allowlist for an id that cannot exist
fake = str(uuid.uuid4())
r = c.request("POST", f"/api/v1/nodes/{M}/move", json={"parent_id": fake}, unscoped=True)
out("move to random-uuid parent ->", dump(r)); out("  mover parent still Q?:", c.get(M).json()["node"]["parent_id"] == Q)
r = c.request("POST", f"/api/v1/nodes/{M}/move", json={"parent_id": "zzzz-not-an-id"}, unscoped=True)
out("move to garbage parent ->", dump(r)); out("  mover parent still Q?:", c.get(M).json()["node"]["parent_id"] == Q)
# nonexistent node
r = c.request("POST", f"/api/v1/nodes/{uuid.uuid4()}/move", json={"parent_id": P}, unscoped=True)
out("move random-uuid node ->", dump(r))
# parent given by short id and URL
r = c.move(M, P.replace("-", "")[-12:]); out("move to short-id parent ->", dump(r), "parent==P:", c.get(M).json()["node"]["parent_id"] == P)
r = c.move(M, "https://workflowy.com/#/" + Q.replace("-", "")[-12:]); out("move to URL parent ->", dump(r), "parent==Q:", c.get(M).json()["node"]["parent_id"] == Q)
# node addressed by short id
r = c.request("POST", f"/api/v1/nodes/{M.replace('-', '')[-12:]}/move", json={"parent_id": P}); out("move node by short id ->", dump(r), "parent==P:", c.get(M).json()["node"]["parent_id"] == P)
c.move(M, Q)
# deleted parent
D = mk(sb, "to-be-deleted", position="bottom")
out("delete D ->", dump(c.delete(D)))
r = c.move(M, D); out("move into deleted parent ->", dump(r)); out("  mover parent:", c.get(M).json()["node"]["parent_id"] == Q)
# deleted node
D2 = mk(sb, "to-be-deleted-2", position="bottom")
out("delete D2 ->", dump(c.delete(D2)))
r = c.move(D2, P); out("move deleted node ->", dump(r))
# missing body / wrong content-type
r = c.request("POST", f"/api/v1/nodes/{M}/move", content=b"parent_id=" + P.encode(), headers={"Content-Type": "application/x-www-form-urlencoded"})
out("form-encoded body ->", dump(r)); out("  mover parent:", "P" if c.get(M).json()["node"]["parent_id"] == P else "Q")
c.move(M, Q)
# GET on move endpoint
r = c.request("GET", f"/api/v1/nodes/{M}/move"); out("GET /move ->", r.status_code, r.text[:200])
