"""Mirror-of-mirror and mirror-parent redirection: which modifiedAt is bumped?
Plus a re-confirmation of: mirror creation does NOT bump the destination parent,
move-in DOES."""
from common import c, create, mirror, probe, show

S = c.make_sandbox("followup-1: mirror-of-mirror bumps")
O = create(S, "O origin", position="bottom")
D1 = create(S, "D1", position="bottom")
D2 = create(S, "D2", position="bottom")
X = create(S, "X to be mirrored under M1", position="bottom")
M1 = mirror(O, D1)
ids = dict(S=S, O=O, D1=D1, D2=D2, M1=M1, X=X)

for i in range(2):
    probe(f"mirror M1 -> D2 (#{i})", ids, lambda: show("mirror M1->D2", c.mirror(M1, D2)))

r = {}
def mx():
    b = show("mirror X -> M1 (redirects to O)", c.mirror(X, M1))
    r["mx"] = b["item_id"]
probe("mirror X with parent_id=M1", ids, mx)
show("GET new mirror of X", c.get(r["mx"]))

# asymmetry re-check on a fresh destination
D3 = create(S, "D3 asym", position="bottom")
Y = create(S, "Y origin", position="bottom")
Z = create(S, "Z loose", position="bottom")
ids2 = dict(D3=D3, Y=Y, Z=Z, S=S)
probe("mirror Y -> D3 (creation)", ids2, lambda: show("mirror", c.mirror(Y, D3)))
probe("move Z -> D3 (move-in)", ids2, lambda: show("move", c.move(Z, D3)))
probe("create child under D3", ids2, lambda: show("create", c.create(D3, "new child")))
