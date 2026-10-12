"""Move timestamp side effects.

Q1: does a move bump the modifiedAt of the moved node's descendants?
Q2: does move-to-self bump the parent?
Q3: is the destination-parent bump on a clean cross-parent move reproducible?
Each action is preceded by a 2.1 s gap so a bump is visible at 1 s resolution.
"""
from common import c, create, probe, show

S = c.make_sandbox("followup-1: move timestamps")
P1 = create(S, "P1", position="bottom")
P2 = create(S, "P2", position="bottom")
X = create(P1, "X (moved)")
X1 = create(X, "X1 child")
X2 = create(X1, "X2 grandchild")
SIB = create(P2, "P2 existing child")
ids = dict(S=S, P1=P1, P2=P2, X=X, X1=X1, X2=X2, P2child=SIB)

for i in range(3):
    dst, src = (P2, "P2") if i % 2 == 0 else (P1, "P1")
    probe(f"trial{i} move X -> {src} bottom", ids,
          lambda: show("move", c.move(X, dst, position="bottom")))

for i in range(2):
    probe(f"self-move{i} X -> X", ids, lambda: show("move self", c.move(X, X)))

# same-parent reorder for comparison
probe("same-parent move X top", ids,
      lambda: show("move", c.move(X, c.get(X).json()["node"]["parent_id"], position="top")))
