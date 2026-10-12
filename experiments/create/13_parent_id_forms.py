"""parent_id forms (full id, short id, URL variants, bad ids), malformed
bodies, and timestamp effects on the parent."""
import json, time, uuid
from _util import ProdClient

c = ProdClient()
sb = c.make_sandbox("create: 13 parent_id forms")
short = sb.replace("-", "")[-12:]
out = []


def rec(label, r, check=True):
    try:
        body = r.json()
    except Exception:
        body = r.text[:300]
    d = {"label": label, "status": r.status_code, "ctype": r.headers.get("content-type"), "response": body}
    if check and r.status_code == 200 and isinstance(body, dict) and "item_id" in body:
        n = c.get(body["item_id"]).json()["node"]
        d["parent_id_stored"] = n["parent_id"]
        d["parent_ok"] = n["parent_id"] == sb
    print(json.dumps(d, ensure_ascii=False))
    out.append(d)


# parent modifiedAt before
before = c.get(sb).json()["node"]
time.sleep(2.5)
rec("full uuid", c.create(sb, "via full uuid"))
after = c.get(sb).json()["node"]
print("parent modifiedAt before/after child create:", before["modifiedAt"], after["modifiedAt"])
out.append({"label": "parent modifiedAt", "before": before["modifiedAt"], "after": after["modifiedAt"]})

rec("short id", c.create(short, "via short id"))
rec("url", c.create(f"https://workflowy.com/#/{short}", "via url"))
# NOTE: an earlier run of this script sent the uppercase short id of our own
# sandbox with unscoped=True (the guard's allow-list is case-sensitive). It
# returned 200 and the node landed in the sandbox (verified by listing it):
# short ids are matched case-insensitively. That is a misuse of unscoped=True
# (it is only for ids that cannot exist), so the other id-variant probes
# (uppercase UUID, undashed UUID, URL without '#', URL trailing slash, URL with
# full UUID) were removed and are left as open questions.

# ids that cannot exist (unscoped OK per rules)
rec("random uuid", c.request("POST", "/api/v1/nodes", json={"parent_id": str(uuid.uuid4()), "name": "x"}, unscoped=True))
rec("random short", c.request("POST", "/api/v1/nodes", json={"parent_id": "0123456789ab", "name": "x"}, unscoped=True))
rec("garbage", c.request("POST", "/api/v1/nodes", json={"parent_id": "not-a-real-id-zzz", "name": "x"}, unscoped=True))
rec("empty string parent", c.request("POST", "/api/v1/nodes", json={"parent_id": "", "name": "x", "position": "middle"}, unscoped=True), check=False)
rec("int parent", c.request("POST", "/api/v1/nodes", json={"parent_id": 123, "name": "x"}, unscoped=True), check=False)
rec("random uuid + missing name", c.request("POST", "/api/v1/nodes", json={"parent_id": str(uuid.uuid4())}, unscoped=True), check=False)
rec("random uuid + bad position", c.request("POST", "/api/v1/nodes", json={"parent_id": str(uuid.uuid4()), "name": "x", "position": "zz"}, unscoped=True), check=False)
fake = str(uuid.uuid4())
rec("invalid json", c.request("POST", "/api/v1/nodes", content='{"parent_id": "%s", "name": ' % fake, headers={"Content-Type": "application/json"}, unscoped=True), check=False)
rec("json list", c.request("POST", "/api/v1/nodes", content='[1,2]', headers={"Content-Type": "application/json"}, unscoped=True), check=False)
rec("empty body", c.request("POST", "/api/v1/nodes", content='', headers={"Content-Type": "application/json"}, unscoped=True), check=False)
rec("form encoded (fake parent)", c.request("POST", "/api/v1/nodes", content=f'parent_id={fake}&name=x', headers={"Content-Type": "application/x-www-form-urlencoded"}, unscoped=True), check=False)
rec("json without content-type", c.request("POST", "/api/v1/nodes", content=json.dumps({"parent_id": fake, "name": "x"}), headers={"Content-Type": "text/plain"}, unscoped=True), check=False)
rec("multiple errors", c.request("POST", "/api/v1/nodes", json={"parent_id": fake, "name": 1, "note": 2, "position": "q", "layoutMode": 3}, unscoped=True), check=False)

with open(__file__.replace(".py", ".out.json"), "w") as f:
    json.dump(out, f, indent=1, ensure_ascii=False)
