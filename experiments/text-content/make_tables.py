"""Render results/*.json from the run_batch() experiments as markdown tables
(used to build the appendix of docs/workflowy/text-and-content.md)."""
import json
from pathlib import Path

R = Path(__file__).resolve().parent / "results"


def esc(v):
    if v is None:
        return "*null*"
    s = json.dumps(v, ensure_ascii=False)[1:-1]  # JSON-escaped, no quotes
    # make invisible / unusual code points visible
    s = "".join(ch if (ch.isprintable() and (ch == " " or not ch.isspace()) and ch not in "\u00ad\ufeff")
                else (f"\\u{ord(ch):04x}" if ord(ch) < 0x10000 else f"\\U{ord(ch):08x}") for ch in s)
    if len(s) > 160:
        s = s[:160] + "…"
    return "`" + s.replace("|", "\\|").replace("`", "ˋ") + "`" if s else "`(empty)`"


def cell(rec, path, f):
    st = rec.get(f"{path}_status")
    if st is None:
        return "—"
    if st >= 400:
        return f"**HTTP {st}**"
    v = rec.get(f"{path}_{f}")
    return "(same)" if v == rec["input"] else esc(v)


for name in ["01_unicode_whitespace", "02_html_tags", "03_markdown", "04_md_parser_edges",
             "05_escapes_order_dates", "09_control_chars", "13_trim_and_md_in_html"]:
    d = json.loads((R / f"{name}.json").read_text())
    fields = d["fields"]
    paths = [p for p in ("create", "update") if d.get(f"sandbox_{p}")]
    cols = [f"{p} {f}" for p in paths for f in fields]
    print(f"\n#### `{name}`\n")
    print("| case | input | " + " | ".join(cols) + " |")
    print("|---|---|" + "---|" * len(cols))
    for rec in d["cases"]:
        print(f"| {rec['label']} | {esc(rec['input'])} | " +
              " | ".join(cell(rec, p, f) for p in paths for f in fields) + " |")
