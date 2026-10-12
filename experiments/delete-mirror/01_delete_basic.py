"""Delete semantics: subtree, re-delete, nonexistent, sibling priorities, parent modifiedAt."""

import time
import uuid

from common import c, children, create, get, show

S = c.make_sandbox("delete-mirror: delete basics")
print("sandbox", S)
P = create(S, "P (parent)")
c1 = create(P, "c1", position="bottom")
c2 = create(P, "c2 (to delete, has child)", position="bottom")
c3 = create(P, "c3", position="bottom")
g = create(c2, "g (grandchild of P, child of c2)")
gg = create(g, "gg (great-grandchild)")

before = children(P, "list P before delete")
pnode_before = get(P, "GET P before")
snode_before = get(S, "GET sandbox before")
print("sleeping 3s so timestamps can move")
time.sleep(3)

t0 = time.time()
show("DELETE c2", c.delete(c2))
show("GET c2 immediately after delete", c.get(c2))
show("GET g (child of deleted)", c.get(g))
show("GET gg (grandchild of deleted)", c.get(gg))
show("LIST children of deleted c2", c.list(c2))
after = children(P, "list P after delete")
pnode_after = get(P, "GET P after")
snode_after = get(S, "GET sandbox after")
print("delete issued at", int(t0))

print("\nsibling priorities before:", [(n["name"], n["priority"]) for n in before])
print("sibling priorities after: ", [(n["name"], n["priority"]) for n in after])
print("P modifiedAt before/after:", pnode_before["modifiedAt"], pnode_after["modifiedAt"])
print("S modifiedAt before/after:", snode_before["modifiedAt"], snode_after["modifiedAt"])

show("DELETE c2 again", c.delete(c2))
show("DELETE g (already gone via ancestor)", c.delete(g))
show("DELETE random uuid (unscoped)", c.request("DELETE", f"/api/v1/nodes/{uuid.uuid4()}", unscoped=True))
show("DELETE garbage id (unscoped)", c.request("DELETE", "/api/v1/nodes/not-a-real-id", unscoped=True))
show("DELETE random short id (unscoped)", c.request("DELETE", "/api/v1/nodes/0123456789ab", unscoped=True))

# delete by short id
c4 = create(P, "c4 (delete via short id)", position="bottom")
short = c4.replace("-", "")[-12:]
show("DELETE c4 by short id", c.delete(short))
show("GET c4 after short-id delete", c.get(c4))

# write ops against a deleted node
show("UPDATE deleted c2", c.update(c2, name="zombie"))
show("COMPLETE deleted c2", c.complete(c2))
show("MOVE deleted c2", c.move(c2, P))
show("CREATE under deleted c2", c.create(c2, "child of deleted"))
show("MIRROR deleted c2", c.mirror(c2, P))
children(P, "final list P")
