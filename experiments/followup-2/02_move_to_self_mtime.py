"""followup-2 / exp02: does move-to-self bump the node's modifiedAt?
(errors-and-http.md said pure no-op; ordering-and-move.md / delete-and-mirrors.md
said it bumps.) Also: does a validation-only move body ({} on a real node) ...
is NOT tested here: {} passes validation, so it would move the node to the
default parent (probably the account root) -- out of scope.

Run:
  SYNCFLOWY_TAG=followup-2 direnv exec . uv run python experiments/followup-2/02_move_to_self_mtime.py
"""
from __future__ import annotations

import json
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "src"))
from syncflowy.prod_client import ProdClient  # noqa: E402

c = ProdClient()
OUT = []


def rec(label, r):
    d = {"label": label, "req": f"{r.request.method} {r.request.url}",
         "status": r.status_code, "body": r.json(), "t": int(time.time())}
    print(json.dumps(d))
    OUT.append(d)
    return r.json()


sb = c.make_sandbox("followup-2: move-to-self modifiedAt")
for trial in range(2):
    y = c.create(sb, f"Y{trial}", position="bottom").json()["item_id"]
    before = rec(f"t{trial} GET Y before", c.get(y))["node"]
    time.sleep(2.5)
    rec(f"t{trial} move Y under Y (bottom)", c.move(y, y, position="bottom"))
    after = rec(f"t{trial} GET Y after", c.get(y))["node"]
    print("DIFF", trial, {k: (before[k], after[k]) for k in before if before[k] != after[k]})
rec("list sandbox", c.list(sb))
with open(Path(__file__).with_suffix(".out.json"), "w") as f:
    json.dump(OUT, f, indent=1)
