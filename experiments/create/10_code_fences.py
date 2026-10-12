"""Code fences in multiline names: interaction with the line-join rule."""
import json
from _util import ProdClient

c = ProdClient()
CASES = [
    "```\na\nb\n```",
    "```\nline one\nline two\n```",
    "```\nline one\n\nline two\n```",
    "p\n\n```\nline one\nline two\n```",
    "p\n\n```\nline one\nline two\n```\n\nAfter",
    "p\n\n```\nline one\nline two\n```\n\nafter",
    "p\n\n```js\nx = 1\n```\n\nAfter",
    "p\n\n```\nline one\n\nline two\n```\n\nAfter",
    "p\n\n```inline```\n\nafter",
    "p\n\n```\nUpper\n```\n\nq",
    "```\n    indented code\n\tTabbed\n```",
]


def tree(nid):
    kids = sorted(c.list(nid).json()["nodes"], key=lambda k: k["priority"])
    return [{"name": k["name"], "lm": k["data"].get("layoutMode"),
             "children": [(g["name"], g["data"].get("layoutMode")) for g in sorted(c.list(k["id"]).json()["nodes"], key=lambda g: g["priority"])]}
            for k in kids]


sb = c.make_sandbox("create: 10 code fences multiline")
results = []
for i, name in enumerate(CASES):
    holder = c.create(sb, f"case {i}", position="bottom").json()["item_id"]
    r = c.create(holder, name)
    rec = {"input": name, "status": r.status_code, "tree": tree(holder)}
    print(json.dumps(rec, ensure_ascii=False))
    results.append(rec)
with open(__file__.replace(".py", ".out.json"), "w") as f:
    json.dump(results, f, indent=1, ensure_ascii=False)
