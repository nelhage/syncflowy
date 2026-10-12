"""modifiedAt semantics. Sleeps ~2.2s between steps so second-granularity
timestamps are distinguishable.

Questions:
  * does a no-op update (same value) bump modifiedAt?
  * does {} bump it? does note=null (ignored) bump it?
  * does a failed (400) update bump it?
  * does creating / updating / completing / deleting / moving a CHILD bump the parent?
  * does complete/uncomplete bump modifiedAt?
  * is modifiedAt ever != the wall-clock time of the last write?
"""

import time

from _common import c, show, pnode, mk, now

SLEEP = 2.2

sb = c.make_sandbox("update-complete: 02 modifiedAt semantics")
print("sandbox", sb)
n = mk(sb, "subject", note="n")
other = mk(sb, "other parent")


def step(label, fn):
    time.sleep(SLEEP)
    t = now()
    if fn is not None:
        r = fn()
        show(label, r)
    pnode(f"{label} (wall={int(t)})", n)


step("baseline", None)
step("no-op update same name", lambda: c.update(n, name="subject"))
step("update {}", lambda: c.update(n))
step("update note=None", lambda: c.update(n, note=None))
step("update name=None", lambda: c.update(n, name=None))
step("failed update name=1", lambda: c.update(n, name=1))
step("unknown field only {'foo':1}", lambda: c.update(n, foo=1))
step("real name change", lambda: c.update(n, name="subject2"))
step("same layoutMode bullets", lambda: c.update(n, layoutMode="bullets"))
step("layoutMode -> todo", lambda: c.update(n, layoutMode="todo"))
step("complete", lambda: c.complete(n))
step("complete again", lambda: c.complete(n))
step("uncomplete", lambda: c.uncomplete(n))
step("uncomplete again", lambda: c.uncomplete(n))

kid = None


def mkkid():
    global kid
    r = c.create(n, "child")
    kid = r.json()["item_id"]
    return r


step("create child", mkkid)
step("update child name", lambda: c.update(kid, name="child2"))
step("complete child", lambda: c.complete(kid))
step("move child away", lambda: c.move(kid, other))
step("move child back", lambda: c.move(kid, n))
step("delete child", lambda: c.delete(kid))
step("move subject to other parent", lambda: c.move(n, other))
pnode("other parent at end", other)
