"""Isolate the cycle behaviours seen in exp07 (each probe is immediately
inspected and then REPAIRED by moving the node back under the sandbox).

1. move Y under Y itself
2. move X under its own child X1 (X sitting normally in the tree)
3. move X under its grandchild X2
4. mirror Z under its own child Z1 (Z sitting normally in the tree); the
   resulting mirror (if any) is removed afterwards.
"""

from common import c, show

sb = c.make_sandbox("errors-http: move/mirror cycles (repaired)")


def mk(name, parent):
    i = c.create(parent, name, position="bottom").json()["item_id"]
    c.allow.add(i)
    return i


def state(label, *ids):
    for i in ids:
        n = c.get(i).json()["node"]
        print(f"{label}: {n['name']} parent={n['parent_id']} priority={n['priority']}")


y = mk("Y", sb)
show("MOVE Y under Y", c.move(y, y))
state("after Y->Y", y)
show("LIST sandbox", c.list(sb))
show("REPAIR Y", c.move(y, sb, position="bottom"))
state("after repair", y)

x = mk("X", sb)
x1 = mk("X1", x)
x2 = mk("X2", x1)
show("MOVE X under child X1", c.move(x, x1))
state("after X->X1", x, x1, x2)
show("REPAIR X", c.move(x, sb, position="bottom"))
state("after repair", x, x1, x2)
show("MOVE X under grandchild X2", c.move(x, x2))
state("after X->X2", x, x1, x2)
show("REPAIR X", c.move(x, sb, position="bottom"))
state("after repair", x, x1, x2)

z = mk("Z", sb)
z1 = mk("Z1", z)
r = c.mirror(z, z1)
show("MIRROR Z under child Z1", r)
if r.status_code == 200:
    m = r.json()["item_id"]
    show("GET mirror", c.get(m))
    show("REMOVE mirror", c.request("DELETE", f"/api/v1/nodes/{m}/mirror"))
show("LIST sandbox (final)", c.list(sb))
