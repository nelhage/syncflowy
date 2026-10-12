"""Priority follow-ups: top/bottom inserts into a parent whose children were
created by multiline name (priorities 0,1,..), after deleting siblings, and
bottom-insert when max priority is not a multiple of 100.

Usage: 14_priority_followups.py [part ...]   (parts: multiline delete max50; default all)
"""
import sys
from _util import ProdClient, children

c = ProdClient()
parts = sys.argv[1:] or ["multiline", "delete", "max50"]


def pri(pid):
    return [(n["name"], n["priority"]) for n in children(c, pid)]


sb = c.make_sandbox("create: 14 priority followups " + ",".join(parts))

if "multiline" in parts:
    p = c.create(sb, "p\n\n- a\n\n- b").json()["item_id"]
    print("multiline kids:", pri(p))
    c.create(p, "bottom-c", position="bottom")
    print("after bottom:", pri(p))
    c.create(p, "top-z", position="top")
    print("after top:", pri(p))

if "delete" in parts:
    q = c.create(sb, "q", position="bottom").json()["item_id"]
    ids = [c.create(q, f"n{i}", position="bottom").json()["item_id"] for i in range(3)]
    print("q initial:", pri(q))
    c.delete(ids[0])
    print("q after deleting first:", pri(q))
    c.create(q, "top-after-delete", position="top")
    print("q after top:", pri(q))
    c.delete(ids[2])
    print("q after deleting last:", pri(q))
    c.create(q, "bottom-after-delete", position="bottom")
    print("q after bottom:", pri(q))

if "max50" in parts:
    r = c.create(sb, "r", position="bottom").json()["item_id"]
    rids = [c.create(r, f"t{i}", position="top").json()["item_id"] for i in range(3)]
    print("r initial:", pri(r))
    c.delete(rids[0])  # remove the 100
    print("r after deleting max:", pri(r))
    c.create(r, "bottom-after-max50", position="bottom")
    print("r after bottom:", pri(r))
    c.create(r, "bottom-again", position="bottom")
    print("r after 2nd bottom:", pri(r))
