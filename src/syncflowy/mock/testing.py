"""Run the mock server in a background thread for tests.

Usage::

    from syncflowy.mock.testing import running_mock

    with running_mock() as base_url:          # e.g. "http://127.0.0.1:54321"
        httpx.get(f"{base_url}/api/v1/nodes", headers={"Authorization": "Bearer test-key"})

``clock`` is an optional zero-argument callable returning float unix time; the
server derives every timestamp from ``int(clock())`` (plus any offset set via
``POST /_mock/clock``). ``db_path=None`` uses a fresh temporary file that is
removed afterwards; pass a path to persist (or to reopen an existing db).
``id_seed`` makes node ids reproducible (a seeded UUIDv4 sequence that
restarts on ``POST /_mock/reset``).
"""

from __future__ import annotations

import contextlib
import socket
import tempfile
import threading
import time
from collections.abc import Callable, Iterator
from pathlib import Path

import uvicorn

from .app import MockApp
from .ops import Config
from .store import Store


class MockURL(str):
    """The base URL (a plain ``str``) with the live :class:`MockApp` attached as ``.app``."""

    app: MockApp


@contextlib.contextmanager
def running_mock(
    db_path: str | Path | None = None,
    api_key: str = "test-key",
    clock: Callable[[], float] | None = None,
    *,
    faithful_cycles: bool = False,
    export_min_interval: float = 60.0,
    extra_api_keys: tuple[str, ...] = (),
    id_seed: int | None = None,
) -> Iterator[MockURL]:
    tmpdir = None
    if db_path is None:
        tmpdir = tempfile.TemporaryDirectory(prefix="syncflowy-mock-")
        db_path = Path(tmpdir.name) / "mock.sqlite3"
    store = Store(str(db_path))
    app = MockApp(
        store,
        api_keys=(api_key, *extra_api_keys),
        clock=clock,
        config=Config(faithful_cycles=faithful_cycles, export_min_interval=export_min_interval),
        id_seed=id_seed,
    )
    sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    sock.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    # Avoid Nagle + delayed-ACK stalls (~40 ms per request) between the
    # separate header and body writes; accepted sockets inherit this on Linux.
    sock.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
    sock.bind(("127.0.0.1", 0))
    port = sock.getsockname()[1]
    # A long keep-alive: uvicorn's default (5 s) closes idle connections, and a
    # long-lived client (e.g. the conformance MockBackend) that sends on a
    # pooled connection just as the server closes it gets a spurious
    # "Server disconnected" -- a plausible source of rare test flakes.
    config = uvicorn.Config(
        app,
        log_level="warning",
        server_header=False,
        lifespan="off",
        access_log=False,
        timeout_keep_alive=3600,
    )
    server = uvicorn.Server(config)
    thread = threading.Thread(target=server.run, kwargs={"sockets": [sock]}, daemon=True, name="syncflowy-mock")
    thread.start()
    try:
        deadline = time.monotonic() + 10
        while not server.started:
            if not thread.is_alive() or time.monotonic() > deadline:
                raise RuntimeError("mock server failed to start")
            time.sleep(0.005)
        url = MockURL(f"http://127.0.0.1:{port}")
        url.app = app
        yield url
    finally:
        server.should_exit = True
        thread.join(timeout=10)
        sock.close()
        store.close()
        if tmpdir is not None:
            tmpdir.cleanup()
