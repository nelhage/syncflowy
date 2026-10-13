"""Differential oracle for the planner's create-name whitelist (design §7.2).

Reads a JSON array of strings on stdin and writes, for each, the blocks that the
mock's create-path name pipeline (``syncflowy.mock.text.transform_create_name``,
data-model.md §6.2) produces: ``[[name, layout, completed], ...]``. A name is a fixed
point of the create pipeline iff the result is ``[[name, null, false]]``.

Run from the repository root, isolated from the script's directory:

    uv run --quiet python -I engine/tests/fixtures/plan/create_oracle.py < names.json
"""

from __future__ import annotations

import json
import sys

from syncflowy.mock.text import transform_create_name


def main() -> None:
    names = json.load(sys.stdin)
    out = [
        [[b.name, b.layout, b.completed] for b in transform_create_name(name)]
        for name in names
    ]
    json.dump(out, sys.stdout, ensure_ascii=True)


if __name__ == "__main__":
    main()
