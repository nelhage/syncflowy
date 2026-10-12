"""Do the mutating endpoints accept short ids / URL forms in the path and in
body parent_id? (exp01 showed update rejects short ids in the path.)

Also probes extra URL spellings for list parent_id.
"""

from urllib.parse import quote

from common import c, show, variant_request

sb = c.make_sandbox("errors-http: short ids on mutating endpoints")
known = {sb}


def mk(name, parent=sb):
    r = c.create(parent, name, position="bottom")
    nid = r.json()["item_id"]
    known.add(nid)
    return nid


dest = mk("dest")
known.add(dest)
dshort = dest.replace("-", "")[-12:]
nodes = {k: mk(k) for k in ("complete", "uncomplete", "move", "mirror", "delete",
                            "delete_mirror", "move_url", "mirror_short")}


def short(i):
    return i.replace("-", "")[-12:]


print("### short ids in PATH")
r = variant_request("POST", f"/api/v1/nodes/{short(nodes['complete'])}/complete", known,
                    [short(nodes['complete'])])
show("COMPLETE short-path", r)
r = c.complete(nodes["uncomplete"])
show("COMPLETE full (setup)", r)
r = variant_request("POST", f"/api/v1/nodes/{short(nodes['uncomplete'])}/uncomplete", known,
                    [short(nodes['uncomplete'])])
show("UNCOMPLETE short-path", r)
r = variant_request("POST", f"/api/v1/nodes/{short(nodes['move'])}/move", known,
                    [short(nodes['move']), dest], json={"parent_id": dest})
show("MOVE short-path", r)
r = variant_request("POST", f"/api/v1/nodes/{short(nodes['mirror'])}/mirror", known,
                    [short(nodes['mirror']), dest], json={"parent_id": dest})
show("MIRROR short-path", r)
if r.status_code < 300:
    c.allow.add(*r.json().values())
r = variant_request("DELETE", f"/api/v1/nodes/{short(nodes['delete'])}", known,
                    [short(nodes['delete'])])
show("DELETE short-path", r)

print("### state after")
r = c.list(sb)
show("LIST sandbox", r)
r = c.list(dest)
show("LIST dest", r)

print("### parent_id variants in BODY for move / mirror")
for k, v in {"short": dshort, "short_upper": dshort.upper(),
             "url": f"https://workflowy.com/#/{dshort}",
             "upper": dest.upper()}.items():
    r = variant_request("POST", f"/api/v1/nodes/{nodes['move_url']}/move", known, [v],
                        json={"parent_id": v})
    show(f"MOVE parent_id={k}", r)
    r = variant_request("POST", f"/api/v1/nodes/{nodes['mirror_short']}/mirror", known, [v],
                        json={"parent_id": v})
    show(f"MIRROR parent_id={k}", r)
    if r.status_code < 300:
        c.allow.add(*r.json().values())

print("### more URL spellings for list parent_id")
for k, v in {
    "no_scheme": f"workflowy.com/#/{dshort}",
    "hash_only": f"#/{dshort}",
    "nested_path": f"https://workflowy.com/#/{short(sb)}/{dshort}",
    "query_suffix": f"https://workflowy.com/#/{dshort}?q=x",
    "beta": f"https://beta.workflowy.com/#/{dshort}",
    "padded": f"  {dshort}  ",
    "full_padded": f" {dest} ",
}.items():
    r = c.request("GET", "/api/v1/nodes", params={"parent_id": v}, unscoped=True)
    show(f"LIST {k}", r)
