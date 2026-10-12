"""Move basics: cross-parent top/bottom, same-parent reorder, default position,
priority assignment, sibling rewrites, list order, modifiedAt effects, ids,
descendants, read-after-write consistency.
"""
import sys, os
sys.path.insert(0, os.path.dirname(__file__))
from common import *

sb = c.make_sandbox("ordering-move: move basics")
out("sandbox", sb)
P1 = mk(sb, "P1", position="bottom")
P2 = mk(sb, "P2", position="bottom")
x = {}
for i in range(1, 6):
    x[i] = mk(P1, f"x{i}", position="bottom")
y = {}
for i in range(1, 4):
    y[i] = mk(P2, f"y{i}", position="bottom")
# give x3 a subtree
x3a = mk(x[3], "x3a", position="bottom")
x3b = mk(x[3], "x3b", position="bottom")
x3a1 = mk(x3a, "x3a1", position="bottom")


def node(i):
    r = c.get(i)
    return r.json()["node"]


def state(label):
    out(f"=== {label}")
    for P, nm in ((P1, "P1"), (P2, "P2")):
        ns = kids(P)
        out(f"  {nm} raw: {[(n['name'], n['priority'], n['modifiedAt']) for n in ns]}")
    out("  parents modifiedAt:", {"sb": node(sb)["modifiedAt"], "P1": node(P1)["modifiedAt"], "P2": node(P2)["modifiedAt"]})


def mv(label, nid, parent, **kw):
    time.sleep(1.5)  # so modifiedAt (1s resolution) changes are visible
    t = time.time()
    r = c.move(nid, parent, **kw)
    out(f"\n### {label}: move {kw} -> {dump(r)}  (t={t:.1f})")
    # immediate read-after-write
    n = node(nid)
    out(f"  immediate GET: id={n['id']} parent={n['parent_id'][-12:] if n['parent_id'] else None} prio={n['priority']} mod={n['modifiedAt']} created={n['createdAt']}")
    state(label)
    return r


state("initial")
mv("x3 (with subtree) -> P2 top", x[3], P2, position="top")
out("  x3 children:", [(n["name"], n["priority"], n["parent_id"][-12:]) for n in kids(x[3])])
out("  x3a children:", [(n["name"], n["priority"]) for n in kids(x3a)])
out("  x3a1 GET parent:", node(x3a1)["parent_id"] == x3a)
mv("x2 -> P2 bottom", x[2], P2, position="bottom")
mv("x1 -> P2 no position", x[1], P2)
mv("x5 -> P1 (same parent) top", x[5], P1, position="top")
mv("x4 -> P1 (same parent) bottom", x[4], P1, position="bottom")
mv("y1 -> P2 (same parent) no position", y[1], P2)
mv("y3 -> P2 (same parent, already bottom) bottom", y[3], P2, position="bottom")
mv("y3 -> P2 bottom again", y[3], P2, position="bottom")
