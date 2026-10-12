"""Batch 8 (cloned from 7): pin down how create-path multi-paragraph names are split into
parent + children (contradicts upstream docs: 'a\\n\\nb' does NOT make a child)."""
import json
from common import ProdClient, children, j, RESULTS

CASES = [
    ("l3", "a\nb\nc"),
    ("l4", "a\nb\nc\nd"),
    ("p4", "a\n\nb\n\nc\n\nd"),
    ("l3-blank-mixed", "a\nb\n\n\nc\nd\ne"),
    ("dash-then-plain", "- a\nb"),
    ("dash-then-plain2", "- a\nb\nc"),
    ("h1-then-2plain", "# H\nb\nc"),
    ("plain-h-plain", "a\n\n# H\n\nb\n\nc"),
    ("todo-then-plain", "- [ ] t\nmore"),
    ("code-then-plain", "```\nx\n```\nafter"),
    ("plain-then-code-inline", "a\n```b```"),
    ("star-then-plain", "* a\nb"),
    ("hr-alone-then-plain", "---\nb"),
    ("plain-hr", "a\n---"),
    ("plain-eq", "a\n==="),
    ("bold-multi-line", "**a\nb**"),
    ("bold-across-paras", "**a\n\nb\n\nc**"),
    ("ws-only-line", "a\n   \nb\n \nc"),
    ("child-inline-md", "a\n\n- **b** [2026-01-15] &amp;"),
    ("child-trailing-space", "a\n\n-   b   "),
    ("h4-variants", "a\n\n#### x"),
    ("h4-nospace", "####x"),
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
    sb = c.make_sandbox("text: 08 multiline rules (create name)")
    recs = []
    for label, val in CASES:
        r = c.request("POST", "/api/v1/nodes", json={"parent_id": sb, "name": val, "position": "bottom"})
        recs.append({"label": label, "input": val, "status": r.status_code, "resp": j(r)})
    top = {n["id"]: n for n in children(c, sb)}
    for rec in recs:
        nid = rec["resp"].get("item_id")
        rec["node"] = top.get(nid)
        rec["children"] = subtree(c, nid)
    (RESULTS / "08_multiline_rules.json").write_text(json.dumps({"sandbox": sb, "cases": recs}, ensure_ascii=False, indent=1))

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
