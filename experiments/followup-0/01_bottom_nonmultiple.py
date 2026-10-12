"""Q1: move/mirror to BOTTOM when the destination's live max is not a multiple
of 100.  Candidates: max+100  vs  ceil((max+100)/100)*100.

Setups:
  A  multiline-create children at 0,1,2   (max 2)   -> 102 vs 200
  B  top-created 100,50,25 then delete 100 (max 50)  -> 150 vs 200
  C  top-created 100,50,25,12,6,3 then delete all but 3 -> max 3 -> 103 vs 200
  D  only child at 0 (multiline child, delete the others) -> 100 either way
  Same-parent: multiline kids 0,1,2; move kid '2' to bottom (others max 1)
  -> 101 vs 200.
"""
from common import c, mk, ok, out, show

sb = c.make_sandbox("followup-0: 01 bottom when max not multiple of 100")
src = mk(sb, "movers (source)", position="bottom")
movers = [mk(src, f"mv{i}", position="bottom") for i in range(8)]
origin = mk(sb, "mirror origin O", position="bottom")


def multiline_parent(label):
    p = mk(sb, f"{label}\n\n- k0\n\n- k1\n\n- k2", position="bottom")
    return p


def deleted_max_parent(label, ntop, keep_last_only=False):
    p = mk(sb, label, position="bottom")
    ids = [mk(p, f"t{i}", position="top") for i in range(ntop)]
    # ids[0]=100, ids[1]=50, ...
    if keep_last_only:
        for i in ids[:-1]:
            ok(c.delete(i))
    else:
        ok(c.delete(ids[0]))
    return p


for kind in ("move", "mirror"):
    out(f"===== {kind} =====")
    setups = {
        "A multiline 0,1,2": multiline_parent(f"A-{kind}"),
        "B 25,50 after deleting 100": deleted_max_parent(f"B-{kind}", 3),
        "C only 3 (others deleted)": deleted_max_parent(f"C-{kind}", 6, True),
    }
    # D: only child at priority 0
    d = multiline_parent(f"D-{kind}")
    dk = [n["id"] for n in ok(c.list(d))["nodes"]]
    dk_sorted = sorted(ok(c.list(d))["nodes"], key=lambda n: n["priority"])
    for n in dk_sorted[1:]:
        ok(c.delete(n["id"]))
    setups["D only child at 0"] = d
    for label, p in setups.items():
        show(p, f"{label} before")
        if kind == "move":
            r = c.move(movers.pop(0), p, position="bottom")
        else:
            r = c.mirror(origin, p, position="bottom")
        out(f"  {kind} bottom ->", r.status_code, r.text)
        show(p, f"{label} after ")
        # second bottom insert to see the step from a multiple
        if kind == "move":
            r = c.move(movers.pop(0), p, position="bottom")
        else:
            r = c.mirror(origin, p, position="bottom")
        show(p, f"{label} after 2nd")

out("===== same-parent move-to-bottom of kid at max (others max 1) =====")
e = multiline_parent("E same-parent")
kids = sorted(ok(c.list(e))["nodes"], key=lambda n: n["priority"])
show(e, "E before")
ok(c.move(kids[2]["id"], e, position="bottom"))
show(e, "E after moving k2(2) bottom")
