"""Move validation odds and ends: non-string parent_id, and precedence of
errors when both parent and position are bad. unscoped=True is used only
because the parent ids here are garbage values that cannot exist; the node
being moved is our own sandbox node. (Deliberately NOT tested: missing
parent_id, parent_id null/"" -- these could resolve to the account root.)"""
import sys, os, uuid
sys.path.insert(0, os.path.dirname(__file__))
from common import *

sb = c.make_sandbox("ordering-move: move validation")
out("sandbox", sb)
m = mk(sb, "m", position="bottom")
for body in ({"parent_id": 123}, {"parent_id": ["x"]},
             {"parent_id": str(uuid.uuid4()), "position": "middle"}):
    r = c.request("POST", f"/api/v1/nodes/{m}/move", json=body, unscoped=True)
    out(json.dumps(body), "->", dump(r))
r = c.request("POST", f"/api/v1/nodes/{uuid.uuid4()}/move", json={"parent_id": sb, "position": "middle"}, unscoped=True)  # random node id cannot exist
out("bad node + bad position ->", dump(r))
r = c.request("POST", f"/api/v1/nodes/{m}/move", content=b"{not json", headers={"Content-Type": "application/json"})
out("invalid JSON ->", dump(r))
out("m parent still sb:", c.get(m).json()["node"]["parent_id"] == sb)
