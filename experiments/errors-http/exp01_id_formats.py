"""Which spellings of a node id does each endpoint accept?

Creates a sandbox with one child `T`, then probes GET /nodes/:id, list
(?parent_id=), create (body parent_id), update (path id) with many spellings of
T's id. Only id-variants that client-side normalize to our own sandbox nodes
are sent (see common.variant_request).
"""

from urllib.parse import quote

from common import c, show, variant_request

sb = c.make_sandbox("errors-http: id formats")
r = c.create(sb, "T (id-format target)")
show("create T", r)
tid = r.json()["item_id"]
r = c.create(tid, "T-child")
show("create T-child", r)
known = {sb, tid}
short = tid.replace("-", "")[-12:]
url = f"https://workflowy.com/#/{short}"

variants = {
    "full": tid,
    "upper": tid.upper(),
    "nodash": tid.replace("-", ""),
    "nodash_upper": tid.replace("-", "").upper(),
    "short": short,
    "short_upper": short.upper(),
    "url": url,
    "url_upper_short": f"https://workflowy.com/#/{short.upper()}",
    "url_http": f"http://workflowy.com/#/{short}",
    "url_www": f"https://www.workflowy.com/#/{short}",
    "url_trailing_slash": url + "/",
    "short_11": short[1:],
    "short_13": tid.replace("-", "")[-13:],
    "last_8": short[-8:],
    "first_12": tid.replace("-", "")[:12],
}

print("### GET /nodes/:id (path, fully percent-encoded)")
for k, v in variants.items():
    try:
        r = variant_request("GET", f"/api/v1/nodes/{quote(v, safe='')}", known, [v]
                            if k not in ("short_11", "short_13", "last_8", "first_12") else [])
    except RuntimeError as e:
        print("skip", k, e)
        continue
    show(f"GET {k}", r)

print("### GET /nodes?parent_id=")
for k, v in variants.items():
    r = c.request("GET", "/api/v1/nodes", params={"parent_id": v}, unscoped=True) \
        if k in ("short_11", "short_13", "last_8", "first_12") else \
        variant_request("GET", "/api/v1/nodes", known, [v], params={"parent_id": v})
    show(f"LIST {k}", r)

print("### POST /nodes (create) parent_id in body")
for k in ("upper", "nodash", "short", "short_upper", "url", "url_http", "url_trailing_slash"):
    v = variants[k]
    r = variant_request("POST", "/api/v1/nodes", known, [v],
                        json={"parent_id": v, "name": f"created via parent_id={k}"})
    show(f"CREATE {k}", r)
    if r.status_code < 300:
        c.allow.add(r.json()["item_id"])

print("### POST /nodes/:id (update) with id variants in path")
for k in ("upper", "nodash", "short", "short_upper", "url"):
    v = variants[k]
    r = variant_request("POST", f"/api/v1/nodes/{quote(v, safe='')}", known, [v],
                        json={"note": f"updated via {k}"})
    show(f"UPDATE {k}", r)
r = c.get(tid)
show("GET after updates", r)
