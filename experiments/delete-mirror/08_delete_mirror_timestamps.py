"""Does DELETE /nodes/:mirror (plain delete) bump the origin's modifiedAt like DELETE /mirror does?
Does deleting an origin bump its mirrors' modifiedAt?"""
import time
from common import c, create, get, show

S = c.make_sandbox("delete-mirror: delete timestamps")
O = create(S, "O origin")
A = create(S, "A dest", position="bottom")
M = show("MIRROR O -> A", c.mirror(O, A))["item_id"]
M2 = show("MIRROR O -> A bottom", c.mirror(O, A, position="bottom"))["item_id"]
time.sleep(2.2)
o0 = get(O, "GET O before")["modifiedAt"]
show("DELETE M via /nodes/:id", c.delete(M))
o1 = get(O, "GET O after")["modifiedAt"]
print("O modifiedAt", o0, "->", o1)
m2 = get(M2, "GET M2 before origin delete")["modifiedAt"]
time.sleep(2.2)
show("DELETE O (origin)", c.delete(O))
m2b = get(M2, "GET M2 after origin delete")["modifiedAt"]
print("M2 modifiedAt", m2, "->", m2b)
