"""Pin down the line-joining rule (see create.md "Multiline names"). Round 1."""



from _util import ProdClient

c = ProdClient()

CASES = [
    "ab\nc\nd",
    "a\nbc\nd",
    "a\n\n\nb",
    "a\nB",
    "a\n1",
    "a1\n2b",
    "a\n#tag",
    "a\n(b)",
    "a\nété",
    "a.\nb",
    "a \nb",
    "a\n*b*",
    "a\n**b**",
    "a\n[link](https://x.com)",
    "a\n_b",
    "a\n`b`",
    "a\n\"q\"",
    "a\n@b",
    "a\nb\nc\nd\ne\nf",
    "aa\nbb\ncc",
    "aa\n\nbb\n\ncc",
    "p\n\n- [x] done\n\nplain\n\n- [ ] open\n\nplain2\n\n# h",
    "p\n\n- one\n\n  - two\n\n- three",
    "p\n\n# h1\n\n## h2\n\n### h3\n\n#### h4\n\n- [ ] t\n\n```c```\n\n> q",
]


def tree(nid, depth=0):
    kids = sorted(c.list(nid).json()["nodes"], key=lambda k: k["priority"])
    return [{"name": k["name"], "pri": k["priority"], "lm": k["data"].get("layoutMode"),
             "completed": k["completed"], "note": k["note"],
             "children": tree(k["id"], depth + 1) if depth < 2 else "..."} for k in kids]


sb = c.make_sandbox("create: 08 line-join rule")
results = []
for i, name in enumerate(CASES):
    holder = c.create(sb, f"case {i}: {name!r}", position="bottom").json()["item_id"]
    r = c.create(holder, name)
    rec = {"input": name, "status": r.status_code, "tree": tree(holder)}
    print(json.dumps(rec, ensure_ascii=False))
    results.append(rec)
with open(__file__.replace(".py", ".out.json"), "w") as f:
    json.dump(results, f, indent=1, ensure_ascii=False)
