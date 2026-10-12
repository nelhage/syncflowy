"""Batch 10: line-separator chars on create (where did 'b' go?), NUL on create,
GET-vs-list consistency, null/absent fields."""
import json
from common import ProdClient, children, j, RESULTS

c = ProdClient()
out = {}
prev = json.load(open(RESULTS / "09_control_chars.json"))
sep = {}
for rec in prev["cases"]:
    if rec.get("create_name") == "a":
        r = c.list(rec["create_id"])
        sep[rec["label"]] = r.json()
        print(rec["label"], "children:", [(n["name"], n["note"], n["data"]) for n in r.json()["nodes"]])
out["linesep_children"] = sep

sb = c.make_sandbox("text: 10 misc (NUL, nulls, GET vs list)")
res = []
def rec(label, method, path, body=None):
    r = c.request(method, path, json=body)
    res.append({"label": label, "method": method, "path": path, "body": body, "status": r.status_code, "resp": j(r)})
    print(f"== {label}: {r.status_code} {json.dumps(j(r), ensure_ascii=False)[:400]}")
    return r

rec("create name with NUL only", "POST", "/api/v1/nodes", {"parent_id": sb, "name": "a\x00b"})
rec("create note with NUL only", "POST", "/api/v1/nodes", {"parent_id": sb, "name": "nul-note", "note": "a\x00b"})
rec("create name with \\x0b and note absent", "POST", "/api/v1/nodes", {"parent_id": sb, "name": "x\x0by\x0bz\x0bw"})
r = rec("create no note field", "POST", "/api/v1/nodes", {"parent_id": sb, "name": "nonote"})
nonote = r.json()["item_id"]
r = rec("create note null", "POST", "/api/v1/nodes", {"parent_id": sb, "name": "notenull", "note": None})
notenull = r.json().get("item_id")
r = rec("create note empty", "POST", "/api/v1/nodes", {"parent_id": sb, "name": "noteempty", "note": ""})
noteempty = r.json().get("item_id")
r = rec("create name empty", "POST", "/api/v1/nodes", {"parent_id": sb, "name": ""})
r = rec("create time+md name", "POST", "/api/v1/nodes", {"parent_id": sb, "name": "**b** [2026-01-15 14:30] <u>u</u>", "note": "n"})
tid = r.json()["item_id"]
rec("GET time node by id", "GET", f"/api/v1/nodes/{tid}")
# update paths: note null / empty / name null
r = rec("upd note->text", "POST", f"/api/v1/nodes/{nonote}", {"note": "has note"})
rec("GET after note set", "GET", f"/api/v1/nodes/{nonote}")
rec("upd note->null", "POST", f"/api/v1/nodes/{nonote}", {"note": None})
rec("GET after note null", "GET", f"/api/v1/nodes/{nonote}")
rec("upd note->''", "POST", f"/api/v1/nodes/{nonote}", {"note": ""})
rec("GET after note ''", "GET", f"/api/v1/nodes/{nonote}")
rec("upd name->null", "POST", f"/api/v1/nodes/{nonote}", {"name": None})
rec("GET after name null", "GET", f"/api/v1/nodes/{nonote}")
rec("upd name->''", "POST", f"/api/v1/nodes/{nonote}", {"name": ""})
rec("GET after name ''", "GET", f"/api/v1/nodes/{nonote}")
rec("upd name non-string int", "POST", f"/api/v1/nodes/{nonote}", {"name": 123})
rec("GET after name 123", "GET", f"/api/v1/nodes/{nonote}")
rec("upd note non-string", "POST", f"/api/v1/nodes/{nonote}", {"note": ["x"]})
rec("GET after note list", "GET", f"/api/v1/nodes/{nonote}")
rec("create name int", "POST", "/api/v1/nodes", {"parent_id": sb, "name": 42})
rec("create note int", "POST", "/api/v1/nodes", {"parent_id": sb, "name": "noteint", "note": 42})
# re-feed create-generated HTML into update: idempotent?
r = c.get(tid); html = r.json()["node"]["name"]
rec("upd name with own create output", "POST", f"/api/v1/nodes/{tid}", {"name": html})
rec("GET after self-roundtrip", "GET", f"/api/v1/nodes/{tid}")
lst = c.list(sb).json()
out["list"] = lst
out["requests"] = res
for n in sorted(lst["nodes"], key=lambda n: n["priority"]):
    print("   list:", json.dumps(n, ensure_ascii=False))
(RESULTS / "10_misc.json").write_text(json.dumps(out, ensure_ascii=False, indent=1))
