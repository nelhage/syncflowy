"""Create long-lived fixtures for the human to inspect in the Workflowy web UI
(see "UI scenarios" in docs/workflowy/update-and-complete.md). Each node is
created with a plain name and then UPDATED to the tricky value, since update
stores strings verbatim while create transforms them."""
import json
from _common import c, mk, node

sb = c.make_sandbox("update-complete: 10 UI inspection fixtures")
print("sandbox", sb)
NAMES = [
    ("U1", "a < b & c > d (raw specials via update)"),
    ("U2", "a &amp; b &lt;tag&gt; (entities via update)"),
    ("U3", "**md bold** *md it* [md link](https://example.com) (raw markdown via update)"),
    ("U4", "<u>u</u> <strong>strong</strong> <span class=\"colored c-red\">red span</span> <script>x</script> (unsupported tags)"),
    ("U5", "<b>unclosed bold"),
    ("U6", "line1\nline2 (embedded newline via update)"),
]
out = {}
for key, val in NAMES:
    n = mk(sb, f"{key} placeholder", position="bottom")
    c.update(n, name=val, note=f"{key} note: <b>bold</b> & **md** <i>it")
    out[key] = n
for key, lm in [("L1", "board"), ("L2", "p"), ("L3", "table"), ("L4", "dashboard"), ("L5", "nonsense"), ("L6", ""), ("L7", "H1")]:
    n = mk(sb, f"{key} layoutMode={lm!r} (set via update)", position="bottom")
    kid = mk(n, f"{key} child 1", position="bottom")
    mk(n, f"{key} child 2", position="bottom")
    c.update(n, layoutMode=lm)
    out[key] = n
# completion fixtures
p = mk(sb, "C1 completed parent with uncompleted children (via API)", position="bottom")
mk(p, "C1 child (not completed)")
c.complete(p)
out["C1"] = p
t = mk(sb, "C2 todo, uncompleted", layoutMode="todo", position="bottom")
out["C2"] = t
for k, v in out.items():
    print(k, v, json.dumps(node(v)["name"]))
