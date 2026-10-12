"""Batch 6: newlines and block-level markdown in create-path name; newlines in
notes and in update-path name/note. Records the full subtree created."""
import json
from common import ProdClient, children, j, RESULTS

CREATE_NAME = [
    ("nl-single", "a\nb"),
    ("nl-double", "a\n\nb"),
    ("nl-double-3", "a\n\nb\n\nc"),
    ("nl-quad", "a\n\n\n\nb"),
    ("crlf-single", "a\r\nb"),
    ("crlf-double", "a\r\n\r\nb"),
    ("cr-only", "a\rb"),
    ("nl-leading", "\n\na"),
    ("nl-trailing", "a\n\n"),
    ("hard-break", "a  \nb"),
    ("h1", "# H1"),
    ("h2", "## H2"),
    ("h3", "### H3"),
    ("h4", "#### H4"),
    ("h-nospace", "#nospace"),
    ("h1-inline-md", "# **bold** head"),
    ("dash", "- item"),
    ("star-bullet", "* item"),
    ("plus-bullet", "+ item"),
    ("ordered", "1. item"),
    ("todo", "- [ ] todo"),
    ("done", "- [x] done"),
    ("done-upper", "- [X] done"),
    ("todo-nodash", "[ ] todo"),
    ("quote", "> quote"),
    ("code-inline-fence", "```code```"),
    ("code-fence-ml", "```\nline1\nline2\n```"),
    ("code-fence-lang", "```py\nx = 1\n```"),
    ("code-fence-md", "```\n**not bold**\n```"),
    ("indented-heading", "  # indented"),
    ("heading-then-body", "# Head\n\nbody **b**"),
    ("list-items", "- one\n- two\n- three"),
    ("list-items-dbl", "- one\n\n- two"),
    ("nested-list", "parent\n\n  - child\n\n    - grandchild"),
    ("tab-nested", "parent\n\n\tchild"),
]
CREATE_NOTE = [
    ("note-nl", "line1\nline2"),
    ("note-dbl", "para1\n\npara2"),
    ("note-crlf", "a\r\nb"),
    ("note-md-blocks", "# H\n- item\n**b**"),
    ("note-trailing-nl", "a\n"),
    ("note-leading-nl", "\na"),
]
UPDATE = [
    ("upd-name-nl", {"name": "a\nb"}),
    ("upd-name-dbl", {"name": "a\n\nb"}),
    ("upd-name-crlf", {"name": "a\r\nb"}),
    ("upd-name-h1", {"name": "# H1"}),
    ("upd-name-todo", {"name": "- [ ] todo"}),
    ("upd-note-nl", {"note": "a\nb"}),
    ("upd-note-crlf", {"note": "a\r\nb"}),
    ("upd-note-cr", {"note": "a\rb"}),
]


def subtree(c, nid, depth=0):
    if depth > 3:
        return []
    kids = children(c, nid)
    for k in kids:
        k["_children"] = subtree(c, k["id"], depth + 1)
    return kids


def main():
    c = ProdClient()
    out = {"create_name": [], "create_note": [], "update": []}
    sb = c.make_sandbox("text: 06 multiline/blocks (create name)")
    sbn = c.make_sandbox("text: 06 multiline (create note)")
    sbu = c.make_sandbox("text: 06 multiline (update)")
    ids = []
    for label, val in CREATE_NAME:
        r = c.request("POST", "/api/v1/nodes", json={"parent_id": sb, "name": val, "position": "bottom"})
        ids.append((label, val, r.status_code, j(r)))
    for label, val in CREATE_NOTE:
        r = c.request("POST", "/api/v1/nodes", json={"parent_id": sbn, "name": label, "note": val, "position": "bottom"})
        out["create_note"].append({"label": label, "input": val, "status": r.status_code, "resp": j(r)})
    for label, body in UPDATE:
        r = c.create(sbu, "u", position="bottom")
        uid = r.json()["item_id"]
        r = c.update(uid, **body)
        out["update"].append({"label": label, "input": body, "id": uid, "status": r.status_code, "resp": j(r)})
    top = {n["id"]: n for n in children(c, sb)}
    for label, val, st, resp in ids:
        rec = {"label": label, "input": val, "status": st, "resp": resp}
        nid = resp.get("item_id") if isinstance(resp, dict) else None
        if nid:
            rec["node"] = top.get(nid)
            rec["children"] = subtree(c, nid)
        out["create_name"].append(rec)
    out["create_name_sandbox_children"] = list(top.values())
    notes = {n["id"]: n for n in children(c, sbn)}
    for rec in out["create_note"]:
        rec["node"] = notes.get(rec["resp"].get("item_id"))
    upd = {n["id"]: n for n in children(c, sbu)}
    for rec in out["update"]:
        rec["node"] = upd.get(rec["id"])
    (RESULTS / "06_multiline_blocks.json").write_text(json.dumps(out, ensure_ascii=False, indent=1))

    def fmt(n, ind=""):
        s = f"{ind}name={n['name']!r} note={n['note']!r} layout={n['data'].get('layoutMode')} completed={n['completed']}"
        for k in n.get("_children", []):
            s += "\n" + fmt(k, ind + "      ")
        return s
    for rec in out["create_name"]:
        print(f"== {rec['label']} input={rec['input']!r} status={rec['status']} resp={rec['resp']}")
        if rec.get("node"):
            print("   " + fmt(rec["node"]))
            for k in rec["children"]:
                print(fmt(k, "      "))
    print("sandbox top-level count:", len(top), "cases:", len(ids))
    for rec in out["create_note"]:
        print(f"== {rec['label']} input={rec['input']!r} -> note={rec['node'] and rec['node']['note']!r}")
    for rec in out["update"]:
        n = rec["node"]
        print(f"== {rec['label']} input={rec['input']!r} status={rec['status']} {rec['resp']} -> name={n['name']!r} note={n['note']!r} layout={n['data']}")


main()
