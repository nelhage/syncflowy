"""After 02's ``DELETE /mirror`` of shell MT (whose persisted child y was itself
mirrored as Y2): is y gone, and is Y2 an orphan? Ids are from 02_followups.out.

    SYNCFLOWY_TAG=beta-mirrors direnv exec . uv run python experiments/beta-mirrors/03_delete_mirror_cascade.py
"""

from syncflowy.prod_client import BETA_URL, ProdClient

b, p = ProdClient(base_url=BETA_URL), ProdClient()
for label, nid in [
    ("y (persisted child of MT)", "5edd7ccc-3eac-41b6-bc88-63f3ae50f57c"),
    ("Y2 (mirror of y)", "dd75912a-6dbd-426e-ab56-e702db934308"),
    ("T (MT's origin)", "5bbbc1d4-af87-4c7c-86d2-8ad9bbd739fa"),
]:
    for host, c in (("beta", b), ("prod", p)):
        r = c.get(nid)
        print(f"{label} [{host}] {r.status_code} {r.text}")
