"""Which id forms update/complete/uncomplete accept, and how they fail on
nonexistent / deleted / malformed ids.

unscoped=True is used ONLY for ids that cannot exist (random UUID, garbage).
"""

import time
import uuid

from _common import c, show, mk, node

sb = c.make_sandbox("update-complete: 07 id forms + missing/deleted nodes")
print("sandbox", sb)
n = mk(sb, "id-form subject")
short = n.replace("-", "")[-12:]
url = f"https://workflowy.com/#/{short}"
time.sleep(3)  # rule out freshness as cause of short-id failures

print("## id forms")
show("GET short", c.get(short))
for label, ident in [("full", n), ("short", short), ("url", url), ("UPPER full", n.upper()),
                     ("no dashes", n.replace("-", ""))]:
    path_id = ident.replace("/", "%2F").replace(":", "%3A").replace("#", "%23") if label == "url" else ident
    for op in ("update", "complete", "uncomplete"):
        if op == "update":
            r = c.request("POST", f"/api/v1/nodes/{path_id}", json={"name": f"via {label}"}, unscoped=(label in ("UPPER full", "no dashes", "url")))
        else:
            r = c.request("POST", f"/api/v1/nodes/{path_id}/{op}", unscoped=(label in ("UPPER full", "no dashes", "url")))
        show(f"{op} via {label}", r)
    got = node(n)
    print("   now:", got["name"], got["completed"])
    c.uncomplete(n)

print("## nonexistent ids (unscoped: these cannot exist)")
rnd = str(uuid.uuid4())
for label, ident in [("random uuid", rnd), ("garbage", "not-a-real-id"), ("random short", "0123456789ab")]:
    show(f"update {label}", c.request("POST", f"/api/v1/nodes/{ident}", json={"name": "x"}, unscoped=True))
    show(f"update {label} invalid body", c.request("POST", f"/api/v1/nodes/{ident}", json={"name": 1}, unscoped=True))
    show(f"complete {label}", c.request("POST", f"/api/v1/nodes/{ident}/complete", unscoped=True))
    show(f"uncomplete {label}", c.request("POST", f"/api/v1/nodes/{ident}/uncomplete", unscoped=True))

print("## deleted node")
d = mk(sb, "to be deleted")
dk = mk(d, "child of deleted")
show("delete", c.delete(d))
show("GET deleted", c.get(d))
show("update deleted", c.update(d, name="zombie"))
show("complete deleted", c.complete(d))
show("uncomplete deleted", c.uncomplete(d))
show("GET child of deleted", c.get(dk))
show("update child of deleted", c.update(dk, name="zombie child"))
show("complete child of deleted", c.complete(dk))
time.sleep(3)
show("GET deleted (after 3s)", c.get(d))
show("update deleted (after 3s)", c.update(d, name="zombie"))
