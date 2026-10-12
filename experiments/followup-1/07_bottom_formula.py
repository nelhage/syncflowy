"""Bottom-priority rule when max sibling priority is not a multiple of 100.
Candidates: max+100  vs  ceil((max+100)/100)*100 (= ceil(max/100)*100 + 100).
Build a parent whose only live child has priority 50 or 150, then do a bottom
create / move / mirror into it and read the priority."""
from common import c, create, show

S = c.make_sandbox("followup-1: bottom formula")
O = create(S, "O mirror source", position="bottom")
loose = [create(S, f"loose{i}", position="bottom") for i in range(2)]

def parent_with(maxp):
    P = create(S, f"P max={maxp}", position="bottom")
    a = create(P, "a", position="bottom")          # 100
    if maxp == 50:
        b = create(P, "b top")                     # 50
        show("delete a", c.delete(a))
    elif maxp == 150:
        b2 = create(P, "b", position="bottom")     # 200
        b3 = create(P, "c", position="bottom")     # 300
        show("delete a", c.delete(a)); show("delete b", c.delete(b2))
        t = create(P, "t top")                     # floor(300/2)=150
        show("delete c", c.delete(b3))
    elif maxp == 250:
        for _ in range(4): create(P, "x", position="bottom")  # 200..500
        # delete all but 500 then top => 250, then delete 500
        kids = c.list(P).json()["nodes"]
        top = max(kids, key=lambda n: n["priority"])
        for n in kids:
            if n["id"] != top["id"]: c.delete(n["id"])
        create(P, "t top")
        c.delete(top["id"])
    pr = sorted(n["priority"] for n in c.list(P).json()["nodes"])
    print(f"    P live priorities: {pr}")
    return P

res = {}
for maxp in (50, 150, 250):
    for op in ("create", "move", "mirror"):
        P = parent_with(maxp)
        if op == "create":
            nid = create(P, "new bottom", position="bottom")
        elif op == "move":
            nid = loose.pop() if loose else create(S, "loose", position="bottom")
            show("move bottom", c.move(nid, P, position="bottom"))
        else:
            nid = show("mirror bottom", c.mirror(O, P, position="bottom"))["item_id"]
        p = c.get(nid).json()["node"]["priority"]
        res[(maxp, op)] = p
        print(f"RESULT max={maxp} op={op} -> {p}  (max+100={maxp+100})")
print(res)
