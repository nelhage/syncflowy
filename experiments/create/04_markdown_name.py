"""Markdown parsing of `name` at create time. One node per input; prints
input -> stored name / layoutMode / completed / note."""
import json
from _util import ProdClient, show

c = ProdClient()
sb = c.make_sandbox("create: 04 markdown in name")

CASES = [
    "plain text",
    "**bold**",
    "*italic*",
    "_underscore italic_",
    "__double underscore__",
    "~~strike~~",
    "`code`",
    "[link text](https://example.com)",
    "<https://example.com>",
    "https://example.com bare url",
    "mixed **bold** and *italic* and `code` here",
    "***bold italic***",
    "**unterminated bold",
    "a * b * c",
    "2 * 3 = 6",
    "snake_case_name",
    "\\*escaped\\*",
    "**`code in bold`**",
    "`**bold in code**`",
    "[2026-01-15]",
    "meeting [2026-01-15] notes",
    "[2026-01-15 14:30]",
    "[2026-13-45]",
    "[2026-01]",
    "# Header one",
    "## Header two",
    "### Header three",
    "#### Header four",
    "#nospace",
    "#tag at start",
    "text with #tag and @person",
    "- bullet",
    "* star bullet",
    "+ plus bullet",
    "1. numbered",
    "- [ ] todo open",
    "- [x] todo done",
    "- [X] todo done caps",
    "[ ] bare checkbox",
    "[x] bare checkbox done",
    "```code fence```",
    "```\nfenced\n```",
    "```python\nprint(1)\n```",
    "> quote",
    ">quote nospace",
    "  leading spaces",
    "trailing spaces   ",
    "<b>raw html bold</b>",
    "<i>i</i> <s>s</s> <code>c</code> <a href=\"https://x.com\">a</a>",
    "<u>underline</u>",
    "<script>alert(1)</script>",
    "a < b > c & d",
    "&amp; entity",
    "emoji \U0001F600 and unicode é中",
    "tab\there",
    "- # header after bullet",
    "# - bullet after header",
    "- [ ] # todo header",
]

results = []
for name in CASES:
    r = c.create(sb, name, position="bottom")
    if r.status_code != 200:
        print("FAIL", repr(name), r.status_code, r.text)
        results.append({"input": name, "status": r.status_code, "body": r.text})
        continue
    nid = r.json()["item_id"]
    n = c.get(nid).json()["node"]
    kids = c.list(nid).json()["nodes"]
    rec = {
        "input": name,
        "name": n["name"],
        "layoutMode": n["data"].get("layoutMode"),
        "data": n["data"],
        "completed": n["completed"],
        "completedAt": n["completedAt"],
        "note": n["note"],
        "children": [k["name"] for k in sorted(kids, key=lambda k: k["priority"])],
    }
    results.append(rec)
    print(json.dumps(rec, ensure_ascii=False))

with open(__file__.replace(".py", ".out.json"), "w") as f:
    json.dump(results, f, indent=1, ensure_ascii=False)
