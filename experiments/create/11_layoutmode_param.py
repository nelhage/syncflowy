"""layoutMode param: valid/invalid values, and conflicts with markdown prefixes.
Also completed-state leak across multiline children."""
import json
from _util import ProdClient

c = ProdClient()

CASES = [
    ("plain", {"layoutMode": "bullets"}),
    ("plain", {"layoutMode": "todo"}),
    ("plain", {"layoutMode": "h1"}),
    ("plain", {"layoutMode": "h2"}),
    ("plain", {"layoutMode": "h3"}),
    ("plain", {"layoutMode": "code-block"}),
    ("plain", {"layoutMode": "quote-block"}),
    ("plain", {"layoutMode": "p"}),
    ("plain", {"layoutMode": "board"}),
    ("plain", {"layoutMode": "dashboard"}),
    ("plain", {"layoutMode": "H1"}),
    ("plain", {"layoutMode": ""}),
    ("plain", {"layoutMode": None}),
    ("plain", {"layoutMode": 1}),
    ("# Head", {"layoutMode": "todo"}),
    ("# Head", {"layoutMode": "bullets"}),
    ("- [x] done", {"layoutMode": "bullets"}),
    ("- [x] done", {"layoutMode": "h1"}),
    ("- [ ] open", {"layoutMode": "h2"}),
    ("```code```", {"layoutMode": "quote-block"}),
    ("- bullet", {"layoutMode": "todo"}),
    ("**bold**", {"layoutMode": "code-block"}),
    ("#### four", {"layoutMode": "h1"}),
    ("plain", {"data": {"layoutMode": "h1"}}),
    ("- [x] parent\n\nchild one\n\nChild two", {}),
    ("p\n\n- [x] a\n\n- [ ] B\n\n- C", {}),
]

sb = c.make_sandbox("create: 11 layoutMode param")
results = []
for name, extra in CASES:
    r = c.create(sb, name, position="bottom", **extra)
    rec = {"input": name, "extra": extra, "status": r.status_code}
    if r.status_code != 200:
        rec["body"] = r.json()
    else:
        nid = r.json()["item_id"]
        n = c.get(nid).json()["node"]
        rec.update(name=n["name"], data=n["data"], completed=n["completed"], completedAt=n["completedAt"])
        if "\n" in name:
            rec["children"] = [(k["name"], k["data"], k["completed"]) for k in sorted(c.list(nid).json()["nodes"], key=lambda k: k["priority"])]
    print(json.dumps(rec, ensure_ascii=False))
    results.append(rec)
with open(__file__.replace(".py", ".out.json"), "w") as f:
    json.dump(results, f, indent=1, ensure_ascii=False)
