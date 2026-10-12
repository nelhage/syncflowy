"""Continuation of 13: ids that cannot exist and malformed bodies (unscoped is
OK here because every parent_id is random / garbage)."""
import json, uuid
from _util import ProdClient

c = ProdClient()
out = []


def rec(label, r):
    try:
        body = r.json()
    except Exception:
        body = r.text[:300]
    d = {"label": label, "status": r.status_code, "ctype": r.headers.get("content-type"), "response": body}
    print(json.dumps(d, ensure_ascii=False))
    out.append(d)


def post(label, **kw):
    rec(label, c.request("POST", "/api/v1/nodes", unscoped=True, **kw))


post("random uuid", json={"parent_id": str(uuid.uuid4()), "name": "x"})
post("random short", json={"parent_id": "0123456789ab", "name": "x"})
post("garbage", json={"parent_id": "not-a-real-id-zzz", "name": "x"})
post("url to random short", json={"parent_id": "https://workflowy.com/#/0123456789ab", "name": "x"})
post("int parent", json={"parent_id": 123, "name": "x"})
post("random uuid + missing name", json={"parent_id": str(uuid.uuid4())})
post("random uuid + bad position", json={"parent_id": str(uuid.uuid4()), "name": "x", "position": "zz"})
fake = str(uuid.uuid4())
H = {"Content-Type": "application/json"}
post("invalid json", content='{"parent_id": "%s", "name": ' % fake, headers=H)
post("json list", content='[1,2]', headers=H)
post("json string", content='"hello"', headers=H)
post("empty body", content='', headers=H)
post("form encoded (fake parent)", content=f'parent_id={fake}&name=x', headers={"Content-Type": "application/x-www-form-urlencoded"})
post("json body with text/plain", content=json.dumps({"parent_id": fake, "name": "x"}), headers={"Content-Type": "text/plain"})
post("multiple errors", json={"parent_id": fake, "name": 1, "note": 2, "position": "q", "layoutMode": 3})
with open(__file__.replace(".py", ".out.json"), "w") as f:
    json.dump(out, f, indent=1, ensure_ascii=False)
