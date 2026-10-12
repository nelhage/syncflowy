"""Mirror creation: response shape, how the mirror appears in GET/list, children of a mirror."""

import json
import time

from common import c, children, create, get, show

S = c.make_sandbox("delete-mirror: mirror basics")
print("sandbox", S)
A = create(S, "A (mirror destination)", position="bottom")
B = create(S, "B (origin container)", position="bottom")
x1 = create(A, "x1 (pre-existing in A)", position="bottom")
x2 = create(A, "x2 (pre-existing in A)", position="bottom")
O = create(B, "O origin **bold**", note="origin note", layoutMode="todo")
oc1 = create(O, "oc1", position="bottom")
oc2 = create(O, "oc2", position="bottom")
occ = create(oc1, "occ (grandchild)")

get(O, "GET O before mirror")
time.sleep(2)
r = c.mirror(O, A)
mbody = show("MIRROR O -> A (default position)", r)
print("response headers:", json.dumps(dict(r.headers)))
M = mbody["item_id"]
show("GET M immediately", c.get(M))
show("GET M by short id", c.get(M.replace("-", "")[-12:]))
get(O, "GET O after mirror")
children(A, "list A (contains mirror?)")
children(B, "list B (origin parent)")
mkids = children(M, "list children of mirror M")
okids = children(O, "list children of origin O")
for k in mkids:
    show(f"GET mirror-child {k['name']}", c.get(k["id"]))
    children(k["id"], f"list children of mirror-child {k['name']}")

r = c.mirror(O, A, position="bottom")
M2 = show("MIRROR O -> A position=bottom", r)["item_id"]
children(A, "list A after second mirror at bottom")
r = c.mirror(O, A, position="top")
M3 = show("MIRROR O -> A position=top explicit", r)["item_id"]
children(A, "list A after third mirror at top")
r = c.mirror(O, S, position="middle")
show("MIRROR with invalid position", r)
r = c.mirror(oc1, A, position="bottom")
show("MIRROR a child (oc1) of origin into A", r)
children(A, "final list A")
print("IDS", json.dumps(dict(S=S, A=A, B=B, O=O, oc1=oc1, oc2=oc2, occ=occ, M=M, M2=M2, M3=M3)))
