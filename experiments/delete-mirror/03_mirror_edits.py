"""Edit propagation between origin and mirror; complete; children under mirror; move mirror."""

import time

from common import c, children, create, get, show

S = c.make_sandbox("delete-mirror: mirror edits")
print("sandbox", S)
A = create(S, "A (mirror destination)", position="bottom")
B = create(S, "B (origin container)", position="bottom")
D = create(S, "D (move destination)", position="bottom")
O = create(B, "O origin", note="origin note")
oc1 = create(O, "oc1", position="bottom")
M = show("MIRROR O -> A", c.mirror(O, A))["item_id"]

print("\n== edit origin name/note, observe mirror")
time.sleep(1.5)
show("UPDATE O name+note", c.update(O, name="O renamed via origin", note="note v2"))
get(O, "GET O")
get(M, "GET M")

print("\n== edit mirror name, observe origin")
time.sleep(1.5)
show("UPDATE M name", c.update(M, name="M renamed via mirror"))
get(M, "GET M after own rename")
get(O, "GET O after mirror rename")
children(A, "list A")

print("\n== edit mirror note")
show("UPDATE M note", c.update(M, note="mirror note"))
get(M, "GET M")
get(O, "GET O")

print("\n== layoutMode on mirror")
show("UPDATE M layoutMode=h1", c.update(M, layoutMode="h1"))
get(M, "GET M")
get(O, "GET O")

print("\n== complete mirror")
show("COMPLETE M", c.complete(M))
get(M, "GET M")
get(O, "GET O")
show("UNCOMPLETE M", c.uncomplete(M))
print("\n== complete origin")
show("COMPLETE O", c.complete(O))
get(M, "GET M")
get(O, "GET O")
show("UNCOMPLETE O", c.uncomplete(O))

print("\n== add child under mirror")
mc = create(M, "child created under mirror M")
get(mc, "GET child-under-mirror")
children(M, "list M")
children(O, "list O")
mcb = create(M, "child created under mirror M (bottom)", position="bottom")
get(mcb, "GET child-under-mirror (bottom)")
children(O, "list O again")

print("\n== add child under origin, observe mirror")
oc2 = create(O, "oc2 created under origin", position="bottom")
children(M, "list M after origin child add")

print("\n== move a node INTO mirror")
loose = create(S, "loose node to move into mirror", position="bottom")
show("MOVE loose -> M", c.move(loose, M))
get(loose, "GET loose after move into mirror")
children(M, "list M")
children(O, "list O")

print("\n== move mirror M to D")
show("MOVE M -> D", c.move(M, D))
get(M, "GET M after move")
children(D, "list D")
children(A, "list A")
get(O, "GET O after mirror move")

print("\n== move origin O to D")
show("MOVE O -> D bottom", c.move(O, D, position="bottom"))
get(O, "GET O after move")
get(M, "GET M after origin move")
children(B, "list B")
children(D, "list D")
