"""05_followups.py accidentally demonstrated that MOVE O -> (O's own child) succeeds
and creates a parent cycle (O.parent=oc, oc.parent=O). This script inspects the
cycle and repairs it by moving O back under the followups sandbox."""
from common import c, children, get, show

S = "2882eb89-25a3-4d38-94be-5335447681ab"   # followups sandbox
O = "f3b37cea-f495-4dba-9fd9-92fff33a02df"
oc = "3c90751a-6a74-4595-a51b-84c4bdae7801"
B = "2e96da4e-90f8-49fa-8a81-7e07799c160c"
M5 = "d2558515-4ca6-419f-a3b0-1a0a9c82e397"
get(O, "GET O (cycle state)")
get(oc, "GET oc (cycle state)")
children(O, "list O")
children(oc, "list oc")
show("MOVE oc -> O (child of its own child, whose parent is oc)", c.move(O, O))
show("REPAIR: MOVE O -> S bottom", c.move(O, S, position="bottom"))
get(O, "GET O after repair")
get(oc, "GET oc after repair")
get(M5, "GET M5")
get(B, "GET B")
children(S, "list sandbox")
