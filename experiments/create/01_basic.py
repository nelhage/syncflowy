"""Basic create: response shape, GET shape, list shape, timing."""
import time
from _util import ProdClient, show, dump_children

c = ProdClient()
sb = c.make_sandbox("create: 01 basic shape")
print("sandbox", sb)

t0 = time.time()
r = c.create(sb, "hello world")
body = show("create plain", r, headers=True)
nid = body["item_id"]
r = c.get(nid)
show("immediate GET", r, headers=True)
print("    time since create:", time.time() - t0)
dump_children(c, sb)
r = c.get(sb)
show("GET sandbox", r)
