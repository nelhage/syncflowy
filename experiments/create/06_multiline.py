"""Multiline names: how many nodes, structure, order, priorities, item_id."""
import json
from _util import ProdClient

c = ProdClient()

CASES = [
    ("a\nb", {}),
    ("a\n\nb", {}),
    ("a\n\nb\n\nc", {}),
    ("a\n\n\n\nb", {}),
    ("a\n  b", {}),
    ("a\n\n  b\n\n    c", {}),
    ("a\n\n- b\n\n- c", {}),
    ("a\n- b\n- c", {}),
    ("# Title\n\n- [ ] t1\n\n- [x] t2\n\n## sub", {}),
    ("a\n\n\tb", {}),
    ("\n\na leading", {}),
    ("a trailing\n\n", {}),
    ("a\nb\n\nc\nd", {}),
    ("first\n\n```\ncode\nmore\n```\n\nafter", {}),
    ("a\n\n> q", {}),
    ("parent\n\nchild1\n\nchild2", {"layoutMode": "todo", "note": "the note"}),
    ("- a\n  - b\n    - c", {}),
    ("- a\n\n  - b\n\n    - c", {}),
    ("1. a\n2. b", {}),
    ("a\r\nb", {}),
    ("a\r\n\r\nb", {}),
    ("p\n\nc1\nc1cont\n\nc2", {}),
]


def tree(nid, depth=0):
    kids = sorted(c.list(nid).json()["nodes"], key=lambda k: k["priority"])
    out = []
    for k in kids:
        out.append({"name": k["name"], "pri": k["priority"], "data": k["data"],
                    "completed": k["completed"], "note": k["note"],
                    "created": k["createdAt"], "children": tree(k["id"], depth + 1) if depth < 3 else "..."})
    return out


results = []
for i, (name, extra) in enumerate(CASES):
    sb = c.make_sandbox(f"create: 06 multiline #{i:02d} {name[:30]!r}")
    r = c.create(sb, name, position="bottom", **extra)
    body = r.json()
    rec = {"input": name, "extra": extra, "status": r.status_code, "response": body}
    if r.status_code == 200:
        item = c.get(body["item_id"]).json()["node"]
        rec["item"] = {"name": item["name"], "parent_is_sandbox": item["parent_id"] == sb, "data": item["data"], "note": item["note"]}
        rec["tree_under_sandbox"] = tree(sb)
    print(json.dumps(rec, ensure_ascii=False))
    results.append(rec)

with open(__file__.replace(".py", ".out.json"), "w") as f:
    json.dump(results, f, indent=1, ensure_ascii=False)
