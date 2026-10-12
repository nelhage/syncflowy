"""SQLite storage for the mock Workflowy server.

Schema (see also docs/mock-server.md §Storage)::

    nodes
      id                TEXT PRIMARY KEY   -- lowercase hyphenated UUIDv4
      short_id          TEXT NOT NULL      -- last 12 hex chars of id (GET / parent_id short-id lookups)
      parent_id         TEXT               -- NULL = top level of the outline (the "root")
      name              TEXT NOT NULL
      note              TEXT               -- NULL and "" are distinct (data-model.md §6.4)
      priority          INTEGER NOT NULL   -- sparse sort key within one parent (§7)
      completed         INTEGER NOT NULL   -- 0/1
      completed_at      INTEGER            -- unix seconds or NULL
      layout_mode       TEXT NOT NULL      -- any string (no enum check)
      created_at        INTEGER NOT NULL   -- unix seconds, never changes
      modified_at       INTEGER NOT NULL   -- unix seconds, rules in §8
      create_seq        INTEGER NOT NULL UNIQUE  -- HIDDEN: creation order; list order (§4.2)
      mirror_origin_id  TEXT               -- HIDDEN: set on mirror shells only; kept dangling
                                           --         after the origin is deleted (§10)
      system_key        TEXT UNIQUE        -- HIDDEN, mock-only: "inbox", "calendar",
                                           --   "calendar:YYYY", "calendar:YYYY-MM",
                                           --   "calendar:YYYY-MM-DD" for materialised
                                           --   special locations (§11)
    shortcuts                              -- user "jump-to" shortcut keys (targets, §11)
      key        TEXT PRIMARY KEY
      node_id    TEXT NOT NULL             -- may dangle after the node is deleted
      created_seq INTEGER NOT NULL
    meta
      key   TEXT PRIMARY KEY               -- "schema_version", "next_create_seq"
      value TEXT NOT NULL

Deletion is hard: rows are removed (with their subtree) and nothing else
remembers them, exactly as the API exposes it (data-model.md §4.6).

Concurrency: a single connection guarded by one re-entrant lock. Every API
request runs inside :meth:`Store.transaction`, which holds the lock and wraps
the request in ``BEGIN IMMEDIATE`` ... ``COMMIT`` (rolled back on any error),
so requests are fully serialised in arrival order -- production behaves "as if
serialised" and never hands out duplicate priorities (§2.8, followup-0 §3).
"""

from __future__ import annotations

import contextlib
import sqlite3
import threading
from collections.abc import Iterator
from dataclasses import dataclass
from typing import Any

SCHEMA_VERSION = "1"

SCHEMA = """
CREATE TABLE IF NOT EXISTS nodes (
    id               TEXT PRIMARY KEY,
    short_id         TEXT NOT NULL,
    parent_id        TEXT,
    name             TEXT NOT NULL,
    note             TEXT,
    priority         INTEGER NOT NULL,
    completed        INTEGER NOT NULL DEFAULT 0,
    completed_at     INTEGER,
    layout_mode      TEXT NOT NULL,
    created_at       INTEGER NOT NULL,
    modified_at      INTEGER NOT NULL,
    create_seq       INTEGER NOT NULL UNIQUE,
    mirror_origin_id TEXT,
    system_key       TEXT UNIQUE
);
CREATE INDEX IF NOT EXISTS nodes_by_parent ON nodes(parent_id, create_seq);
CREATE INDEX IF NOT EXISTS nodes_by_short ON nodes(short_id, create_seq);
CREATE TABLE IF NOT EXISTS shortcuts (
    key         TEXT PRIMARY KEY,
    node_id     TEXT NOT NULL,
    created_seq INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
"""

NODE_COLUMNS = (
    "id, short_id, parent_id, name, note, priority, completed, completed_at, layout_mode, "
    "created_at, modified_at, create_seq, mirror_origin_id, system_key"
)


@dataclass
class Node:
    id: str
    short_id: str
    parent_id: str | None
    name: str
    note: str | None
    priority: int
    completed: bool
    completed_at: int | None
    layout_mode: str
    created_at: int
    modified_at: int
    create_seq: int
    mirror_origin_id: str | None
    system_key: str | None

    @classmethod
    def from_row(cls, row: sqlite3.Row | tuple[Any, ...]) -> Node:
        (id_, short, parent, name, note, pri, comp, comp_at, layout, created, modified, seq, origin, sk) = row
        return cls(id_, short, parent, name, note, pri, bool(comp), comp_at, layout, created, modified, seq, origin, sk)

    def to_api(self) -> dict[str, Any]:
        """Serialised node, keys in production's exact order (data-model.md §1.2).

        No hidden field (create_seq, mirror_origin_id, system_key) is exposed.
        """
        return {
            "id": self.id,
            "name": self.name,
            "note": self.note,
            "parent_id": self.parent_id,
            "priority": self.priority,
            "completed": self.completed,
            "data": {"layoutMode": self.layout_mode},
            "createdAt": self.created_at,
            "modifiedAt": self.modified_at,
            "completedAt": self.completed_at,
        }

    def to_dump(self) -> dict[str, Any]:
        """Admin dump: the API shape plus the hidden columns."""
        d = self.to_api()
        d["_create_seq"] = self.create_seq
        d["_mirror_origin_id"] = self.mirror_origin_id
        d["_system_key"] = self.system_key
        return d


def short_id_of(node_id: str) -> str:
    """Short id = last 12 hex characters of the dash-stripped UUID (§1.1)."""
    return node_id.replace("-", "")[-12:].lower()


class Store:
    def __init__(self, path: str):
        self.path = path
        self.lock = threading.RLock()
        self.conn = sqlite3.connect(path, check_same_thread=False, isolation_level=None)
        with self.lock:
            self.conn.execute("PRAGMA journal_mode=WAL")
            self.conn.execute("PRAGMA synchronous=NORMAL")
            self.conn.executescript(SCHEMA)
            self.conn.execute(
                "INSERT OR IGNORE INTO meta(key, value) VALUES ('schema_version', ?)", (SCHEMA_VERSION,)
            )
            self.conn.execute("INSERT OR IGNORE INTO meta(key, value) VALUES ('next_create_seq', '1')")

    def close(self) -> None:
        with self.lock:
            self.conn.close()

    @contextlib.contextmanager
    def transaction(self) -> Iterator[Tx]:
        """Serialise one request: global lock + one SQLite write transaction."""
        with self.lock:
            self.conn.execute("BEGIN IMMEDIATE")
            try:
                yield Tx(self.conn)
                self.conn.execute("COMMIT")
            except BaseException:
                # Also covers a failing COMMIT (e.g. disk full): without the
                # rollback the connection would stay inside a transaction and
                # every later BEGIN would fail.
                if self.conn.in_transaction:
                    self.conn.execute("ROLLBACK")
                raise

    def reset(self) -> None:
        with self.transaction() as tx:
            tx.conn.execute("DELETE FROM nodes")
            tx.conn.execute("DELETE FROM shortcuts")
            tx.conn.execute("UPDATE meta SET value = '1' WHERE key = 'next_create_seq'")


class Tx:
    """Data-access helpers used inside one transaction."""

    def __init__(self, conn: sqlite3.Connection):
        self.conn = conn

    # -- reads ---------------------------------------------------------------

    def get(self, node_id: str) -> Node | None:
        """Exact, case-sensitive id lookup."""
        row = self.conn.execute(f"SELECT {NODE_COLUMNS} FROM nodes WHERE id = ?", (node_id,)).fetchone()
        return Node.from_row(row) if row else None

    def get_by_short(self, short: str) -> Node | None:
        """Short-id lookup. Collisions cannot be produced; first by create_seq (open-questions D3)."""
        row = self.conn.execute(
            f"SELECT {NODE_COLUMNS} FROM nodes WHERE short_id = ? ORDER BY create_seq LIMIT 1", (short.lower(),)
        ).fetchone()
        return Node.from_row(row) if row else None

    def get_by_system_key(self, key: str) -> Node | None:
        row = self.conn.execute(f"SELECT {NODE_COLUMNS} FROM nodes WHERE system_key = ?", (key,)).fetchone()
        return Node.from_row(row) if row else None

    def children(self, parent_id: str | None) -> list[Node]:
        """Direct children in creation order (list order, data-model.md §4.2)."""
        if parent_id is None:
            rows = self.conn.execute(
                f"SELECT {NODE_COLUMNS} FROM nodes WHERE parent_id IS NULL ORDER BY create_seq"
            ).fetchall()
        else:
            rows = self.conn.execute(
                f"SELECT {NODE_COLUMNS} FROM nodes WHERE parent_id = ? ORDER BY create_seq", (parent_id,)
            ).fetchall()
        return [Node.from_row(r) for r in rows]

    def all_nodes(self) -> list[Node]:
        rows = self.conn.execute(f"SELECT {NODE_COLUMNS} FROM nodes ORDER BY create_seq").fetchall()
        return [Node.from_row(r) for r in rows]

    # -- writes --------------------------------------------------------------

    def next_seq(self) -> int:
        (v,) = self.conn.execute("SELECT value FROM meta WHERE key = 'next_create_seq'").fetchone()
        seq = int(v)
        self.conn.execute("UPDATE meta SET value = ? WHERE key = 'next_create_seq'", (str(seq + 1),))
        return seq

    def insert(
        self,
        *,
        node_id: str,
        parent_id: str | None,
        name: str,
        note: str | None,
        priority: int,
        completed: bool,
        completed_at: int | None,
        layout_mode: str,
        now: int,
        mirror_origin_id: str | None = None,
        system_key: str | None = None,
    ) -> Node:
        seq = self.next_seq()
        self.conn.execute(
            f"INSERT INTO nodes ({NODE_COLUMNS}) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            (
                node_id,
                short_id_of(node_id),
                parent_id,
                name,
                note,
                priority,
                int(completed),
                completed_at,
                layout_mode,
                now,
                now,
                seq,
                mirror_origin_id,
                system_key,
            ),
        )
        node = self.get(node_id)
        assert node is not None
        return node

    def set_fields(self, node_id: str, **fields: Any) -> None:
        if not fields:
            return
        cols = ", ".join(f"{k} = ?" for k in fields)
        self.conn.execute(f"UPDATE nodes SET {cols} WHERE id = ?", (*fields.values(), node_id))

    def touch(self, node_id: str, now: int) -> None:
        self.set_fields(node_id, modified_at=now)

    def delete_subtree(self, node_id: str) -> None:
        """Hard-delete a node and everything under it.

        ``UNION`` (not ``UNION ALL``) makes the recursion terminate even if a
        faithful-cycles move created a parent cycle.
        """
        self.conn.execute(
            """
            WITH RECURSIVE sub(id) AS (
                SELECT ?
                UNION
                SELECT n.id FROM nodes n JOIN sub ON n.parent_id = sub.id
            )
            DELETE FROM nodes WHERE id IN (SELECT id FROM sub)
            """,
            (node_id,),
        )

    # -- shortcuts -----------------------------------------------------------

    def shortcut(self, key: str) -> str | None:
        row = self.conn.execute("SELECT node_id FROM shortcuts WHERE key = ?", (key,)).fetchone()
        return row[0] if row else None

    def shortcuts(self) -> list[tuple[str, str]]:
        return [tuple(r) for r in self.conn.execute("SELECT key, node_id FROM shortcuts ORDER BY created_seq")]

    def set_shortcut(self, key: str, node_id: str) -> None:
        self.conn.execute(
            "INSERT INTO shortcuts(key, node_id, created_seq) VALUES (?, ?, "
            "(SELECT COALESCE(MAX(created_seq), 0) + 1 FROM shortcuts)) "
            "ON CONFLICT(key) DO UPDATE SET node_id = excluded.node_id",
            (key, node_id),
        )

    def delete_shortcut(self, key: str) -> bool:
        return self.conn.execute("DELETE FROM shortcuts WHERE key = ?", (key,)).rowcount > 0
