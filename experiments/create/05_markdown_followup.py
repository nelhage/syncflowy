"""Follow-ups on surprising markdown results from 04 (re-run for determinism,
quote variants, h4+ -> "p", html entity handling)."""
import json
from _util import ProdClient

c = ProdClient()
sb = c.make_sandbox("create: 05 markdown followups")

CASES = [
    "> quote",            # re-run: 04 showed NOT parsed
    "> quote",            # determinism
    ">  two spaces",
    "> **bold quote**",
    "\n> quote after newline",
    "> line1\n> line2",
    "#### Header four",   # re-run: 04 showed layoutMode "p"
    "##### Header five",
    "###### Header six",
    "####### Header seven",
    "#### **already bold**",
    "***bold italic***",
    "**unterminated bold",
    "*unterminated italic",
    "a * b * c",
    "&amp; entity",
    "&lt;b&gt;not bold&lt;/b&gt;",
    "&quot;q&quot; &#39;a&#39; &nbsp;nb",
    "AT&T",
    "``` ```",
    "```",
    "`unterminated code",
    "~~unterminated strike",
    "[text](not a url)",
    "[text](javascript:alert(1))",
    "[](https://empty.text)",
    "![img](https://example.com/a.png)",
    "- [ ]",
    "- [x]",
    "-  [ ] two spaces",
    "- [ ]no space after",
    "-no space",
    "# ",
    "#",
    "-",
    "- ",
    "    indented four spaces",
    " nbsp leading",
    "​zero width",
    "line\rwith cr",
    "a b line sep",
]

results = []
for name in CASES:
    r = c.create(sb, name, position="bottom")
    if r.status_code != 200:
        rec = {"input": name, "status": r.status_code, "body": r.json() if r.headers.get("content-type","").startswith("application/json") else r.text}
        print(json.dumps(rec, ensure_ascii=False))
        results.append(rec)
        continue
    nid = r.json()["item_id"]
    n = c.get(nid).json()["node"]
    kids = c.list(nid).json()["nodes"]
    rec = {"input": name, "name": n["name"], "data": n["data"], "completed": n["completed"],
           "note": n["note"], "children": [(k["name"], k["data"]) for k in sorted(kids, key=lambda k: k["priority"])]}
    results.append(rec)
    print(json.dumps(rec, ensure_ascii=False))

with open(__file__.replace(".py", ".out.json"), "w") as f:
    json.dump(results, f, indent=1, ensure_ascii=False)
