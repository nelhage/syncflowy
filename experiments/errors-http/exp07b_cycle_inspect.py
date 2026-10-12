"""Inspect (and repair) the cycle created by exp07 "MOVE A under its child A1".

Usage: exp07b_cycle_inspect.py <sandbox_id> <A_id> <A1_id> [repair]
"""
import sys
from common import c, show
sb, a, a1 = sys.argv[1:4]
show("GET A", c.get(a))
show("GET A1", c.get(a1))
show("LIST A1", c.list(a1))
show("LIST sandbox", c.list(sb))
if len(sys.argv) > 4 and sys.argv[4] == "repair":
    show("MOVE A back under sandbox", c.move(a, sb))
    show("GET A", c.get(a))
    show("GET A1", c.get(a1))
    show("LIST sandbox", c.list(sb))
    show("LIST A", c.list(a))
