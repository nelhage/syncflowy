"""followup-2 / exp01: validation-error shapes, model names, parent_id
requiredness, and not-found messages for URL-form parent_ids.

Safety: every request either (a) targets a random UUID path id, so nothing can
execute, or (b) fails body validation (name: 1) so nothing executes, or (c)
uses a parent_id that cannot exist (random UUID / random 12-hex / URL around
one) with a path id that is a node we just created inside our own sandbox.
`unscoped=True` is used only because the guard refuses parent ids that are not
on the allow-list; in all such requests the *probed* id cannot exist.

Run:
  SYNCFLOWY_TAG=followup-2 direnv exec . uv run python \
      experiments/followup-2/01_validation_and_parent_resolution.py
"""
from __future__ import annotations

import json
import secrets
import sys
import uuid
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "src"))
from syncflowy.prod_client import ProdClient  # noqa: E402

c = ProdClient()
OUT = []


def rnd_uuid() -> str:
    return str(uuid.uuid4())


def rnd_short() -> str:
    return secrets.token_hex(6)


def rec(label, r, sent=None):
    d = {
        "label": label,
        "req": f"{r.request.method} {r.request.url}",
        "sent": sent,
        "status": r.status_code,
        "ctype": r.headers.get("content-type"),
        "body": r.text if len(r.text) < 2000 else r.text[:2000] + "...",
    }
    print(json.dumps(d, ensure_ascii=False))
    OUT.append(d)
    return r


def post(label, path, body, raw=False):
    if raw:
        r = c.request("POST", path, content=body,
                      headers={"Content-Type": "application/json"}, unscoped=True)
    else:
        r = c.request("POST", path, json=body, unscoped=True)
    return rec(label, r, body)


def get_list(label, parent):
    return rec(label, c.request("GET", "/api/v1/nodes", params={"parent_id": parent},
                                unscoped=True), {"parent_id": parent})


# ---------------------------------------------------------------- A. update multi-error
R = rnd_uuid()
U = f"/api/v1/nodes/{R}"
post("A1 update name,note,layoutMode all bad", U, {"name": 1, "note": 2, "layoutMode": 3})
post("A2 same, reversed key order", U, {"layoutMode": 3, "note": 2, "name": 1})
post("A3 update name+layoutMode bad", U, {"name": [1], "layoutMode": {"a": 1}})
post("A4 update bad parent_id/position/completed (ignored fields?)", U,
     {"parent_id": 1, "position": "q", "completed": "x", "priority": "z"})
post("A5 update name bad + bad position", U, {"name": 1, "position": "q"})
post("A1-repeat", U, {"name": 1, "note": 2, "layoutMode": 3})

# ---------------------------------------------------------------- B. model names
for ep in ("move", "mirror"):
    post(f"B {ep} body [1]", f"/api/v1/nodes/{rnd_uuid()}/{ep}", "[1]", raw=True)
    post(f"B {ep} body \"s\"", f"/api/v1/nodes/{rnd_uuid()}/{ep}", '"s"', raw=True)
    post(f"B {ep} body null", f"/api/v1/nodes/{rnd_uuid()}/{ep}", "null", raw=True)
    post(f"B {ep} empty body", f"/api/v1/nodes/{rnd_uuid()}/{ep}", "", raw=True)
post("B create body [1] (recheck)", "/api/v1/nodes", "[1]", raw=True)
post("B update body [1] (recheck)", U, "[1]", raw=True)

# ---------------------------------------------------------------- C. parent_id requiredness
for ep in ("move", "mirror"):
    P = f"/api/v1/nodes/{rnd_uuid()}/{ep}"
    post(f"C {ep} {{}}", P, {})
    post(f"C {ep} parent_id null", P, {"parent_id": None})
    post(f"C {ep} parent_id ''", P, {"parent_id": ""})
    post(f"C {ep} parent_id 1", P, {"parent_id": 1})
    post(f"C {ep} parent_id 1 + position q", P, {"parent_id": 1, "position": "q"})
    post(f"C {ep} position q + parent_id 1 (key order swapped)", P, {"position": "q", "parent_id": 1})
    post(f"C {ep} {{}} + position q", P, {"position": "q"})
    post(f"C {ep} parent_id [x]", P, {"parent_id": ["x"]})
N = "/api/v1/nodes"
post("C create {name:1} (parent_id omitted)", N, {"name": 1})
post("C create {name:1, parent_id:null}", N, {"name": 1, "parent_id": None})
post("C create {name:1, parent_id:''}", N, {"name": 1, "parent_id": ""})
post("C create {parent_id:1, name:1}", N, {"parent_id": 1, "name": 1})
post("C create {name:1, parent_id:1} (key order swapped)", N, {"name": 1, "parent_id": 1})
post("C create {parent_id:1} (name missing)", N, {"parent_id": 1})
post("C create {} ", N, {})
post("C create all bad incl parent", N,
     {"position": "q", "layoutMode": 3, "note": 2, "name": 1, "parent_id": 1})

# ---------------------------------------------------------------- D. URL / short-id parent misses
sb = c.make_sandbox("followup-2: parent_id resolution misses")
# a real node to move (path id); its parent ids below can never exist
mv = c.create(sb, "mover (should never actually move)").json()["item_id"]
rec("D sandbox list (before)", c.list(sb))

rs = rnd_short()
ru = rnd_uuid()
variants = {
    "url-random-short": f"https://workflowy.com/#/{rs}",
    "bare-random-short": rs,
    "url-random-full-uuid": f"https://workflowy.com/#/{ru}",
    "url-random-uuid-nodash": f"https://workflowy.com/#/{ru.replace('-', '')}",
    "random-uuid-UPPER": ru.upper(),
    "random-uuid-nodash": ru.replace("-", ""),
    "url-random-short-UPPER": f"https://workflowy.com/#/{rs.upper()}",
}
for k, v in variants.items():
    get_list(f"D list {k}", v)
    post(f"D move-real {k}", f"/api/v1/nodes/{mv}/move", {"parent_id": v})
    post(f"D create {k}", N, {"parent_id": v, "name": "should not exist"})
    post(f"D mirror-real {k}", f"/api/v1/nodes/{mv}/mirror", {"parent_id": v})
# determinism repeat with fresh random short
rs2 = rnd_short()
get_list("D list url-random-short #2", f"https://workflowy.com/#/{rs2}")
post("D create url-random-short #2", N, {"parent_id": f"https://workflowy.com/#/{rs2}", "name": "x"})
post("D move-real url-random-short #2", f"/api/v1/nodes/{mv}/move",
     {"parent_id": f"https://workflowy.com/#/{rs2}"})
get_list("D list bare-random-short #2", rs2)
# URL that embeds the full UUID of our own sandbox (read-only list).
full_url = f"https://workflowy.com/#/{sb}"
c.allow.ids()  # no-op; list of a URL embedding a known id is checked client-side:
assert sb in full_url
get_list("D list url-embedding-real-full-uuid (sandbox, read-only)", full_url)

rec("D sandbox list (after; mover must still be here)", c.list(sb))
rec("D mover GET (after)", c.get(mv))

with open(Path(__file__).with_suffix(".out.json"), "w") as f:
    json.dump(OUT, f, indent=1, ensure_ascii=False)
