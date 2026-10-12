"""Request body encodings and wrong field types.

SAFETY: a create whose body the server fails to parse might be treated as
"no parent_id" and land at the account root (outside the scratchpad). So
encoding probes (form, invalid JSON, arrays, missing content-type) are run
against UPDATE (path id is ours) first; a create with that encoding is only
attempted if the update showed the server actually parsed the body.
Wrong-*type* probes on create always send a valid JSON object with a valid
parent_id.
"""

import json

from common import c, show

sb = c.make_sandbox("errors-http: body encodings + field types")
n = c.create(sb, "encoding target").json()["item_id"]
c.allow.add(n)
U = f"/api/v1/nodes/{n}"


def note_now():
    return c.get(n).json()["node"]["note"]


print("### update: encodings")
probes = [
    ("form-encoded", b"note=form-encoded-note", {"Content-Type": "application/x-www-form-urlencoded"}),
    ("json, no content-type", b'{"note": "json-no-ctype"}', {"Content-Type": ""}),
    ("json, text/plain", b'{"note": "json-text-plain"}', {"Content-Type": "text/plain"}),
    ("json, charset", b'{"note": "json-charset"}', {"Content-Type": "application/json; charset=utf-8"}),
    ("invalid json", b'{"note": ', {"Content-Type": "application/json"}),
    ("json array", b'[{"note": "array"}]', {"Content-Type": "application/json"}),
    ("json string", b'"hello"', {"Content-Type": "application/json"}),
    ("json null", b'null', {"Content-Type": "application/json"}),
    ("empty body", b'', {"Content-Type": "application/json"}),
    ("multipart", b'--XX\r\nContent-Disposition: form-data; name="note"\r\n\r\nmultipart-note\r\n--XX--\r\n',
     {"Content-Type": "multipart/form-data; boundary=XX"}),
]
parsed = {}
for label, body, hdrs in probes:
    before = note_now()
    r = c.request("POST", U, content=body, headers=hdrs)
    show(f"UPDATE {label}", r)
    after = note_now()
    parsed[label] = after != before
    print(json.dumps({"label": f"note after {label}", "note": after, "changed": after != before}))

print("### update: query params instead of body")
r = c.request("POST", U, params={"note": "from-query"}, json={})
show("UPDATE note in query, {} body", r)
print(json.dumps({"note_after": note_now()}))
r = c.request("POST", U, params={"note": "from-query-2"}, json={"note": "from-body-2"})
show("UPDATE note in query AND body", r)
print(json.dumps({"note_after": note_now()}))

print("### update: wrong types / odd fields")
cases = [
    {"name": 123},
    {"name": None},
    {"name": ["a"]},
    {"name": {"x": 1}},
    {"name": True},
    {"note": 456},
    {"note": None},
    {"layoutMode": "nonsense"},
    {"layoutMode": 7},
    {"unknown_field": "x"},
    {"completed": True},
    {"priority": 5},
    {"parent_id": sb},
    {"id": "00000000-0000-0000-0000-000000000000"},
    {},
]
for body in cases:
    r = c.request("POST", U, json=body)
    show(f"UPDATE {json.dumps(body)}", r)
    show("  -> GET", c.get(n))

print("### create: wrong types (valid JSON object, valid parent_id)")
ccases = [
    {"name": 123},
    {"name": None},
    {"name": True},
    {"name": ["a"]},
    {"name": ""},
    {},  # name missing
    {"name": "pos=7", "position": 7},
    {"name": "pos=middle", "position": "middle"},
    {"name": "pos=TOP", "position": "TOP"},
    {"name": "pos=null", "position": None},
    {"name": "note=7", "note": 7},
    {"name": "layout=bad", "layoutMode": "nonsense"},
    {"name": "layout=7", "layoutMode": 7},
    {"name": "unknown field", "foo": "bar"},
    {"name": "priority=5", "priority": 5},
    {"name": "completed=true", "completed": True},
]
for extra in ccases:
    r = c.request("POST", "/api/v1/nodes", json={"parent_id": sb, **extra})
    show(f"CREATE {json.dumps(extra)}", r)
    if r.status_code == 200:
        c.allow.add(r.json()["item_id"])
        show("  -> GET", c.get(r.json()["item_id"]))

print("### create: query params on POST alongside a valid body")
r = c.request("POST", "/api/v1/nodes", params={"name": "from-query", "position": "bottom"},
              json={"parent_id": sb, "name": "from-body"})
show("CREATE name in query+body", r)
if r.status_code == 200:
    show("  -> GET", c.get(r.json()["item_id"]))

if parsed.get("form-encoded"):
    print("### create: form-encoded (update showed form bodies are parsed)")
    r = c.request("POST", "/api/v1/nodes", content=f"parent_id={sb}&name=form-created".encode(),
                  headers={"Content-Type": "application/x-www-form-urlencoded"}, unscoped=True)
    show("CREATE form-encoded", r)
else:
    print("### create: form-encoded SKIPPED (update ignored form bodies; unsafe)")

print("### parent_id wrong types")
r = c.request("POST", "/api/v1/nodes", json={"parent_id": 123456789, "name": "x"}, unscoped=True)
show("CREATE parent_id=123456789 (number)", r)
r = c.request("POST", "/api/v1/nodes", json={"parent_id": [sb], "name": "x"}, unscoped=True)
show("CREATE parent_id=[sb] (array)", r)
r = c.request("POST", f"/api/v1/nodes/{n}/move", json={"parent_id": 123456789}, unscoped=True)
show("MOVE parent_id=number", r)
r = c.request("POST", f"/api/v1/nodes/{n}/move", json={"parent_id": sb, "position": 3})
show("MOVE position=3", r)
r = c.request("POST", f"/api/v1/nodes/{n}/move", json={"parent_id": sb, "position": "middle"})
show("MOVE position=middle", r)
show("LIST sandbox", c.list(sb))
