"""Generate rich_tree.json: an export-shaped (`{"nodes": [...]}`, DM§4.10) fixture for
the mdblock snapshot tests.

Run from anywhere: `python3 make_fixture.py` rewrites rich_tree.json next to it. Ids are
deterministic: node number n gets the UUID `00000000-0000-4000-8000-<n as 12 hex>`, so
its short id is the 12-hex number itself (`000000000007` for node 7).

The tree covers: a root note (the preamble), completed subtrees (hidden with incomplete
descendants), notes with empty lines, every layout kind (todo, code-block, quote-block,
numbered, divider, h1, p, board), mirror shells (`note ""` and `note null`, UI:H2) with
vestigial children, tier-3 values (malformed WF-HTML, a newline in a name, a code line
that would close the fence), a lossy value, text that needs line-start escapes, links,
dates, passthrough HTML, and nesting six levels deep.
"""

import json
import os

nodes = []
counter = [0]


def uid(n):
    return f"00000000-0000-4000-8000-{n:012x}"


def node(parent, name, note=None, layout="bullets", completed=False, children=()):
    counter[0] += 1
    n = counter[0]
    rec = {
        "id": uid(n),
        "name": name,
        "note": note,
        "parent_id": uid(parent) if parent is not None else None,
        "priority": 0,
        "completed": completed,
        "data": {"layoutMode": layout},
        "createdAt": 1791834396 + n,
        "modifiedAt": 1791834396 + n,
        "completedAt": 1791834400 + n if completed else None,
    }
    nodes.append(rec)
    for i, c in enumerate(children):
        c(n, i)
    return n


def child(name, note=None, layout="bullets", completed=False, children=()):
    def make(parent, i):
        n = node(parent, name, note, layout, completed, children)
        for rec in nodes:
            if rec["id"] == uid(n):
                rec["priority"] = 100 * (i + 1)

    return make


DATE = '<time startYear="2026" startMonth="1" startDay="15">Thu, Jan 15, 2026</time>'
DATETIME = (
    '<time startYear="2026" startMonth="1" startDay="15" startHour="14" '
    'startMinute="30">Thu, Jan 15, 2026 at 2:30 PM</time>'
)

node(
    None,
    "Projects",
    "Free text here is the sync root's note.\nIt has <b>two</b> lines &amp; an ampersand.",
    children=[
        child(
            "Planning",
            "Note on &quot;Planning&quot;: lines right under the heading.",
            children=[
                child(
                    "Draft the <i>spec</i>",
                    "A note line.\n\nAfter an empty line.\n- not a child, a note line",
                    children=[
                        child("Write the mock", layout="todo"),
                        child(
                            "Run the UI scenarios",
                            completed=True,
                            children=[child("hidden incomplete descendant")],
                        ),
                        child(
                            "Deep",
                            children=[
                                child(
                                    "Deeper",
                                    children=[
                                        child(
                                            "Deeper still",
                                            children=[
                                                child(
                                                    "Nearly there",
                                                    children=[
                                                        child("Bottom <code>x = 1</code>")
                                                    ],
                                                )
                                            ],
                                        )
                                    ],
                                )
                            ],
                        ),
                    ],
                ),
                child(
                    "Numbered steps",
                    layout="numbered",
                    children=[
                        child("first"),
                        child("second (done)", completed=True),
                        child("third", note="with a note"),
                    ],
                ),
                child("Board layout is kept as a plain item", layout="board"),
                child("Paragraph layout", layout="p"),
                child("An h1 inside the outline", layout="h1"),
            ],
        ),
        child(
            "fn main() {\n    println!(&quot;hi &amp; bye&quot;);\n}",
            note="runs on save",
            layout="code-block",
        ),
        child("To be or not to be", layout="quote-block"),
        child("", note="", layout="divider"),
        child("", note="", children=[child("vestigial child of an API shell")]),
        child("", note=None),
        child("broken <b>bold", note="a malformed name (UI:F1)"),
        child("line one\nline two"),
        child("x\n```\ny", layout="code-block"),
        child("<b>bold</b> and <foo>unknown</foo> markup"),
        child(
            'See <a href="https://example.com/a?b=1&c=2">https://example.com/a?b=1&amp;c=2</a>'
            f" on {DATE} and at {DATETIME}"
        ),
        child(
            '<u>underlined</u>, <span class="colored c-red">red</span>, '
            '<a href="https://example.com">a link</a> and #tag @who'
        ),
        child("1. not a list"),
        child("- not a child"),
        child("# not a heading"),
        child("[ ] not a box"),
        child("a ^wf-8e1f00aa2b3c in the text and %% percents"),
        child(
            "Done stuff",
            completed=True,
            children=[child("incomplete under completed"), child("also hidden")],
        ),
        child(
            "A todo with children",
            layout="todo",
            children=[child("leaf one"), child("leaf two", note="  indented note line")],
        ),
        child("Trailing empty note lines", note="first\n\n"),
        child("Leading empty note line", note="\nsecond"),
    ],
)

with open(os.path.join(os.path.dirname(os.path.abspath(__file__)), "rich_tree.json"), "w") as f:
    json.dump({"nodes": nodes}, f, indent=1, ensure_ascii=False)
    f.write("\n")
