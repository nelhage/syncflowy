"""Move the current first child to top again: is the moved node excluded when
computing min(priority)/2?  (excluded => unchanged-ish; included => halves)"""
import sys, os
sys.path.insert(0, os.path.dirname(__file__))
from common import *

sb = c.make_sandbox("ordering-move: top when already top")
out("sandbox", sb)
for i in range(1, 4):
    mk(sb, f"k{i}", position="bottom")   # 100 200 300
k0 = mk(sb, "k0", position="top")        # 50
for _ in range(3):
    r = c.move(k0, sb, position="top")
    out("move k0 (already first) top ->", dump(r), sorted(pmap(kids(sb)).items(), key=lambda kv: kv[1]))
k3 = [n for n in kids(sb) if n["name"] == "k3"][0]["id"]
r = c.move(k3, sb, position="bottom"); out("move k3 (already last) bottom ->", dump(r), sorted(pmap(kids(sb)).items(), key=lambda kv: kv[1]))
