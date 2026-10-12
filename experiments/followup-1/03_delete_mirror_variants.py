"""Plain DELETE /nodes/:mirror vs DELETE /nodes/:mirror/mirror, 3 trials each,
interleaved, each with a fresh origin. Which bumps the origin's modifiedAt?
Also: deleting an ancestor containing a mirror; does that bump the origin?"""
from common import c, create, mirror, probe, show

S = c.make_sandbox("followup-1: delete vs delete-mirror")
D = create(S, "D dest", position="bottom")
results = []
for i in range(3):
    for variant in ("plain", "mirror"):
        O = create(S, f"O{i}-{variant}", position="bottom")
        M = mirror(O, D, position="bottom")
        ids = dict(O=O, D=D, S=S)
        if variant == "plain":
            act = lambda: show("DELETE /nodes/M", c.delete(M))
        else:
            act = lambda: show("DELETE /nodes/M/mirror", c.request("DELETE", f"/api/v1/nodes/{M}/mirror"))
        _, bumped = probe(f"trial{i} {variant}", ids, act)
        show("GET M after", c.get(M))
        results.append((i, variant, bumped))

# delete an ancestor H that contains a mirror shell
O = create(S, "O-ancestor-case", position="bottom")
H = create(S, "H holder", position="bottom")
M = mirror(O, H)
_, b = probe("delete H (contains mirror of O)", dict(O=O, S=S), lambda: show("DELETE H", c.delete(H)))
results.append(("H", "ancestor", b))
print("RESULTS", results)
