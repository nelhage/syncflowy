"""Contrast: the same strings via CREATE (which docs say parses markdown) vs
UPDATE (observed verbatim in 03). Also probe max name/note size on update."""

import json

from _common import c, show, mk, node, children

sb = c.make_sandbox("update-complete: 04 create-vs-update contrast, size limits")
print("sandbox", sb)

CASES = [
    "**bold md**",
    "a < b & c > d",
    "a &amp; b",
    "<u>underline</u>",
    "<b>bold</b>",
    "# heading md",
    "- [x] done md",
    "line1\nline2",
]
for inp in CASES:
    r = c.create(sb, inp, position="bottom")
    nid = r.json()["item_id"]
    got = node(nid)
    kids = children(nid) if "\n" in inp else []
    print(json.dumps({"in": inp, "out": got["name"], "layout": got["data"]["layoutMode"],
                      "completed": got["completed"], "children": [k["name"] for k in kids]},
                     ensure_ascii=False))

n = mk(sb, "size subject")
for size in (10_000, 100_000, 1_000_000):
    for field in ("name", "note"):
        r = c.update(n, **{field: "y" * size})
        body = r.json() if r.headers.get("content-type", "").startswith("application/json") else r.text[:300]
        got = node(n)
        print(json.dumps({"field": field, "size": size, "status": r.status_code, "resp": body,
                          "stored_len": len(got[field] or "")}))
c.update(n, name="size subject", note="")
