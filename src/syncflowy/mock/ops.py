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

from . import models
from .models import (
    CreateItemRequest,
    MirrorNodeRequest,
    MoveNodeRequest,
    UpdateNodeRequest,
    position_str,
)
from .store import Node, Store, Tx
from .text import format_date_text, has_lone_surrogate, strip_update_text, transform_create_name


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
    #: Minimum seconds between nodes-export calls per API key (upstream: 1 req/min).
    #: 0 disables the limit.
    export_min_interval: float = 60.0
    #: Weekday the week starts on for "next_week" (0 = Monday). UNVERIFIED: the
    #: real value is a per-account setting (upstream docs).
    week_start: int = 0

    def to_json(self) -> dict[str, Any]:
        return {
            "faithful_cycles": self.faithful_cycles,
            "export_min_interval": self.export_min_interval,
            "week_start": self.week_start,
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


@dataclass
class _SpecialStep:
    system_key: str
    name: str
    position: str  # where a freshly materialised node is placed under its parent


@dataclass
class Ops:
    store: Store
    clock: Callable[[], float] = time.time
    config: Config = field(default_factory=Config)
    #: Id generator. None -> random uuid4; a :class:`SeededIds` makes ids reproducible.
    id_factory: Callable[[], str] | None = None
    _export_last: dict[str, float] = field(default_factory=dict)

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

    # ------------------------------------------------------ parent resolution

    def _special_chain(self, key: str) -> list[_SpecialStep] | None:
        """Special ``parent_id`` keys (§11). Everything here is UNVERIFIED.

        The upstream docs list ``inbox``, ``calendar``, ``today``,
        ``tomorrow``, ``next_week``, ``YYYY``, ``YYYY-MM``, ``YYYY-MM-DD``.
        We model each as a lazily materialised node identified by
        ``system_key``. Names and placement are guesses:

        * inbox: a top-level node "Inbox" at the top of the outline.
        * calendar: a top-level node "Calendar" at the bottom of the outline.
        * year "2026", month "January", day "Thu, Jan 15, 2026", each
          appended at the bottom of its parent (chronological when
          materialised in order).
        * today / tomorrow / next_week use the mock clock in UTC.
        """
        if key == "inbox":
            return [_SpecialStep("inbox", "Inbox", "top")]
        cal = [_SpecialStep("calendar", "Calendar", "bottom")]
        if key == "calendar":
            return cal
        date: datetime.date | None = None
        level = 3
        today = datetime.datetime.fromtimestamp(self.now(), datetime.UTC).date()
        if key == "today":
            date = today
        elif key == "tomorrow":
            date = today + datetime.timedelta(days=1)
        elif key == "next_week":
            days = (self.config.week_start - today.weekday()) % 7 or 7
            date = today + datetime.timedelta(days=days)
        elif re.fullmatch(r"[0-9]{4}", key):
            if int(key) < 1:
                return None
            date, level = datetime.date(int(key), 1, 1), 1
        elif m := re.fullmatch(r"([0-9]{4})-([0-9]{2})", key):
            try:
                date, level = datetime.date(int(m[1]), int(m[2]), 1), 2
            except ValueError:
                return None
        elif m := re.fullmatch(r"([0-9]{4})-([0-9]{2})-([0-9]{2})", key):
            try:
                date = datetime.date(int(m[1]), int(m[2]), int(m[3]))
            except ValueError:
                return None
        else:
            return None
        chain = cal + [_SpecialStep(f"calendar:{date.year:04d}", str(date.year), "bottom")]
        if level >= 2:
            chain.append(_SpecialStep(f"calendar:{date.year:04d}-{date.month:02d}", date.strftime("%B"), "bottom"))
        if level >= 3:
            chain.append(_SpecialStep(f"calendar:{date.isoformat()}", format_date_text(date), "bottom"))
        return chain

    def _materialise(self, tx: Tx, raw: str, chain: list[_SpecialStep], materialise: bool) -> Node:
        parent: Parent = ROOT
        node: Node | None = None
        for step in chain:
            node = tx.get_by_system_key(step.system_key)
            if node is None:
                if not materialise:
                    # Upstream: "Calendar keys return 404 if the node hasn't been
                    # created yet". The message is UNVERIFIED (open-questions S2).
                    raise not_found(f"Location '{raw}' not found")
                now = self.now()
                node = tx.insert(
                    node_id=self._new_id(),
                    parent_id=parent.id,
                    name=step.name,
                    note=None,
                    priority=self.assign_priority(tx, parent, step.position),
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
        if raw is None or raw == "":
            # UNVERIFIED (open-questions S1/S4): omitted, null and "" pass
            # validation in production; what they resolve to is untested.
            # We treat them as the root.
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
        # UNVERIFIED (S2): special keys and user shortcuts are checked here,
        # between the URL form and the bare short-id form; the relative
        # order could not be observed.
        if raw == "None":
            return ROOT
        if (chain := self._special_chain(raw)) is not None:
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
        """GET /nodes/:id -- exact UUID, or a 12-hex short id in any case (§3.1, §4.1)."""
        with self.store.transaction() as tx:
            node = tx.get(node_id)
            if node is None and SHORT_RE.fullmatch(node_id):
                node = tx.get_by_short(node_id.lower())
            if node is None:
                raise not_found(ITEM_NOT_FOUND)
            return {"node": node.to_api()}

    def list_children(self, parent_id: str | None) -> dict[str, Any]:
        """GET /nodes?parent_id= -- depth 1, completed included, creation order (§4.2)."""
        with self.store.transaction() as tx:
            parent = self.resolve_parent(tx, parent_id, materialise=False)
            return {"nodes": [n.to_api() for n in tx.children(parent.id)]}

    def create(self, body: Any) -> dict[str, Any]:
        """POST /nodes (§4.3)."""
        req = models.validate(CreateItemRequest, body)
        with self.store.transaction() as tx:
            parent = self.resolve_parent(tx, req.parent_id, materialise=True)
            # A mirror shell as parent is NOT redirected (§4.3 step 2).
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
            node = tx.insert(
                node_id=self._new_id(),
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
            node = self._lookup_exact(tx, node_id)
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
            # Always bumped, even for {} or a no-op (§4.4 step 4).
            fields["modified_at"] = self.now()
            tx.set_fields(node.id, **fields)
            return OK

    def complete(self, node_id: str, completed: bool) -> dict[str, Any]:
        """POST /nodes/:id/complete and /uncomplete (§4.5). No cascade."""
        with self.store.transaction() as tx:
            node = self._lookup_exact(tx, node_id)
            now = self.now()
            tx.set_fields(
                node.id,
                completed=int(completed),
                completed_at=now if completed else None,  # re-completing re-stamps
                modified_at=now,
            )
            return OK

    def delete(self, node_id: str) -> dict[str, Any]:
        """DELETE /nodes/:id (§4.6): hard, recursive, bumps nobody."""
        with self.store.transaction() as tx:
            node = self._lookup_exact(tx, node_id)
            tx.delete_subtree(node.id)
            return OK

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
            tx.touch(origin.id, now)  # the resolved origin only
            return {"item_id": shell.id, "origin_id": origin.id}

    def delete_mirror(self, node_id: str) -> dict[str, Any]:
        """DELETE /nodes/:id/mirror (§10.3)."""
        with self.store.transaction() as tx:
            node = self._lookup_exact(tx, node_id)
            if node.mirror_origin_id is None:
                raise invalid_request("Node is not a mirror")
            tx.delete_subtree(node.id)
            # Orphans succeed; bump the origin only if it still exists (it may
            # even have been inside the deleted subtree).
            if tx.get(node.mirror_origin_id) is not None:
                tx.touch(node.mirror_origin_id, self.now())
            return OK

    def export(self, api_key: str) -> dict[str, Any]:
        """GET /nodes-export. Shape per upstream; everything about it is UNVERIFIED (S5).

        Returns every stored node (including orphan shells and detached
        cycles) in creation order, without hidden metadata. Rate limit: one
        call per ``export_min_interval`` seconds per API key, measured on the
        mock clock. The 429 body is a guess; production's was never observed.
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
            return {"nodes": [n.to_api() for n in tx.all_nodes()]}

    SYSTEM_TARGETS = ("inbox", "calendar", "today", "tomorrow", "next_week")

    def targets(self) -> dict[str, Any]:
        """GET /targets. Shape per upstream; the set of system targets is UNVERIFIED (S6).

        User shortcuts first (in creation order; shortcuts whose node was
        deleted are omitted), then the system targets, which are always
        listed with ``name: null`` until their node is materialised.
        """
        with self.store.transaction() as tx:
            out: list[dict[str, Any]] = []
            for key, node_id in tx.shortcuts():
                node = tx.get(node_id)
                if node is not None:
                    out.append({"key": key, "type": "shortcut", "name": node.name})
            for key in self.SYSTEM_TARGETS:
                chain = self._special_chain(key)
                assert chain is not None
                node = tx.get_by_system_key(chain[-1].system_key)
                out.append({"key": key, "type": "system", "name": node.name if node else None})
            return {"targets": out}

    # ------------------------------------------------------------ admin hooks

    def dump(self) -> dict[str, Any]:
        with self.store.transaction() as tx:
            return {
                "nodes": [n.to_dump() for n in tx.all_nodes()],
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
