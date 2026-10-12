"""Misc priority behaviour:
 - can 'priority' be set via create or update? (any way to place in the middle?)
 - top into an empty parent
 - renumbering triggered by *move* to top when min priority is 0
 - delete: are sibling priorities rewritten?
 - move to self: effect on priority / modifiedAt
 - raw list order after rename / complete / cross-parent move
"""
import sys, os
sys.path.insert(0, os.path.dirname(__file__))
from common import *

sb = c.make_sandbox("ordering-move: priority misc")
out("sandbox", sb)
A = mk(sb, "A", position="bottom")
B = mk(sb, "B", position="bottom")
E = mk(sb, "E-empty", position="bottom")

out("\n## top into empty parent")
e1 = mk(E, "e1", position="top"); out("first child created top:", pmap(kids(E)))
F = mk(sb, "F-empty", position="bottom")
r = c.move(e1, F, position="top"); out("move into empty parent top:", dump(r), pmap(kids(F)))
G = mk(sb, "G-empty", position="bottom")
r = c.move(e1, G, position="bottom"); out("move into empty parent bottom:", dump(r), pmap(kids(G)))

out("\n## priority via create / update")
for i in range(1, 4):
    mk(A, f"a{i}", position="bottom")
r = c.create(A, "with-priority", priority=150); out("create priority=150 ->", dump(r)); out("  ", sorted(pmap(kids(A)).items(), key=lambda kv: kv[1]))
r = c.create(A, "with-priority-bottom", priority=150, position="bottom"); out("create priority=150 bottom ->", dump(r)); out("  ", sorted(pmap(kids(A)).items(), key=lambda kv: kv[1]))
a1 = [n for n in kids(A) if n["name"] == "a1"][0]["id"]
r = c.update(a1, priority=250); out("update a1 priority=250 ->", dump(r)); out("  ", sorted(pmap(kids(A)).items(), key=lambda kv: kv[1]))
r = c.update(a1, priority="bottom"); out("update a1 priority='bottom' ->", dump(r))
r = c.update(a1, position="bottom"); out("update a1 position=bottom ->", dump(r)); out("  ", sorted(pmap(kids(A)).items(), key=lambda kv: kv[1]))
r = c.update(a1, parent_id=B); out("update a1 parent_id=B ->", dump(r)); out("  a1 parent==B?", c.get(a1).json()["node"]["parent_id"] == B)
r = c.create(A, "bad-pos", position="middle"); out("create position=middle ->", dump(r))
r = c.create(A, "pos-null", position=None); out("create position=null ->", dump(r)); out("  ", sorted(pmap(kids(A)).items(), key=lambda kv: kv[1]))

out("\n## move-to-top renumbering when min priority hits 0")
for i in range(1, 3):
    mk(B, f"b{i}", position="bottom")          # 100, 200
movers = [mk(sb, f"m{i}", position="bottom") for i in range(1, 9)]
prev = pmap(kids(B))
for i, m in enumerate(movers, 1):
    r = c.move(m, B, position="top")
    cur = pmap(kids(B))
    d = {k: v for k, v in diff(prev, cur).items() if k != f"m{i}"}
    out(f"move m{i} -> B top: {r.status_code} new={cur[f'm{i}']} others_changed={d}")
    prev = cur
# also same-parent move to top of the current min-0 node? sort and show
out("  B sorted:", sorted(cur.items(), key=lambda kv: kv[1]))

out("\n## delete: sibling priorities rewritten?")
before_nodes = kids(B)
before = pmap(before_nodes)
victim = [n for n in before_nodes if n["name"] == "m3"][0]["id"]
out("delete m3:", dump(c.delete(victim)))
after = pmap(kids(B)); out("  changed:", diff(before, after))
first = sorted(before_nodes, key=lambda n: n["priority"])[0]
out(f"delete current first {first['name']}:", dump(c.delete(first["id"])))
after2 = pmap(kids(B)); out("  changed:", diff(after, after2))

out("\n## move to self")
time.sleep(1.5)
bn = kids(B); b2 = [n for n in bn if n["name"] == "b2"][0]
r = c.move(b2["id"], b2["id"]); out("move b2 -> b2:", dump(r))
g = c.get(b2["id"]).json()["node"]
out(f"  before prio={b2['priority']} mod={b2['modifiedAt']} parent={b2['parent_id'][-12:]};  after prio={g['priority']} mod={g['modifiedAt']} parent={g['parent_id'][-12:]}")
r = c.move(b2["id"], b2["id"], position="bottom"); out("move b2 -> b2 bottom:", dump(r)); out("  prio now", c.get(b2["id"]).json()["node"]["priority"])

out("\n## raw list order")
out("  B raw:", [n["name"] for n in kids(B)])
b1 = [n for n in kids(B) if n["name"] == "b1"][0]["id"]
c.update(b1, name="b1-renamed"); out("  after rename b1:", [n["name"] for n in kids(B)])
c.complete(b1); out("  after complete b1:", [n["name"] for n in kids(B)])
c.uncomplete(b1)
c.move(b1, sb, position="bottom"); c.move(b1, B, position="bottom"); out("  after b1 out&back (bottom):", [n["name"] for n in kids(B)])
mx = [n for n in kids(B) if n["name"] == "m7"][0]["id"]
c.move(mx, B, position="bottom"); out("  after m7 same-parent -> bottom:", [n["name"] for n in kids(B)])
out("  sorted by priority:", [n["name"] for n in sorted(kids(B), key=lambda n: n["priority"])])
