"""Batch 12: request body size limit, probed against a random (non-existent) UUID
so that nothing is stored. Also deletes the 1M-char nodes from batch 11 so we do
not leave megabytes of junk in the user's real account (UI performance)."""
import json, uuid
from common import ProdClient, j, RESULTS

c = ProdClient()
out = []
for n in (2_000_000, 5_000_000, 20_000_000):
    fake = str(uuid.uuid4())
    r = c.request("POST", f"/api/v1/nodes/{fake}", json={"name": "x" * n}, unscoped=True)
    rec = {"size": n, "status": r.status_code, "resp": j(r) if len(r.content) < 3000 else r.text[:500],
           "headers": dict(r.headers)}
    print(n, r.status_code, str(rec["resp"])[:300])
    out.append(rec)
deleted = []
# find 1M nodes in sandbox via log of batch 11
import pathlib
log = pathlib.Path(__file__).resolve().parents[2] / ".prod-state" / "requests.jsonl"
ids = set()
for line in log.read_text().splitlines():
    d = json.loads(line)
    if d.get("tag") != "text-content" or d["method"] != "GET" or d["status"] != 200:
        continue
    node = (d.get("response") or {}).get("node") if isinstance(d.get("response"), dict) else None
    if node and (len(node.get("name") or "") >= 1_000_000 or len(node.get("note") or "") >= 1_000_000):
        ids.add(node["id"])
for nid in sorted(ids):
    r = c.delete(nid)
    print("deleted", nid, r.status_code, j(r))
    deleted.append(nid)
(RESULTS / "12_body_size_limit.json").write_text(json.dumps({"probes": out, "deleted_1M_nodes": deleted}, indent=1))
