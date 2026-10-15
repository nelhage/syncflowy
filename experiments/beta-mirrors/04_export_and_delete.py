"""Beta ``nodes-export`` and the remaining delete questions (open-questions R10, R11).

``nodes-export`` returns the whole account, so it is fetched with
``readonly_global=True`` (GET only; authorised for production 2026-10-12, for
the beta host by the user 2026-10-13). Only records inside the scratchpad are
printed; the rest of the account is reduced to counts.

    SYNCFLOWY_TAG=beta-mirrors direnv exec . uv run python experiments/beta-mirrors/04_export_and_delete.py \
        | tee experiments/beta-mirrors/04_export_and_delete.out
"""

from __future__ import annotations

import importlib.util
import json
from pathlib import Path

_spec = importlib.util.spec_from_file_location("probe_common", Path(__file__).with_name("common.py"))
common = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(common)  # type: ignore[union-attr]
b, p, show, ok, tick, section = common.b, common.p, common.show, common.ok, common.tick, common.section

H6_O2 = "b64a9cba-fca8-49de-92c4-0a25f84c980c"  # UI-promoted origin with a stale (empty) mirror registry
H6_O2C = "fbf20163-c8ee-4e01-bfbf-abc22ef70c68"
H6_M1 = "44ea8125-0fce-4c95-88a8-69d74b27b01c"  # its surviving shell
Y2 = "dd75912a-6dbd-426e-ab56-e702db934308"  # orphan made by 02's DELETE /mirror

section("A. fixtures")
sb = b.make_sandbox("beta-mirrors 04")
print("sandbox", sb)
O = ok("create O", b.create(sb, "O"))["item_id"]
oc = ok("create oc", b.create(O, "oc"))["item_id"]
M = ok("beta mirror O -> sb", b.mirror(O, sb, position="bottom"))["item_id"]
h = ok("create h", b.create(sb, "h hidden", position="bottom"))["item_id"]
ok("beta move h -> M (hidden persisted child)", b.move(h, M))
P = ok("create P", b.create(sb, "P", position="bottom"))["item_id"]
MP = ok("prod mirror P -> sb", p.mirror(P, sb, position="bottom"))["item_id"]
ok("prod DELETE P (orphans MP)", p.delete(P))

section("B. nodes-export, both hosts")


def export(client, label: str) -> dict[str, dict]:
    r = client.request("GET", "/api/v1/nodes-export", readonly_global=True)
    body = r.json()
    print(f"--- {label}: {r.status_code}, top-level keys {list(body)}, {len(body['nodes'])} records")
    return {n["id"]: n for n in body["nodes"]}


bx = export(b, "beta export")
px = export(p, "prod export")
print("same id set on both hosts:", set(bx) == set(px))

scratch = b.scratchpad_id()


def in_scratchpad(nid: str, idx: dict[str, dict]) -> bool:
    seen = set()
    while nid and nid not in seen:
        if nid == scratch:
            return True
        seen.add(nid)
        nid = (idx.get(nid) or {}).get("parent_id")
    return False


for label, nid in [("O", O), ("oc", oc), ("M", M), ("h (hidden child of M)", h), ("MP (orphan)", MP),
                   ("Y2 (orphan)", Y2), ("H6 O2", H6_O2), ("H6 M1", H6_M1)]:
    g = b.get(nid).json().get("node")
    rec = bx.get(nid)
    print(f"--- {label}")
    print("    beta export:", json.dumps(rec))
    print("    == beta GET:", rec == g)
    print("    prod export:", json.dumps(px.get(nid)))
print("oc appears once, under O:", sum(1 for n in bx.values() if n["id"] == oc), bx[oc]["parent_id"] == O)
print("records with parent_id == M:", [n["id"] for n in bx.values() if n["parent_id"] == M])
diff = [i for i in bx if in_scratchpad(i, bx) and bx[i] != px.get(i)]
print("scratchpad records that differ between hosts:", len(diff),
      "all shells or origins:", all("mirror" in bx[i]["data"] for i in diff))

section("C. whole-account counts (beta export; no content)")
shells = {i: n for i, n in bx.items() if "origin_id" in n["data"].get("mirror", {})}
origins = {i: n for i, n in bx.items() if "mirror_ids" in n["data"].get("mirror", {})}
orphans = [i for i, n in shells.items() if n["data"]["mirror"]["origin_id"] is None]
stale = [i for i, n in shells.items() if (o := n["data"]["mirror"]["origin_id"]) and i not in
         bx.get(o, {}).get("data", {}).get("mirror", {}).get("mirror_ids", [])]
dangling = [(o, m) for o, n in origins.items() for m in n["data"]["mirror"]["mirror_ids"] if m not in shells]
unsorted = [o for o, n in origins.items() if n["data"]["mirror"]["mirror_ids"] != sorted(n["data"]["mirror"]["mirror_ids"])]
both = [i for i in shells if i in origins]
print(json.dumps({
    "records": len(bx), "shells": len(shells), "orphans": len(orphans), "origins": len(origins),
    "shells missing from their origin's mirror_ids": len(stale),
    "of which in scratchpad": [i for i in stale if in_scratchpad(i, bx)],
    "mirror_ids entries that are not live shells": len(dangling),
    "origins with unsorted mirror_ids": len(unsorted),
    "nodes that are both shell and origin": len(both),
}))

section("D. deletes")
before = ok("prod GET O", p.get(O))["node"]["modifiedAt"]
tick()
show("beta DELETE O (has mirror M)", b.delete(O))
print("blocked delete bumped O:", ok("prod GET O", p.get(O))["node"]["modifiedAt"] != before)
Q = ok("create Q", b.create(sb, "Q", position="bottom"))["item_id"]
MQ = ok("beta mirror Q -> sb", b.mirror(Q, sb, position="bottom"))["item_id"]
show("prod DELETE Q (beta-made mirror; does prod still allow it?)", p.delete(Q))
show("beta GET MQ", b.get(MQ))
holder = ok("create holder", b.create(sb, "holder", position="bottom"))["item_id"]
ok("beta move MP (orphan) -> holder", b.move(MP, holder))
show("beta DELETE holder (holds an orphan shell)", b.delete(holder))
show("beta GET MP", b.get(MP))
show("beta GET H6 O2 before", b.get(H6_O2))
show("beta DELETE H6 O2 (live shell M1, empty registry)", b.delete(H6_O2))
show("beta GET H6 O2 after", b.get(H6_O2))
show("beta GET H6 o2c after", b.get(H6_O2C))
show("beta GET H6 M1 after", b.get(H6_M1))
show("beta list H6 M1 after", b.list(H6_M1))
print("\ndone; sandbox", sb)
