"""Follow-ups: delayed consistency of mirror edits, timestamps, cycles via move,
deleting containers of mirrors/origins, mirroring completed nodes, trash probes."""

import time

from common import c, children, create, get, show

# --- re-read nodes from 03 (created minutes ago) to check for delayed propagation
print("== delayed re-read of experiment 03 nodes")
get("df4441f5-0176-4146-ad60-a12993560042", "GET M (03) now")
get("c60064cd-5db2-40fd-9e20-b1a827e9a5bc", "GET O (03) now")
children("df4441f5-0176-4146-ad60-a12993560042", "list M (03) now -- children moved with mirror?")

S = c.make_sandbox("delete-mirror: followups")
print("sandbox", S)
A = create(S, "A (mirror dest)", position="bottom")
B = create(S, "B (origin container)", position="bottom")
O = create(B, "O origin")
oc = create(O, "oc")
time.sleep(1.2)

print("\n== timestamps: which nodes does mirror create/delete touch?")
before = {k: get(v, f"GET {k} before") for k, v in dict(A=A, B=B, O=O, oc=oc).items()}
time.sleep(2.1)
M = show("MIRROR O -> A", c.mirror(O, A))["item_id"]
after = {k: get(v, f"GET {k} after mirror") for k, v in dict(A=A, B=B, O=O, oc=oc).items()}
for k in before:
    print(f"  {k}: modifiedAt {before[k]['modifiedAt']} -> {after[k]['modifiedAt']}")
time.sleep(2.1)
show("COMPLETE O", c.complete(O))
mm = get(M, "GET M after completing origin")
print("  M modifiedAt", mm["modifiedAt"], "completed", mm["completed"])
show("UNCOMPLETE O", c.uncomplete(O))
time.sleep(2.1)
show("UPDATE O name", c.update(O, name="O renamed"))
print("  M modifiedAt after origin rename:", get(M, "GET M")["modifiedAt"])
time.sleep(2.1)
show("UPDATE M name", c.update(M, name="M renamed"))
print("  O modifiedAt after mirror rename:", get(O, "GET O")["modifiedAt"])
time.sleep(2.1)
t = int(time.time())
show("DELETE M via /mirror", c.request("DELETE", f"/api/v1/nodes/{M}/mirror"))
print("  delete at ~", t)
for k, v in dict(A=A, B=B, O=O, oc=oc).items():
    print(f"  {k} modifiedAt after mirror delete:", get(v, f"GET {k}")["modifiedAt"])

print("\n== mirror of a completed / todo node")
T = create(B, "T todo completed", layoutMode="todo", position="bottom")
show("COMPLETE T", c.complete(T))
MT = show("MIRROR T -> A", c.mirror(T, A, position="bottom"))["item_id"]
get(MT, "GET mirror of completed todo")

print("\n== cycles via move")
M5 = show("MIRROR O -> A", c.mirror(O, A))["item_id"]
show("MOVE O -> M5 (into its own mirror)", c.move(O, M5))
get(O, "GET O")
show("MOVE B -> M5 (origin's parent into origin's mirror)", c.move(B, M5))
get(B, "GET B")
show("MOVE O -> oc (into own child)", c.move(O, oc))
show("MOVE M5 -> oc (mirror into origin's child)", c.move(M5, oc))
get(M5, "GET M5")

print("\n== delete a container that holds a mirror")
H = create(S, "H holds a mirror", position="bottom")
MH = show("MIRROR O -> H", c.mirror(O, H))["item_id"]
show("DELETE H", c.delete(H))
show("GET MH", c.get(MH))
get(O, "GET O (should survive)")
children(O, "list O (children survive)")

print("\n== delete a container that holds an origin with a mirror elsewhere")
H2 = create(S, "H2 holds an origin", position="bottom")
O3 = create(H2, "O3 origin inside H2")
MO3 = show("MIRROR O3 -> A", c.mirror(O3, A, position="bottom"))["item_id"]
show("DELETE H2", c.delete(H2))
show("GET O3", c.get(O3))
show("GET MO3 (mirror of deleted-by-ancestor origin)", c.get(MO3))
show("MIRROR MO3 -> S", c.mirror(MO3, S, position="bottom"))
show("UPDATE MO3 name (orphan mirror)", c.update(MO3, name="orphan mirror renamed"))
get(MO3, "GET MO3 after rename")

print("\n== delete sibling then insert at top: priority computation after delete")
P = create(S, "P priorities", position="bottom")
p1 = create(P, "p1", position="bottom")
p2 = create(P, "p2", position="bottom")
p3 = create(P, "p3", position="bottom")
show("DELETE p1 (lowest priority)", c.delete(p1))
create(P, "new top after deleting p1")
show("DELETE p3 (highest priority)", c.delete(p3))
create(P, "new bottom after deleting p3", position="bottom")
children(P, "list P")

print("\n== trash probes")
show("GET /api/v1/trash", c.request("GET", "/api/v1/trash", unscoped=True))
show("DELETE /api/v1/nodes (no id)", c.request("DELETE", "/api/v1/nodes", unscoped=True))
show("GET deleted with include_deleted", c.request("GET", f"/api/v1/nodes/{p1}", params={"include_deleted": "true"}))
show("POST /nodes/:deleted/restore", c.request("POST", f"/api/v1/nodes/{p1}/restore"))
show("POST /nodes/:deleted/undelete", c.request("POST", f"/api/v1/nodes/{p1}/undelete"))
