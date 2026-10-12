"""layoutMode values (valid/invalid/case/type) and attempts to set fields the
docs do not list as updatable (parent_id, priority, completed, completedAt,
createdAt, modifiedAt, id, data.layoutMode, position)."""

import json

from _common import c, show, mk, node, pnode

sb = c.make_sandbox("update-complete: 05 layoutMode + read-only fields")
print("sandbox", sb)
n = mk(sb, "subject")
other = mk(sb, "other parent")
pnode("start", n)

print("## layoutMode values")
for v in ["todo", "h1", "h2", "h3", "code-block", "quote-block", "bullets",
          "board", "p", "table", "dashboard", "H1", "Todo", "", "nonsense", None, 1, True, ["todo"]]:
    r = c.update(n, layoutMode=v)
    got = node(n)
    print(json.dumps({"in": v, "status": r.status_code, "resp": r.json(),
                      "layout": got["data"]["layoutMode"], "data": got["data"]}))

c.update(n, layoutMode="bullets")

print("## layout_mode / data.layoutMode spellings")
for body in [{"layout_mode": "h2"}, {"data": {"layoutMode": "h3"}}, {"layoutmode": "todo"}]:
    r = c.request("POST", f"/api/v1/nodes/{n}", json=body)
    got = node(n)
    print(json.dumps({"body": body, "status": r.status_code, "resp": r.json(), "layout": got["data"]["layoutMode"]}))
c.update(n, layoutMode="bullets")

print("## read-only / unlisted fields")
bodies = [
    {"parent_id": other},
    {"priority": 5},
    {"priority": "x"},
    {"completed": True},
    {"completed": "yes"},
    {"completedAt": 1000},
    {"createdAt": 1000},
    {"modifiedAt": 1000},
    {"id": "00000000-0000-0000-0000-000000000000"},
    {"position": "bottom"},
    {"name": "with extra", "bogus": {"nested": [1]}},
    {"name": "Name-case", "Name": "capital"},
]
for b in bodies:
    r = c.request("POST", f"/api/v1/nodes/{n}", json=b)
    got = node(n)
    print(json.dumps({"body": b, "status": r.status_code, "resp": r.json(),
                      "after": {k: got[k] for k in ("id", "name", "parent_id", "priority", "completed",
                                                      "completedAt", "createdAt", "modifiedAt")}}))
