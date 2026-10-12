"""Q3 (cont.): concurrent top inserts that straddle the min==0 renumber.

Parent starts with children at 100, 50, 25, 12, 6, 3, 1 (min 1). Fire 5 top
creates concurrently: a serial server would give 0, then 100 + renumber, then
50, 25, 12. Then the same with a mix of top creates and top moves.
"""
from collections import Counter

from common import c, mk, out, show
from pathlib import Path


# Re-implement fire() locally (03 runs experiments at import time).
import threading, time
from common import ProdClient

N = 5
clients = [ProdClient() for _ in range(N)]
for cl in clients:
    cl.scratchpad_id()


def fire(fns):
    res = [None] * len(fns)
    barrier = threading.Barrier(len(fns))

    def run(i):
        barrier.wait()
        r = fns[i](clients[i])
        res[i] = (r.status_code, r.text)

    ths = [threading.Thread(target=run, args=(i,)) for i in range(len(fns))]
    [t.start() for t in ths]
    [t.join() for t in ths]
    return res


sb = c.make_sandbox("followup-0: 04 concurrent inserts across renumber")
for trial in range(2):
    p = mk(sb, f"R{trial} creates", position="bottom")
    for i in range(7):
        mk(p, f"pre{i}", position="top")  # 100..1
    show(p, "before")
    out(fire([lambda cl, i=i: cl.create(p, f"c{i}", position="top") for i in range(N)]))
    nodes = show(p, "after")
    out("  dups:", {k: v for k, v in Counter(n["priority"] for n in nodes).items() if v > 1})

    p = mk(sb, f"R{trial} mixed", position="bottom")
    src = mk(sb, f"R{trial} mixed src", position="bottom")
    ms = [mk(src, f"m{i}", position="bottom") for i in range(2)]
    for i in range(7):
        mk(p, f"pre{i}", position="top")
    fns = [lambda cl: cl.create(p, "cA", position="top"),
           lambda cl: cl.move(ms[0], p, position="top"),
           lambda cl: cl.create(p, "cB", position="top"),
           lambda cl: cl.move(ms[1], p, position="top"),
           lambda cl: cl.create(p, "cC", position="bottom")]
    out(fire(fns))
    nodes = show(p, "mixed after")
    out("  dups:", {k: v for k, v in Counter(n["priority"] for n in nodes).items() if v > 1})
