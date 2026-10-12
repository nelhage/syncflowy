"""Create-path text pipeline: markdown, entities, prefixes, multiline (§6.2).

Each input is created, fetched, and its children listed (``create_and_show``),
so the transcript shows the transformed name, layoutMode, completion and any
child blocks.
"""

from ..registry import scenario
from ._util import create_and_show, node_sandbox


def _batch(ctx, names, **fields):
    p = node_sandbox(ctx)
    for n in names:
        create_and_show(ctx, p, n, **fields)


@scenario
def text_bold_italic(ctx):
    """Inline ** and * (naive non-greedy regexes; bold before italic) (§6.2 step 10)."""
    _batch(ctx, ["**b**", "*i*", "***x***", "**x", "a*b*c*d", "**", "* not a bullet*", "2*3*4"])


@scenario
def text_code_and_strike(ctx):
    """Inline `code`, ~~strike~~, and unsupported _x_ / ~x~ (§6.2 step 10)."""
    _batch(ctx, ["`c`", "~~s~~", "`**x**`", "~x~", "_u_ __b__", "``", "a `b` c `d`", "~~a~~b~~c~~"])


@scenario
def text_links(ctx):
    """[t](u) links (lazy regex), images, raw URLs not autolinked (§6.2 step 10)."""
    _batch(
        ctx,
        [
            "[t](http://x.com)",
            "[t](http://x.com/a(b))",
            "![img](http://x/y.png)",
            "https://example.com/a?b=c",
            "[a]",
            "[a](b) and [c](d)",
            "[**bold**](u)",
            "<a href=\"http://x/*y*\">z</a>",
        ],
    )


@scenario
def text_backslash_escapes(ctx):
    r"""Only \* \[ \] \` \( \) are escapable; other backslashes are kept (§6.2 step 10)."""
    _batch(ctx, ["\\*x\\*", "\\[a\\](b)", "\\_x\\_", "\\`c\\`", "\\\\", "a\\b", "\\(p\\)", "\\**b**"])


@scenario
def text_html_entities(ctx):
    """html.unescape runs before markdown: &#42;x&#42; -> <i>x</i>, &lt;b&gt; becomes real <b> (§6.2 step 2)."""
    _batch(
        ctx,
        ["&amp;", "&lt;b&gt;x&lt;/b&gt;", "&#42;x&#42;", "&ampx", "&bogus;", "&nbsp;x&nbsp;", "&#x1F600;", "&quot;q&quot;"],
    )


@scenario
def text_html_passthrough(ctx):
    """Raw HTML and < > & are stored as-is: no sanitising or escaping (§6.5)."""
    _batch(
        ctx,
        [
            "<b>x</b>",
            "<script>alert(1)</script>",
            "a < b & c > d",
            "<i>unclosed",
            "<!-- c -->",
            "<span style=\"color:red\">r</span>",
        ],
    )


@scenario
def text_dates(ctx):
    """[YYYY-MM-DD( HH:MM)] becomes <time>; invalid dates stay literal (§6.2 step 11)."""
    _batch(
        ctx,
        [
            "[2026-01-15]",
            "[2026-01-15 14:30]",
            "[2026-02-30]",
            "[2026-1-5]",
            "[2026-01-15 24:00]",
            "[2026-01-15 09:05]",
            "[2026-01-15 0:00]",
            "due [2024-02-29] ok",
        ],
    )


@scenario
def text_dates_more(ctx):
    """More date edges: seconds, T separator, AM/PM, ranges, links after dates (§6.2 step 11)."""
    _batch(
        ctx,
        [
            "[2026-01-15 14:30:00]",
            "[2026-01-15T14:30]",
            "[2026-01-15 2:30 PM]",
            "[2026-01-15] - [2026-01-20]",
            "[2026-12-31 23:59](http://x)",
            "[1999-13-01]",
        ],
    )


@scenario
def text_prefix_headers(ctx):
    """# / ## / ### -> h1/h2/h3; #### -> p with <b>; ##### and #x literal (§6.2 step 8)."""
    _batch(ctx, ["# h1", "## h2", "### h3", "#### h4", "##### h5", "#x", "#", "# **bold** head"])


@scenario
def text_prefix_lists(ctx):
    """'- ' strips to bullets; *, +, 1., >, --- are literal (§6.2 step 8)."""
    _batch(ctx, ["- item", "-   b", "* star", "+ plus", "1. one", "> quote", "---", "-no space"])


@scenario
def text_prefix_todo(ctx):
    """- [ ] / - [x] -> todo (x completes); - [X] only eats '- '; blind 6-char cut (§6.2 step 8)."""
    _batch(ctx, ["- [ ] todo", "- [x] done", "- [X] big", "- [ ]no", "- [x]", "- [ ] - [x] nested", "[ ] bare"])


@scenario
def text_unicode_roundtrip(ctx):
    """Unicode round-trips: emoji, CJK, RTL, astral, NFD vs NFC; JSON is ASCII-escaped (§1.3, §6)."""
    _batch(
        ctx,
        [
            "emoji \U0001F600 \U0001F468‍\U0001F469‍\U0001F467",
            "中文 עברית",
            "é vs é",
            "astral \U0001D518 \U0010FFFF",
            "bom﻿ zwsp​",
            " nbsp-stripped ",
            "　ideographic space　",
        ],
    )


@scenario
def text_control_chars_create_name(ctx):
    """Create name strips C0 controls (except TAB/LF/CR/VT/FF); DEL and C1 kept (§6.2 step 1)."""
    _batch(ctx, ["a\x01b", "a\tb", "a\x7fb", "a\x00b", "a\x1fb", "a\x80b\x9fc", "\x02lead"])


@scenario
def text_whitespace_strip(ctx):
    """Each block is str.strip()ped; internal whitespace kept (§6.2 step 7)."""
    _batch(ctx, ["  x  ", "\tx\t", "a  b", "x​", " em ", "  - item  "])


@scenario
def text_multiline_split_and_join(ctx):
    """Soft-wrap join regex ([^.!?\\n])\\n+([a-z]): lowercase continuation joins, others split (§6.2 step 5)."""
    _batch(
        ctx,
        ["Parent\nChild", "a\nb", "a\n\nb", "Hello world.\nthis", "end!\nnext", "q?\nnext", "a\nB", "a\n1"],
    )


@scenario
def text_multiline_join_chain(ctx):
    """Join chains: hello/world/again/more -> one node; a/b/c -> 'a b' + child c (§6.2 step 5)."""
    _batch(ctx, ["hello\nworld\nagain\nmore", "a\nb\nc", "a \nb", "x\n\n\ny\nZ", "parent\nchild1\nchild2", "A\n- b\nc"])


@scenario
def text_multiline_separators(ctx):
    """VT, FF, NEL, LS, PS always split; CRLF/CR normalise to LF (§6.2 steps 3, 6)."""
    _batch(ctx, ["a\vb", "a\fb", "a b", "a b", "a\x85b", "a\r\nB", "a\rB", "a\r\nb"])


@scenario
def text_multiline_prefixed_children(ctx):
    """Children get prefix layouts, plain children get 'p', priorities 0,1,2..., note '' (§4.3 step 5)."""
    p = node_sandbox(ctx)
    n = create_and_show(ctx, p, "Title\n- one\n- [ ] two\n# three\n#### four\nPlain\n```\ncode\n```")
    if n:
        for c in ctx.children(n):
            ctx.list(c["id"])


@scenario
def text_multiline_completion_leak(ctx):
    """- [x] completes its block and every later block in the request (§6.2 step 9)."""
    _batch(ctx, ["Top\n- [ ] a\n- [x] b\n- [ ] c\n# d\nPlain", "- [x] first\nSecond\n- [ ] Third"])


@scenario
def text_multiline_code_fences(ctx):
    """Fences group into one code-block; info string kept; unclosed fence eats the rest (§6.2 steps 4, 7)."""
    _batch(
        ctx,
        [
            "```py\nx = 1\n\ny = 2\n```",
            "```\nunclosed\nMore",
            "```one```",
            "Before\n```\ncode **not md**\n```\nAfter",
            "```\n  indented\n```",
        ],
    )


@scenario
def text_multiline_note_and_layout_param(ctx):
    """With a multiline name, note and layoutMode params apply to block 0 only (§4.3)."""
    p = node_sandbox(ctx)
    create_and_show(ctx, p, "A\nB\nC", note="the note", layoutMode="h2")
    create_and_show(ctx, p, "A\n- [x] B\nC", layoutMode="todo")


@scenario
def text_multiline_children_are_flat(ctx):
    """Multiline blocks become direct children of block 0, never grandchildren (§4.3)."""
    p = node_sandbox(ctx)
    n = create_and_show(ctx, p, "A\nB\nC\n- D\n  - E")
    if n:
        kids = ctx.children(n)
        if kids:
            ctx.list(kids[0]["id"])
            ctx.list(kids[-1]["id"])


@scenario
def text_create_long_name(ctx):
    """A 20k-character name round-trips (no length limit) (§2.4)."""
    p = node_sandbox(ctx)
    create_and_show(ctx, p, "x" * 20000)


@scenario
def text_entities_structural(ctx):
    """Entities are decoded before splitting, prefixes and fences; Python html.unescape edge cases (open question P2)."""
    _batch(
        ctx,
        [
            "a&#10;b",
            "A&#10;B",
            "&#35; head",
            "&#45; [x] done",
            "x&#13;Y",
            "```&#10;**x** &amp; *y*&#10;```",
            "```**x**```",
            "&#0;z",
            "&#1;z",
            "&#x80;",
            "&#xD800;",
            "&#x110000;",
        ],
    )


@scenario
def text_dates_edges(ctx):
    """Date regex edges: 1-3/5 digit years, 1-digit minutes, year 0/1, noon/midnight, non-ASCII digits (open question P1)."""
    _batch(
        ctx,
        [
            "[2026-01-15 9:5]",
            "[999-01-15]",
            "[0000-01-01]",
            "[0001-01-01]",
            "[12026-01-15]",
            "[2026-001-15]",
            "[2026-01-15 12:00]",
            "[2026-01-15 00:00]",
            "[2026-01-15 014:30]",
            "[٢٠٢٦-01-15]",
            "[２０２６-01-15]",
        ],
    )
