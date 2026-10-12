"""Batch 5: backslash escapes, entity-vs-markdown ordering, date formatting (create name)."""
from common import run_batch

CASES = [
    # which characters are backslash-escapable?
    ("esc-tilde", r"\~\~x\~\~"),
    ("esc-underscore", r"\_x\_"),
    ("esc-parens", r"\(x\)"),
    ("esc-bang", r"\!x"),
    ("esc-close-bracket", r"a\]b"),
    ("esc-gt", r"\>x"),
    ("esc-minus", r"\-x"),
    ("esc-dot", r"\.x"),
    ("esc-amp", r"\&amp;"),
    ("esc-star-in-code", r"`\*x\*`"),
    ("esc-link", r"\[t](https://e.com)"),
    ("esc-at-end", "x\\"),
    # entity decoding vs markdown ordering
    ("entity-star", "&#42;x&#42;"),
    ("entity-backtick", "&#96;x&#96;"),
    ("entity-bracket-date", "&#91;2026-01-15&#93;"),
    ("entity-in-bold", "**&amp;**"),
    # date formatting
    ("date-midnight", "[2026-01-15 00:00]"),
    ("date-noon", "[2026-01-15 12:00]"),
    ("date-morning", "[2026-01-15 09:05]"),
    ("date-2359", "[2026-01-15 23:59]"),
    ("date-hour24", "[2026-01-15 24:00]"),
    ("date-min60", "[2026-01-15 12:60]"),
    ("date-single-digit-hour", "[2026-01-15 9:05]"),
    ("date-seconds", "[2026-01-15 12:00:30]"),
    ("date-leap-valid", "[2024-02-29]"),
    ("date-leap-invalid", "[2025-02-29]"),
    ("date-apr31", "[2026-04-31]"),
    ("date-old", "[1999-12-31]"),
    ("date-far", "[9999-12-31]"),
    ("date-year0", "[0001-01-01]"),
    ("date-short-month", "[2026-1-5]"),
    ("date-T", "[2026-01-15T14:30]"),
    ("date-ampm", "[2026-01-15 2:30 PM]"),
    ("date-in-bold", "**[2026-01-15]**"),
    ("date-as-link-text", "[[2026-01-15]](https://e.com)"),
    ("date-followed-paren", "[2026-01-15](https://e.com)"),
    ("date-current-year", "[2026-10-12]"),
    ("date-each-month", " ".join(f"[2026-{m:02d}-01]" for m in range(1, 13))),
]
if __name__ == "__main__":
    run_batch("05_escapes_order_dates", CASES, fields=("name",), do_update=False)
