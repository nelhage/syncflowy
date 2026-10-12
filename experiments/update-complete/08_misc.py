"""Misc: read-your-writes via list, sibling order after complete, alternative
body encodings, duplicate keys, query-string params, mirrors + complete,
response headers."""

import json
import time

from _common import c, show, mk, node, children

sb = c.make_sandbox("update-complete: 08 misc (consistency, encodings, mirrors)")
print("sandbox", sb)
a = mk(sb, "A", position="bottom")
b = mk(sb, "B", position="bottom")
cc = mk(sb, "C", position="bottom")

print("## read-your-writes via LIST immediately after update / complete (5 reps)")
for i in range(5):
    c.update(b, name=f"B v{i}", note=f"note v{i}")
    c.complete(b) if i % 2 == 0 else c.uncomplete(b)
    lst = {x["id"]: x for x in children(sb)}
    print(i, lst[b]["name"], lst[b]["note"], lst[b]["completed"], "expected completed", i % 2 == 0)

print("## sibling priorities after complete")
c.complete(a)
print([(x["name"], x["priority"], x["completed"]) for x in sorted(children(sb), key=lambda x: x["priority"])])

print("## encodings")
show("form-encoded name+note+layoutMode", c.request("POST", f"/api/v1/nodes/{cc}", content=b"name=form%20name&note=form+note&layoutMode=h2",
                                                     headers={"content-type": "application/x-www-form-urlencoded"}))
print(json.dumps(node(cc)))
show("text/plain with json body", c.request("POST", f"/api/v1/nodes/{cc}", content=b'{"name":"textplain"}', headers={"content-type": "text/plain"}))
print(node(cc)["name"])
show("duplicate keys", c.request("POST", f"/api/v1/nodes/{cc}", content=b'{"name":"first","name":"second"}', headers={"content-type": "application/json"}))
print(node(cc)["name"])
show("query string params only", c.request("POST", f"/api/v1/nodes/{cc}", params={"name": "from-query"}, json={}))
print(node(cc)["name"])
show("json null body", c.request("POST", f"/api/v1/nodes/{cc}", content=b"null", headers={"content-type": "application/json"}))
show("json string body", c.request("POST", f"/api/v1/nodes/{cc}", content=b'"x"', headers={"content-type": "application/json"}))
show("trailing slash", c.request("POST", f"/api/v1/nodes/{cc}/", json={"name": "trailing slash"}))
print(node(cc)["name"])

print("## mirrors + update/complete")
r = c.mirror(a, cc)
show("mirror A under C", r)
if r.status_code == 200:
    m = r.json()["item_id"]
    print("origin A", json.dumps(node(a)))
    print("mirror  ", json.dumps(node(m)))
    show("uncomplete via mirror", c.uncomplete(m))
    print("origin A", node(a)["completed"], "mirror", node(m)["completed"])
    show("rename via mirror", c.update(m, name="A renamed via mirror", note="mirror note"))
    print("origin A", node(a)["name"], node(a)["note"], "| mirror", node(m)["name"], node(m)["note"])
    show("complete origin", c.complete(a))
    print("origin A", node(a)["completed"], node(a)["completedAt"], "mirror", node(m)["completed"], node(m)["completedAt"])
    show("layoutMode via mirror", c.update(m, layoutMode="h2"))
    print("origin A", node(a)["data"], "mirror", node(m)["data"])

print("## headers of a typical update response")
r = c.update(b, name="B final")
print(json.dumps(dict(r.headers), indent=1))
