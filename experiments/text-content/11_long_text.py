"""Batch 11: very long names/notes. Is there a length limit? Truncation?"""
import json, sys
from common import ProdClient, children, j, RESULTS

SIZES = [int(s) for s in sys.argv[1:]] or [10_000, 100_000, 1_000_000]
c = ProdClient()
sb = c.make_sandbox(f"text: 11 long text {SIZES}")
out = []
for n in SIZES:
    s = ("0123456789" * (n // 10 + 1))[:n]
    for path in ("create", "update"):
        for field in ("name", "note"):
            rec = {"size": n, "path": path, "field": field}
            if path == "create":
                body = {"parent_id": sb, "name": f"long {field} {n}", "position": "bottom"}
                body[field] = s
                r = c.request("POST", "/api/v1/nodes", json=body)
                rec["status"], resp = r.status_code, j(r)
                nid = resp.get("item_id") if isinstance(resp, dict) else None
            else:
                nid = c.create(sb, f"long upd {field} {n}", position="bottom").json()["item_id"]
                r = c.update(nid, **{field: s})
                rec["status"], resp = r.status_code, j(r)
            rec["resp"] = resp if len(json.dumps(resp)) < 2000 else str(resp)[:2000]
            if nid:
                g = c.get(nid)
                node = g.json().get("node", {})
                v = node.get(field) or ""
                rec["stored_len"] = len(v)
                rec["equal"] = v == s
                rec["get_bytes"] = len(g.content)
            print(rec["size"], path, field, rec["status"], str(rec["resp"])[:200], rec.get("stored_len"), rec.get("equal"))
            out.append(rec)
p = RESULTS / "11_long_text.json"
prev = json.loads(p.read_text()) if p.exists() else []
p.write_text(json.dumps(prev + out, indent=1))
