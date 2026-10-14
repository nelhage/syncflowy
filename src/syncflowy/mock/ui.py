"""Test-only simulation of edits made in the Workflowy **web UI** (``/_mock/ui/*``).

Production's UI writes to the same data model as the API but with different
semantics (docs/workflowy/ui-findings.md). These operations let sync-engine
tests produce exactly the data a human editing in the UI would produce:

* **placement** -- a node placed between two siblings gets the integer
  midpoint of their priorities; placed first, the midpoint of 0 and the first
  sibling; placed last, the API bottom rule ``ceil((max+100)/100)*100``; first
  child of an empty parent, 100 [O: A1–A4, A2 step 3, B4 (50 = midpoint(0,100))].
  No sibling is ever rewritten [O].
* **bumps** -- creates bump only the new node; moves bump the node and the
  destination parent (also for same-parent reorders); edits/complete bump the
  node only; deletes bump nothing; restores bump every restored node
  [O: A1–A4, C1, D1, B1/B2].
* **mirrors** -- in the UI a mirror *is* its origin: a shell used as a parent,
  edit target or completion target resolves to its origin [O: H3/H4; H5
  UNVERIFIED for completion].
* **text** -- names/notes are WF-HTML (:mod:`syncflowy.wfhtml`); a UI save
  re-serialises the value canonically [O: E1–E8, F step 2].

Every route takes and returns JSON; node ids are exact lowercase UUIDs. Errors:
``404 {"code": "not_found", ...}`` for unknown/deleted ids, ``400
{"code": "bad_ui_request", ...}`` for bad arguments.
"""

from __future__ import annotations

from collections.abc import Callable
from dataclasses import dataclass
from typing import Any

from .. import wfhtml
from .ops import ROOT, ApiError, Ops, Parent, not_found
from .store import Node, Tx


def bad(message: str) -> ApiError:
    return ApiError(400, {"code": "bad_ui_request", "message": message})


@dataclass
class UiOps:
    ops: Ops

    # ---------------------------------------------------------------- helpers

    @property
    def now(self) -> Callable[[], int]:
        return self.ops.now

    @staticmethod
    def _get(tx: Tx, node_id: Any, what: str = "id") -> Node:
        if not isinstance(node_id, str):
            raise bad(f"{what} must be a node id string")
        node = tx.get(node_id)
        if node is None:
            raise not_found(f"Item not found: {node_id}")
        return node

    def _via_mirror(self, tx: Tx, node: Node) -> Node:
        """A shell resolves to its origin [O: H3, H4]."""
        if node.mirror_origin_id is None:
            return node
        origin = self.ops._follow_mirror(tx, node)
        if origin is None:
            # UNVERIFIED: what the UI shows for an orphan shell (H11 not run);
            # we treat it as an ordinary node.
            return node
        return origin

    def _parent(self, tx: Tx, raw: Any) -> Parent:
        if raw is None or raw == "None":
            return ROOT
        return self._via_mirror(tx, self._get(tx, raw, "parent_id"))

    @staticmethod
    def _display(tx: Tx, parent: Parent, exclude: str | None = None) -> list[Node]:
        kids = [n for n in tx.children(parent.id) if n.id != exclude]
        return sorted(kids, key=lambda n: (n.priority, n.create_seq))

    def _target(self, tx: Tx, body: dict[str, Any], exclude: str | None = None) -> tuple[Parent, int]:
        """Resolve a placement spec to (parent, slot index in display order).

        Spec keys: ``parent_id`` (null/"None" = top level), and at most one of
        ``after_id`` / ``before_id`` (a sibling in that parent; the parent may
        then be omitted), ``index`` (0-based slot among the other children),
        ``position`` ("top" / "bottom"). Default: bottom.
        """
        anchor_key = next((k for k in ("after_id", "before_id") if body.get(k) is not None), None)
        if anchor_key:
            anchor = self._get(tx, body[anchor_key], anchor_key)
            if "parent_id" in body:
                parent = self._parent(tx, body["parent_id"])
            else:
                parent = ROOT if anchor.parent_id is None else self._get(tx, anchor.parent_id)
            if anchor.parent_id != parent.id:
                raise bad(f"{anchor_key} is not a child of the target parent")
            if anchor.id == exclude:
                raise bad(f"{anchor_key} cannot be the node itself")
            sibs = self._display(tx, parent, exclude)
            i = next(k for k, n in enumerate(sibs) if n.id == anchor.id)
            return parent, i + 1 if anchor_key == "after_id" else i
        if "parent_id" not in body:
            raise bad("parent_id (or after_id / before_id) is required")
        parent = self._parent(tx, body["parent_id"])
        n = len(self._display(tx, parent, exclude))
        if body.get("index") is not None:
            idx = body["index"]
            if not isinstance(idx, int) or not 0 <= idx <= n:
                raise bad(f"index must be an integer in 0..{n}")
            return parent, idx
        pos = body.get("position", "bottom")
        if pos not in ("top", "bottom"):
            raise bad("position must be 'top' or 'bottom'")
        return parent, 0 if pos == "top" else n

    def _slot_priority(self, tx: Tx, parent: Parent, slot: int, exclude: str | None = None) -> int:
        """UI placement rule (module docstring)."""
        sibs = self._display(tx, parent, exclude)
        if not sibs:
            return 100
        if slot >= len(sibs):
            return -(-(sibs[-1].priority + 100) // 100) * 100
        lo = sibs[slot - 1].priority if slot > 0 else 0
        hi = sibs[slot].priority
        if hi - lo >= 2:
            return (lo + hi) // 2
        # UNVERIFIED (ui-scenarios A5, not run): what the UI does when there
        # is no integer gap. We renumber the siblings 100, 200, ... in display
        # order, leaving room at the slot, without bumping them (like the API's
        # renumber, §7).
        for rank, n in enumerate(sibs):
            tx.set_fields(n.id, priority=100 * (rank + 1) + (100 if rank >= slot else 0))
        return 100 * (slot + 1)

    @staticmethod
    def _name_from(body: dict[str, Any], html_key: str, text_key: str) -> str | None:
        if body.get(html_key) is not None and body.get(text_key) is not None:
            raise bad(f"give {html_key} or {text_key}, not both")
        if body.get(html_key) is not None:
            if not isinstance(body[html_key], str):
                raise bad(f"{html_key} must be a string")
            return wfhtml.canonicalize(body[html_key])
        if body.get(text_key) is not None:
            if not isinstance(body[text_key], str):
                raise bad(f"{text_key} must be a string")
            return wfhtml.escape_text(body[text_key])
        return None

    def _insert(
        self,
        tx: Tx,
        parent: Parent,
        priority: int,
        name: str,
        note: str | None,
        now: int,
        layout: str = "bullets",
        completed: bool = False,
        mirror_origin_id: str | None = None,
    ) -> Node:
        return tx.insert(
            node_id=self.ops._new_id(),
            parent_id=parent.id,
            name=name,
            note=note,
            priority=priority,
            completed=completed,
            completed_at=now if completed else None,
            layout_mode=layout,
            now=now,
            mirror_origin_id=mirror_origin_id,
        )

    # ----------------------------------------------------------------- routes

    def create(self, body: dict[str, Any]) -> dict[str, Any]:
        """Enter + typing [O: A2]. Bumps only the new node; ``note`` null."""
        with self.ops.store.transaction() as tx:
            parent, slot = self._target(tx, body)
            name = self._name_from(body, "name", "text") or ""
            note = self._name_from(body, "note", "note_text")
            layout = body.get("layoutMode", "bullets")
            if not isinstance(layout, str):
                raise bad("layoutMode must be a string")
            now = self.now()
            node = self._insert(tx, parent, self._slot_priority(tx, parent, slot), name, note, now, layout)
            return {"id": node.id}

    def edit(self, body: dict[str, Any]) -> dict[str, Any]:
        """Edit name and/or note; through a mirror this edits the origin [O: H3].

        ``name``/``text`` replace the name (HTML is canonicalised, text is
        escaped); ``append_text`` types at the end of the current name (a
        malformed name counts as empty) [O: F step 2]; ``note``/``note_text``
        set the note (canonicalised like a name, UNVERIFIED), null clears it to
        "" (UNVERIFIED). A note-only edit leaves the name untouched
        (UNVERIFIED). Bumps only the edited node.
        """
        with self.ops.store.transaction() as tx:
            node = self._via_mirror(tx, self._get(tx, body.get("id")))
            fields: dict[str, Any] = {}
            name = self._name_from(body, "name", "text")
            if body.get("append_text") is not None:
                if name is not None:
                    raise bad("give name/text or append_text, not both")
                if not isinstance(body["append_text"], str):
                    raise bad("append_text must be a string")
                name = wfhtml.append_typed_text(node.name, body["append_text"])
            if name is not None:
                fields["name"] = name
            if "note" in body or "note_text" in body:
                note = self._name_from(body, "note", "note_text")
                fields["note"] = "" if note is None else note
            if not fields:
                raise bad("nothing to edit (name, text, append_text, note, note_text)")
            fields["modified_at"] = self.now()
            tx.set_fields(node.id, **fields)
            return {"status": "ok", "id": node.id}

    def move(self, body: dict[str, Any]) -> dict[str, Any]:
        """Drag / indent / outdent (``method`` "drag", default) or item-level
        cut + paste (``method`` "cut_paste") [O: A1–A4, B4, C1, H4].

        Same id; priority by the placement rule; bumps the node and the
        destination parent (not the old parent); cut_paste also bumps every
        descendant [O: B4]. A shell as destination resolves to its origin
        [O: H4]. Dragging a shell itself moves the shell (UNVERIFIED, H8 not
        run). A move into the node's own subtree is refused (the UI does not
        offer it; UNVERIFIED, B5 not run).
        """
        method = body.get("method", "drag")
        if method not in ("drag", "cut_paste"):
            raise bad("method must be 'drag' or 'cut_paste'")
        with self.ops.store.transaction() as tx:
            node = self._get(tx, body.get("id"))
            parent, slot = self._target(tx, body, exclude=node.id)
            if isinstance(parent, Node) and (parent.id == node.id or self.ops._is_descendant(tx, parent, node)):
                raise bad("cannot move a node into itself or its own descendant")
            now = self.now()
            priority = self._slot_priority(tx, parent, slot, exclude=node.id)
            tx.set_fields(node.id, parent_id=parent.id, priority=priority, modified_at=now)
            if isinstance(parent, Node):
                tx.touch(parent.id, now)
            if method == "cut_paste":
                for i in tx.subtree_ids(node.id):
                    tx.touch(i, now)
            return {"status": "ok", "priority": priority}

    def copy(self, body: dict[str, Any]) -> dict[str, Any]:
        """Item-level copy + paste: a deep copy with new ids [O: B4 step 3].

        The copy's root is placed by the placement rule; its descendants get
        priorities 0, 1, 2, ... in display order, and every copied node whose
        note was null gets note "" (the multiline-import signature) [O].
        createdAt == modifiedAt == now; nothing else is bumped. Names, layouts
        and completion are copied (UNVERIFIED for completion / layouts).
        Shells inside the copied subtree are copied as shells of the same
        origin (UNVERIFIED).
        """
        with self.ops.store.transaction() as tx:
            src = self._get(tx, body.get("id"))
            parent, slot = self._target(tx, body)
            if isinstance(parent, Node) and self.ops._is_descendant(tx, parent, src):
                raise bad("cannot paste a copy inside the copied subtree")  # UNVERIFIED
            now = self.now()
            mapping: dict[str, str] = {}

            def clone(n: Node, into: Parent, priority: int, seen: frozenset[str]) -> None:
                c = self._insert(
                    tx,
                    into,
                    priority,
                    n.name,
                    "" if n.note is None else n.note,
                    now,
                    n.layout_mode,
                    n.completed,
                    n.mirror_origin_id,
                )
                mapping[n.id] = c.id
                for i, k in enumerate(self._display(tx, n)):
                    if k.id not in seen:  # cycle guard (faithful_cycles)
                        clone(k, c, i, seen | {k.id})

            clone(src, parent, self._slot_priority(tx, parent, slot), frozenset({src.id}))
            return {"id": mapping[src.id], "ids": mapping}

    def complete(self, body: dict[str, Any], completed: bool) -> dict[str, Any]:
        """Ctrl+Enter / checkbox [O: D1, D2]: the node only, no cascade;
        completing re-stamps completedAt. Through a shell the origin is
        completed (UNVERIFIED, H5 not run)."""
        with self.ops.store.transaction() as tx:
            node = self._via_mirror(tx, self._get(tx, body.get("id")))
            now = self.now()
            tx.set_fields(
                node.id, completed=int(completed), completed_at=now if completed else None, modified_at=now
            )
            return {"status": "ok", "id": node.id}

    def mirror(self, body: dict[str, Any]) -> dict[str, Any]:
        """Alt-drag to mirror [O: H2]: a new shell (name "", **note null**,
        bullets, not copying layout/completion) placed by the placement rule;
        bumps the origin only. Mirroring a shell mirrors its origin; a shell
        destination resolves to its origin (as in the API, §10.2)."""
        with self.ops.store.transaction() as tx:
            origin = self._via_mirror(tx, self._get(tx, body.get("id")))
            parent, slot = self._target(tx, body)
            if isinstance(parent, Node) and parent.id == origin.id:
                raise bad("cannot mirror a node under itself")
            now = self.now()
            shell = self._insert(
                tx, parent, self._slot_priority(tx, parent, slot), "", None, now, mirror_origin_id=origin.id
            )
            tx.register_mirror(origin.id, shell.id)  # [I] as for API-made mirrors
            tx.touch(origin.id, now)
            return {"id": shell.id, "origin_id": origin.id}

    def set_layout(self, body: dict[str, Any]) -> dict[str, Any]:
        """Bullet menu -> layout [O: G2]. Values the UI offers: bullets, todo,
        h1, h2, h3, p, numbered, board, table, quote-block, divider (any string
        is accepted here). Side effects [O]:

        * ``board``: creates one empty child (name "", note null) at the
          bottom (100 in an empty node);
        * ``table``: creates children "Headers" and "Row" (bottom placement),
          each with two empty cells at priorities 0, 1 (note null);
        * ``divider``: a null note becomes "".

        UNVERIFIED: whether board/table add children when the node already has
        some (G2's nodes had none); we add them only to childless nodes.
        Only the node itself is bumped.
        """
        layout = body.get("layoutMode")
        if not isinstance(layout, str):
            raise bad("layoutMode must be a string")
        with self.ops.store.transaction() as tx:
            node = self._via_mirror(tx, self._get(tx, body.get("id")))
            now = self.now()
            fields: dict[str, Any] = {"layout_mode": layout, "modified_at": now}
            if layout == "divider" and node.note is None:
                fields["note"] = ""
            tx.set_fields(node.id, **fields)
            created: list[str] = []
            if not tx.children(node.id):
                if layout == "board":
                    created.append(self._insert(tx, node, 100, "", None, now).id)
                elif layout == "table":
                    for i, label in enumerate(("Headers", "Row")):
                        row = self._insert(tx, node, 100 * (i + 1), label, None, now)
                        created.append(row.id)
                        for cell in range(2):
                            created.append(self._insert(tx, row, cell, "", None, now).id)
            return {"status": "ok", "id": node.id, "created": created}

    def delete(self, body: dict[str, Any]) -> dict[str, Any]:
        """Ctrl+Shift+Backspace [O: B1, C1(g)]: soft delete of the subtree,
        nothing bumped.

        Deleting an **origin that has live mirrors** promotes one [O: H6]:
        the origin is *moved* (same id, children kept) into the place of its
        most recently created shell (same parent, same priority), that shell
        and its subtree are deleted, and the origin, the new parent and the
        old parent are bumped. UNVERIFIED: the choice of shell (H6 had two;
        the newer, also later in tree order, won); shells inside the origin's
        own subtree are never chosen.

        Deleting a shell deletes just the shell (UNVERIFIED, H7 not run; the
        origin is not bumped).

        Mirror registry (data-model §10.5): a promotion **empties** the
        origin's registry, so the surviving shells are no longer listed in
        ``mirror_ids`` [O: H6 seen through beta, one case]. A plain UI delete
        unregisters the deleted shells (UNVERIFIED).
        """
        with self.ops.store.transaction() as tx:
            node = self._get(tx, body.get("id"))
            now = self.now()
            sub = set(tx.subtree_ids(node.id))
            shells = [
                n
                for n in tx.all_nodes()
                if n.mirror_origin_id == node.id and n.id not in sub
            ]
            if node.mirror_origin_id is None and shells:
                shell = max(shells, key=lambda n: n.create_seq)
                old_parent = node.parent_id
                op = tx.delete_subtree(shell.id, now)
                tx.set_fields(node.id, parent_id=shell.parent_id, priority=shell.priority, modified_at=now)
                tx.clear_registry(node.id)
                for p in (shell.parent_id, old_parent):
                    if p is not None:
                        tx.touch(p, now)
                return {
                    "status": "ok",
                    "deleted": [shell.id],
                    "delete_op": op,
                    "promoted": {"origin_id": node.id, "replaced_shell_id": shell.id, "parent_id": shell.parent_id},
                }
            ids = tx.subtree_ids(node.id)
            shells = [i for i in ids if (n := tx.get(i)) is not None and n.mirror_origin_id is not None]
            op = tx.delete_subtree(node.id, now)
            tx.unregister_shells(shells)
            return {"status": "ok", "deleted": ids, "delete_op": op, "promoted": None}

    def restore(self, body: dict[str, Any]) -> dict[str, Any]:
        """Undo (Ctrl+Z) of a delete, or "restore" from the Trash [O: B1, B2].

        ``{"id": X}`` restores the delete operation whose root is X;
        ``{"last": true}`` restores the most recent delete (undo). The subtree
        returns with the same ids, parent, priorities, createdAt and list
        (creation-order) position; every restored node's modifiedAt is set to
        now; the parent is not bumped. Works for API deletes too [O: B2].
        Restoring an origin re-attaches its (orphaned) shells.

        UNVERIFIED: restoring when the original parent no longer exists -- we
        put the node at the top level. Restoring a non-root member of a
        delete is refused (the Trash lists delete roots). Undo of an H6
        promotion is not modelled (only the deleted shell comes back).
        """
        with self.ops.store.transaction() as tx:
            if body.get("last"):
                ops_ = tx.trash_ops()
                if not ops_:
                    raise not_found("Trash is empty")
                op = ops_[0]["delete_op"]
                root_id = ops_[0]["id"]
            else:
                t = tx.trashed(body.get("id")) if isinstance(body.get("id"), str) else None
                if t is None:
                    raise not_found(f"Not in trash: {body.get('id')}")
                node, op, is_root, _ = t
                if not is_root:
                    raise bad("not the root of a delete; restore its deleted ancestor instead")
                root_id = node.id
            now = self.now()
            restored = tx.restore_op(op, now)
            root = tx.get(root_id)
            if root is not None and root.parent_id is not None and tx.get(root.parent_id) is None:
                tx.set_fields(root.id, parent_id=None)  # UNVERIFIED (see docstring)
            return {"status": "ok", "restored": restored}

    def trash(self) -> dict[str, Any]:
        """The Trash view: delete operations, most recent first."""
        with self.ops.store.transaction() as tx:
            return {"trash": tx.trash_ops()}

    # ---------------------------------------------------------------- routing

    def dispatch(self, action: str, method: str, body: Any) -> dict[str, Any] | None:
        if method == "GET" and action == "trash":
            return self.trash()
        if method != "POST":
            return None
        if not isinstance(body, dict):
            raise bad("body must be a JSON object")
        handlers: dict[str, Callable[[dict[str, Any]], dict[str, Any]]] = {
            "create": self.create,
            "edit": self.edit,
            "move": self.move,
            "copy": self.copy,
            "complete": lambda b: self.complete(b, True),
            "uncomplete": lambda b: self.complete(b, False),
            "mirror": self.mirror,
            "set_layout": self.set_layout,
            "delete": self.delete,
            "restore": self.restore,
        }
        h = handlers.get(action)
        return h(body) if h else None
