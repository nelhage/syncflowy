"""Probe the beta API's mirror-aware semantics (data-model.md §10.5, open-questions R1-R9).

Runs against https://beta.workflowy.com (``b``) inside a fresh scratchpad
sandbox, using production (``p``, same account) to read the raw rows: a
production GET of a shell shows the shell's own fields, which beta hides.

    SYNCFLOWY_TAG=beta-mirrors direnv exec . uv run python experiments/beta-mirrors/01_probe.py \
        | tee experiments/beta-mirrors/01_probe.out
"""

from __future__ import annotations

import importlib.util
from pathlib import Path

_spec = importlib.util.spec_from_file_location("probe_common", Path(__file__).with_name("common.py"))
common = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(common)  # type: ignore[union-attr]
b, p, show, ok, tick, section = common.b, common.p, common.show, common.ok, common.tick, common.section


section("A. setup and shape")
sb = b.make_sandbox("beta-mirrors 01")
print("sandbox", sb)
O = ok("create O", b.create(sb, "O origin", note="o note", layoutMode="todo"))["item_id"]
oc = ok("create oc", b.create(O, "oc"))["item_id"]
dest = ok("create dest", b.create(sb, "dest", position="bottom"))["item_id"]
tick()
M = ok("beta mirror O -> dest", b.mirror(O, dest))["item_id"]
show("beta GET O", b.get(O))
show("beta GET M", b.get(M))
show("prod GET M (raw shell row)", p.get(M))
show("prod GET O", p.get(O))
show("beta GET M by short id", b.get(M.replace("-", "")[-12:]))
show("beta list dest", b.list(dest))
show("beta list M", b.list(M))
show("beta list O", b.list(O))
show("beta list sb", b.list(sb))

section("B. second mirror: mirror_ids order; mirror of a mirror; shell as parent_id")
tick()
M2 = ok("beta mirror M (a mirror) -> sb bottom", b.mirror(M, sb, position="bottom"))["item_id"]
show("beta GET O", b.get(O))
dest3 = ok("create dest3", b.create(sb, "dest3", position="bottom"))["item_id"]
P0 = ok("create P0", b.create(sb, "P0", position="bottom"))["item_id"]
MP0 = ok("beta mirror P0 -> M (shell as parent)", b.mirror(P0, M))["item_id"]
show("beta GET MP0", b.get(MP0))
show("beta list O", b.list(O))
show("beta mirror O -> M (shell of O as parent)", b.mirror(O, M))
show("beta mirror O -> O", b.mirror(O, O))

section("C. writes through M")
tick()
show("prod GET O before", p.get(O))
show("prod GET M before", p.get(M))
ok("beta update M", b.update(M, name="renamed via M", note="n2", layoutMode="h1"))
show("beta GET O", b.get(O))
show("beta GET M", b.get(M))
show("prod GET M (raw)", p.get(M))
tick()
ok("beta complete M", b.complete(M))
show("beta GET O", b.get(O))
show("prod GET M (raw)", p.get(M))
tick()
ok("beta uncomplete M", b.uncomplete(M))
show("beta GET O", b.get(O))
tick()
c1 = ok("beta create under M (bottom)", b.create(M, "child via M", position="bottom"))["item_id"]
show("beta GET c1", b.get(c1))
show("beta list O", b.list(O))
show("beta list M", b.list(M))
show("prod GET O (bumped by create?)", p.get(O))
show("prod GET M (raw)", p.get(M))

section("D. move into M, and move of M")
x = ok("create x", b.create(sb, "x", position="bottom"))["item_id"]
tick()
ok("beta move x -> M", b.move(x, M, position="bottom"))
show("beta GET x", b.get(x))
show("beta list M", b.list(M))
show("beta list O", b.list(O))
show("prod GET M (raw, bumped as destination?)", p.get(M))
show("beta GET M", b.get(M))
tick()
ok("beta move M -> sb bottom", b.move(M, sb, position="bottom"))
show("beta GET M", b.get(M))
show("beta GET O", b.get(O))
show("prod GET O", p.get(O))

section("E. deletes")
show("beta DELETE O (has mirrors)", b.delete(O))
show("beta GET O", b.get(O))
show("beta list dest (M moved out)", b.list(dest))
O2 = ok("create O2", b.create(sb, "O2", position="bottom"))["item_id"]
o2c = ok("create o2c", b.create(O2, "o2c"))["item_id"]
M2r = ok("beta mirror O2 -> o2c (recursive)", b.mirror(O2, o2c))["item_id"]
show("beta DELETE O2 (only mirror is inside its subtree)", b.delete(O2))
show("beta GET O2", b.get(O2))
holder = ok("create holder", b.create(sb, "holder", position="bottom"))["item_id"]
O3 = ok("create O3 under holder", b.create(holder, "O3"))["item_id"]
ok("beta mirror O3 -> sb", b.mirror(O3, sb, position="bottom"))
show("beta DELETE holder (descendant is an origin)", b.delete(holder))
dest2 = ok("create dest2", b.create(sb, "dest2", position="bottom"))["item_id"]
tick()
M3 = ok("beta mirror O -> dest2", b.mirror(O, dest2))["item_id"]
show("beta GET O", b.get(O))
tick()
ok("beta DELETE dest2 (holds shell M3)", b.delete(dest2))
show("beta GET M3", b.get(M3))
show("beta GET O (M3 detached?)", b.get(O))
show("prod GET O (bumped?)", p.get(O))
tick()
ok("beta DELETE /mirror M2", b.request("DELETE", f"/api/v1/nodes/{M2}/mirror"))
show("beta GET O", b.get(O))
tick()
ok("beta plain DELETE M", b.delete(M))
show("beta GET O (no mirrors left)", b.get(O))
show("prod GET O (bumped by plain delete of shell?)", p.get(O))
show("beta GET x (persisted child of M)", b.get(x))
show("beta DELETE O (now without mirrors)", b.delete(O))

section("F. prod-made mirrors and orphans seen through beta")
Q = ok("create Q", b.create(sb, "Q", position="bottom"))["item_id"]
MQ = ok("prod mirror Q -> sb", p.mirror(Q, sb, position="bottom"))["item_id"]
show("beta GET Q (prod-made mirror)", b.get(Q))
show("beta GET MQ", b.get(MQ))
show("beta DELETE Q", b.delete(Q))
R = ok("create R", b.create(sb, "R", position="bottom"))["item_id"]
MR = ok("prod mirror R -> sb", p.mirror(R, sb, position="bottom"))["item_id"]
ok("prod create own child under MR", p.create(MR, "own child of MR"))
show("prod DELETE R (prod allows; orphans MR)", p.delete(R))
show("beta GET MR (orphan)", b.get(MR))
show("beta list MR (orphan)", b.list(MR))
show("beta update MR (orphan)", b.update(MR, name="orphan edit"))
show("prod GET MR (raw)", p.get(MR))
show("beta create under MR (orphan)", b.create(MR, "under orphan"))

section("G. errors and leftovers")
show("beta list sb", b.list(sb))
print("\ndone; sandbox", sb)
