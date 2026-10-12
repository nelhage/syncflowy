"""When a node already at top/bottom is moved to top/bottom of the same parent
and there is a gap next to it, is its priority recomputed (remove-then-insert)
or left alone?"""
import sys, os
sys.path.insert(0, os.path.dirname(__file__))
from common import *

sb = c.make_sandbox("ordering-move: recompute vs noop")
out("sandbox", sb)
n = {i: mk(sb, f"n{i}", position="bottom") for i in range(1, 5)}   # 100..400
n0 = mk(sb, "n0", position="top")                                   # 50
c.delete(n[1]); c.delete(n[3])
out("after deleting n1,n3:", sorted(pmap(kids(sb)).items(), key=lambda kv: kv[1]))
r = c.move(n[4], sb, position="bottom"); out("n4 (last, 400; max other 200) -> bottom:", dump(r), sorted(pmap(kids(sb)).items(), key=lambda kv: kv[1]))
r = c.move(n0, sb, position="top"); out("n0 (first, 50; min other 200) -> top:", dump(r), sorted(pmap(kids(sb)).items(), key=lambda kv: kv[1]))
