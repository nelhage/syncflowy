"""Shared harness for text-content fidelity experiments.

Each "case" is a (label, input_string) pair. For every case we exercise:

* create path:  POST /api/v1/nodes {parent_id: <create sandbox>, name: X, note: X}
* update path:  POST /api/v1/nodes {name: "u"}  then  POST /api/v1/nodes/:id {name: X, note: X}

and then read the stored values back with a single GET /api/v1/nodes?parent_id=
per sandbox (and optionally per-node GETs). Results are written as JSON to
results/<batch>.json so the docs can cite them, and printed as a table.
"""
from __future__ import annotations

import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "src"))
from syncflowy.prod_client import ProdClient  # noqa: E402

HERE = Path(__file__).resolve().parent
RESULTS = HERE / "results"
RESULTS.mkdir(exist_ok=True)


def j(r):
    try:
        return r.json()
    except Exception:
        return r.text[:2000]


def children(c, pid):
    r = c.list(pid)
    r.raise_for_status()
    return sorted(r.json()["nodes"], key=lambda n: n["priority"])


def run_batch(batch: str, cases, *, fields=("name", "note"), do_create=True,
              do_update=True, sandbox_label=None):
    """Run cases through create and update paths. Returns result dicts."""
    c = ProdClient()
    out = {"batch": batch, "fields": list(fields), "cases": []}
    sb_c = sb_u = None
    if do_create:
        sb_c = c.make_sandbox(f"text: {sandbox_label or batch} (create path)")
    if do_update:
        sb_u = c.make_sandbox(f"text: {sandbox_label or batch} (update path)")
    out["sandbox_create"], out["sandbox_update"] = sb_c, sb_u
    recs = []
    for label, val in cases:
        rec = {"label": label, "input": val}
        if do_create:
            body = {"parent_id": sb_c, "position": "bottom"}
            for f in fields:
                body[f] = val
            r = c.request("POST", "/api/v1/nodes", json=body)
            rec["create_status"], rec["create_resp"] = r.status_code, j(r)
            rec["create_id"] = rec["create_resp"].get("item_id") if isinstance(rec["create_resp"], dict) else None
        if do_update:
            r = c.create(sb_u, "u", position="bottom")
            uid = r.json()["item_id"]
            rec["update_id"] = uid
            r = c.update(uid, **{f: val for f in fields})
            rec["update_status"], rec["update_resp"] = r.status_code, j(r)
        recs.append(rec)
    by_id = {}
    for sb in (sb_c, sb_u):
        if sb:
            for n in children(c, sb):
                by_id[n["id"]] = n
    for rec in recs:
        for path in ("create", "update"):
            nid = rec.get(f"{path}_id")
            if nid:
                n = by_id.get(nid)
                rec[f"{path}_node"] = n
                if n is None:
                    rec[f"{path}_node_missing"] = True
                    continue
                for f in fields:
                    rec[f"{path}_{f}"] = n.get(f)
        out["cases"].append(rec)
    # extra children created by multi-line names: record all of them
    out["all_children_create"] = children(c, sb_c) if sb_c else None
    (RESULTS / f"{batch}.json").write_text(json.dumps(out, ensure_ascii=False, indent=1))
    print_table(out)
    return out


def print_table(out):
    fields = out["fields"]
    for rec in out["cases"]:
        print(f"== {rec['label']}  input={rec['input'][:200]!r}")
        for path in ("create", "update"):
            st = rec.get(f"{path}_status")
            if st is None:
                continue
            if st >= 400 or rec.get(f"{path}_node_missing"):
                print(f"   {path}: status={st} resp={rec.get(path + '_resp')}")
                continue
            for f in fields:
                v = rec.get(f"{path}_{f}")
                same = "SAME" if v == rec["input"] else "DIFF"
                print(f"   {path}.{f} [{same}]: {v if v is None else v[:300]!r}")
