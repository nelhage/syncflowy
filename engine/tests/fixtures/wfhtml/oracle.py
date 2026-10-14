"""Differential-test oracle for the Rust port of syncflowy.wfhtml.

Reads JSON lines ``{"s": ..., "typed": ...?}`` on stdin and writes, for each, one JSON
line ``{"well_formed": bool, "canonical": str, "plain": str, "append": str?}`` on
stdout (flushed per line, so a caller may keep one process and query it case by case).
``append`` is present only when ``typed`` is: ``append_typed_text(s, typed)``. If the
Python module raises, the line is ``{"error": "..."}`` instead.

Run from the repository root: ``uv run python -I engine/tests/fixtures/wfhtml/oracle.py``.
"""

import json
import sys

from syncflowy import wfhtml


def answer(req: dict) -> dict:
    s = req["s"]
    out = {
        "well_formed": wfhtml.is_well_formed(s),
        "canonical": wfhtml.canonicalize(s),
        "plain": wfhtml.to_plain_text(s),
    }
    if req.get("typed") is not None:
        out["append"] = wfhtml.append_typed_text(s, req["typed"])
    return out


def main() -> None:
    sys.stdin.reconfigure(encoding="utf-8", newline="\n")
    sys.stdout.reconfigure(encoding="utf-8", newline="\n")
    while True:
        line = sys.stdin.readline()
        if not line:
            break
        if not line.strip():
            continue
        try:
            out = answer(json.loads(line))
        except Exception as e:  # report, keep serving
            out = {"error": f"{type(e).__name__}: {e}"}
        sys.stdout.write(json.dumps(out) + "\n")
        sys.stdout.flush()


if __name__ == "__main__":
    main()
