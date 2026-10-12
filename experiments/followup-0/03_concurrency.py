"""Q3: concurrent inserts into one parent. Can the server hand out duplicate
priorities?

Each trial fires N requests from N threads (each with its own ProdClient, so
its own HTTP connection). The shared cross-process rate limiter still spaces
request *starts* >= 1/9 s apart, but requests take ~0.2-0.4 s, so several are
in flight at once. We record per-request start/end times to prove overlap.

Usage: 03_concurrency.py [trials]   (default 3)
"""
import sys
import threading
import time
from collections import Counter

from common import ProdClient, c, mk, ok, out, show

TRIALS = int(sys.argv[1]) if len(sys.argv) > 1 else 3
N = 5
sb = c.make_sandbox("followup-0: 03 concurrency")
clients = [ProdClient() for _ in range(N)]
for cl in clients:
    cl.scratchpad_id()


def fire(fns):
    """Run fns[i](clients[i]) concurrently; return [(t0,t1,status,text)]."""
    res = [None] * len(fns)
    barrier = threading.Barrier(len(fns))

    def run(i):
        barrier.wait()
        t0 = time.time()
        r = fns[i](clients[i])
        res[i] = (t0, time.time(), r.status_code, r.text)

    ths = [threading.Thread(target=run, args=(i,)) for i in range(len(fns))]
    for t in ths:
        t.start()
    for t in ths:
        t.join()
    return res


def report(p, res, label):
    base = min(r[0] for r in res)
    out(f"  {label}: request windows (s, relative):",
        [(round(r[0] - base, 2), round(r[1] - base, 2), r[2]) for r in res])
    nodes = show(p, label)
    cnt = Counter(n["priority"] for n in nodes)
    d = {k: v for k, v in cnt.items() if v > 1}
    out(f"  DUPLICATES: {d}" if d else "  no duplicates")
    return nodes, d


dup_parents = []
for trial in range(TRIALS):
    out(f"===== trial {trial} =====")
    # top creates into a parent with one child at 100
    p = mk(sb, f"T{trial} top creates", position="bottom")
    mk(p, "seed100", position="bottom")
    res = fire([lambda cl, i=i: cl.create(p, f"top{i}", position="top") for i in range(N)])
    _, d = report(p, res, "top creates")
    if d:
        dup_parents.append(p)

    # bottom creates
    p = mk(sb, f"T{trial} bottom creates", position="bottom")
    mk(p, "seed100", position="bottom")
    res = fire([lambda cl, i=i: cl.create(p, f"bot{i}", position="bottom") for i in range(N)])
    _, d = report(p, res, "bottom creates")
    if d:
        dup_parents.append(p)

    # top moves
    src = mk(sb, f"T{trial} move source", position="bottom")
    ms = [mk(src, f"m{i}", position="bottom") for i in range(N)]
    p = mk(sb, f"T{trial} top moves", position="bottom")
    mk(p, "seed100", position="bottom")
    res = fire([lambda cl, i=i: cl.move(ms[i], p, position="top") for i in range(N)])
    _, d = report(p, res, "top moves")
    if d:
        dup_parents.append(p)

    # bottom mirrors (one origin)
    p = mk(sb, f"T{trial} bottom mirrors", position="bottom")
    mk(p, "seed100", position="bottom")
    res = fire([lambda cl: cl.mirror(src, p, position="bottom") for i in range(N)])
    _, d = report(p, res, "bottom mirrors")
    if d:
        dup_parents.append(p)

out("parents with duplicates:", dup_parents)
