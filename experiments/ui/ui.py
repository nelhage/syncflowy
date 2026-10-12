"""Helper for human-in-the-loop UI scenarios (docs/workflowy/ui-scenarios.md).

    ui.py setup <scenario> '<tree-json>'   create sandbox "ui: <scenario>" with a fixture tree
    ui.py snap <scenario> <label>          snapshot every node in the sandbox subtree
    ui.py diff <scenario> <labelA> <labelB>
    ui.py get <scenario> <name-or-id>...    GET specific nodes (e.g. to see 404s)

Tree JSON: a list of nodes, each either "name" or {"name":..., "note":..., "layoutMode":...,
"children":[...]}. Nodes are created at bottom, in order. Names map to ids in
experiments/ui/state/<scenario>.json; snapshots go to experiments/ui/snaps/<scenario>/.
"""

import json
import sys
from pathlib import Path

from syncflowy.prod_client import ProdClient

HERE = Path(__file__).parent
STATE = HERE / "state"
SNAPS = HERE / "snaps"

FIELDS = ["name", "note", "parent_id", "priority", "completed", "completedAt",
          "data", "createdAt", "modifiedAt"]


def load(scn):
    return json.loads((STATE / f"{scn}.json").read_text())


def save(scn, st):
    STATE.mkdir(exist_ok=True)
    (STATE / f"{scn}.json").write_text(json.dumps(st, indent=1))


def build(c, parent, nodes, ids):
    for n in nodes:
        if isinstance(n, str):
            n = {"name": n}
        extra = {k: n[k] for k in ("note", "layoutMode") if k in n}
        r = c.create(parent, n["name"], position="bottom", **extra)
        r.raise_for_status()
        nid = r.json()["item_id"]
        ids[n.get("key", n["name"])] = nid
        build(c, nid, n.get("children", []), ids)


def setup(c, scn, tree):
    root = c.make_sandbox(f"ui: {scn}")
    ids = {"<ROOT>": root}
    build(c, root, json.loads(tree), ids)
    save(scn, {"root": root, "ids": ids})
    print(f"sandbox: https://workflowy.com/#/{root.replace('-', '')[-12:]}")
    for k, v in ids.items():
        print(f"  {k}: {v}")


def walk(c, root):
    r = c.get(root)
    r.raise_for_status()
    out = {root: r.json()["node"]}
    order = {}
    stack = [root]
    while stack:
        p = stack.pop()
        r = c.list(p)
        r.raise_for_status()
        kids = r.json()["nodes"]
        order[p] = [k["id"] for k in kids]
        for k in kids:
            if k["id"] in out:  # cycle guard
                continue
            out[k["id"]] = k
            stack.append(k["id"])
    return {"nodes": out, "list_order": order}


def snap(c, scn, label):
    st = load(scn)
    s = walk(c, st["root"])
    d = SNAPS / scn
    d.mkdir(parents=True, exist_ok=True)
    (d / f"{label}.json").write_text(json.dumps(s, indent=1))
    names = name_map(st, s)
    print(f"snapshot {label}: {len(s['nodes'])} nodes")
    show_tree(s, st["root"], names)


def name_map(st, *snaps):
    m = {v: k for k, v in st["ids"].items()}
    for s in snaps:
        for i, n in s["nodes"].items():
            m.setdefault(i, f"[{(n['name'] or '')[:30]}]~{i[-4:]}")
    return m


def show_tree(s, root, names, depth=0):
    nodes = s["nodes"]
    kids = sorted(s["list_order"].get(root, []), key=lambda i: nodes[i]["priority"])
    for k in kids:
        n = nodes[k]
        flags = []
        if n["completed"]:
            flags.append("done")
        if n["data"].get("layoutMode") != "bullets":
            flags.append(n["data"].get("layoutMode"))
        print(f"  {'  ' * depth}- {names[k]} p={n['priority']} mod={n['modifiedAt']}"
              f"{' ' + ','.join(map(str, flags)) if flags else ''}  name={n['name']!r}"
              f"{'  note=' + repr(n['note']) if n['note'] is not None else ''}")
        show_tree(s, k, names, depth + 1)


def diff(scn, a, b):
    st = load(scn)
    A = json.loads((SNAPS / scn / f"{a}.json").read_text())
    B = json.loads((SNAPS / scn / f"{b}.json").read_text())
    names = name_map(st, A, B)
    an, bn = A["nodes"], B["nodes"]
    for i in an.keys() - bn.keys():
        print(f"REMOVED {names[i]} ({i})")
    for i in bn.keys() - an.keys():
        print(f"ADDED   {names[i]} ({i}): " + json.dumps({k: bn[i].get(k) for k in FIELDS}))
    for i in an.keys() & bn.keys():
        ch = {k: (an[i].get(k), bn[i].get(k)) for k in FIELDS if an[i].get(k) != bn[i].get(k)}
        extra = set(an[i]) ^ set(bn[i])
        if ch or extra:
            print(f"CHANGED {names[i]}: " + ", ".join(f"{k}: {x!r} -> {y!r}" for k, (x, y) in ch.items())
                  + (f" keys±{extra}" if extra else ""))
    for p in set(A["list_order"]) | set(B["list_order"]):
        oa, ob = A["list_order"].get(p), B["list_order"].get(p)
        if oa is not None and ob is not None and oa != ob:
            print(f"LIST ORDER {names.get(p, p)}: {[names[x] for x in oa]} -> {[names[x] for x in ob]}")


def get(c, scn, keys):
    st = load(scn)
    for k in keys:
        i = st["ids"].get(k, k)
        r = c.get(i)
        print(k, r.status_code, json.dumps(r.json() if r.headers.get("content-type", "").startswith("application/json") else r.text[:200]))


if __name__ == "__main__":
    cmd, scn, *rest = sys.argv[1:]
    if cmd == "diff":
        diff(scn, *rest)
    else:
        c = ProdClient()
        {"setup": lambda: setup(c, scn, rest[0]),
         "snap": lambda: snap(c, scn, rest[0]),
         "get": lambda: get(c, scn, rest)}[cmd]()
