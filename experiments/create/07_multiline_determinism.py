"""Multiline results in 06 looked inconsistent. Re-run selected cases 2x each
to check determinism, and isolate the effect of layoutMode/note params."""
import json, sys
from _util import ProdClient

c = ProdClient()

CASES = [
    ("a\n\nb\n\nc", {}),
    ("a\n\nb\n\nc", {}),
    ("parent\n\nchild1\n\nchild2", {}),
    ("parent\n\nchild1\n\nchild2", {}),
    ("parent\n\nchild1\n\nchild2", {"layoutMode": "todo"}),
    ("parent\n\nchild1\n\nchild2", {"note": "n"}),
    ("a\n\nb\n\nc", {"layoutMode": "todo", "note": "the note"}),
    ("p\n\nc1\nc1cont\n\nc2", {}),
    ("a\n\nb", {}),
    ("x\n\ny\n\nz\n\nw", {}),
    ("x\n\ny\n\nz\n\nw\n\nv", {}),
]


def tree(nid, depth=0):
    kids = sorted(c.list(nid).json()["nodes"], key=lambda k: k["priority"])
    return [{"name": k["name"], "pri": k["priority"], "lm": k["data"].get("layoutMode"),
             "children": tree(k["id"], depth + 1) if depth < 2 else "..."} for k in kids]


sb = c.make_sandbox("create: 07 multiline determinism")
results = []
for i, (name, extra) in enumerate(CASES):
    holder = c.create(sb, f"case {i}: {name!r} {extra}", position="bottom").json()["item_id"]
    r = c.create(holder, name, **extra)
    rec = {"input": name, "extra": extra, "status": r.status_code, "tree": tree(holder)}
    print(json.dumps(rec, ensure_ascii=False))
    results.append(rec)
with open(__file__.replace(".py", ".out.json"), "w") as f:
    json.dump(results, f, indent=1, ensure_ascii=False)
