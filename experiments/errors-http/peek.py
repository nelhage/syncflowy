"""Print a sandbox's children sorted by priority (helper for inspection)."""
import sys
from common import c
r = c.list(sys.argv[1])
for n in sorted(r.json()["nodes"], key=lambda n: n["priority"]):
    print(n["priority"], repr(n["name"]), n["note"], n["data"], n["completed"], n["createdAt"], n["modifiedAt"])
