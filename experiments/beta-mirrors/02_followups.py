"""Follow-ups to 01_probe: mirror_ids order, orphans, parent_id forms, blocked shell deletes.

    SYNCFLOWY_TAG=beta-mirrors direnv exec . uv run python experiments/beta-mirrors/02_followups.py \
        | tee experiments/beta-mirrors/02_followups.out
"""

from __future__ import annotations

import importlib.util
from pathlib import Path

_spec = importlib.util.spec_from_file_location("probe_common", Path(__file__).with_name("common.py"))
common = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(common)  # type: ignore[union-attr]
b, p, show, ok, tick, section = common.b, common.p, common.show, common.ok, common.tick, common.section

section("A. mirror_ids order")
sb = b.make_sandbox("beta-mirrors 02")
print("sandbox", sb)
S = ok("create S", b.create(sb, "S"))["item_id"]
made = []
for i in range(5):
    made.append(ok(f"beta mirror S -> sb #{i}", b.mirror(S, sb, position="bottom"))["item_id"])
ids = ok("beta GET S", b.get(S))["node"]["data"]["mirror"]["mirror_ids"]
print("creation order:", made)
print("mirror_ids    :", ids)
print("sorted?", ids == sorted(made), "creation?", ids == made, "reverse?", ids == made[::-1])

section("B. parent_id forms naming a shell, on create")
T = ok("create T", b.create(sb, "T", position="bottom"))["item_id"]
MT = ok("beta mirror T -> sb", b.mirror(T, sb, position="bottom"))["item_id"]
short = MT.replace("-", "")[-12:]
c1 = ok("beta create parent=short id of shell", b.create(short, "via short"))["item_id"]
show("beta GET c1", b.get(c1))
c2 = ok("beta create parent=URL of shell", b.create(f"https://workflowy.com/#/{short}", "via url"))["item_id"]
show("beta GET c2", b.get(c2))
show("beta list MT", b.list(MT))
show("beta list MT by short id", b.list(short))

section("C. persisted children of a shell")
y = ok("create y", b.create(sb, "y", position="bottom"))["item_id"]
ok("beta move y -> MT", b.move(y, MT))
show("beta GET y", b.get(y))
show("beta list MT (y?)", b.list(MT))
show("beta list y's parent via prod (raw)", p.list(MT))
Y2 = ok("beta mirror y -> sb (y is a persisted child of a shell)", b.mirror(y, sb, position="bottom"))["item_id"]
show("beta DELETE MT (persisted child y has mirrors)", b.delete(MT))
show("beta DELETE /mirror MT", b.request("DELETE", f"/api/v1/nodes/{MT}/mirror"))
show("beta GET MT", b.get(MT))

section("D. orphans")
U = ok("create U", b.create(sb, "U", position="bottom"))["item_id"]
MU = ok("prod mirror U -> sb", p.mirror(U, sb, position="bottom"))["item_id"]
ok("prod create own child under MU", p.create(MU, "own child of MU"))
ok("prod DELETE U", p.delete(U))
show("beta list MU (orphan)", b.list(MU))
show("beta complete MU", b.complete(MU))
show("beta uncomplete MU", b.uncomplete(MU))
dest = ok("create dest", b.create(sb, "dest", position="bottom"))["item_id"]
show("beta move MU -> dest", b.move(MU, dest))
z = ok("create z", b.create(sb, "z", position="bottom"))["item_id"]
show("beta move z -> MU", b.move(z, MU))
show("beta mirror MU -> sb (orphan source)", b.mirror(MU, sb))
show("beta mirror z -> MU (orphan parent)", b.mirror(z, MU))
show("beta DELETE /mirror MU", b.request("DELETE", f"/api/v1/nodes/{MU}/mirror"))
show("beta GET MU", b.get(MU))

section("E. second orphan: plain delete")
V = ok("create V", b.create(sb, "V", position="bottom"))["item_id"]
MV = ok("prod mirror V -> sb", p.mirror(V, sb, position="bottom"))["item_id"]
ok("prod DELETE V", p.delete(V))
show("beta DELETE MV (orphan)", b.delete(MV))
show("beta GET MV", b.get(MV))
print("\ndone; sandbox", sb)
