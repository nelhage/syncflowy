"""Does MOVE X -> X (self as parent) succeed, and what is X.parent_id afterward?
Repairs by moving X back under the sandbox."""
from common import c, children, create, get, show

S = c.make_sandbox("delete-mirror: self-move (cycle check)")
Q = create(S, "Q self-move subject")
q = create(Q, "q child of Q")
show("MOVE Q -> Q", c.move(Q, Q))
get(Q, "GET Q after self-move")
children(S, "list S")
children(Q, "list Q")
show("REPAIR MOVE Q -> S", c.move(Q, S))
get(Q, "GET Q after repair")
show("DELETE /nodes/Q with child still present (cleanup not done; just GET)", c.get(q))
