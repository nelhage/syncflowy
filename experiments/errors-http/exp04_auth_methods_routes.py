"""Auth failures, wrong HTTP methods, unknown routes, trailing slashes.

Auth probes target a nonexistent random UUID so that nothing real is touched
even if auth unexpectedly succeeds. The "missing Authorization" probe uses a
bare httpx client, but still goes through ProdClient's shared rate limiter
(c.limiter.acquire()) and is logged by hand to .prod-state/requests.jsonl.
"""

import json
import os
import time

import httpx

from common import c, fake_uuid, show
from syncflowy.prod_client import BASE_URL, LOG_FILE

sb = c.make_sandbox("errors-http: auth, methods, routes")
n = c.create(sb, "method target").json()["item_id"]
c.allow.add(n)
key = c.api_key

print("### auth")
bare = httpx.Client(base_url=BASE_URL, timeout=30)


def bare_req(label, method, path, headers=None, **kw):
    c.limiter.acquire()
    t0 = time.time()
    r = bare.request(method, path, headers=headers or {}, **kw)
    with open(LOG_FILE, "a") as f:
        f.write(json.dumps({
            "ts": t0, "elapsed": round(time.time() - t0, 4), "method": method,
            "path": path, "params": kw.get("params"), "request_json": kw.get("json"),
            "request_headers": {k: ("<redacted>" if key in v else v) for k, v in (headers or {}).items()},
            "status": r.status_code, "response_headers": dict(r.headers),
            "response": r.text[:5000], "tag": os.environ.get("SYNCFLOWY_TAG"),
            "note": "bare httpx (no default Authorization header)",
        }) + "\n")
    return show(label, r)


bare_req("GET no Authorization header", "GET", f"/api/v1/nodes/{fake_uuid()}")
bare_req("LIST no Authorization header", "GET", "/api/v1/nodes", params={"parent_id": fake_uuid()})
bare_req("POST create no Authorization header", "POST", "/api/v1/nodes",
         json={"parent_id": fake_uuid(), "name": "x"})
bare_req("GET no auth, bogus path", "GET", "/api/v1/bogus-endpoint")
for label, h in {
    "bad key": "Bearer not-a-real-key-0000",
    "bearer only": "Bearer",
    "lowercase scheme": f"bearer {key}",
    "no scheme": key,
    "Token scheme": f"Token {key}",
    "Basic scheme": "Basic dXNlcjpwYXNz",
    "double space": f"Bearer  {key}",
}.items():
    r = c.request("GET", f"/api/v1/nodes/{fake_uuid()}", headers={"Authorization": h}, unscoped=True)
    show(f"GET auth={label}", r)
r = c.request("POST", f"/api/v1/nodes/{fake_uuid()}", json={"name": "x"},
              headers={"Authorization": "Bearer not-a-real-key-0000"}, unscoped=True)
show("UPDATE bad key", r)
r = c.request("GET", f"/api/v1/nodes/{fake_uuid()}",
              headers={"Authorization": "Bearer not-a-real-key-0000"}, unscoped=True)
show("GET bad key (repeat)", r, full_headers=True)

print("### wrong methods")
for m in ("PUT", "PATCH", "HEAD", "OPTIONS"):
    show(f"{m} /nodes/:id", c.request(m, f"/api/v1/nodes/{n}", json={"name": f"via {m}"} if m in ("PUT", "PATCH") else None), full_headers=(m == "OPTIONS"))
for m in ("PUT", "PATCH", "DELETE"):
    show(f"{m} /nodes?parent_id", c.request(m, "/api/v1/nodes", params={"parent_id": sb}))
for sub in ("move", "complete", "uncomplete", "mirror"):
    show(f"GET /nodes/:id/{sub}", c.request("GET", f"/api/v1/nodes/{n}/{sub}"))
show("DELETE /nodes/:id/move", c.request("DELETE", f"/api/v1/nodes/{n}/move"))
show("PUT /nodes/:id/complete", c.request("PUT", f"/api/v1/nodes/{n}/complete"))
show("POST /nodes/:id/mirror/x", c.request("POST", f"/api/v1/nodes/{n}/mirror/x"))
show("GET after method probes", c.get(n))

print("### unknown routes")
for p in ("/api/v1/foo", "/api/v1/", "/api/v1", "/api/v2/nodes", "/api/v1/me", "/api/v1/user",
          f"/api/v1/nodes/{n}/children", f"/api/v1/nodes/{n}/foo", "/api/v1/nodes-export/x"):
    if p == "/api/v1/nodes-export/x":
        continue  # guard forbids nodes-export prefix variants? keep out-of-scope.
    show(f"GET {p}", c.request("GET", p))
show("POST /api/v1/foo", c.request("POST", "/api/v1/foo", json={}))

print("### trailing slashes")
show("GET /nodes/:id/", c.request("GET", f"/api/v1/nodes/{n}/"))
show("POST /nodes/:id/ (update)", c.request("POST", f"/api/v1/nodes/{n}/", json={"note": "via trailing slash"}))
show("POST /nodes/ (create)", c.request("POST", "/api/v1/nodes/", json={"parent_id": sb, "name": "created via /nodes/"}))
show("POST /nodes/:id/complete/", c.request("POST", f"/api/v1/nodes/{n}/complete/"))
show("GET after trailing-slash probes", c.get(n))
show("LIST sandbox", c.list(sb))
