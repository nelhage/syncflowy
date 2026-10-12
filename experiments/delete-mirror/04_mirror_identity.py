"""Mirror-of-mirror, mirror as parent, self/subtree mirrors, DELETE /mirror, deleting origins/mirrors."""

import uuid

from common import c, children, create, get, show

S = c.make_sandbox("delete-mirror: mirror identity + deletion")
print("sandbox", S)
A = create(S, "A (mirror dest)", position="bottom")
B = create(S, "B (origin container)", position="bottom")
C = create(S, "C (second mirror dest)", position="bottom")
O = create(B, "O origin")
oc1 = create(O, "oc1", position="bottom")
occ = create(oc1, "occ", position="bottom")
M1 = show("MIRROR O -> A", c.mirror(O, A))["item_id"]

print("\n== mirror of a mirror")
r = show("MIRROR M1 -> C", c.mirror(M1, C))
M2 = r.get("item_id")
children(C, "list C")

print("\n== mirror short-id / URL forms for :id and parent_id")
show("MIRROR O(short) -> C", c.mirror(O.replace("-", "")[-12:], C, position="bottom"))
show("MIRROR O -> C(short)", c.mirror(O, C.replace("-", "")[-12:], position="bottom"))
show("MIRROR O -> C(url)", c.mirror(O, "https://workflowy.com/#/" + C.replace("-", "")[-12:], position="bottom"))
children(C, "list C")

print("\n== parent_id is a mirror")
X = create(S, "X (to be mirrored into a mirror)", position="bottom")
r = show("MIRROR X -> M1 (parent is a mirror)", c.mirror(X, M1))
MX = r.get("item_id")
if MX:
    get(MX, "GET MX")
children(M1, "list M1")
children(O, "list O")

print("\n== mirror onto itself / into own subtree")
show("MIRROR O -> O", c.mirror(O, O))
show("MIRROR O -> oc1 (child of origin)", c.mirror(O, oc1))
show("MIRROR O -> occ (grandchild of origin)", c.mirror(O, occ))
show("MIRROR O -> M1 (its own mirror)", c.mirror(O, M1))
show("MIRROR M1 -> M1", c.mirror(M1, M1))
show("MIRROR oc1 -> O (into its own parent)", c.mirror(oc1, O, position="bottom"))
children(O, "list O")

print("\n== bad inputs")
show("MIRROR missing parent_id", c.request("POST", f"/api/v1/nodes/{O}/mirror", json={}))
show("MIRROR no body", c.request("POST", f"/api/v1/nodes/{O}/mirror"))
show("MIRROR nonexistent :id (unscoped)", c.request("POST", f"/api/v1/nodes/{uuid.uuid4()}/mirror", json={"parent_id": A}, unscoped=True))
show("MIRROR to nonexistent parent (unscoped)", c.request("POST", f"/api/v1/nodes/{O}/mirror", json={"parent_id": str(uuid.uuid4())}, unscoped=True))

print("\n== DELETE /mirror on non-mirror and nonexistent")
show("DELETE /mirror on origin O", c.request("DELETE", f"/api/v1/nodes/{O}/mirror"))
show("DELETE /mirror on plain node X", c.request("DELETE", f"/api/v1/nodes/{X}/mirror"))
show("DELETE /mirror on nonexistent (unscoped)", c.request("DELETE", f"/api/v1/nodes/{uuid.uuid4()}/mirror", unscoped=True))
show("GET /mirror (wrong method)", c.request("GET", f"/api/v1/nodes/{M1}/mirror"))

print("\n== DELETE /mirror on a real mirror that has a child")
M3 = show("MIRROR O -> C bottom", c.mirror(O, C, position="bottom"))["item_id"]
m3c = create(M3, "child created under M3")
show("DELETE /mirror on M3", c.request("DELETE", f"/api/v1/nodes/{M3}/mirror"))
show("GET M3", c.get(M3))
show("GET M3's child", c.get(m3c))
get(O, "GET O (origin intact?)")
children(O, "list O")
show("DELETE /mirror on M3 again", c.request("DELETE", f"/api/v1/nodes/{M3}/mirror"))

print("\n== DELETE /nodes/:id on a mirror")
M4 = show("MIRROR O -> C bottom", c.mirror(O, C, position="bottom"))["item_id"]
show("DELETE M4 via /nodes/:id", c.delete(M4))
show("GET M4", c.get(M4))
get(O, "GET O")
children(O, "list O")

print("\n== delete origin while mirrors exist")
O2 = create(B, "O2 origin to delete", position="bottom")
o2c = create(O2, "o2c")
N1 = show("MIRROR O2 -> A", c.mirror(O2, A, position="bottom"))["item_id"]
N2 = show("MIRROR O2 -> C", c.mirror(O2, C, position="bottom"))["item_id"]
n1c = create(N1, "child under mirror N1")
show("DELETE O2 (origin)", c.delete(O2))
show("GET O2", c.get(O2))
show("GET N1 (mirror of deleted origin)", c.get(N1))
show("GET N2", c.get(N2))
show("GET n1c (child of mirror)", c.get(n1c))
children(A, "list A")
children(C, "list C")
show("DELETE /mirror on N2 (orphan mirror)", c.request("DELETE", f"/api/v1/nodes/{N2}/mirror"))
show("MIRROR N1 (orphan) -> C", c.mirror(N1, C, position="bottom"))

print("\nFINAL")
for k, v in dict(S=S, A=A, B=B, C=C, O=O, M1=M1, M2=M2, X=X, MX=MX).items():
    print(k, v)
