"""What happens when repeated top-inserts drive the minimum priority to 0?
And bottom after top-only inserts."""
from _util import ProdClient, show, dump_children, children

c = ProdClient()


def pri(kids):
    return [(n["name"], n["priority"]) for n in kids]


sb = c.make_sandbox("create: 03 priority exhaustion via top")
for i in range(11):
    c.create(sb, f"top-{i}", position="top")
    print(f"after top-{i}:", pri(children(c, sb)))
c.create(sb, "bottom-after", position="bottom")
print("after bottom:", pri(children(c, sb)))

# bottom after only tops (max is 100)
sb2 = c.make_sandbox("create: 03b bottom after tops")
for i in range(3):
    c.create(sb2, f"top-{i}", position="top")
c.create(sb2, "bottom", position="bottom")
print("03b:", pri(children(c, sb2)))
