"""Misc error cases: 401 shape on other endpoints (bad key only), not-found
message variants for non-UUID parent_ids in create/move/mirror, cycles,
body-size limits."""

import json

from common import c, fake_uuid, show

BAD = {"Authorization": "Bearer not-a-real-key-0000"}
print("### 401 on endpoints we otherwise never call (bad key => cannot read data)")
show("GET /nodes-export bad key", c.request("GET", "/api/v1/nodes-export", headers=BAD, unscoped=True))
show("GET /targets bad key", c.request("GET", "/api/v1/targets", headers=BAD, unscoped=True))
show("POST /targets bad key", c.request("POST", "/api/v1/targets", headers=BAD, unscoped=True))

sb = c.make_sandbox("errors-http: misc errors (cycles, not-found variants, size)")
a = c.create(sb, "A").json()["item_id"]
a1 = c.create(a, "A1").json()["item_id"]
c.allow.add(a, a1)

print("### not-found message for non-UUID parent_id on create/move/mirror")
show("CREATE parent_id=zzzzzzzzzzzz", c.request("POST", "/api/v1/nodes", json={"parent_id": "zzzzzzzzzzzz", "name": "x"}, unscoped=True))
show("CREATE parent_id=garbage-zq9x", c.request("POST", "/api/v1/nodes", json={"parent_id": "garbage-zq9x", "name": "x"}, unscoped=True))
show("CREATE parent_id=ffffffffffff (hex, nonexistent short)", c.request("POST", "/api/v1/nodes", json={"parent_id": "ffffffffffff", "name": "x"}, unscoped=True))
show("MOVE parent_id=garbage-zq9x", c.request("POST", f"/api/v1/nodes/{a1}/move", json={"parent_id": "garbage-zq9x"}, unscoped=True))
show("MIRROR parent_id=garbage-zq9x", c.request("POST", f"/api/v1/nodes/{a1}/mirror", json={"parent_id": "garbage-zq9x"}, unscoped=True))
show("GET ffffffffffff", c.request("GET", "/api/v1/nodes/ffffffffffff", unscoped=True))

print("### cycles")
show("MOVE A under itself", c.move(a, a))
show("MOVE A under its child A1", c.move(a, a1))
show("MIRROR A under itself", c.mirror(a, a))
show("MIRROR A under its child A1", c.mirror(a, a1))
show("LIST sandbox after", c.list(sb))
show("LIST A after", c.list(a))

print("### note clearing semantics (null vs empty string)")
c.update(a1, note="has a note")
show("UPDATE note=''", c.update(a1, note=""))
show("  -> GET", c.get(a1))
c.update(a1, note="has a note")
show("UPDATE note=null", c.request("POST", f"/api/v1/nodes/{a1}", json={"note": None}))
show("  -> GET", c.get(a1))

print("### body size limits (note on A1); cleaned up afterwards")
for size in (100_000, 1_100_000):
    r = c.request("POST", f"/api/v1/nodes/{a1}", json={"note": "x" * size})
    show(f"UPDATE note of {size} bytes", r)
    if r.status_code == 200:
        g = c.get(a1).json()["node"]
        print(json.dumps({"stored_note_len": len(g["note"] or "")}))
show("cleanup note", c.update(a1, note="size probes done"))
