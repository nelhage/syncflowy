"""Batch 13: what counts as trimmable whitespace on create; markdown applied
inside existing HTML (attributes, <code>); layoutMode param vs markdown prefix."""
from common import run_batch

CASES = [
    ("lead-nbsp", " x "),
    ("lead-ideographic", "　x　"),
    ("lead-zwsp", "​x​"),
    ("lead-em-space", " x "),
    ("lead-tab", "\tx\t"),
    ("lead-feff", "﻿x﻿"),
    ("md-in-href", '<a href="https://e.com/*a*/[2026-01-15]">l</a>'),
    ("md-in-code-tag", "<code>*x* **y**</code>"),
    ("md-in-attr-b", '<b title="**t**">x</b>'),
    ("md-link-in-html-text", "<b>[t](https://e.com)</b>"),
    ("underscore-tag", "#a_b_c_"),
    ("star-tag", "#a*b*c"),
    ("tilde-url", "https://e.com/~~x~~"),
    ("lt-entity-then-md", "&lt;**x**&gt;"),
]
if __name__ == "__main__":
    run_batch("13_trim_and_md_in_html", CASES, fields=("name",), do_update=False)
