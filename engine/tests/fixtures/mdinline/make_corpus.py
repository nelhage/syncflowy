"""Write corpus.jsonl: realistic WF values for the mdinline corpus test.

Each entry is a value the web UI (or the API's create pipeline) stores, as recorded in
docs/workflowy/ui-findings.md (UI:xx), docs/workflowy/text-and-content.md (T§x) and
docs/workflowy/data-model.md (DM§x), plus everyday names typed the way the UI stores
them (escaped with wfhtml.escape_text, formatting as bare tags, UI:E1/E3).

`expect` is "ui" for values the UI produces (they must be tier Exact or Passthrough,
§5.3) and "any" for API-written values (non-canonical or malformed, UI:F1/F2), whose
tier is whatever the snapshot says.

The expected Markdown and tier are filled in by the Rust test
(`MDINLINE_BLESS=1 cargo test --test mdinline_corpus`), then reviewed by hand.

Run from the repository root: `uv run python engine/tests/fixtures/mdinline/make_corpus.py`.
"""

from __future__ import annotations

import json
import pathlib
import sys

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[4] / "src"))

from syncflowy.wfhtml import canonicalize, escape_text, is_well_formed  # noqa: E402

E = escape_text


def t(y: int, m: int, d: int, text: str, hm: tuple[int, int] | None = None) -> str:
    attrs = f'startYear="{y}" startMonth="{m}" startDay="{d}"'
    if hm:
        attrs += f' startHour="{hm[0]}" startMinute="{hm[1]}"'
    return f"<time {attrs}>{text}</time>"


UI_NAMES = [
    # UI:E1 / E2: typed special characters.
    ("UI:E1", E("a < b > c & d \"q\" 'q'")),
    ("UI:E1", E("5 > 3")),
    ("UI:E1", E("AT&T")),
    ("UI:E2", E("<b>not bold</b>")),
    # UI:E3: formatting shortcuts.
    ("UI:E3", "plain <b>WORD</b> plain"),
    ("UI:E3", "plain <i>WORD</i> plain"),
    ("UI:E3", "plain <u>WORD</u> plain"),
    ("UI:E3", "plain <s>WORD</s> plain"),
    ("UI:E3", "plain <b><i>WORD</i></b> plain"),
    ("UI:E3", "plain <code>CODE</code> plain"),
    # UI:E5: links.
    ("UI:E5", 'see <a href="https://example.com/a?b=1&c=2">https://example.com/a?b=1&amp;c=2</a>'),
    ("UI:E5", 'click <a href="https://example.org/">here</a>'),
    # UI:F step 2: what the UI leaves after canonicalising (all canonical).
    ("UI:F01", "F01 <u>underline z</u>"),
    ("UI:F02", 'F02 <span class="colored c-red">red span z</span>'),
    ("UI:F09", "F09 <b>upper z</b>"),
    ("UI:F14", "F14 a &amp; b &lt;tag&gt; z"),
    ("UI:F16", "F16 he said &quot;hi&quot; z"),
    # UI:C2 / E7: tags and mentions are plain text.
    ("UI:C2", "X #t"),
    ("UI:E7", "#urgent call @alice re #a-b_c"),
    # DM§11: calendar day nodes; T§7 / DM§6.2 step 11: dates from the create pipeline.
    ("DM§11", t(2026, 7, 5, "Sun, Jul 5, 2026")),
    ("T§7", t(2026, 1, 15, "Thu, Jan 15, 2026")),
    ("T§7", t(2026, 1, 15, "Thu, Jan 15, 2026 at 2:30 PM", (14, 30))),
    ("T§7", t(2026, 1, 15, "Thu, Jan 15, 2026 at 12:00 AM", (0, 0))),
    ("T§7", t(2026, 1, 5, "Mon, Jan 5, 2026")),
    ("T§7", t(1, 1, 1, "Mon, Jan 1, 1")),
    ("T§7", "<b>" + t(2026, 1, 15, "Thu, Jan 15, 2026") + "</b>"),
    ("T§7", '<a href="https://e.com">' + t(2026, 1, 15, "Thu, Jan 15, 2026") + "</a>"),
    ("T§7", t(2026, 1, 15, "Thu, Jan 15, 2026") + " - " + t(2026, 1, 20, "Tue, Jan 20, 2026")),
    # T§6.1: create-path inline markdown output (canonical ones).
    ("T§6.1", "x <b>bold</b> y"),
    ("T§6.1", "x <i>it</i> y"),
    ("T§6.1", "x <s>st</s> y"),
    ("T§6.1", 'x <a href="https://example.com/a?b=1&c=2">text</a> y'),
    ("T§6.1", '<a href="/foo">rel</a>'),
    ("T§6.1", '<a href="https://e.com"><b>b</b></a>'),
    ("T§6.1", "<b>bold <i>ital</i> bold</b>"),
    ("T§6.1", "x <code>co<i>de</i></code> y"),
    ("T§6.1", 'x <code>&lt;b&gt;</code> y'),
    ("T§6.1", '<a href="https://e.com"></a>'),
    ("T§6.1", '<a href="">t</a>'),
    ("T§6.1", '<a href="a b">t</a>'),
    ("T§6.1", '<a href="https://e.com">a [b] c</a>'),
    ("T§6.1", '!<a href="https://example.com/x.png">alt</a>'),
    ("T§6.1", "#a<i>b</i>c"),
    ("T§6.3", "<b><i>bi</i></b>"),
    ("T§6.3", "<i></i>unclosed"),
    ("T§6.3", "<b></b>x<b></b>"),
    ("T§6.3", "<b> spaced </b>"),
    ("T§6.3", "2<i>3</i>4"),
    ("T§6.3", "<s>~x</s>~"),
    ("T§6.3", "<code></code>a <code> b</code>`"),
    ("T§5", "<b> H4</b>"),
    # Everyday names, typed in the UI.
    ("typed", E("Buy milk & eggs (2x) — ") + "<b>today</b>!"),
    ("typed", E("Meeting w/ @bob re: Q3 #planning")),
    ("typed", "<i>Note:</i> use <code>cargo test -- --nocapture</code>"),
    ("typed", E("10:30 standup; 1:1 @ 2pm")),
    ("typed", E("C++ vs C# vs F*")),
    ("typed", E("*args, **kwargs")),
    ("typed", E("foo_bar_baz and __dunder__")),
    ("typed", E("price: $10-$20")),
    ("typed", E("x <= y && y >= z")),
    ("typed", E("TODO: fix [bug] in ") + "<code>parse()</code>"),
    ("typed", E("2 * 3 = 6")),
    ("typed", E("100% done!")),
    ("typed", "<s>cancelled</s> moved to " + t(2026, 10, 20, "Tue, Oct 20, 2026")),
    ("typed", E("→ next step 🎉")),
    ("typed", E("~/.bashrc and C:\\Users\\me")),
    ("typed", E("1. first thing")),
    ("typed", E("- dash at start")),
    ("typed", E("# not a heading")),
    ("typed", E("> quoted")),
    ("typed", E("[ ] fake box")),
    ("typed", E("---")),
    ("typed", E("[[Wiki Link]] and ==hl== and $x^2$")),
    ("typed", E("block id ^abc")),
    ("typed", E("%%not a comment%%")),
    ("typed", E("  leading and trailing  ")),
    ("typed", E("日本語のメモ")),
    ("typed", E("emoji 📅 2026-01-15 typed by hand")),
    ("typed", 'read <a href="https://en.wikipedia.org/wiki/Foo_(bar)">https://en.wikipedia.org/wiki/Foo_(bar)</a> later'),
    ("typed", 'docs: <a href="https://docs.rs/similar/latest/similar/">https://docs.rs/similar/latest/similar/</a>.'),
    ("typed", '<span class="colored c-red">urgent</span> <b>call</b> <u>Alice</u>'),
    ("typed", "<b>Bold</b>: <i>it</i>, <u>under</u>"),
    ("typed", "a<b>b</b>c"),
]

UI_NOTES = [
    # UI:E8: the observed multi-line note.
    ("UI:E8", "line one\nline two\n\nhas &lt;angle&gt; &amp; <b>bold</b>"),
    # UI:E5: a raw URL pasted into a note.
    ("UI:E5", '<a href="https://example.net/x">https://example.net/x</a>'),
    ("typed", E("- not a child\n1. not a list\n> not a quote\n# not a heading\n---\n```")),
    ("typed", "steps:\n" + E("1) open\n2) close")),
    ("typed", "<b>bold across\nlines</b> then text"),
    ("typed", "<b>bold across\n\nparagraphs</b>"),
    ("typed", E("  indented\n\ttabbed\ntrailing  ")),
    ("typed", E("a\n\n\nb\n")),
    ("typed", "see " + '<a href="https://e.com/a">https://e.com/a</a>' + "\nand " + '<a href="https://e.com/b">this</a>'),
    ("typed", "<code>one</code>\n<code>``two``</code>"),
]

API_VALUES = [
    # UI:F1/F2: written through the API update path.
    ("UI:F03", "F03 <script>alert(1)</script>after script"),
    ("UI:F04", "F04 <div>in div</div>"),
    ("UI:F05", "F05 line<br>break"),
    ("UI:F06", 'F06 <img src="x">img'),
    ("UI:F07", 'F07 <a href="javascript:alert(1)">js link</a>'),
    ("UI:F08", "F08 <b>unclosed bold"),
    ("UI:F09", "F09 <B>upper</B>"),
    ("UI:F10", "F10 <strong>strong</strong>"),
    ("UI:F11", "F11 x<!-- comment -->y"),
    ("UI:F12", "F12 <mark>mark</mark>"),
    ("UI:F13", "F13 a < b & c > d"),
    ("UI:F15", "F15 nb&nbsp;sp &#39;q&#39; &quot;dq&quot;"),
    ("UI:F16", 'F16 he said "hi"'),
    ("T§4.3", "<a href='https://e.com' target=\"_blank\">t</a>"),
    ("T§4.3", '<b class="x" style="y">b</b>'),
    ("T§4.3", "<em>em</em>"),
    ("T§4.3", "<span>plain span</span>"),
    ("T§5", "multi\nline name"),
]


def main() -> None:
    out = pathlib.Path(__file__).with_name("corpus.jsonl")
    old = {}
    if out.exists():
        for line in out.read_text().splitlines():
            e = json.loads(line)
            old[(e["kind"], e["wf"])] = e
    rows = []
    for kind, expect, items in (
        ("name", "ui", UI_NAMES),
        ("note", "ui", UI_NOTES),
        ("name", "any", API_VALUES),
    ):
        for src, wf in items:
            if expect == "ui":
                assert is_well_formed(wf) and canonicalize(wf) == wf, (src, wf)
            prev = old.get((kind, wf), {})
            rows.append(
                {
                    "src": src,
                    "kind": kind,
                    "expect": expect,
                    "wf": wf,
                    "md": prev.get("md"),
                    "tier": prev.get("tier"),
                }
            )
    out.write_text("".join(json.dumps(r, ensure_ascii=False) + "\n" for r in rows))
    print(f"wrote {len(rows)} entries to {out}")


if __name__ == "__main__":
    main()
