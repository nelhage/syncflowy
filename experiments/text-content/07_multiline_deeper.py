"""Batch 7: pin down how create-path multi-paragraph names are split into
parent + children (contradicts upstream docs: 'a\\n\\nb' does NOT make a child)."""
import json
from common import ProdClient, children, j, RESULTS

CASES = [
    ("p5", "a\n\nb\n\nc\n\nd\n\ne"),
    ("p3-repeat", "a\n\nb\n\nc"),           # determinism re-check of batch 6
    ("p2-repeat", "a\n\nb"),
    ("nl-then-p", "a\nb\n\nc"),
    ("p-then-nl", "a\n\nb\nc"),
    ("p-list", "a\n\n- b"),
    ("p-list2", "a\n\n- b\n\n- c"),
    ("p-list-single-nl", "a\n- b\n- c"),
    ("p-h1", "a\n\n# b"),
    ("h1-h2", "# A\n\n## B\n\n### C"),
    ("p-todo", "a\n\n- [ ] t\n\n- [x] d"),
    ("p-code", "a\n\n```\ncode\n```"),
    ("p-quote", "a\n\n> q"),
    ("list-dash-3-dbl", "- one\n\n- two\n\n- three"),
    ("h4-repeat", "#### H4"),
    ("h5", "##### H5"),
    ("h6", "###### H6"),
    ("h7", "####### H7"),
    ("quote-repeat", "> quote"),
    ("quote-nospace", ">quote"),
    ("star-list-multi", "* a\n* b"),
    ("ordered-multi", "1. a\n2. b"),
    ("nested-dash-indent", "- a\n  - b\n    - c"),
    ("p-indented-para", "a\n\n    indented"),
    ("hr-between", "a\n\n---\n\nb"),
]


def subtree(c, nid, depth=0):
    if depth > 3:
        return []
    kids = children(c, nid)
    for k in kids:
        k["_children"] = subtree(c, k["id"], depth + 1) if depth < 2 else []
    return kids


def main():
    c = ProdClient()
    sb = c.make_sandbox("text: 07 multiline deeper (create name)")
    recs = []
    for label, val in CASES:
        r = c.request("POST", "/api/v1/nodes", json={"parent_id": sb, "name": val, "position": "bottom"})
        recs.append({"label": label, "input": val, "status": r.status_code, "resp": j(r)})
    top = {n["id"]: n for n in children(c, sb)}
    for rec in recs:
        nid = rec["resp"].get("item_id")
        rec["node"] = top.get(nid)
        rec["children"] = subtree(c, nid)
    (RESULTS / "07_multiline_deeper.json").write_text(json.dumps({"sandbox": sb, "cases": recs}, ensure_ascii=False, indent=1))

    def fmt(n, ind=""):
        s = f"{ind}name={n['name']!r} note={n['note']!r} layout={n['data'].get('layoutMode')} completed={n['completed']} prio={n['priority']}"
        for k in n.get("_children", []):
            s += "\n" + fmt(k, ind + "      ")
        return s
    for rec in recs:
        print(f"== {rec['label']} input={rec['input']!r} status={rec['status']}")
        print("   " + fmt(rec["node"]))
        for k in rec["children"]:
            print(fmt(k, "      "))
    print("sandbox top-level count:", len(top), "cases:", len(recs))


main()
