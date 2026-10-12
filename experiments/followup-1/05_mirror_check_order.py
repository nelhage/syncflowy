"""POST /nodes/:id/mirror validation/lookup precedence, orphan parents, and
DELETE /:id/mirror on a plain child of a shell.

unscoped=True is used ONLY for requests whose path :id is a random UUID that
cannot exist (the guard would otherwise refuse it); any parent_id in those
bodies is either random or a sandbox node."""
import uuid
from common import c, create, mirror, show

S = c.make_sandbox("followup-1: mirror check order")
O = create(S, "O origin", position="bottom")
D = create(S, "D dest", position="bottom")
X = create(S, "X plain", position="bottom")
M = mirror(O, D)
K = create(M, "K plain child of shell")
R1, R2 = str(uuid.uuid4()), str(uuid.uuid4())

def mp(label, nid, body=None, content=None, unscoped=False):
    return show(label, c.request("POST", f"/api/v1/nodes/{nid}/mirror", json=body,
                                  content=content, unscoped=unscoped,
                                  headers={"Content-Type": "application/json"} if content else None))

print("##### precedence")
mp("random id + bad position + real parent", R1, {"parent_id": D, "position": "middle"}, unscoped=True)
mp("random id + random parent", R1, {"parent_id": R2}, unscoped=True)
mp("random id + random parent + bad position", R1, {"parent_id": R2, "position": "middle"}, unscoped=True)
mp("random id + missing parent_id", R1, {}, unscoped=True)
mp("random id + invalid json", R1, content=b"{nope", unscoped=True)
mp("random id + parent_id int", R1, {"parent_id": 5}, unscoped=True)
mp("real id + random parent + bad position", X, {"parent_id": R2, "position": "middle"}, unscoped=True)
mp("real id + random parent", X, {"parent_id": R2}, unscoped=True)
mp("O -> O (self) + bad position", O, {"parent_id": O, "position": "middle"})
mp("O -> O (self)", O, {"parent_id": O})
mp("orphan-src placeholder skipped", X, {"parent_id": "zzzz-not-an-id"}, unscoped=True)

print("##### DELETE /mirror on plain child of shell")
show("DELETE K/mirror", c.request("DELETE", f"/api/v1/nodes/{K}/mirror"))
show("GET K after", c.get(K))
show("DELETE X/mirror (plain, control)", c.request("DELETE", f"/api/v1/nodes/{X}/mirror"))

print("##### orphan shell as parent_id / as source")
O2 = create(S, "O2 origin to be deleted", position="bottom")
H = create(S, "H holds orphan", position="bottom")
N = mirror(O2, H)
nc = create(N, "nc child of orphan-to-be")
show("DELETE O2", c.delete(O2))
show("GET N (orphan)", c.get(N))
body = mp("mirror X -> N (orphan parent)", X, {"parent_id": N})
if isinstance(body, dict) and "item_id" in body:
    show("GET result", c.get(body["item_id"]))
    show("list N", c.list(N))
mp("mirror N (orphan) -> D", N, {"parent_id": D})
mp("mirror N (orphan) -> D bad position", N, {"parent_id": D, "position": "middle"})
mp("mirror N (orphan) -> random parent", N, {"parent_id": R2}, unscoped=True)
mp("mirror N (orphan) -> N", N, {"parent_id": N})
mp("mirror nc (plain child of orphan) -> D", nc, {"parent_id": D})
