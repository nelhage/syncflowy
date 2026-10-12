"""Partial update semantics: name/note/layoutMode, empty vs null vs omitted.

Establishes:
  * response body/status of POST /api/v1/nodes/:id
  * which fields are touched when only one is supplied
  * note "" vs null, name "" vs null
"""

from _common import c, show, pnode, mk

sb = c.make_sandbox("update-complete: 01 partial update semantics")
print("sandbox", sb)

n = mk(sb, "orig name", note="orig note")
pnode("created", n)

show("update name only", c.update(n, name="new name"))
pnode("after name-only", n)

show("update note only", c.update(n, note="new note"))
pnode("after note-only", n)

show("update empty body {}", c.update(n))
pnode("after {}", n)

show("update note=''", c.update(n, note=""))
pnode("after note=''", n)

show("update note='x' (restore)", c.update(n, note="x"))
show("update note=None", c.update(n, note=None))
pnode("after note=null", n)

show("update name=''", c.update(n, name=""))
pnode("after name=''", n)

show("update name=None", c.update(n, name=None))
pnode("after name=null", n)

show("update name=123 (int)", c.update(n, name=123))
pnode("after name=123", n)

show("update note=456 (int)", c.update(n, note=456))
pnode("after note=456", n)

show("update name=['a'] (list)", c.update(n, name=["a"]))
pnode("after name=list", n)

show("update name=True", c.update(n, name=True))
pnode("after name=True", n)

show("update name+note together", c.update(n, name="both", note="both note"))
pnode("after both", n)

# Raw bodies
show("raw body: empty content", c.request("POST", f"/api/v1/nodes/{n}", content=b""))
show("raw body: invalid json", c.request("POST", f"/api/v1/nodes/{n}", content=b"{not json", headers={"content-type": "application/json"}))
show("raw body: json array", c.request("POST", f"/api/v1/nodes/{n}", content=b"[1,2]", headers={"content-type": "application/json"}))
show("raw body: form-encoded", c.request("POST", f"/api/v1/nodes/{n}", content=b"name=formname", headers={"content-type": "application/x-www-form-urlencoded"}))
pnode("after raw bodies", n)
show("raw body: json without content-type", c.request("POST", f"/api/v1/nodes/{n}", content=b'{"name":"noct"}'))
pnode("after json-without-ct", n)

# PUT / PATCH method
show("PATCH", c.request("PATCH", f"/api/v1/nodes/{n}", json={"name": "patched"}))
show("PUT", c.request("PUT", f"/api/v1/nodes/{n}", json={"name": "put"}))
pnode("after PATCH/PUT", n)
