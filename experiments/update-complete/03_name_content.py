"""How name/note content is transformed on update (and, for contrast, on create).

For each input string: update an existing node's name (and separately note) to
that string, then GET and compare. Prints a table of input -> stored output.
"""

import json
import sys

from _common import c, show, mk, node, children

CASES = [
    "plain text",
    "**bold md**",
    "*italic md*",
    "~~strike md~~",
    "`code md`",
    "[link md](https://example.com)",
    "# heading md",
    "- [ ] todo md",
    "- [x] done md",
    "> quote md",
    "```code block```",
    "[2026-01-15]",
    "<b>bold</b>",
    "<i>it</i> <s>s</s> <code>c</code>",
    '<a href="https://example.com">link</a>',
    "<u>underline</u>",
    "<strong>strong</strong>",
    "<em>em</em>",
    "<span class=\"colored c-red\">span</span>",
    "<div>div</div>",
    "<script>alert(1)</script>",
    "<br>",
    "a < b",
    "a > b",
    "a & b",
    "a &amp; b",
    "a &lt; b &gt; c",
    "&nbsp;x&#39;y&quot;z",
    "<b>unclosed",
    "</b>stray close",
    "<B>upper</B>",
    "<b class=\"x\">attr</b>",
    "<a href='https://example.com'>sq</a>",
    "<a href=\"javascript:alert(1)\">js</a>",
    "it's \"quoted\"",
    "line1\nline2",
    "para1\n\npara2",
    "trailing space   ",
    "   leading space",
    "tab\there",
    "emoji \U0001F600 é",
    "#tag @person",
    "x" * 5000,
]

which = sys.argv[1] if len(sys.argv) > 1 else "name"

sb = c.make_sandbox(f"update-complete: 03 {which} content transforms (update)")
print("sandbox", sb)
n = mk(sb, "subject")

rows = []
for inp in CASES:
    r = c.update(n, **{which: inp})
    status = r.status_code
    got = node(n)
    out = got[which] if got else None
    kids = children(n) if ("\n" in inp) else []
    rows.append({
        "in": inp if len(inp) < 100 else f"x*{len(inp)}",
        "status": status,
        "resp": r.json() if status != 200 else "ok",
        "out": out if (out is None or len(out) < 200) else f"<len {len(out)}>",
        "layout": got["data"]["layoutMode"] if got else None,
        "completed": got["completed"] if got else None,
        "children": [k["name"] for k in kids],
    })
    print(json.dumps(rows[-1], ensure_ascii=False))
    # reset layout/completion if a markdown prefix changed it
    if got and (got["data"]["layoutMode"] != "bullets" or got["completed"]):
        c.update(n, layoutMode="bullets")
        c.uncomplete(n)
