"""``syncflowy-mock``: run the mock Workflowy API server.

    syncflowy-mock --db ./mock.sqlite3 --port 8765 --api-key k1,k2

API keys: ``--api-key`` may be repeated and/or comma-separated. Default: the
``SYNCFLOWY_MOCK_API_KEY`` environment variable (also comma-separated), else
``test-key``.
"""

from __future__ import annotations

import argparse
import logging
import os
import socket

import uvicorn

from .app import MockApp
from .ops import Config
from .store import Store


def parse_keys(values: list[str] | None) -> list[str]:
    if not values:
        values = [os.environ.get("SYNCFLOWY_MOCK_API_KEY") or "test-key"]
    keys = [k.strip() for v in values for k in v.split(",") if k.strip()]
    return keys or ["test-key"]


def main(argv: list[str] | None = None) -> None:
    p = argparse.ArgumentParser(prog="syncflowy-mock", description="Mock Workflowy API server (SQLite-backed).")
    p.add_argument("--db", required=True, help="SQLite database file (created if missing)")
    p.add_argument("--host", default="127.0.0.1")
    p.add_argument("--port", type=int, default=8765)
    p.add_argument("--api-key", action="append", help="accepted API key(s); repeatable or comma-separated")
    p.add_argument(
        "--faithful-cycles",
        action="store_true",
        help="accept moves into a descendant and create a parent cycle, like production "
        "(default: reject with a mock-specific 400)",
    )
    p.add_argument(
        "--export-min-interval",
        type=float,
        default=0.0,
        help="seconds between nodes-export calls per key (default 0 = no limit, as observed "
        "in production; upstream documents 60)",
    )
    p.add_argument(
        "--id-seed",
        type=int,
        default=None,
        help="generate node ids from a seeded RNG (reproducible; restarts on POST /_mock/reset)",
    )
    p.add_argument(
        "--xml-check",
        choices=Config.XML_CHECK_MODES,
        default="warn",
        help="mock-only check that API-written names/notes are well-formed WF-HTML: off, warn "
        "(default; GET /_mock/warnings) or reject (mock-specific 400)",
    )
    p.add_argument("--log-level", default="info")
    args = p.parse_args(argv)

    logging.basicConfig(level=args.log_level.upper())
    app = MockApp(
        Store(args.db),
        api_keys=parse_keys(args.api_key),
        config=Config(
            faithful_cycles=args.faithful_cycles,
            export_min_interval=args.export_min_interval,
            xml_check=args.xml_check,
        ),
        id_seed=args.id_seed,
    )
    family = socket.AF_INET6 if ":" in args.host else socket.AF_INET
    sock = socket.socket(family, socket.SOCK_STREAM)
    sock.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    # Avoid ~40 ms Nagle/delayed-ACK stalls between header and body writes.
    sock.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
    sock.bind((args.host, args.port))
    # Long keep-alive (uvicorn's default is 5 s): avoids a client sending on a
    # pooled connection just as the server closes it ("Server disconnected").
    config = uvicorn.Config(
        app, server_header=False, lifespan="off", log_level=args.log_level, timeout_keep_alive=3600
    )
    print(f"syncflowy-mock listening on http://{args.host}:{args.port} (db: {args.db})", flush=True)
    uvicorn.Server(config).run(sockets=[sock])


if __name__ == "__main__":
    main()
