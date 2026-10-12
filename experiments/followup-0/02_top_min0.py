"""Q2: move/mirror to TOP into a parent whose min is 0 (multiline children
0,1,2). Is the full renumber triggered like for create? Is a mirror shell that
is already a sibling renumbered like an ordinary sibling?

Q5: same-parent move-to-top where the only 0-priority sibling is the moved
node itself: siblings [self=0, 100, 200]. Expect 50, no renumber.
Plus variants: [a=0, self=1, 100] move self top (others min 0 -> renumber?),
and [self=0] alone, move self top.
"""
import time

from common import c, mk, ok, out, show

sb = c.make_sandbox("followup-0: 02 top into min-0 parent; self-at-0")
src = mk(sb, "movers (source)", position="bottom")
mv = [mk(src, f"mv{i}", position="bottom") for i in range(3)]
origin = mk(sb, "mirror origin O", position="bottom")


def ml(label):
    return mk(sb, f"{label}\n\n- k0\n\n- k1\n\n- k2", position="bottom")


def mtimes(p):
    return {(n["name"] or "<m>" + n["id"][-4:]): n["modifiedAt"] for n in ok(c.list(p))["nodes"]}


out("=== A: move to top into [0,1,2]")
a = ml("A move-top")
time.sleep(1.2)
m0 = mtimes(a)
show(a, "before")
out("  move ->", c.move(mv[0], a, position="top").text)
show(a, "after ")
out("  modifiedAt before", m0, "after", mtimes(a))

out("=== B: mirror to top into [0,1,2]")
b = ml("B mirror-top")
show(b, "before")
out("  mirror ->", c.mirror(origin, b, position="top").text)
show(b, "after ")

out("=== C: mirror shell as an existing sibling, then a top create renumbers")
cp = ml("C shell renumber")
r = ok(c.mirror(origin, cp, position="bottom"))  # shell at 200
show(cp, "after bottom mirror")
mk(cp, "top-create", position="top")
show(cp, "after top create")

out("=== C2: mirror shell sitting at priority 0 among others gets renumbered too?")
c2 = mk(sb, "C2 shell at 0", position="bottom")
mk(c2, "x100", position="bottom")
for i in range(6):  # 50,25,12,6,3,1
    mk(c2, f"t{i}", position="top")
ok(c2 and c.mirror(origin, c2, position="top"))  # shell -> 0
show(c2, "shell at 0")
ok(c.move(mv[1], c2, position="top"))
show(c2, "after top move in")

out("=== D: [self=0, 100, 200], move self top (same parent)")
d = mk(sb, "D self at 0", position="bottom")
keep = [mk(d, "b100", position="bottom"), mk(d, "b200", position="bottom")]
tops = [mk(d, f"t{i}", position="top") for i in range(7)]  # 50..0
for t in tops[:-1]:
    ok(c.delete(t))
show(d, "before")
out("  move self(0) top ->", c.move(tops[-1], d, position="top").text)
show(d, "after ")

out("=== E: [a=0, self=1, 100], move self top")
e = mk(sb, "E self at 1, other at 0", position="bottom")
mk(e, "b100", position="bottom")
tops = [mk(e, f"t{i}", position="top") for i in range(7)]  # 50,25,12,6,3,1,0
for t in tops[:-2]:
    ok(c.delete(t))
show(e, "before")
out("  move t5(1) top ->", c.move(tops[-2], e, position="top").text)
show(e, "after ")

out("=== F: [self=0] alone, move self top / bottom")
f = ml("F lone zero")
ks = sorted(ok(c.list(f))["nodes"], key=lambda n: n["priority"])
ok(c.delete(ks[1]["id"])); ok(c.delete(ks[2]["id"]))
show(f, "before")
out("  move k0 top ->", c.move(ks[0]["id"], f, position="top").text)
show(f, "after top")

out("=== G: same-parent move-to-top of k2 in [0,1,2] (others min 0)")
g = ml("G same-parent top")
ks = sorted(ok(c.list(g))["nodes"], key=lambda n: n["priority"])
ok(c.move(ks[2]["id"], g, position="top"))
show(g, "after k2 top")
