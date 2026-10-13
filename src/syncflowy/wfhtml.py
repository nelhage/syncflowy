"""Workflowy's rich-text format for ``name`` and ``note`` ("WF-HTML").

Established by the human-in-the-loop UI scenarios (docs/workflowy/ui-findings.md,
E1–E8 and F1/F2). The web UI treats a name (and a note) as an **XML fragment**:

* text is escaped as ``&amp; &lt; &gt; &quot;`` (``'`` stays raw) -- E1/E2;
* formatting uses bare ``<b> <i> <u> <s> <code>``, links ``<a href="…">``,
  colours ``<span class="colored c-red">`` and dates ``<time …>`` -- E3/E5/E6;
* notes use the same format, with raw ``\\n`` line breaks -- E8;
* a name that is not well-formed XML renders as an **empty bullet**, and the
  next UI edit replaces it with just the newly typed text (data loss) -- F1/F2;
* a UI save re-serialises the name canonically: unknown tags are dropped but
  their text kept, ``<B>``/``<strong>`` become ``<b>``, comments vanish,
  ``"`` becomes ``&quot;``, ``javascript:`` links vanish with their text -- F step 2.

This module is shared by the mock server (UI simulation, ``xml_check``) and the
sync engine (anything it writes must satisfy :func:`is_well_formed`, ideally be
a fixed point of :func:`canonicalize`).

Choices not established by observation are marked ``UNVERIFIED``.
"""

from __future__ import annotations

import html
import re
import xml.parsers.expat
from dataclasses import dataclass, field

__all__ = [
    "escape_text",
    "is_well_formed",
    "canonicalize",
    "to_plain_text",
    "append_typed_text",
    "FORMAT_TAGS",
]

#: Tags the UI keeps verbatim (no attributes) when re-serialising.
FORMAT_TAGS = ("b", "i", "u", "s", "code")

#: Aliases the UI rewrites on save [O: F10 <strong> -> <b>].
#: UNVERIFIED: <em> -> <i> (by analogy; not observed).
TAG_ALIASES = {"strong": "b", "em": "i"}

#: Tags whose trailing run is extended when the user types at the very end of
#: the name [O: F01 <u>, F02 <span>, F09/F10 <b>]. UNVERIFIED for <i>, <s>,
#: <code> (by analogy) and deliberately excluded for <a> and <time> (editors
#: usually do not extend links; not observed).
_EXTENDING_TAGS = {"b", "i", "u", "s", "code", "span"}


def escape_text(text: str) -> str:
    """Escape plain text the way the UI stores it [O: E1].

    ``&`` ``<`` ``>`` ``"`` become entities; ``'`` stays raw.
    """
    return text.replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;").replace('"', "&quot;")


def _escape_attr(value: str) -> str:
    # [O: E5] the UI writes href="https://example.com/a?b=1&c=2" with a raw
    # "&" (while escaping it as &amp; in the link text). We keep "&" raw and
    # escape only what would break the attribute.
    return value.replace('"', "&quot;").replace("<", "&lt;")


# A bare "&" inside a start tag's attribute values that does not start an
# entity reference. The UI itself writes these (E5), so they must not make the
# fragment "malformed". UNVERIFIED: whether the UI also tolerates them in text
# (F13 suggests not: a raw "&" in text blanked the bullet).
_START_TAG_RE = re.compile(r"<[A-Za-z][^<>]*>")
_BARE_AMP_RE = re.compile(r"&(?![A-Za-z][A-Za-z0-9]*;|#[0-9]+;|#x[0-9A-Fa-f]+;)")


def _fix_attr_amps(s: str) -> str:
    return _START_TAG_RE.sub(lambda m: _BARE_AMP_RE.sub("&amp;", m[0]), s)


@dataclass
class _El:
    tag: str
    attrs: list[tuple[str, str]]
    children: list[_El | str] = field(default_factory=list)


def _parse(s: str) -> _El | None:
    """Parse ``s`` as an XML fragment; None if it is not well-formed.

    Entities: only XML's five built-ins plus numeric character references are
    accepted (expat rejects ``&nbsp;``). [O: F15 ``&nbsp;`` + ``&#39;`` was
    blank.] UNVERIFIED: whether numeric references (``&#39;``) alone are
    accepted by the UI -- F15 mixed them with ``&nbsp;``.
    """
    root = _El("#root", [])
    stack = [root]
    p = xml.parsers.expat.ParserCreate()
    p.ordered_attributes = True

    def start(tag: str, attrs: list[str]) -> None:
        el = _El(tag, list(zip(attrs[0::2], attrs[1::2])))
        stack[-1].children.append(el)
        stack.append(el)

    def end(tag: str) -> None:
        stack.pop()

    def chars(data: str) -> None:
        kids = stack[-1].children
        if kids and isinstance(kids[-1], str):
            kids[-1] += data
        else:
            kids.append(data)

    p.StartElementHandler = start
    p.EndElementHandler = end
    p.CharacterDataHandler = chars
    # Comments and processing instructions have no handler: dropped [O: F11].
    try:
        p.Parse("<_wfroot>" + _fix_attr_amps(s) + "</_wfroot>", True)
    except xml.parsers.expat.ExpatError:
        return None
    # Unwrap the synthetic root.
    assert len(root.children) == 1 and isinstance(root.children[0], _El)
    return root.children[0]


def is_well_formed(s: str) -> bool:
    """True if the UI can parse ``s`` (otherwise it renders an empty bullet).

    [O: F05 ``<br>``, F06 ``<img …>``, F08 unclosed ``<b>``, F13 raw ``<``/``&``
    in text, F15 ``&nbsp;`` -> blank.] Well-formed unknown markup is fine
    (F03/F04/F12 rendered their text).
    """
    return _parse(s) is not None


def _serialize(children: list[_El | str], out: list[str]) -> None:
    for c in children:
        if isinstance(c, str):
            out.append(escape_text(c))
            continue
        tag = TAG_ALIASES.get(c.tag.lower(), c.tag.lower())  # [O: F09 <B> -> <b>]
        attrs = {k.lower(): v for k, v in c.attrs}
        if tag in FORMAT_TAGS:
            # Attributes dropped. UNVERIFIED (never observed with attributes).
            out.append(f"<{tag}>")
            _serialize(c.children, out)
            out.append(f"</{tag}>")
        elif tag == "a":
            href = attrs.get("href")
            if href is not None and href.strip().lower().startswith("javascript:"):
                continue  # [O: F07] link and its text both dropped
            if href is None:
                _serialize(c.children, out)  # UNVERIFIED: <a> without href unwrapped
                continue
            out.append(f'<a href="{_escape_attr(href)}">')  # [O: E5] only href kept
            _serialize(c.children, out)
            out.append("</a>")
        elif tag == "span" and attrs.get("class", "").split()[:1] == ["colored"]:
            # [O: F02] colour spans survive. UNVERIFIED: other span classes
            # (e.g. highlights) -- we keep any class starting with "colored".
            out.append(f'<span class="{_escape_attr(attrs["class"])}">')
            _serialize(c.children, out)
            out.append("</span>")
        elif tag == "time":
            # Dates (calendar day names, [YYYY-MM-DD] conversions). UNVERIFIED:
            # assumed to survive a UI save with all attributes, in order.
            attr_s = "".join(f' {k}="{_escape_attr(v)}"' for k, v in c.attrs)
            out.append(f"<time{attr_s}>")
            _serialize(c.children, out)
            out.append("</time>")
        else:
            # [O: F03 <script>, F04 <div>, F12 <mark>] unknown tag dropped,
            # text kept. Void tags (<br/>, <img/>) therefore vanish.
            _serialize(c.children, out)


def canonicalize(s: str) -> str:
    """What the UI stores after any edit of a node whose current value is ``s``.

    Malformed input becomes ``""`` [O: F05/F06/F08/F13/F15 -> only the newly
    typed text survived].
    """
    tree = _parse(s)
    if tree is None:
        return ""
    out: list[str] = []
    _serialize(tree.children, out)
    return "".join(out)


def _text_of(children: list[_El | str], out: list[str]) -> None:
    for c in children:
        if isinstance(c, str):
            out.append(c)
        else:
            _text_of(c.children, out)


_TAG_RE = re.compile(r"<[^>]*>")


def to_plain_text(s: str) -> str:
    """The visible text of a name or note, entities decoded.

    For malformed input (which the UI shows as empty) this returns a
    best-effort text (tags stripped with a regex, entities decoded) rather
    than ``""``, so that callers can still show/salvage the content; use
    :func:`is_well_formed` to detect that case.
    """
    tree = _parse(s)
    if tree is None:
        return html.unescape(_TAG_RE.sub("", s))
    out: list[str] = []
    _text_of(tree.children, out)
    return "".join(out)


_TRAILING_CLOSERS_RE = re.compile(r"(?:</([a-z]+)>)+$")


def append_typed_text(s: str, typed: str) -> str:
    """Simulate typing plain text ``typed`` at the very end of a name in the UI.

    The current value is first canonicalised (a malformed name is treated as
    empty [O: F step 2]); the escaped text is placed *inside* any trailing
    formatting run [O: F01 ``<u>underline z</u>``, F02, F09, F10].
    """
    base = canonicalize(s)
    esc = escape_text(typed)
    m = _TRAILING_CLOSERS_RE.search(base)
    if m:
        closers = re.findall(r"</([a-z]+)>", m[0])
        # Extend only through a suffix of extending tags.
        k = len(closers)
        while k > 0 and closers[k - 1] in _EXTENDING_TAGS:
            k -= 1
        # closers[k:] are all extending; insert before them.
        split = m.start() + sum(len(f"</{t}>") for t in closers[:k])
        if k < len(closers):
            return base[:split] + esc + base[split:]
    return base + esc
