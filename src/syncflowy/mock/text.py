"""Text processing for the mock Workflowy API.

Production has three text paths that behave differently (data-model.md §6):

* **update** (`POST /nodes/:id`): `name` and `note` are stored verbatim except that the C0 control characters
  ``[\\x00-\\x08\\x0b\\x0c\\x0e-\\x1f]`` are deleted (§6.1). Update's
  `layoutMode` is fully verbatim (golden update_layoutmode_control_chars).
* **create note**: stored completely verbatim. A NUL makes production return
  HTTP 500 (§6.3); that check lives in the request handler, not here.
* **create name**: the only transforming path (§6.2). :func:`transform_create_name`
  implements the full pipeline: control-character strip, ``html.unescape``,
  CR normalisation, code-fence region detection, the soft-wrap join regex,
  ``splitlines`` splitting, per-line strip, fence grouping, prefix detection,
  the ``- [x]`` completion leak, naive inline-markdown regexes and ``<time>``
  dates.

Everything here is a pure function so it can be unit tested in isolation; the
HTTP tests in ``tests/mock`` replay the production golden vectors through it.
"""

from __future__ import annotations

import datetime
import html
import re
from dataclasses import dataclass

# §6.1: update path strip set (VT and FF are stripped here).
UPDATE_STRIP_RE = re.compile(r"[\x00-\x08\x0b\x0c\x0e-\x1f]")
# §6.2 step 1: create-name strip set. VT (\x0b) and FF (\x0c) are KEPT here,
# and later act as line breaks via str.splitlines(). \x1c-\x1e are stripped
# before splitting, so they never split (T§2).
CREATE_NAME_STRIP_RE = re.compile(r"[\x00-\x08\x0e-\x1f]")
# §6.2 step 5: the soft-wrap join. One non-overlapping left-to-right pass.
JOIN_RE = re.compile(r"([^.!?\n])\n+([a-z])")

FENCE = "```"

# §6.2 step 10: escapable characters and their placeholders. The NUL byte
# cannot survive step 1, so placeholders containing it cannot collide with
# user text.
_ESC = {c: f"\x00E{i}\x00" for i, c in enumerate("*[]`()")}
_ESC_RE = re.compile(r"\\([*\[\]`()])")
_CODE_RE = re.compile(r"`(.*?)`")
_BOLD_RE = re.compile(r"\*\*(.*?)\*\*")
_ITALIC_RE = re.compile(r"\*(.*?)\*")
_STRIKE_RE = re.compile(r"~~(.*?)~~")
# §6.2 step 11 / T§7.
# Minutes are 1-2 digits ("[2026-01-15 9:5]" -> 9:05 AM) and \d is the
# Unicode class: Arabic-Indic and full-width digits convert too [O: golden
# text_dates_edges, which answered open question P1].
_DATE_RE = re.compile(r"\[(\d{4})-(\d{1,2})-(\d{1,2})(?: (\d{1,2}):(\d{1,2}))?\]")
_LINK_RE = re.compile(r"\[(.*?)\]\((.*?)\)")

_MON = "Jan Feb Mar Apr May Jun Jul Aug Sep Oct Nov Dec".split()
_DOW = "Mon Tue Wed Thu Fri Sat Sun".split()


_SURROGATE_RE = re.compile("[\ud800-\udfff]")


def has_lone_surrogate(value: str) -> bool:
    """True if ``value`` holds a lone surrogate (JSON ``"\\ud800"``), which cannot be stored as UTF-8."""
    return _SURROGATE_RE.search(value) is not None


def replace_lone_surrogates(value: str) -> str:
    """Each lone surrogate becomes ``?`` (``str.encode("utf-8", "replace")`` semantics).

    Production does this to create-path names and update-path names and notes
    [O: goldens create_layoutmode_controls_nul_and_surrogates and
    update_nul_layoutmode_and_lone_surrogates: ``"x\\ud800y"`` -> ``"x?y"``].
    Fields stored raw (create note, layoutMode) give 500 instead.
    """
    return _SURROGATE_RE.sub("?", value)


def strip_update_text(value: str) -> str:
    """Update-path transformation (data-model.md §6.1): drop C0 controls, lone surrogates -> ``?``."""
    return replace_lone_surrogates(UPDATE_STRIP_RE.sub("", value))


def format_date_text(d: datetime.date) -> str:
    """``Thu, Jan 15, 2026`` (English, unpadded day and year; T§7)."""
    return f"{_DOW[d.weekday()]}, {_MON[d.month - 1]} {d.day}, {d.year}"


def format_time_elem(d: datetime.date) -> str:
    """A date-only ``<time>`` element, as in production calendar day names:
    ``<time startYear="2026" startMonth="7" startDay="5">Sun, Jul 5, 2026</time>``."""
    return f'<time startYear="{d.year}" startMonth="{d.month}" startDay="{d.day}">{format_date_text(d)}</time>'


def _time_elem(m: re.Match[str]) -> str:
    y, mo, d = int(m[1]), int(m[2]), int(m[3])
    try:
        dt = datetime.date(y, mo, d)
    except ValueError:
        return m[0]  # invalid calendar date: left literal
    attrs = f'startYear="{y}" startMonth="{mo}" startDay="{d}"'
    text = format_date_text(dt)
    if m[4] is not None:
        h, mi = int(m[4]), int(m[5])
        if h > 23 or mi > 59:
            return m[0]
        attrs += f' startHour="{h}" startMinute="{mi}"'
        text += f" at {(h % 12) or 12}:{mi:02d} {'AM' if h < 12 else 'PM'}"
    # UNVERIFIED (open-questions P1/H11): locale / timezone / 24h settings of
    # the account may change the display text. We always use the en-US form.
    return f"<time {attrs}>{text}</time>"


def inline_markdown(s: str) -> str:
    """Naive inline-markdown regexes (data-model.md §6.2 step 10).

    These deliberately ignore HTML context: they run inside attributes,
    ``<code>`` and URLs, and produce mis-nested output such as
    ``***x***`` -> ``<b><i>x</b></i>`` and ``**x`` -> ``<i></i>x``.
    Order matters: bold before italic, date before link.
    """
    s = _ESC_RE.sub(lambda m: _ESC[m[1]], s)
    s = _CODE_RE.sub(r"<code>\1</code>", s)
    s = _BOLD_RE.sub(r"<b>\1</b>", s)
    s = _ITALIC_RE.sub(r"<i>\1</i>", s)
    s = _STRIKE_RE.sub(r"<s>\1</s>", s)
    s = _DATE_RE.sub(_time_elem, s)
    s = _LINK_RE.sub(r'<a href="\2">\1</a>', s)
    for k, v in _ESC.items():
        s = s.replace(v, k)
    return s


@dataclass(frozen=True)
class Block:
    """One node produced by a create-path ``name``.

    ``layout`` is None for a *plain* block: the caller turns that into
    ``bullets`` for block 0 and ``p`` for children (§6.2 step 8).
    """

    name: str
    layout: str | None
    completed: bool


def _is_one_line_fence(stripped: str) -> bool:
    return len(stripped) > 3 and stripped.startswith(FENCE) and stripped.endswith(FENCE)


def _soft_join(text: str) -> str:
    """Apply JOIN_RE outside code-fence regions (§6.2 steps 4-5, C§5.4).

    A line starting with ``` opens a region unless it is a one-line fence or
    no later line ends with ``` (an unclosed fence protects nothing). The
    region is the opening line plus following lines up to, but NOT including,
    the next line that ends with ```. The closing line is outside the region
    (so ```` ```\\n\\nafter ```` joins into ```` ``` after ````). The regex
    runs separately on each maximal run of outside lines, so no join crosses a
    region boundary.
    """
    lines = text.split("\n")
    n = len(lines)
    inside = [False] * n
    i = 0
    while i < n:
        st = lines[i].strip()  # UNVERIFIED: whether indented fences open a region
        if st.startswith(FENCE) and not _is_one_line_fence(st):
            j = i + 1
            while j < n and not lines[j].rstrip().endswith(FENCE):
                j += 1
            if j >= n:
                # Unclosed fence: NOT a protected region (golden
                # text_multiline_code_fences: "```\nunclosed\nMore" joins the
                # opener, giving the code block " unclosed\nMore"). Treat the
                # opener as an ordinary outside line.
                i += 1
                continue
            for k in range(i, j):
                inside[k] = True
            # Skip the closing line: it is outside the region and must not be
            # re-examined as an opener (a bare ``` closes, it does not open).
            i = j + 1
        else:
            i += 1
    out: list[str] = []
    k = 0
    while k < n:
        if inside[k]:
            out.append(lines[k])
            k += 1
            continue
        j = k
        while j < n and not inside[j]:
            j += 1
        out.append(JOIN_RE.sub(r"\1 \2", "\n".join(lines[k:j])))
        k = j
    return "\n".join(out)


def _group(lines: list[str]) -> list[tuple[str, str]]:
    """Group split lines into ("code", content) / ("text", stripped) blocks.

    §6.2 step 7 and C§5.4 step 3: a line starting with ``` starts a code block
    that swallows the following non-blank lines (unstripped) up to and
    including the first one ending with ```, or to the end of input. A line
    that both starts and ends with ``` (length > 3) is a block by itself. The
    block text has a leading and a trailing ``` removed and is then stripped of
    ``\\n`` only (so ```` ``` ``` ```` gives ``" "``).
    """
    blocks: list[tuple[str, str]] = []
    i, n = 0, len(lines)
    while i < n:
        st = lines[i].strip()
        if not st:
            i += 1
            continue
        if st.startswith(FENCE):
            parts = [st]
            i += 1
            if not _is_one_line_fence(st):
                while i < n:
                    line = lines[i]
                    i += 1
                    if not line.strip():
                        continue  # blank lines inside a fence are dropped
                    if line.rstrip().endswith(FENCE):
                        parts.append(line.rstrip())
                        break
                    parts.append(line)
            content = "\n".join(parts)[len(FENCE):]
            if content.endswith(FENCE):
                content = content[: -len(FENCE)]
            blocks.append(("code", content.strip("\n")))
        else:
            blocks.append(("text", st))
            i += 1
    return blocks


def transform_create_name(raw: str) -> list[Block]:
    """The create-path ``name`` pipeline (data-model.md §6.2).

    Returns at least one block. Block 0 becomes the created node; the rest
    become its flat children.
    """
    s = replace_lone_surrogates(CREATE_NAME_STRIP_RE.sub("", raw))  # step 1 (+ lone surrogates -> "?")
    s = html.unescape(s)  # step 2: before markdown, single pass
    s = s.replace("\r\n", "\n").replace("\r", "\n")  # step 3
    s = _soft_join(s)  # steps 4-5
    grouped = _group(s.splitlines())  # steps 6-7
    if not grouped:
        grouped = [("text", "")]  # "", "   ", "\n\n" -> one node named ""

    out: list[Block] = []
    leak = False  # step 9: once set by a "- [x]" block it is never reset
    for kind, text in grouped:
        if kind == "code":
            # UNVERIFIED (open-questions P2): fence content gets no inline
            # markdown (T§5.1 step 3 says so for multi-line fences).
            out.append(Block(text, "code-block", leak))
            continue
        layout: str | None
        if text.startswith("- [ ]"):
            layout, rest = "todo", inline_markdown(text[6:])  # blindly cuts 6
        elif text.startswith("- [x]"):
            leak = True
            layout, rest = "todo", inline_markdown(text[6:])
        elif text.startswith("- "):
            layout, rest = "bullets", inline_markdown(text[2:])  # not re-stripped
        elif text.startswith("#### "):
            # Undocumented: "#### x" -> layout p, name "<b> x</b>" (leading
            # space kept).
            layout, rest = "p", "<b>" + inline_markdown(text[4:]) + "</b>"
        elif text.startswith("### "):
            layout, rest = "h3", inline_markdown(text[4:])
        elif text.startswith("## "):
            layout, rest = "h2", inline_markdown(text[3:])
        elif text.startswith("# "):
            layout, rest = "h1", inline_markdown(text[2:])
        else:
            # Includes "> quote" (NOT quote-block, despite upstream docs),
            # "* x", "+ x", "1. x", "---", "- [X] x" (handled above as "- ").
            layout, rest = None, inline_markdown(text)
        out.append(Block(rest, layout, leak))
    return out
