"""Does an orphan mirror shell (origin deleted) behave like a normal node for
move / move-in / create-under / update / complete / list / plain delete?"""
from common import c, create, mirror, probe, show

S = c.make_sandbox("followup-1: orphan ops")
O = create(S, "O origin (will be deleted)", position="bottom")
H = create(S, "H", position="bottom")
H2 = create(S, "H2", position="bottom")
L = create(S, "L loose", position="bottom")
N = mirror(O, H)
N2 = mirror(O, H, position="bottom")
nc = create(N, "nc child of N")
show("DELETE O", c.delete(O))
ids = dict(H=H, H2=H2, N=N, nc=nc, L=L)

probe("move orphan N -> H2", ids, lambda: show("move N->H2", c.move(N, H2, position="bottom")))
show("GET N", c.get(N)); show("list N", c.list(N)); show("list H2", c.list(H2))
probe("move L -> N (into orphan)", ids, lambda: show("move L->N", c.move(L, N)))
probe("create under orphan N", ids, lambda: show("create", c.create(N, "created under orphan")))
show("list N", c.list(N))
probe("update orphan N", ids, lambda: show("update", c.update(N, name="orphan renamed", note="n")))
probe("complete orphan N", ids, lambda: show("complete", c.complete(N)))
show("GET N", c.get(N))
show("mirror X -> N2 (orphan parent, recheck)", c.mirror(L, N2))
show("DELETE N2/mirror (orphan)", c.request("DELETE", f"/api/v1/nodes/{N2}/mirror"))
probe("plain DELETE orphan N", ids, lambda: show("DELETE N", c.delete(N)))
show("GET nc after", c.get(nc)); show("GET L after", c.get(L))
