"""Endpoint semantics of the mock Workflowy API, independent of HTTP.

Each public method of :class:`Ops` implements one endpoint after routing,
method and auth checks and body parsing (data-model.md §2.2 steps 1-4) have
been done by :mod:`syncflowy.mock.app`. Methods run the remaining steps in
production's order:

5. schema validation (``400 {"errors": [...]}``)
6. path-id lookup (``404 Item not found``), for mirror also origin resolution
7. ``parent_id`` lookup (``404 Parent item not found`` / ``Location '…' not found``)
8. semantic checks (``400 invalid_request``)

and then perform the change inside one serialised transaction.

Section references (§x.y) are to docs/workflowy/data-model.md.
"""

from __future__ import annotations

import copy
import datetime
import random
import re
import time
import uuid
from collections.abc import Callable
from dataclasses import dataclass, field
from typing import Any

from .. import wfhtml
from . import models
from .models import (
    CreateItemRequest,
    MirrorNodeRequest,
    MoveNodeRequest,
    UpdateNodeRequest,
    position_str,
)
from .store import Node, Store, Tx
from .text import format_time_elem, has_lone_surrogate, strip_update_text, transform_create_name


class ApiError(Exception):
    """An error response with a JSON body."""

    def __init__(self, status: int, body: Any, headers: dict[str, str] | None = None):
        super().__init__(status, body)
        self.status = status
        self.body = body
        self.headers = headers or {}


def not_found(message: str) -> ApiError:
    return ApiError(404, {"code": "not_found", "message": message})


def invalid_request(message: str) -> ApiError:
    return ApiError(400, {"code": "invalid_request", "message": message})


def internal_error() -> ApiError:
    return ApiError(500, {"code": "internal_error", "message": "An internal server error occurred"})


ITEM_NOT_FOUND = "Item not found"
PARENT_NOT_FOUND = "Parent item not found"
OK = {"status": "ok"}

# §3.2 resolver regexes. fullmatch() is used so that a trailing "\n" never
# sneaks past a "$" anchor.
UUID_RE = re.compile(r"[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}", re.I)
# Prefix match. The 12 hex digits must be followed by the end of the string,
# "/" or "?" -- anything else ("-x", "#y", ".", "g", "_", " ", "&", another
# hex digit) is a Location miss [O: golden ids_url_parent_boundaries, which
# answered open question P6; the earlier guess only rejected a hex digit].
URL_RE = re.compile(r"https?://(beta\.)?workflowy\.com/#/([0-9a-f]{12})(?=[/?]|$)", re.I)
SHORT_RE = re.compile(r"[0-9a-f]{12}", re.I)

# Mock-specific (NOT production) error for a move into the node's own subtree.
# Production returns 200 and creates a parent cycle (§4.7 step 5).
CYCLE_MESSAGE = "Cannot move a node under its own descendant"

# beta_mirrors: deleting a subtree that holds an origin with live mirrors
# [O: experiments/beta-mirrors/01_probe].
HAS_MIRRORS_MESSAGE = "Cannot delete a node that has mirrors"


class _Root:
    """The top level of the outline. Its children have ``parent_id: null``."""

    id = None

    def __repr__(self) -> str:
        return "ROOT"


ROOT = _Root()
Parent = Node | _Root


@dataclass
class Config:
    #: Reproduce production's acceptance of moves into a descendant (creating
    #: a parent cycle) instead of rejecting them with a mock-specific 400.
    faithful_cycles: bool = False
    #: Minimum seconds between nodes-export calls per API key. 0 (default)
    #: disables the limit: upstream documents 1 req/min, but production did
    #: not enforce it (two 200s 10 s apart, 2026-10-12).
    export_min_interval: float = 0.0
    #: Weekday the week starts on (0 = Monday, 6 = Sunday), used by
    #: "next_week" and the date-key week computation. A per-account setting
    #: upstream; the default 6 is inferred from the observed 500 window
    #: 0001-01-01..06 (global-readonly.md).
    week_start: int = 6
    #: MOCK-ONLY well-formedness check of names/notes written through the API
    #: (ui-findings F1/F2: the UI renders malformed XML as an empty bullet and
    #: destroys it on the next edit). "off": nothing; "warn" (default): record
    #: a warning, readable at GET /_mock/warnings -- API responses unchanged;
    #: "reject": a mock-specific 400 (production accepts anything).
    xml_check: str = "warn"
    #: Mirror-aware semantics of the beta API (beta.workflowy.com, announced
    #: 2026-07-22; data-model.md §10.5). Off (default): production's observed
    #: behaviour, where a shell is an independent empty node. On: reads expose
    #: ``data.mirror`` and show the origin's content through a shell, writes
    #: through a shell reach the origin, and deleting an origin that still has
    #: mirrors fails. Observed on the beta host 2026-10-13
    #: (experiments/beta-mirrors/).
    beta_mirrors: bool = False

    XML_CHECK_MODES = ("off", "warn", "reject")

    def to_json(self) -> dict[str, Any]:
        return {
            "faithful_cycles": self.faithful_cycles,
            "export_min_interval": self.export_min_interval,
            "week_start": self.week_start,
            "xml_check": self.xml_check,
            "beta_mirrors": self.beta_mirrors,
        }


@dataclass
class Clock:
    """Wall clock with an adjustable offset (used by the /_mock/clock admin route)."""

    base: Callable[[], float] = time.time
    offset: float = 0.0

    def __call__(self) -> float:
        return self.base() + self.offset


class SeededIds:
    """Deterministic UUIDv4 generator for reproducible tests (``id_seed``).

    Ids are well-formed lowercase UUIDv4 strings drawn from ``random.Random(seed)``;
    :meth:`reset` restarts the sequence (``POST /_mock/reset``).
    """

    def __init__(self, seed: int):
        self.seed = seed
        self.reset()

    def reset(self) -> None:
        self._rng = random.Random(self.seed)

    def __call__(self) -> str:
        return str(uuid.UUID(int=self._rng.getrandbits(128), version=4))


class _EmptyLocation(Exception):
    """A special location that exists conceptually but has no node yet and
    lists as empty (the never-materialised inbox)."""


#: Namespace for the mock's deterministic (UUIDv5) calendar node ids.
_SYSTEM_NS = uuid.UUID("6b1d5c2e-58a4-4f43-9d0e-7f3c2a1b9e10")


@dataclass
class _SpecialStep:
    system_key: str
    name: str
    position: str  # "top" / "bottom" (assign_priority) or "calendar" (chronological)


@dataclass
class Ops:
    store: Store
    clock: Callable[[], float] = time.time
    config: Config = field(default_factory=Config)
    #: Id generator. None -> random uuid4; a :class:`SeededIds` makes ids reproducible.
    id_factory: Callable[[], str] | None = None
    _export_last: dict[str, float] = field(default_factory=dict)
    #: xml_check="warn" findings, oldest first (GET /_mock/warnings).
    warnings: list[dict[str, Any]] = field(default_factory=list)

    def __post_init__(self) -> None:
        # Startup config, restored by reset(full=True).
        self._initial_config = copy.copy(self.config)

    def now(self) -> int:
        # §1.1: int(time.time()) -- floor of wall-clock seconds.
        return int(self.clock())

    # ------------------------------------------------------------------ helpers

    def _new_id(self) -> str:
        if self.id_factory is not None:
            return self.id_factory()
        return str(uuid.uuid4())

    def assign_priority(self, tx: Tx, parent: Parent, position: str | None, exclude: str | None = None) -> int:
        """§7: shared by create, move (incl. same-parent) and mirror."""
        others = [n for n in tx.children(parent.id) if n.id != exclude]  # mirror shells included
        if not others:
            return 100
        if position == "bottom":
            # ceil((max + 100) / 100) * 100. UNVERIFIED (open-questions P10):
            # whether very large bottom priorities ever trigger a renumber.
            return -(-(max(n.priority for n in others) + 100) // 100) * 100
        # "top", null or absent
        m = min(n.priority for n in others)
        if m > 0:
            return m // 2
        # m == 0 (or negative, which the API cannot produce): full renumber.
        # Renumbered siblings keep their modifiedAt (§8). The tie-break
        # (priority, create_seq) is UNVERIFIED (open-questions D4): ties
        # cannot be produced through the API.
        for rank, n in enumerate(sorted(others, key=lambda n: (n.priority, n.create_seq))):
            tx.set_fields(n.id, priority=100 * (rank + 2))
        return 100

    @staticmethod
    def _follow_mirror(tx: Tx, node: Node) -> Node | None:
        """Resolve a node to its mirror origin (itself if it is not a shell).

        Returns None for an orphan (origin deleted). Chains cannot form
        through the API, but we follow them anyway, guarding against loops.
        """
        seen = set()
        while node.mirror_origin_id is not None:
            if node.id in seen:
                return None
            seen.add(node.id)
            nxt = tx.get(node.mirror_origin_id)
            if nxt is None:
                return None
            node = nxt
        return node

    def _through_mirror(self, tx: Tx, node: Node, miss: str) -> Node:
        """beta_mirrors: the node whose content a write through ``node`` reaches.

        A shell resolves to its origin, and a non-shell to itself. An orphan
        shell refuses the write with ``404 miss`` [O: beta-mirrors 01/02:
        update, complete, uncomplete -> Item not found; create under it ->
        Parent item not found]. Without beta_mirrors, always ``node``.
        """
        if not self.config.beta_mirrors or node.mirror_origin_id is None:
            return node
        origin = self._follow_mirror(tx, node)
        if origin is None:
            raise not_found(miss)
        return origin

    def _to_api(self, tx: Tx, node: Node, parent_id: str | None = None) -> dict[str, Any]:
        """Serialise ``node`` for GET, list and export (§1.2; §10.5 with beta_mirrors).

        ``parent_id`` overrides the reported parent (an origin's child listed
        under a mirror root reports that root as its parent).

        With beta_mirrors [O: beta-mirrors 01/02], a shell reports its own
        ``id``, ``parent_id``, ``priority`` and ``createdAt`` and its origin's
        ``name``, ``note``, ``layoutMode``, completion and ``modifiedAt``,
        plus ``data.mirror.origin_id``; an orphan shell falls back to its own
        row with ``origin_id: null``. A node with a non-empty mirror registry
        carries ``data.mirror.mirror_ids``, **sorted by id** (not creation
        order). ``mirror`` follows ``layoutMode`` inside ``data``.

        ``mirror_ids`` is the origin's registry (``mirror_registry``), not a
        reverse lookup of the shells: it can hold ghosts (shells deleted
        through the production host) and miss live shells (after a UI
        delete-promotion) [O: beta-mirrors 04-06].
        """
        d = node.to_api()
        if parent_id is not None:
            d["parent_id"] = parent_id
        if not self.config.beta_mirrors:
            return d
        if node.mirror_origin_id is not None:
            origin = self._follow_mirror(tx, node)
            if origin is None:
                d["data"]["mirror"] = {"origin_id": None}
                return d
            d.update(
                name=origin.name,
                note=origin.note,
                completed=origin.completed,
                data={"layoutMode": origin.layout_mode, "mirror": {"origin_id": origin.id}},
                modifiedAt=origin.modified_at,
                completedAt=origin.completed_at,
            )
            return d
        if mirror_ids := tx.registered_mirrors(node.id):
            d["data"]["mirror"] = {"mirror_ids": mirror_ids}
        return d

    @staticmethod
    def _is_descendant(tx: Tx, candidate: Node, ancestor: Node) -> bool:
        """True if ``candidate`` lies in ``ancestor``'s subtree (walks up parent links)."""
        seen: set[str] = set()
        cur: Node | None = candidate
        while cur is not None and cur.id not in seen:
            if cur.id == ancestor.id:
                return True
            seen.add(cur.id)
            cur = tx.get(cur.parent_id) if cur.parent_id is not None else None
        return False

    def _xml_check(self, op: str, node_id: str, values: list[tuple[str, str | None]]) -> None:
        """Apply ``config.xml_check`` to the *stored* values of a write.

        Called after all production checks (validation, lookups) and before
        anything is written, so "reject" leaves the outline unchanged.
        """
        mode = self.config.xml_check
        if mode == "off":
            return
        bad = [(f, v) for f, v in values if v is not None and not wfhtml.is_well_formed(v)]
        if not bad:
            return
        if mode == "reject":
            # MOCK-SPECIFIC (not production): production stores anything.
            raise ApiError(
                400,
                {
                    "code": "mock_malformed_xml",
                    "message": "Mock xml_check=reject: "
                    + ", ".join(f for f, _ in bad)
                    + " is not well-formed WF-HTML (the Workflowy UI would show an empty bullet)",
                },
            )
        for f, v in bad:
            self.warnings.append({"op": op, "node_id": node_id, "field": f, "value": v, "time": self.now()})

    # ------------------------------------------------------ parent resolution

    # Special parent_id keys (§11). Observed 2026-10-12 with read-only GETs
    # (docs/workflowy/global-readonly.md); materialisation through create /
    # move could not be observed and is inferred.
    _DATE_KEY_RE = re.compile(r"([0-9]{4})(?:-([0-9]{2})(?:-([0-9]{2}))?)?")
    _FIXED_KEYS = ("inbox", "calendar", "today", "tomorrow", "next_week")

    def is_special_key(self, key: str) -> bool:
        return key in self._FIXED_KEYS or self._DATE_KEY_RE.fullmatch(key) is not None

    def _special_chain(self, key: str, *, for_list: bool) -> list[_SpecialStep] | None:
        """The chain of system nodes a special key denotes, root first.

        Returns None if ``key`` is not a special key. Raises 500 for the
        failure modes observed in production:

        * a date key whose week (per ``week_start``) starts before
          0001-01-01 (e.g. ``0001``, ``0001-01``, ``0001-01-01`` ..
          ``0001-01-06`` with a Sunday week start): 500 on GET and list.
        * list / create / move only (``for_list``): an impossible date
          (``2026-13``, ``2026-02-30``, ``0000``) and any key in year 9999
          give 500. GET answers those with ``404 Item not found`` instead,
          which the caller handles by getting ``None`` back.

        Observed names (§11): ``📆 Calendar`` (top level, appended at the
        bottom), year ``2026``, month ``07`` (two digits), day
        ``<time startYear="2026" startMonth="7" startDay="5">Sun, Jul 5,
        2026</time>``. Calendar children are kept in chronological order
        (see :meth:`_calendar_priority`). The inbox name ``Inbox`` and its
        top placement are UNVERIFIED (the account has never materialised
        its inbox).
        """
        if key == "inbox":
            return [_SpecialStep("inbox", "Inbox", "top")]
        cal = [_SpecialStep("calendar", "\U0001f4c6 Calendar", "bottom")]
        if key == "calendar":
            return cal
        level = 3
        # UNVERIFIED (open-questions S2): today / tomorrow / next_week use the
        # mock clock in UTC; production presumably uses the account timezone.
        today = datetime.datetime.fromtimestamp(self.now(), datetime.UTC).date()
        if key == "today":
            date = today
        elif key == "tomorrow":
            date = today + datetime.timedelta(days=1)
        elif key == "next_week":
            days = (self.config.week_start - today.weekday()) % 7 or 7
            date = today + datetime.timedelta(days=days)
        elif m := self._DATE_KEY_RE.fullmatch(key):
            level = 1 if m[2] is None else 2 if m[3] is None else 3
            try:
                date = datetime.date(int(m[1]), int(m[2] or 1), int(m[3] or 1))
            except ValueError:
                if for_list:
                    raise internal_error() from None
                return None
            # [I] fitted to the observed 500 window 0001-01-01..06 (Sunday
            # week start): the resolver computes the start of the date's week.
            if date.toordinal() - (date.weekday() - self.config.week_start) % 7 < 1:
                raise internal_error()
            if for_list and date.year == 9999:
                raise internal_error()
        else:
            return None
        chain = cal + [_SpecialStep(f"calendar:{date.year:04d}", str(date.year), "calendar")]
        if level >= 2:
            chain.append(_SpecialStep(f"calendar:{date.year:04d}-{date.month:02d}", f"{date.month:02d}", "calendar"))
        if level >= 3:
            chain.append(_SpecialStep(f"calendar:{date.isoformat()}", format_time_elem(date), "calendar"))
        return chain

    def _calendar_priority(self, tx: Tx, parent: Parent, system_key: str) -> int:
        """Priority of a new year / month / day node: chronological among its
        calendar siblings [I: the UI-made calendar holds days 5..9 at
        100..500, then the 16th at 600 and the 15th at 550 -- bottom rule
        after the last, integer midpoint between neighbours otherwise]."""
        siblings = tx.children(parent.id)
        cal = sorted(
            (n for n in siblings if (n.system_key or "").startswith("calendar:")),
            key=lambda n: n.system_key or "",
        )
        before = [n for n in cal if (n.system_key or "") < system_key]
        after = [n for n in cal if (n.system_key or "") > system_key]
        if not after:
            return self.assign_priority(tx, parent, "bottom")
        hi = after[0].priority
        lo = before[-1].priority if before else 0
        if hi - lo >= 2:
            return (lo + hi) // 2
        # No integer gap: renumber the siblings (creation-independent, by
        # priority) and retry. Cannot be produced in practice.
        for rank, n in enumerate(sorted(siblings, key=lambda n: (n.priority, n.create_seq))):
            tx.set_fields(n.id, priority=100 * (rank + 1))
        return self._calendar_priority(tx, parent, system_key)

    def _materialise(self, tx: Tx, raw: str, chain: list[_SpecialStep], materialise: bool) -> Node:
        parent: Parent = ROOT
        node: Node | None = None
        for step in chain:
            node = tx.get_by_system_key(step.system_key)
            if node is None:
                if not materialise:
                    if step.system_key == "inbox":
                        # [O] listing a never-materialised inbox gives 200 [].
                        raise _EmptyLocation()
                    # [O] e.g. list ?parent_id=today with no day node yet.
                    raise not_found(f"Location '{raw}' not found")
                now = self.now()
                if step.position == "calendar":
                    priority = self._calendar_priority(tx, parent, step.system_key)
                else:
                    priority = self.assign_priority(tx, parent, step.position)
                node = tx.insert(
                    # Production's calendar ids are UUIDv5 (deterministic) [O];
                    # we derive ours from the system key.
                    node_id=str(uuid.uuid5(_SYSTEM_NS, step.system_key)) if step.system_key != "inbox" else self._new_id(),
                    parent_id=parent.id,
                    name=step.name,
                    note=None,
                    priority=priority,
                    completed=False,
                    completed_at=None,
                    layout_mode="bullets",
                    now=now,
                    system_key=step.system_key,
                )
            parent = node
        assert node is not None
        return node

    def resolve_parent(self, tx: Tx, raw: str | None, *, materialise: bool) -> Parent:
        """``parent_id`` resolution for list, create and move (§3.2)."""
        if raw is None:
            # [O] list with no parent_id -> the top level. [I] create / move
            # with parent_id omitted or null (same resolver).
            return ROOT
        if UUID_RE.fullmatch(raw):
            # uuid-shaped (any case), but the lookup is exact and
            # case-sensitive: an UPPERCASE UUID always misses.
            node = tx.get(raw)
            if node is None:
                raise not_found(PARENT_NOT_FOUND)
            return node
        if m := URL_RE.match(raw):
            node = tx.get_by_short(m[2].lower())
            if node is None:
                raise not_found(PARENT_NOT_FOUND)
            return node
        # Special keys and user shortcuts are checked here, between the URL
        # form and the bare short-id form; the relative order cannot be
        # observed (no special key is URL- or short-id-shaped).
        if raw == "None":
            return ROOT  # [O] case-sensitive: "none", "null" -> Location
        if (chain := self._special_chain(raw, for_list=True)) is not None:
            return self._materialise(tx, raw, chain, materialise)
        if has_lone_surrogate(raw):
            # A JSON lone surrogate cannot be a shortcut key or a short id:
            # production answers with a plain Location miss (golden
            # ids_parent_id_lone_surrogate), unlike mirror's exact lookup (500).
            raise not_found(f"Location '{raw}' not found")
        if (target := tx.shortcut(raw)) is not None:
            node = tx.get(target)
            if node is not None:
                return node
            # UNVERIFIED (S3): a shortcut whose node was deleted -> Location miss.
            raise not_found(f"Location '{raw}' not found")
        if SHORT_RE.fullmatch(raw):
            node = tx.get_by_short(raw.lower())
            if node is not None:
                return node
        # Includes "" [O: list ?parent_id= -> Location '' not found].
        raise not_found(f"Location '{raw}' not found")

    @staticmethod
    def _lookup_exact(tx: Tx, node_id: str) -> Node:
        """Path ``:id`` for every endpoint except GET: exact lowercase UUID only (§3.1)."""
        node = tx.get(node_id)
        if node is None:
            raise not_found(ITEM_NOT_FOUND)
        return node

    # --------------------------------------------------------------- endpoints

    def get_node(self, node_id: str) -> dict[str, Any]:
        """GET /nodes/:id (§3.1, §4.1): exact UUID, a 12-hex short id in any
        case, or a special key whose node already exists (never materialises).

        [O] ``calendar``, ``2026``, ``2026-07``, ``2026-07-05`` resolve when
        their node exists; a missing or impossible date, ``None`` and an
        unmaterialised inbox give ``404 Item not found``; a week underflow
        (``0001-01-01``) gives 500.
        """
        with self.store.transaction() as tx:
            node = tx.get(node_id)
            if node is None and SHORT_RE.fullmatch(node_id):
                node = tx.get_by_short(node_id.lower())
            if node is None and (chain := self._special_chain(node_id, for_list=False)) is not None:
                node = tx.get_by_system_key(chain[-1].system_key)
            if node is None and (target := tx.shortcut(node_id)) is not None:
                # UNVERIFIED (S3): shortcut keys on GET (the account has none).
                node = tx.get(target)
            if node is None:
                raise not_found(ITEM_NOT_FOUND)
            return {"node": self._to_api(tx, node)}

    def list_children(self, parent_id: str | None) -> dict[str, Any]:
        """GET /nodes?parent_id= -- depth 1, completed included, creation order (§4.2).

        With beta_mirrors [O: beta-mirrors 01/02], listing a mirror root
        returns only its origin's children, reported under the root, plus a
        top-level ``mirror: {"origin_id": ...}``. The root's own persisted
        children (moved in, or made through production) are not listed. An
        orphan lists as ``{"nodes": []}``, without the ``mirror`` key.
        """
        with self.store.transaction() as tx:
            try:
                parent = self.resolve_parent(tx, parent_id, materialise=False)
            except _EmptyLocation:
                return {"nodes": []}
            if not (self.config.beta_mirrors and isinstance(parent, Node) and parent.mirror_origin_id is not None):
                return {"nodes": [self._to_api(tx, n) for n in tx.children(parent.id)]}
            origin = self._follow_mirror(tx, parent)
            if origin is None:
                return {"nodes": []}
            shown = [self._to_api(tx, n, parent_id=parent.id) for n in tx.children(origin.id)]
            return {"nodes": shown, "mirror": {"origin_id": origin.id}}

    def create(self, body: Any) -> dict[str, Any]:
        """POST /nodes (§4.3)."""
        req = models.validate(CreateItemRequest, body)
        with self.store.transaction() as tx:
            parent = self.resolve_parent(tx, req.parent_id, materialise=True)
            # A mirror shell as parent is NOT redirected in production (§4.3
            # step 2); the beta API creates under the origin, whatever form
            # named the shell (UUID, short id, URL) (§10.5).
            if isinstance(parent, Node):
                parent = self._through_mirror(tx, parent, PARENT_NOT_FOUND)
            blocks = transform_create_name(req.name)
            # §6.3: the note and layoutMode reach storage raw, so a NUL or a
            # JSON lone surrogate ("\ud800") in either gives 500 and the
            # transaction is rolled back (nothing is created). [O: goldens
            # create_note_nul_500, create_layoutmode_controls_nul_and_surrogates]
            # The name never fails: its pipeline strips NUL and turns lone
            # surrogates into "?".
            for value in (req.note, req.layoutMode):
                if value is not None and ("\x00" in value or has_lone_surrogate(value)):
                    raise internal_error()
            now = self.now()
            first = blocks[0]
            layout = req.layoutMode if req.layoutMode else (first.layout or "bullets")
            new_id = self._new_id()
            checks: list[tuple[str, str | None]] = [("name", first.name), ("note", req.note)]
            checks += [(f"name[{i + 1}]", b.name) for i, b in enumerate(blocks[1:])]
            self._xml_check("create", new_id, checks)
            node = tx.insert(
                node_id=new_id,
                parent_id=parent.id,
                name=first.name,
                note=req.note,  # verbatim; null when absent or null
                priority=self.assign_priority(tx, parent, position_str(req.position)),
                completed=first.completed,
                completed_at=now if first.completed else None,
                layout_mode=layout,
                now=now,
            )
            # Blocks 1..n: flat children of block 0, priorities 0, 1, 2, ...,
            # note "", layout from prefix or "p" (the layoutMode param applies
            # to block 0 only). UNVERIFIED (open-questions P3): the note param
            # goes to block 0 only.
            for i, b in enumerate(blocks[1:]):
                tx.insert(
                    node_id=self._new_id(),
                    parent_id=node.id,
                    name=b.name,
                    note="",
                    priority=i,
                    completed=b.completed,
                    completed_at=now if b.completed else None,
                    layout_mode=b.layout or "p",
                    now=now,
                )
            # The parent's modifiedAt is NOT bumped (§8).
            return {"item_id": node.id}

    def update(self, node_id: str, body: Any) -> dict[str, Any]:
        """POST /nodes/:id (§4.4)."""
        req = models.validate(UpdateNodeRequest, body)
        with self.store.transaction() as tx:
            node = self._through_mirror(tx, self._lookup_exact(tx, node_id), ITEM_NOT_FOUND)
            fields: dict[str, Any] = {}
            if req.name is not None:
                fields["name"] = strip_update_text(req.name)
            if req.note is not None:
                fields["note"] = strip_update_text(req.note)
            if req.layoutMode is not None:
                # Stored verbatim, no control-character strip (open-questions
                # P4, answered by golden update_layoutmode_control_chars). So
                # a NUL or a lone surrogate reaches storage and gives 500 with
                # nothing changed, not even modifiedAt [O: golden
                # update_nul_layoutmode_and_lone_surrogates].
                if "\x00" in req.layoutMode or has_lone_surrogate(req.layoutMode):
                    raise internal_error()
                fields["layout_mode"] = req.layoutMode
            self._xml_check("update", node.id, [(f, fields.get(f)) for f in ("name", "note") if f in fields])
            # Always bumped, even for {} or a no-op (§4.4 step 4).
            fields["modified_at"] = self.now()
            tx.set_fields(node.id, **fields)
            return OK

    def complete(self, node_id: str, completed: bool) -> dict[str, Any]:
        """POST /nodes/:id/complete and /uncomplete (§4.5). No cascade."""
        with self.store.transaction() as tx:
            node = self._through_mirror(tx, self._lookup_exact(tx, node_id), ITEM_NOT_FOUND)
            now = self.now()
            tx.set_fields(
                node.id,
                completed=int(completed),
                completed_at=now if completed else None,  # re-completing re-stamps
                modified_at=now,
            )
            return OK

    def delete(self, node_id: str) -> dict[str, Any]:
        """DELETE /nodes/:id (§4.6): recursive, bumps nobody.

        Soft: the subtree moves to the trash (restorable via
        ``/_mock/ui/restore``) [O: ui-findings B2 -- API-deleted nodes appear in
        the UI's Trash and restore with the same ids]. Through the API it is
        indistinguishable from a hard delete (404 afterwards).
        """
        with self.store.transaction() as tx:
            node = self._lookup_exact(tx, node_id)
            if not self.config.beta_mirrors:
                # Production leaves the shells in their origins' registries:
                # "ghosts" that later block a beta delete of the origin [O:
                # beta-mirrors 06].
                tx.delete_subtree(node.id, self.now())
                return OK
            self._check_no_mirrored_origins(tx, node)
            shells = self._shells_in(tx, node)
            tx.delete_subtree(node.id, self.now())
            tx.unregister_shells([s.id for s in shells])
            for origin_id in {s.mirror_origin_id for s in shells}:
                # [O: beta-mirrors 01] unlike production's §8
                if origin_id is not None and tx.get(origin_id) is not None:
                    tx.touch(origin_id, self.now())
            return OK

    def _check_no_mirrored_origins(self, tx: Tx, node: Node) -> None:
        """beta_mirrors: refuse to delete a subtree holding an origin that still has mirrors (§10.5).

        [O: beta-mirrors 01-06] The node itself, a descendant, or a persisted
        child of a shell; a mirror lying inside the deleted subtree blocks
        too. The check reads the **registry**: a ghost entry blocks, and a
        live shell missing from it does not. ``DELETE /mirror`` does NOT
        check (it can make orphans).
        """
        for i in tx.subtree_ids(node.id):
            if tx.registered_mirrors(i):
                raise invalid_request(HAS_MIRRORS_MESSAGE)

    @staticmethod
    def _shells_in(tx: Tx, node: Node) -> list[Node]:
        out = []
        for i in tx.subtree_ids(node.id):
            n = tx.get(i)
            if n is not None and n.mirror_origin_id is not None:
                out.append(n)
        return out

    def move(self, node_id: str, body: Any) -> dict[str, Any]:
        """POST /nodes/:id/move (§4.7)."""
        req = models.validate(MoveNodeRequest, body)
        with self.store.transaction() as tx:
            node = self._lookup_exact(tx, node_id)
            parent = self.resolve_parent(tx, req.parent_id, materialise=True)
            now = self.now()
            if isinstance(parent, Node) and parent.id == node.id:
                # Move to self: 200, only the node's modifiedAt changes.
                tx.touch(node.id, now)
                return OK
            if isinstance(parent, Node) and self._is_descendant(tx, parent, node):
                if not self.config.faithful_cycles:
                    # MOCK-SPECIFIC (not production): production returns 200
                    # and creates a parent cycle (§4.7 step 5, open-questions D1).
                    raise invalid_request(CYCLE_MESSAGE)
                # faithful_cycles: fall through and create the cycle.
            priority = self.assign_priority(tx, parent, position_str(req.position), exclude=node.id)
            tx.set_fields(node.id, parent_id=parent.id, priority=priority, modified_at=now)
            if isinstance(parent, Node):
                tx.touch(parent.id, now)  # destination parent only
            return OK

    def mirror(self, node_id: str, body: Any) -> dict[str, Any]:
        """POST /nodes/:id/mirror (§10.2)."""
        req = models.validate(MirrorNodeRequest, body)
        with self.store.transaction() as tx:
            src = self._lookup_exact(tx, node_id)
            origin = self._follow_mirror(tx, src)
            if origin is None:  # orphan source
                raise not_found(ITEM_NOT_FOUND)
            # Exact UUID lookup only: every miss (short id, URL, garbage,
            # "None", special keys, "") is "Parent item not found".
            if has_lone_surrogate(req.parent_id):
                # Production's exact lookup fails to encode it: 500 (golden
                # ids_parent_id_lone_surrogate), not Parent item not found.
                raise internal_error()
            parent = tx.get(req.parent_id)
            if parent is None:
                raise not_found(PARENT_NOT_FOUND)
            parent = self._follow_mirror(tx, parent)  # a shell parent redirects to its origin
            if parent is None:  # orphan parent
                raise not_found(PARENT_NOT_FOUND)
            if parent.id == origin.id:
                raise invalid_request("Cannot mirror a node under itself")
            # Descendants of the origin are allowed (recursive mirrors).
            now = self.now()
            shell = tx.insert(
                node_id=self._new_id(),
                parent_id=parent.id,
                name="",
                note="",
                priority=self.assign_priority(tx, parent, position_str(req.position)),
                completed=False,
                completed_at=None,
                layout_mode="bullets",
                now=now,
                mirror_origin_id=origin.id,
            )
            tx.register_mirror(origin.id, shell.id)  # either host [O: beta-mirrors 01 §F]
            tx.touch(origin.id, now)  # the resolved origin only
            return {"item_id": shell.id, "origin_id": origin.id}

    def delete_mirror(self, node_id: str) -> dict[str, Any]:
        """DELETE /nodes/:id/mirror (§10.3)."""
        with self.store.transaction() as tx:
            node = self._lookup_exact(tx, node_id)
            if node.mirror_origin_id is None:
                raise invalid_request("Node is not a mirror")
            # Both hosts drop the shell from its origin's registry [O:
            # beta-mirrors 06]. UNVERIFIED (R12): whether shells nested in the
            # removed subtree are dropped too; with beta_mirrors we drop them.
            nested = self._shells_in(tx, node) if self.config.beta_mirrors else [node]
            tx.delete_subtree(node.id, self.now())
            tx.unregister_shells([s.id for s in nested])
            # Orphans succeed; bump the origin only if it still exists (it may
            # even have been inside the deleted subtree).
            if tx.get(node.mirror_origin_id) is not None:
                tx.touch(node.mirror_origin_id, self.now())
            return OK

    def export(self, api_key: str) -> dict[str, Any]:
        """GET /nodes-export (§4.10, S5).

        [O] ``{"nodes": [...]}``: every node, flat, each record byte-identical
        to what GET / list return (no mirror metadata; shells look like
        ordinary nodes; with beta_mirrors, the mirror-aware shape of
        :meth:`_to_api`), completed nodes included. Production's order is an
        unspecified but stable storage order; we use creation order.
        Detached cycles were not present in the account, so whether they are
        exported is UNVERIFIED (we include them).

        Rate limit: production did NOT enforce the documented 1 req/min
        (calls 10 s apart both returned 200), so it is off by default
        (``export_min_interval`` 0). When enabled, the 429 body is a guess.
        """
        with self.store.transaction() as tx:
            interval = self.config.export_min_interval
            now = self.clock()
            last = self._export_last.get(api_key)
            if interval > 0 and last is not None and now - last < interval:
                raise ApiError(
                    429,
                    {"code": "rate_limited", "message": "Rate limit exceeded. Please try again later."},
                )
            self._export_last[api_key] = now
            return {"nodes": [self._to_api(tx, n) for n in tx.all_nodes()]}

    #: [O] order and labels of the system targets. ``None`` = the node's name
    #: (or null while the node does not exist); a string = a fixed label,
    #: returned even though the day node does not exist.
    SYSTEM_TARGETS: tuple[tuple[str, str | None], ...] = (
        ("inbox", None),
        ("today", "Today"),
        ("tomorrow", "Tomorrow"),
        ("next_week", "Next Week"),
        ("calendar", None),
    )

    def targets(self) -> dict[str, Any]:
        """GET /targets (§11, S6).

        [O] system targets in the order inbox, today, tomorrow, next_week,
        calendar; ``inbox`` was null (never materialised), ``calendar`` was
        its node's name. UNVERIFIED (S3): user shortcuts (the account has
        none) -- we list them first, in creation order, omitting shortcuts
        whose node was deleted, as in the upstream example.
        """
        with self.store.transaction() as tx:
            out: list[dict[str, Any]] = []
            for key, node_id in tx.shortcuts():
                node = tx.get(node_id)
                if node is not None:
                    out.append({"key": key, "type": "shortcut", "name": node.name})
            for key, label in self.SYSTEM_TARGETS:
                if label is None:
                    node = tx.get_by_system_key(key)
                    label = node.name if node else None
                out.append({"key": key, "type": "system", "name": label})
            return {"targets": out}

    # ------------------------------------------------------------ admin hooks

    def dump(self) -> dict[str, Any]:
        with self.store.transaction() as tx:
            return {
                "nodes": [n.to_dump() for n in tx.all_nodes()],
                "trash": tx.trash_dump(),
                "mirror_registry": tx.registry_dump(),
                "shortcuts": [{"key": k, "node_id": v} for k, v in tx.shortcuts()],
                "config": self.config.to_json(),
                "now": self.now(),
            }

    def reset(self, full: bool = False) -> None:
        """Empty the outline. ``full`` also restores the startup config.

        A seeded id generator restarts its sequence either way, so a test that
        resets gets the same ids on every run.
        """
        self.store.reset()
        self._export_last.clear()
        self.warnings.clear()
        if isinstance(self.id_factory, SeededIds):
            self.id_factory.reset()
        if full:
            for k, v in vars(self._initial_config).items():
                setattr(self.config, k, v)

    def set_shortcut(self, key: str, node_id: str) -> None:
        with self.store.transaction() as tx:
            if tx.get(node_id) is None:
                raise not_found(ITEM_NOT_FOUND)
            tx.set_shortcut(key, node_id)

    def delete_shortcut(self, key: str) -> bool:
        with self.store.transaction() as tx:
            return tx.delete_shortcut(key)
