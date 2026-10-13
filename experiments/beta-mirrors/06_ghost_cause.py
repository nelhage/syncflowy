"""Which deletes leave a ghost in the origin's ``mirror_ids``, and can one be cleared?

    SYNCFLOWY_TAG=beta-mirrors direnv exec . uv run python experiments/beta-mirrors/06_ghost_cause.py
"""

from __future__ import annotations

import importlib.util
from pathlib import Path

_spec = importlib.util.spec_from_file_location("probe_common", Path(__file__).with_name("common.py"))
common = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(common)  # type: ignore[union-attr]
b, p, show, ok, section = common.b, common.p, common.show, common.ok, common.section

sb = b.make_sandbox("beta-mirrors 06")
print("sandbox", sb)
cases = {
    "prod plain DELETE of the shell": lambda s: p.delete(s),
    "prod DELETE /mirror": lambda s: p.request("DELETE", f"/api/v1/nodes/{s}/mirror"),
    "prod DELETE of the shell's parent": None,  # handled below
    "beta plain DELETE of the shell": lambda s: b.delete(s),
}
for label, fn in cases.items():
    section(label)
    o = ok("create origin", b.create(sb, label, position="bottom"))["item_id"]
    if fn is None:
        holder = ok("create holder", b.create(sb, "holder", position="bottom"))["item_id"]
        s = ok("beta mirror origin -> holder", b.mirror(o, holder))["item_id"]
        show("prod DELETE holder", p.delete(holder))
    else:
        s = ok("beta mirror origin -> sb", b.mirror(o, sb, position="bottom"))["item_id"]
        show(label, fn(s))
    show("beta GET origin", b.get(o))
    show("beta DELETE origin", b.delete(o))

section("clearing a ghost")
o = ok("create origin", b.create(sb, "ghost target", position="bottom"))["item_id"]
s = ok("beta mirror origin -> sb", b.mirror(o, sb, position="bottom"))["item_id"]
ok("prod DELETE shell (leaves a ghost?)", p.delete(s))
show("beta DELETE /mirror on the ghost id", b.request("DELETE", f"/api/v1/nodes/{s}/mirror"))
s2 = ok("beta mirror origin -> sb again", b.mirror(o, sb, position="bottom"))["item_id"]
show("beta GET origin", b.get(o))
ok("beta DELETE the new shell", b.delete(s2))
show("beta GET origin (ghost still there?)", b.get(o))
show("beta DELETE origin", b.delete(o))
show("prod DELETE origin (the only way out?)", p.delete(o))
print("\ndone; sandbox", sb)
