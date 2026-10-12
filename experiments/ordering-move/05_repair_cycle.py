"""Repair the cycle created by 03_ (P moved into its own descendant): move P
back under its sandbox and verify the tree is sane again."""
import sys, os
sys.path.insert(0, os.path.dirname(__file__))
from common import *

sb = "7f261b8d-2e2b-4a1b-a776-207c6f28b6d1"
P = "5442c5ee-05a7-4c31-85fa-79292f9df69c"
p1 = "f7970556-cf61-4aeb-8b16-a0ce47aafeca"
g1 = "c7652587-dab8-4667-abac-7fb80715d672"
out("move P -> sandbox top:", dump(c.move(P, sb, position="top")))
for nm, i in (("P", P), ("p1", p1), ("p1-child", g1)):
    n = c.get(i).json()["node"]
    out(nm, "parent:", n["parent_id"], "prio:", n["priority"])
out("sandbox kids:", [(n["name"], n["priority"]) for n in kids(sb)])
out("p1-child kids:", [(n["name"]) for n in kids(g1)])
