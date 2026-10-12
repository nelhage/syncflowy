"""Normalise raw transcripts so production and mock runs are comparable.

The normaliser is a pure function of a *raw* transcript (as produced by
:class:`~.context.Ctx` plus run metadata). Golden files store the raw
transcript, so improvements to the normaliser apply to existing goldens
without re-recording (``check`` always re-normalises the golden's raw part).

What is normalised
------------------

**Node ids.** Every UUID-looking token, anywhere (request path, params, body,
raw content, response body, error messages, ``item_id``/``origin_id``,
``Location '…' not found`` echoes), is replaced by a symbol assigned by order
of first appearance: ``<N1>``, ``<N2>``, ... The sandbox root is ``<ROOT>``
and its parent (scratchpad) is ``<SANDBOX_PARENT>``; ``ctx.alias(id, "P")``
names a node ``<P>``. The *form* of each reference is preserved:

====================  ======================
``<uuid>``            ``<N1>``
``<UUID>``            ``<N1:UPPER>``
mixed case            ``<N1:MIXED>``
dashless 32-hex       ``<N1:dashless>`` (+ case suffix)
12-hex short id       ``<N1:short>`` / ``<N1:short:UPPER>`` / ``<N1:short:MIXED>``
====================  ======================

A 12-hex token is only rewritten if it is the suffix of an id already seen
(otherwise it is literal content, e.g. a deliberately fake short id).

**Timestamps.** ``createdAt``/``modifiedAt``/``completedAt`` of every node
object are replaced by the *epoch* in which the server produced them:
``<E0>`` (sandbox creation), ``<E1>`` (first scenario epoch), ... Epochs are
separated by ``ctx.sleep()`` (>= 2 s), and each epoch's possible server
timestamps are ``floor(t + skew ± slack)`` over the backend-clock window of
its requests. For production the client/server clock skew is estimated from
the ``Date`` response headers; for the fake-clock mock it is 0. This makes
"was modifiedAt bumped by step X?" comparable exactly, while tolerating the
arbitrary second-boundary crossings that happen *within* an epoch. A value
that falls in no epoch window renders as ``<E?+N>`` (N seconds after the
sandbox was created) and a value in two windows as ``<E1|E2>``; both indicate
a harness problem (or a server bug) and will show up in diffs.

**The sandbox root's priority** (where the sandbox landed under the
scratchpad) is replaced with ``<ROOT_PRIORITY>``.

**HTML documents** (production's GET 404 page) are collapsed to
``<HTML page title='404'>``: the page carries a per-response CSP nonce and
deploy-specific asset hashes (see :func:`summarize_html`).

**Response metadata** kept for comparison: status, ``content-type``,
``allow`` and ``www-authenticate`` headers, and whether a JSON body is
byte-identical to Python ``json.dumps(parsed)`` (``"json_bytes":
"python-default"``; production is, so the mock must use the same separators
and ``ensure_ascii``). Everything else (``date``, ``server``, ...) is dropped.

List order is **kept**: production lists children in creation order
deterministically (data-model.md §4.2), so order is part of the behaviour.
"""

from __future__ import annotations

import email.utils
import json
import math
import re
from typing import Any

from .backend import HEX_TOKEN_RE

TS_KEYS = ("createdAt", "modifiedAt", "completedAt")
COMPARED_HEADERS = ("content-type", "allow", "www-authenticate")


def _dash(hex32: str) -> str:
    h = hex32.lower()
    return f"{h[:8]}-{h[8:12]}-{h[12:16]}-{h[16:20]}-{h[20:]}"


def _case_suffix(tok: str) -> str:
    letters = [c for c in tok if c.isalpha()]
    if not letters or tok == tok.lower():
        return ""
    if tok == tok.upper():
        return ":UPPER"
    return ":MIXED"


class Symbols:
    """Assigns stable symbolic names to UUIDs by first appearance."""

    def __init__(self, seed: dict[str, str] | None = None, aliases: dict[str, str] | None = None):
        self.names: dict[str, str] = {}
        self.aliases = {k.lower(): v for k, v in (aliases or {}).items()}
        self.counter = 0
        for u, n in (seed or {}).items():
            if u:
                self.names[u.lower()] = n

    def sym(self, uuid: str) -> str:
        u = uuid.lower()
        if u not in self.names:
            if u in self.aliases:
                self.names[u] = self.aliases[u]
            else:
                self.counter += 1
                self.names[u] = f"N{self.counter}"
        return self.names[u]

    def short_lookup(self, short: str) -> str | None:
        s = short.lower()
        for u, n in self.names.items():
            if u.replace("-", "").endswith(s):
                return n
        for u, n in self.aliases.items():
            if u.replace("-", "").endswith(s):
                return self.sym(u)
        return None

    def replace(self, s: str) -> str:
        def sub(m: re.Match) -> str:
            tok = m.group(1)
            suffix = _case_suffix(tok)
            if len(tok) == 36:
                return f"<{self.sym(tok)}{suffix}>"
            if len(tok) == 32:
                return f"<{self.sym(_dash(tok))}:dashless{suffix}>"
            name = self.short_lookup(tok)
            if name is None:
                return tok
            return f"<{name}:short{suffix}>"

        return HEX_TOKEN_RE.sub(sub, s)


class Clock:
    """Maps server timestamps to epoch labels."""

    def __init__(self, raw: dict):
        self.slack = 0.3
        self.skew = 0.0
        self.skew_interval = None
        if raw.get("clock_kind") != "fake":
            lo, hi = -math.inf, math.inf
            for st in raw["steps"]:
                if st.get("type") != "request":
                    continue
                d = (st.get("response_headers") or {}).get("date")
                if not d:
                    continue
                try:
                    D = email.utils.parsedate_to_datetime(d).timestamp()
                except Exception:
                    continue
                lo = max(lo, D - st["t1"])
                hi = min(hi, D + 1 - st["t0"])
            if lo <= hi and math.isfinite(lo) and math.isfinite(hi):
                self.skew_interval = (lo, hi)
                self.skew = (lo + hi) / 2
                self.slack = min(0.9, 0.3 + (hi - lo) / 2)
        windows: dict[int, list[float]] = {}
        sw = raw.get("setup_window")
        if sw:
            windows[0] = [sw[0], sw[1]]
        for st in raw["steps"]:
            if st.get("type") != "request":
                continue
            w = windows.setdefault(st["epoch"], [st["t0"], st["t1"]])
            w[0] = min(w[0], st["t0"])
            w[1] = max(w[1], st["t1"])
        self.ranges = {
            e: (
                math.floor(a + self.skew - self.slack),
                math.floor(b + self.skew + self.slack),
            )
            for e, (a, b) in sorted(windows.items())
        }
        items = list(self.ranges.items())
        self.overlaps = [
            (e1, e2) for (e1, r1), (e2, r2) in zip(items, items[1:]) if r1[1] >= r2[0]
        ]
        self.base = self.ranges[0][0] if 0 in self.ranges else (
            min(r[0] for r in self.ranges.values()) if self.ranges else 0
        )

    def label(self, ts: Any) -> Any:
        if not isinstance(ts, (int, float)) or isinstance(ts, bool):
            return ts
        hits = [e for e, (lo, hi) in self.ranges.items() if lo <= ts <= hi]
        if len(hits) == 1:
            return f"<E{hits[0]}>"
        if hits:
            return "<" + "|".join(f"E{e}" for e in hits) + ">"
        return f"<E?{int(ts - self.base):+d}>"


_HTML_DOC_RE = re.compile(r"^\s*<!DOCTYPE html", re.I)
_HTML_TITLE_RE = re.compile(r"<title>(.*?)</title>", re.I | re.S)


def summarize_html(text: str) -> str:
    """Collapse a full HTML document to ``<HTML page title='…'>``.

    Production's GET 404 page is a ~11 KB marketing page that embeds a fresh
    CSP nonce (``<script nonce="…">``) on every response and content-hashed
    asset URLs that change on every deploy, so its bytes are not
    reproducible. Only "it is an HTML document" and its ``<title>`` are
    behaviour (data-model.md §2.1). Empty bodies and non-document text are
    returned unchanged.
    """
    if not _HTML_DOC_RE.match(text):
        return text
    m = _HTML_TITLE_RE.search(text)
    title = m.group(1).strip() if m else None
    return f"<HTML page title={title!r}>"


def _is_node(d: dict) -> bool:
    return "id" in d and "createdAt" in d and "modifiedAt" in d and "priority" in d


class Normalizer:
    def __init__(self, raw: dict):
        aliases = {
            st["id"]: st["name"]
            for st in raw["steps"]
            if st.get("type") == "alias" and isinstance(st.get("id"), str)
        }
        self.root = (raw.get("root") or "").lower()
        self.syms = Symbols(
            seed={raw.get("root"): "ROOT", raw.get("sandbox_parent"): "SANDBOX_PARENT"},
            aliases=aliases,
        )
        self.clock = Clock(raw)
        self.raw = raw

    def value(self, v: Any, response: bool = True) -> Any:
        """Normalise ids everywhere; timestamps/root priority only in responses."""
        if isinstance(v, str):
            return self.syms.replace(v)
        if isinstance(v, list):
            return [self.value(x, response) for x in v]
        if isinstance(v, dict):
            node = response and _is_node(v)
            is_root = node and isinstance(v.get("id"), str) and v["id"].lower() == self.root
            out = {}
            for k, x in v.items():
                if node and k in TS_KEYS:
                    out[k] = self.clock.label(x)
                elif is_root and k == "priority":
                    out[k] = "<ROOT_PRIORITY>"
                else:
                    out[k] = self.value(x, response)
            return out
        return v

    def step(self, st: dict) -> dict | None:
        t = st.get("type")
        if t == "alias":
            return None
        if t == "note":
            return {"note": st["text"]}
        if t == "sleep":
            return {"sleep": f"-> E{st['epoch']}"}
        if t == "error":
            return {"error": self.value(st["message"])}
        if t != "request":
            return {"unknown": t}
        out: dict[str, Any] = {"request": f"{st['method']} {self.value(st['path'])}"}
        for k in ("params", "json", "content", "headers"):
            if st.get(k) is not None:
                out[k] = self.value(st[k], response=False)
        out["status"] = st["status"]
        rh = {k.lower(): v for k, v in (st.get("response_headers") or {}).items()}
        for h in COMPARED_HEADERS:
            if h in rh:
                out[h] = rh[h]
        text = st.get("body_text") or ""
        if st.get("body_is_json"):
            parsed = json.loads(text)
            out["json_bytes"] = (
                "python-default" if text == json.dumps(parsed) else "non-default"
            )
            out["body"] = self.value(parsed)
        else:
            out["text"] = self.value(summarize_html(text))
        return out

    def run(self) -> list[dict]:
        out = []
        for st in self.raw["steps"]:
            n = self.step(st)
            if n is not None:
                out.append(n)
        return out


def normalize(raw: dict) -> list[dict]:
    return Normalizer(raw).run()


def render(steps: list[dict]) -> list[str]:
    """Render a normalised transcript as diff-friendly text lines."""
    lines: list[str] = []
    for i, st in enumerate(steps):
        head = st.get("request") or (
            f"sleep {st['sleep']}" if "sleep" in st else ("note" if "note" in st else "error")
        )
        lines.append(f"## [{i}] {head}")
        for k, v in st.items():
            if k == "request" or (k in ("sleep",) and len(st) == 1):
                continue
            dumped = json.dumps(v, indent=2, ensure_ascii=False)
            dl = dumped.splitlines()
            lines.append(f"  {k}: {dl[0]}")
            lines.extend("    " + x for x in dl[1:])
    return lines
