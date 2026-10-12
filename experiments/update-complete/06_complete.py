"""complete / uncomplete semantics: units, idempotency, descendants, layout,
interaction with update, request bodies, methods."""

import json
import time

from _common import c, show, mk, node, pnode, children, now


def b(nid):
    n = node(nid)
    return {k: n[k] for k in ("name", "completed", "completedAt", "modifiedAt")} | {"layout": n["data"]["layoutMode"]}


sb = c.make_sandbox("update-complete: 06 complete/uncomplete semantics")
print("sandbox", sb)

print("## complete on a plain bullet; units")
p = mk(sb, "parent (bullets)")
k1 = mk(p, "child 1", position="bottom")
k2 = mk(p, "child 2 (pre-completed)", position="bottom")
g = mk(k1, "grandchild")
show("complete k2 first", c.complete(k2))
k2_before = b(k2)
print("k2", k2_before)
time.sleep(2.2)
t = now()
show("complete parent", c.complete(p))
print("wall", t)
print("parent", b(p))
print("child1", b(k1))
print("child2", b(k2))
print("grandchild", b(g))
print("list(parent):", json.dumps([{k: x[k] for k in ("name", "completed", "completedAt")} for x in children(p)]))
print("list(sandbox):", json.dumps([{k: x[k] for k in ("name", "completed", "completedAt")} for x in children(sb)]))

time.sleep(2.2)
show("uncomplete parent", c.uncomplete(p))
print("parent", b(p))
print("child1", b(k1))
print("child2 (should still be completed?)", b(k2))
print("grandchild", b(g))

print("## complete idempotency (completedAt moves?)")
x = mk(sb, "idempotency subject", layoutMode="todo")
show("complete #1", c.complete(x))
a1 = b(x)
print(a1)
time.sleep(2.2)
show("complete #2", c.complete(x))
a2 = b(x)
print(a2)
print("completedAt changed on re-complete:", a1["completedAt"] != a2["completedAt"])

print("## update a completed node: does completion survive?")
time.sleep(2.2)
show("update name on completed", c.update(x, name="renamed while completed"))
print(b(x))
show("change layout todo->bullets on completed", c.update(x, layoutMode="bullets"))
print(b(x))
show("change layout ->h1 on completed", c.update(x, layoutMode="h1"))
print(b(x))
show("update completed:false via update body", c.update(x, completed=False))
print(b(x))

print("## bodies / methods on complete endpoint")
y = mk(sb, "body subject")
show("complete with body {completed:false}", c.request("POST", f"/api/v1/nodes/{y}/complete", json={"completed": False}))
print(b(y))
show("uncomplete with junk body", c.request("POST", f"/api/v1/nodes/{y}/uncomplete", content=b"{garbage", headers={"content-type": "application/json"}))
print(b(y))
show("GET /complete", c.request("GET", f"/api/v1/nodes/{y}/complete"))
show("DELETE /complete", c.request("DELETE", f"/api/v1/nodes/{y}/complete"))
show("POST /completed (typo)", c.request("POST", f"/api/v1/nodes/{y}/completed"))
show("POST /toggle", c.request("POST", f"/api/v1/nodes/{y}/toggle"))
print(b(y))

print("## uncomplete a never-completed node")
z = mk(sb, "never completed")
time.sleep(1.2)
show("uncomplete never-completed", c.uncomplete(z))
print(b(z))

print("## markdown-created completed todo, then uncomplete")
r = c.create(sb, "- [x] created done", position="bottom")
w = r.json()["item_id"]
print(b(w))
show("uncomplete", c.uncomplete(w))
print(b(w))

print("## complete via short id")
s = mk(sb, "short id subject")
short = s.replace("-", "")[-12:]
show("complete short", c.complete(short))
print(b(s))
show("update short", c.update(short, name="renamed via short id"))
print(b(s))
show("uncomplete short", c.uncomplete(short))
print(b(s))
