"""Side check (critic list): on a move, are descendants' modifiedAt bumped?
On a move-to-self, is the parent's modifiedAt bumped? Uses >1 s sleeps to
beat the 1-second timestamp resolution.
"""
import time

from common import c, mk, ok, out

sb = c.make_sandbox("followup-0: 06 move modifiedAt of descendants / self-move parent")
a = mk(sb, "A (old parent)", position="bottom")
b = mk(sb, "B (new parent)", position="bottom")
x = mk(a, "X (moved)", position="bottom")
x1 = mk(x, "X1 child", position="bottom")
x11 = mk(x1, "X11 grandchild", position="bottom")
sib = mk(a, "S sibling of X", position="bottom")
ids = {"sb": sb, "A": a, "B": b, "X": x, "X1": x1, "X11": x11, "S": sib}


def snap():
    return {k: ok(c.get(v))["node"]["modifiedAt"] for k, v in ids.items()}


time.sleep(2.1)
s0 = snap()
out("before          ", s0)
t = time.time()
ok(c.move(x, b, position="bottom"))
out("move at", int(t))
s1 = snap()
out("after X->B      ", s1)
time.sleep(2.1)
t = time.time()
ok(c.move(x, x))
out("self-move at", int(t))
out("after X->X      ", snap())
