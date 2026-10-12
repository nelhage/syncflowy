"""Which modifiedAt changes when nodes are moved into / out of a mirror shell,
or when children under a shell are created / updated / completed / deleted?
Does the ORIGIN get bumped (as it does for POST/DELETE /mirror)?"""
from common import c, create, mirror, probe, show

S = c.make_sandbox("followup-1: shell side effects")
O = create(S, "O origin", position="bottom")
oc = create(O, "oc origin child")
D = create(S, "D dest", position="bottom")
L = create(S, "L loose", position="bottom")
M = mirror(O, D)
ids = dict(S=S, O=O, oc=oc, D=D, M=M, L=L)

probe("move L -> M (into shell)", ids, lambda: show("move", c.move(L, M)))
probe("move L -> S (out of shell)", ids, lambda: show("move", c.move(L, S, position="bottom")))
probe("move L -> M again", ids, lambda: show("move", c.move(L, M, position="bottom")))

holder = {}
def mk():
    holder["k"] = create(M, "K child created under shell")
probe("create child under M", ids, mk)
ids["K"] = holder["k"]
probe("update K name", ids, lambda: show("update", c.update(ids["K"], name="K renamed")))
probe("complete K", ids, lambda: show("complete", c.complete(ids["K"])))
probe("uncomplete K", ids, lambda: show("uncomplete", c.uncomplete(ids["K"])))
probe("update M (shell) name", ids, lambda: show("update", c.update(M, name="shell renamed")))
probe("complete M (shell)", ids, lambda: show("complete", c.complete(M)))
probe("move M (shell) -> S bottom", ids, lambda: show("move", c.move(M, S, position="bottom")))
probe("move M (shell) -> D back", ids, lambda: show("move", c.move(M, D)))
probe("delete K (child of shell)", ids, lambda: show("delete", c.delete(ids["K"])))
del ids["K"]
# control: same operations on the origin side
probe("update O name (control)", ids, lambda: show("update", c.update(O, name="O renamed")))
show("list M", c.list(M))
show("list O", c.list(O))
