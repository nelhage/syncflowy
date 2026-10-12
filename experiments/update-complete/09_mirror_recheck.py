"""Re-check (after a delay) the mirror created in 08: do edits made through
the mirror ever reach the origin (eventual consistency)?"""
import json
from _common import c, node, children

ORIGIN = "6cf802c5-17bf-4207-8491-1ff0de047ad0"
MIRROR = "0ac88ffc-b941-4cfc-b9a2-d0b5c64e23d7"
for nid in (ORIGIN, MIRROR):
    print(json.dumps(node(nid)))
print("list under mirror parent:", json.dumps([x for x in children("97501530-2d02-4c0d-801d-01a481d1dfae")]))
