"""Misc params: name edge cases, note, completed, unknown fields, types."""
import json
from _util import ProdClient

c = ProdClient()
sb = c.make_sandbox("create: 12 params misc")


def go(label, body, raw=None):
    if raw is not None:
        r = c.request("POST", "/api/v1/nodes", content=raw, headers={"Content-Type": "application/json"})
    else:
        r = c.request("POST", "/api/v1/nodes", json=body)
    try:
        resp = r.json()
    except Exception:
        resp = r.text[:500]
    rec = {"label": label, "status": r.status_code, "response": resp}
    if r.status_code == 200 and isinstance(resp, dict) and "item_id" in resp:
        n = c.get(resp["item_id"]).json()["node"]
        rec["node"] = {k: n[k] for k in ("name", "note", "completed", "completedAt", "data", "priority", "createdAt", "modifiedAt")}
        kids = c.list(resp["item_id"]).json()["nodes"]
        if kids:
            rec["children"] = [(k["name"], k["note"]) for k in kids]
    print(json.dumps(rec, ensure_ascii=False))
    return rec


P = sb
out = []
out.append(go("name empty", {"parent_id": P, "name": ""}))
out.append(go("name whitespace", {"parent_id": P, "name": "   "}))
out.append(go("name newlines only", {"parent_id": P, "name": "\n\n\n"}))
out.append(go("name missing", {"parent_id": P}))
out.append(go("name null", {"parent_id": P, "name": None}))
out.append(go("name int", {"parent_id": P, "name": 42}))
out.append(go("name bool", {"parent_id": P, "name": True}))
out.append(go("name list", {"parent_id": P, "name": ["a", "b"]}))
out.append(go("name 10k chars", {"parent_id": P, "name": "x" * 10000}))
out.append(go("name 100k chars", {"parent_id": P, "name": ("word " * 20000).strip()}))
out.append(go("note plain", {"parent_id": P, "name": "with note", "note": "a note"}))
out.append(go("note markdown", {"parent_id": P, "name": "note md", "note": "**bold** *it* `code` [l](https://x.com) # h [2026-01-15]"}))
out.append(go("note multiline", {"parent_id": P, "name": "note ml", "note": "line1\nline2\n\nline4\n- bullet\n  indented"}))
out.append(go("note html", {"parent_id": P, "name": "note html", "note": "<b>b</b> &amp; &lt;x&gt; a<b"}))
out.append(go("note empty", {"parent_id": P, "name": "note empty", "note": ""}))
out.append(go("note null", {"parent_id": P, "name": "note null", "note": None}))
out.append(go("note whitespace", {"parent_id": P, "name": "note ws", "note": "  padded  "}))
out.append(go("note int", {"parent_id": P, "name": "note int", "note": 5}))
out.append(go("completed true", {"parent_id": P, "name": "completed true", "completed": True}))
out.append(go("completed false + - [x]", {"parent_id": P, "name": "- [x] x", "completed": False}))
out.append(go("completedAt given", {"parent_id": P, "name": "completedAt", "completedAt": 1700000000}))
out.append(go("priority given", {"parent_id": P, "name": "priority 5", "priority": 5}))
out.append(go("id given", {"parent_id": P, "name": "id given", "id": "11111111-2222-3333-4444-555555555555"}))
out.append(go("createdAt given", {"parent_id": P, "name": "createdAt", "createdAt": 1600000000}))
out.append(go("unknown field", {"parent_id": P, "name": "unknown", "foo": "bar", "mirror": True}))
out.append(go("parentId camel", {"parent_id": P, "name": "both", "parentId": "whatever"}))
# Malformed bodies (no parent_id) are blocked by the scope guard; see
# 13b_bad_ids_and_bodies.py which sends them with a random parent_id.
with open(__file__.replace(".py", ".out.json"), "w") as f:
    json.dump(out, f, indent=1, ensure_ascii=False)
