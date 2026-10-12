"""Exact status + body for nonexistent / malformed ids on every endpoint.

Nonexistent ids are fresh random UUIDs (cannot exist), sent with
unscoped=True. Where an endpoint needs one *real* id plus one bogus id
(e.g. move a real node under a nonexistent parent), the real id is a node in
our sandbox.
"""

from urllib.parse import quote

from common import c, fake_uuid, show

sb = c.make_sandbox("errors-http: nonexistent + malformed ids")
real = c.create(sb, "real node").json()["item_id"]
c.allow.add(real)

F = fake_uuid
print("### random-UUID ids on each endpoint")
show("GET nonexistent", c.request("GET", f"/api/v1/nodes/{F()}", unscoped=True))
show("LIST nonexistent parent", c.request("GET", "/api/v1/nodes", params={"parent_id": F()}, unscoped=True))
show("CREATE under nonexistent parent", c.request("POST", "/api/v1/nodes", json={"parent_id": F(), "name": "x"}, unscoped=True))
show("UPDATE nonexistent", c.request("POST", f"/api/v1/nodes/{F()}", json={"name": "x"}, unscoped=True))
show("UPDATE nonexistent (empty body)", c.request("POST", f"/api/v1/nodes/{F()}", json={}, unscoped=True))
show("DELETE nonexistent", c.request("DELETE", f"/api/v1/nodes/{F()}", unscoped=True))
show("MOVE nonexistent -> real parent", c.request("POST", f"/api/v1/nodes/{F()}/move", json={"parent_id": sb}, unscoped=True))
show("MOVE real -> nonexistent parent", c.request("POST", f"/api/v1/nodes/{real}/move", json={"parent_id": F()}, unscoped=True))
show("MOVE nonexistent -> nonexistent", c.request("POST", f"/api/v1/nodes/{F()}/move", json={"parent_id": F()}, unscoped=True))
show("COMPLETE nonexistent", c.request("POST", f"/api/v1/nodes/{F()}/complete", unscoped=True))
show("UNCOMPLETE nonexistent", c.request("POST", f"/api/v1/nodes/{F()}/uncomplete", unscoped=True))
show("MIRROR nonexistent -> real parent", c.request("POST", f"/api/v1/nodes/{F()}/mirror", json={"parent_id": sb}, unscoped=True))
show("MIRROR real -> nonexistent parent", c.request("POST", f"/api/v1/nodes/{real}/mirror", json={"parent_id": F()}, unscoped=True))
show("DELETE MIRROR nonexistent", c.request("DELETE", f"/api/v1/nodes/{F()}/mirror", unscoped=True))
show("DELETE MIRROR on real non-mirror node", c.request("DELETE", f"/api/v1/nodes/{real}/mirror"))
show("GET real (still there?)", c.get(real))

print("### re-run a few to confirm determinism")
show("GET nonexistent #2", c.request("GET", f"/api/v1/nodes/{F()}", unscoped=True))
show("DELETE nonexistent #2", c.request("DELETE", f"/api/v1/nodes/{F()}", unscoped=True))

print("### malformed ids in path (GET and DELETE)")
malformed = {
    "garbage": "garbage-zq9x",
    "numeric": "12345",
    "braced_uuid": "{" + F() + "}",
    "uuid_plus_x": F() + "x",
    "uuid_trunc": F()[:-1],
    "non_hex_12": "zzzzzzzzzzzz",
    "unicode": "nöde",
    "space": "a b",
    "long": "a" * 300,
    "dotdot": "..",
}
for k, v in malformed.items():
    show(f"GET malformed {k}", c.request("GET", f"/api/v1/nodes/{quote(v, safe='')}", unscoped=True))
for k in ("garbage", "non_hex_12", "braced_uuid"):
    v = malformed[k]
    show(f"DELETE malformed {k}", c.request("DELETE", f"/api/v1/nodes/{quote(v, safe='')}", unscoped=True))
    show(f"UPDATE malformed {k}", c.request("POST", f"/api/v1/nodes/{quote(v, safe='')}", json={"name": "x"}, unscoped=True))

# NOTE: parent_id="" deliberately NOT sent: it might resolve to the account root.
print("### malformed parent_id in list")
for k, v in {"garbage": "garbage-zq9x", "non_hex_12": "zzzzzzzzzzzz",
             "uuid_trunc": F()[:-1], "long": "a" * 300}.items():
    show(f"LIST parent_id={k}", c.request("GET", "/api/v1/nodes", params={"parent_id": v}, unscoped=True))

print("### empty path segment")
show("GET /api/v1/nodes/ (trailing slash, no id)", c.request("GET", "/api/v1/nodes/", params={"parent_id": sb}))
