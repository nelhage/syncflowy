"""How are priorities assigned on create top/bottom? Are siblings rewritten?

Creates nodes in a fresh sandbox with a mix of top/bottom/default positions,
re-listing after each op and diffing all sibling priorities.
"""
import sys, os
sys.path.insert(0, os.path.dirname(__file__))
from common import *

sb = c.make_sandbox("ordering-move: create priorities top/bottom")
out("sandbox", sb)

prev = {}
seq = (
    [("b%d" % i, "bottom") for i in range(1, 6)]
    + [("t%d" % i, "top") for i in range(1, 6)]
    + [("d%d" % i, None) for i in range(1, 3)]
    + [x for i in range(6, 9) for x in (("t%d" % i, "top"), ("b%d" % i, "bottom"))]
)
for name, pos in seq:
    kw = {"position": pos} if pos else {}
    mk(sb, name, **kw)
    nodes = kids(sb)
    cur = pmap(nodes)
    raw = [n["name"] for n in nodes]
    srt = [n["name"] for n in sorted(nodes, key=lambda n: n["priority"])]
    d = diff(prev, cur)
    out(f"create {name} pos={pos}: new={cur.get(name)} changed_others={ {k:v for k,v in d.items() if k!=name} }")
    out("   sorted-by-priority:", [(n, cur[n]) for n in srt])
    out("   list order == priority order:", raw == srt)
    prev = cur
