"""Priority assignment for position top / bottom / omitted / invalid."""
from _util import ProdClient, show, dump_children

c = ProdClient()


def pri(kids):
    return [(n["name"], n["priority"]) for n in kids]


# A: omitted position, repeated -> which end, what priorities?
sb = c.make_sandbox("create: 02a priorities omitted position")
for i in range(4):
    show(f"omit {i}", c.create(sb, f"omit-{i}"))
    print("   ", pri(dump_children(c, sb)))

# B: top repeatedly
sb = c.make_sandbox("create: 02b priorities top")
for i in range(4):
    c.create(sb, f"top-{i}", position="top")
    print("   top", i, pri(dump_children(c, sb, "kids")))

# C: bottom repeatedly
sb = c.make_sandbox("create: 02c priorities bottom")
for i in range(4):
    c.create(sb, f"bot-{i}", position="bottom")
    print("   bottom", i, pri(dump_children(c, sb, "kids")))

# D: interleave
sb = c.make_sandbox("create: 02d priorities interleaved")
for i, pos in enumerate(["bottom", "top", "bottom", "top", "top", "bottom"]):
    c.create(sb, f"{pos}-{i}", position=pos)
print("   interleaved", pri(dump_children(c, sb, "kids")))

# E: invalid position values
sb = c.make_sandbox("create: 02e invalid positions")
c.create(sb, "seed-1", position="bottom")
c.create(sb, "seed-2", position="bottom")
for pos in ["middle", "TOP", "Bottom", "", 0, 1, 5, -1, None, True, "1", ["top"]]:
    show(f"position={pos!r}", c.create(sb, f"pos={pos!r}", position=pos))
print("   invalid", pri(dump_children(c, sb, "kids")))
