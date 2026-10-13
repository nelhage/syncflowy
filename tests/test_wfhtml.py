"""Tests for syncflowy.wfhtml, using the observed UI tables in docs/workflowy/ui-findings.md."""

from __future__ import annotations

import pytest

from syncflowy.wfhtml import append_typed_text, canonicalize, escape_text, is_well_formed, to_plain_text

# E1/E2: text typed in the UI -> stored name [O].
E_TYPED = [
    ("a < b > c & d \"q\" 'q'", "a &lt; b &gt; c &amp; d &quot;q&quot; 'q'"),
    ("5 > 3", "5 &gt; 3"),
    ("AT&T", "AT&amp;T"),
    ("<b>not bold</b>", "&lt;b&gt;not bold&lt;/b&gt;"),
]

# E3/E5/E8: names and notes the UI produced [O]; all must be well-formed and canonical.
E_STORED = [
    "plain <b>WORD</b> plain",
    "plain <i>WORD</i> plain",
    "plain <u>WORD</u> plain",
    "plain <s>WORD</s> plain",
    "plain <b><i>WORD</i></b> plain",
    "plain <code>CODE</code> plain",
    'see <a href="https://example.com/a?b=1&c=2">https://example.com/a?b=1&amp;c=2</a>',
    'click <a href="https://example.org/">here</a>',
    '<a href="https://example.net/x">https://example.net/x</a>',
    "line one\nline two\n\nhas &lt;angle&gt; &amp; <b>bold</b>",
    "X #t",
    '<time startYear="2026" startMonth="7" startDay="5">Sun, Jul 5, 2026</time>',
    '<span class="colored c-red">red span</span>',
]

# F1/F2: API-written name -> (renders?, canonical form after a UI save) [O].
# For F07/F08 the observed results differ only by the text the user typed
# ("z" vs " z"); the canonical part is what we pin.
F_TABLE = [
    ("F01 <u>underline</u>", True, "F01 <u>underline</u>"),
    ('F02 <span class="colored c-red">red span</span>', True, 'F02 <span class="colored c-red">red span</span>'),
    ("F03 <script>alert(1)</script>after script", True, "F03 alert(1)after script"),
    ("F04 <div>in div</div>", True, "F04 in div"),
    ("F05 line<br>break", False, ""),
    ('F06 <img src="x">img', False, ""),
    ('F07 <a href="javascript:alert(1)">js link</a>', True, "F07 "),
    ("F08 <b>unclosed bold", False, ""),
    ("F09 <B>upper</B>", True, "F09 <b>upper</b>"),
    ("F10 <strong>strong</strong>", True, "F10 <b>strong</b>"),
    ("F11 x<!-- comment -->y", True, "F11 xy"),
    ("F12 <mark>mark</mark>", True, "F12 mark"),
    ("F13 a < b & c > d", False, ""),
    ("F14 a &amp; b &lt;tag&gt;", True, "F14 a &amp; b &lt;tag&gt;"),
    ("F15 nb&nbsp;sp &#39;q&#39; &quot;dq&quot;", False, ""),
    ('F16 he said "hi"', True, "F16 he said &quot;hi&quot;"),
]

# F step 2 exactly as observed, where the user appended " z".
F_APPEND_OBSERVED = [
    ("F01 <u>underline</u>", "F01 <u>underline z</u>"),
    ('F02 <span class="colored c-red">red span</span>', 'F02 <span class="colored c-red">red span z</span>'),
    ("F03 <script>alert(1)</script>after script", "F03 alert(1)after script z"),
    ("F04 <div>in div</div>", "F04 in div z"),
    ("F05 line<br>break", " z"),
    ('F06 <img src="x">img', " z"),
    ("F09 <B>upper</B>", "F09 <b>upper z</b>"),
    ("F10 <strong>strong</strong>", "F10 <b>strong z</b>"),
    ("F11 x<!-- comment -->y", "F11 xy z"),
    ("F12 <mark>mark</mark>", "F12 mark z"),
    ("F13 a < b & c > d", " z"),
    ("F14 a &amp; b &lt;tag&gt;", "F14 a &amp; b &lt;tag&gt; z"),
    ("F15 nb&nbsp;sp &#39;q&#39; &quot;dq&quot;", " z"),
    ('F16 he said "hi"', "F16 he said &quot;hi&quot; z"),
]


@pytest.mark.parametrize("typed,stored", E_TYPED)
def test_escape_text_matches_ui(typed: str, stored: str) -> None:
    assert escape_text(typed) == stored
    assert is_well_formed(stored)
    assert to_plain_text(stored) == typed


@pytest.mark.parametrize("stored", E_STORED)
def test_ui_output_is_well_formed_and_canonical(stored: str) -> None:
    assert is_well_formed(stored)
    assert canonicalize(stored) == stored


@pytest.mark.parametrize("name,renders,canonical", F_TABLE)
def test_f_table(name: str, renders: bool, canonical: str) -> None:
    assert is_well_formed(name) is renders
    assert canonicalize(name) == canonical
    # canonicalize is idempotent
    assert canonicalize(canonical) == canonical


@pytest.mark.parametrize("name,after", F_APPEND_OBSERVED)
def test_append_typed_text_matches_f_step2(name: str, after: str) -> None:
    assert append_typed_text(name, " z") == after


def test_append_text_escapes_and_respects_links() -> None:
    assert append_typed_text("a", " & <b>") == "a &amp; &lt;b&gt;"
    # Typing after a link does not extend it (UNVERIFIED choice).
    assert append_typed_text('<a href="https://x/">x</a>', "!") == '<a href="https://x/">x</a>!'
    # Nested formatting runs are all extended.
    assert append_typed_text("<b><i>w</i></b>", "z") == "<b><i>wz</i></b>"
    # The text goes after the link but stays inside the enclosing bold run.
    assert append_typed_text('<b><a href="u">w</a></b>', "z") == '<b><a href="u">w</a>z</b>'


def test_entities() -> None:
    for ok in ("&amp;", "&lt;", "&gt;", "&quot;", "&apos;", "&#39;", "&#x41;"):
        assert is_well_formed(ok), ok
    for badent in ("&nbsp;", "&copy;", "&", "a & b", "&#0;"):
        assert not is_well_formed(badent), badent


def test_misc_well_formedness() -> None:
    assert is_well_formed("")
    assert is_well_formed("plain")
    assert is_well_formed("<br/>")  # void tag closed XML-style: fine (then dropped on save)
    assert canonicalize("a<br/>b") == "ab"
    assert not is_well_formed("<b>x</B>")  # case-mismatched close
    assert not is_well_formed("<b>x</i>")
    assert not is_well_formed('<a href=x>y</a>')  # unquoted attribute


def test_canonicalize_attributes() -> None:
    assert canonicalize('<b class="x">w</b>') == "<b>w</b>"
    assert canonicalize('<a href="u" target="_blank">w</a>') == '<a href="u">w</a>'
    assert canonicalize("<a>w</a>") == "w"
    assert canonicalize('<span class="other">w</span>') == "w"
    assert canonicalize("<em>w</em>") == "<i>w</i>"
    assert canonicalize('<a href="  JavaScript:x">w</a>tail') == "tail"


def test_plain_text() -> None:
    assert to_plain_text('see <a href="u">a &amp; b</a> <b>x</b>') == "see a & b x"
    assert to_plain_text('<time startYear="2026" startMonth="1" startDay="15">Thu, Jan 15, 2026</time>') == (
        "Thu, Jan 15, 2026"
    )
    # Malformed input: best-effort salvage (the UI itself would show nothing).
    assert to_plain_text("F08 <b>unclosed bold") == "F08 unclosed bold"
    assert to_plain_text("a < b & c") == "a < b & c"
