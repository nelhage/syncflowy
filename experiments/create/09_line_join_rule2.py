"""Line-joining rule, round 2: which characters at end-of-line block a join."""
import json
from _util import ProdClient

c = ProdClient()
BEFORE = [",", ":", ";", "!", "?", ")", "*", "-", "\"", "'", "1", "é", "A", "]", ">", "`", "~", "#", "/", "。"]
CASES = [f"x{ch}\nb" for ch in BEFORE] + [
    "a\n \nb",
    "a\n\n\n\n\n\nb",
    "a.\n\nb",
    "a\n\n b",
    "Hello world.\nthis is lowercase",
    "Hello world\nThis is upper",
    "hello\nworld\nagain\nmore",
]


def tree(nid):
    kids = sorted(c.list(nid).json()["nodes"], key=lambda k: k["priority"])
    return [{"name": k["name"], "lm": k["data"].get("layoutMode"),
             "children": [g["name"] for g in sorted(c.list(k["id"]).json()["nodes"], key=lambda g: g["priority"])]}
            for k in kids]


sb = c.make_sandbox("create: 09 line-join rule round 2")
results = []
for i, name in enumerate(CASES):
    holder = c.create(sb, f"case {i}: {name!r}", position="bottom").json()["item_id"]
    r = c.create(holder, name)
    rec = {"input": name, "status": r.status_code, "tree": tree(holder)}
    print(json.dumps(rec, ensure_ascii=False))
    results.append(rec)
with open(__file__.replace(".py", ".out.json"), "w") as f:
    json.dump(results, f, indent=1, ensure_ascii=False)
