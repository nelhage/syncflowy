"""Error precedence, list semantics (depth, completed, pagination params),
read-after-write consistency, raw JSON byte format, misc headers."""

import json
import time

import httpx

from common import c, fake_uuid, show
from syncflowy.prod_client import BASE_URL, LOG_FILE
import os

sb = c.make_sandbox("errors-http: precedence, list, timing")
BAD = {"Authorization": "Bearer not-a-real-key-0000"}

print("### 401 body shape per method")
show("DELETE bad key", c.request("DELETE", f"/api/v1/nodes/{fake_uuid()}", headers=BAD, unscoped=True))
show("POST move bad key", c.request("POST", f"/api/v1/nodes/{fake_uuid()}/move", json={"parent_id": fake_uuid()}, headers=BAD, unscoped=True))
show("POST complete bad key", c.request("POST", f"/api/v1/nodes/{fake_uuid()}/complete", headers=BAD, unscoped=True))
show("DELETE mirror bad key", c.request("DELETE", f"/api/v1/nodes/{fake_uuid()}/mirror", headers=BAD, unscoped=True))
show("PUT /nodes/:id bad key", c.request("PUT", f"/api/v1/nodes/{fake_uuid()}", headers=BAD, unscoped=True))
show("GET /nodes/:id/move bad key", c.request("GET", f"/api/v1/nodes/{fake_uuid()}/move", headers=BAD, unscoped=True))
show("POST invalid json bad key", c.request("POST", f"/api/v1/nodes/{fake_uuid()}", content=b"{", headers={**BAD, "Content-Type": "application/json"}, unscoped=True))

print("### precedence: validation vs not-found")
show("UPDATE nonexistent + name=123", c.request("POST", f"/api/v1/nodes/{fake_uuid()}", json={"name": 123}, unscoped=True))
show("UPDATE nonexistent + invalid json", c.request("POST", f"/api/v1/nodes/{fake_uuid()}", content=b"{", headers={"Content-Type": "application/json"}, unscoped=True))
show("CREATE nonexistent parent + name missing", c.request("POST", "/api/v1/nodes", json={"parent_id": fake_uuid()}, unscoped=True))
show("CREATE nonexistent parent + position bad", c.request("POST", "/api/v1/nodes", json={"parent_id": fake_uuid(), "name": "x", "position": "x"}, unscoped=True))
show("MOVE nonexistent + position bad", c.request("POST", f"/api/v1/nodes/{fake_uuid()}/move", json={"parent_id": fake_uuid(), "position": "x"}, unscoped=True))
show("CREATE multiple errors", c.request("POST", "/api/v1/nodes", json={"parent_id": sb, "name": 1, "note": 2, "position": 3}))

print("### list semantics")
a = c.create(sb, "A", position="bottom").json()["item_id"]
b = c.create(sb, "B (completed)", position="bottom").json()["item_id"]
c.complete(b)
a1 = c.create(a, "A1 (grandchild of sandbox)").json()["item_id"]
c.create(a1, "A1a (great-grandchild)")
show("LIST sandbox (depth? completed?)", c.list(sb))
for p in ({"limit": 1}, {"offset": 1}, {"cursor": "x"}, {"page": 2}, {"per_page": 1},
          {"depth": 2}, {"include_completed": "false"}, {"completed": "false"}, {"recursive": "true"}):
    show(f"LIST sandbox + {p}", c.request("GET", "/api/v1/nodes", params={"parent_id": sb, **p}))
# both ids are our own sandbox nodes; unscoped only because the guard cannot parse repeated params
show("LIST repeated parent_id (sb, a)", c.request("GET", f"/api/v1/nodes?parent_id={sb}&parent_id={a}", unscoped=True))
show("HEAD list", c.request("HEAD", "/api/v1/nodes", params={"parent_id": sb}))

print("### read-after-write timing")
for i in range(3):
    t0 = time.time()
    nid = c.create(sb, f"raw-{i}").json()["item_id"]
    r1 = c.get(nid)
    r2 = c.list(sb)
    seen = any(n["id"] == nid for n in r2.json()["nodes"])
    print(json.dumps({"iter": i, "get_status": r1.status_code, "in_list": seen, "dt": round(time.time() - t0, 3)}))
    c.move(nid, a, position="bottom")
    r3 = c.list(a)
    r4 = c.get(nid)
    print(json.dumps({"iter": i, "moved_in_new_parent_list": any(n["id"] == nid for n in r3.json()["nodes"]),
                      "get_parent_after_move": r4.json()["node"]["parent_id"] == a}))
    c.update(nid, name=f"raw-{i}-renamed")
    print(json.dumps({"iter": i, "rename_visible": c.get(nid).json()["node"]["name"] == f"raw-{i}-renamed"}))
    c.delete(nid)
    r5 = c.get(nid)
    print(json.dumps({"iter": i, "get_after_delete": r5.status_code, "body": r5.text}))
    r6 = c.delete(nid)
    print(json.dumps({"iter": i, "delete_again": r6.status_code, "body": r6.text}))

print("### raw JSON bytes: unicode, escaping")
u = c.create(sb, "nöde ☃ \"quoted\" </script> \U0001F600").json()["item_id"]
r = c.get(u)
print(json.dumps({"raw_bytes": r.content.decode("latin-1")}))
r = c.request("GET", "/api/v1/nodes", params={"parent_id": "nöde-zq9x"}, unscoped=True)
print(json.dumps({"status": r.status_code, "raw_bytes": r.content.decode("latin-1")}))

print("### Accept header variations")
show("GET Accept: text/html", c.request("GET", f"/api/v1/nodes/{a}", headers={"Accept": "text/html"}))
show("GET Accept: application/xml", c.request("GET", f"/api/v1/nodes/{a}", headers={"Accept": "application/xml"}))
show("GET full headers of a 200", c.get(a), full_headers=True)
r = c.request("GET", "/api/v1/nodes", params={"parent_id": sb}, headers={"Accept-Encoding": "identity"})
show("LIST identity encoding", r, full_headers=True)

print("### URL host variants (list parent_id)")
s = a.replace("-", "")[-12:]
for host in ("https://evil.example.com/#/", "https://app.workflowy.com/#/", "https://workflowy.com/#/x/",
             "https://WORKFLOWY.COM/#/", "https://workflowy.com/?q=1#/", "https://workflowy.com/s/abc#/",
             "https://workflowy.com#/"):
    show(f"LIST url={host}<short>", c.request("GET", "/api/v1/nodes", params={"parent_id": host + s}, unscoped=True))
