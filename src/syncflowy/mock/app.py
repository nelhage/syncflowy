"""ASGI application for the mock Workflowy API.

This is a deliberately small hand-written ASGI app (no framework) so that
every byte of routing, 404/405 handling, auth and error formatting is under
our control. Request handling follows production's check order
(data-model.md §2.2):

1. routing          -> 404 text/html (full page for GET, empty otherwise)
2. method           -> 405 (JSON style or empty-HTML style, per route)
3. auth             -> 401 (body shape depends on the method)
4. body parse       -> 400 invalid_json (create, update, move, mirror only)
5. validation       -> 400 {"errors": [...]}
6.-8. lookups and semantic checks (in :mod:`syncflowy.mock.ops`)

Test-only admin routes live under ``/_mock/`` (no auth); see
docs/mock-server.md.
"""

from __future__ import annotations

import json
import logging
import traceback
import urllib.parse
from collections.abc import Awaitable, Callable, Iterable
from email.parser import BytesParser
from email.policy import HTTP
from typing import Any

from .models import RequestValidationFailed
from .ops import ApiError, Clock, Config, Ops, SeededIds, internal_error
from .store import Store

log = logging.getLogger("syncflowy.mock")

Scope = dict[str, Any]
Receive = Callable[[], Awaitable[dict[str, Any]]]
Send = Callable[[dict[str, Any]], Awaitable[None]]

# Static headers production sends on every response (data-model.md §2.7).
# Optional fidelity; no rate-limit, caching or CORS headers.
FIDELITY_HEADERS: list[tuple[str, str]] = [
    ("server", "nginx"),
    ("referrer-policy", "same-origin"),
    ("service-worker-allowed", "/"),
    ("vary", "Accept-Encoding, Cookie, Origin"),
    ("x-content-type-options", "nosniff"),
    ("x-frame-options", "SAMEORIGIN"),
    ("x-xss-protection", "1; mode=block"),
    ("strict-transport-security", "max-age=63072000; includeSubDomains; preload"),
]

# Production's GET 404 page is a ~20 KB marketing page; only the doctype and
# <title>404</title> are meaningful, so we ship a stub with the same markers.
NOT_FOUND_HTML = (
    b'<!DOCTYPE html>\n<html lang="en-US">\n<head>\n  <meta charset="utf-8">\n  <title>404</title>\n'
    b"</head>\n<body>\n  <h1>404</h1>\n  <p>Page not found (syncflowy mock).</p>\n</body>\n</html>\n"
)

INVALID_JSON = {"code": "invalid_json", "message": "Invalid JSON format in request body"}
AUTH_MESSAGE = "Invalid Credentials, try again."

# Route kinds -> (allowed methods, 405 style). "json" 405s carry
# {"error", "allowed_methods"} and no Allow header; "html" 405s are empty
# text/html with an Allow header (data-model.md §2.1).
ROUTES: dict[str, tuple[tuple[str, ...], str, str | None]] = {
    "nodes": (("GET", "POST"), "json", None),
    "node": (("GET", "POST", "DELETE"), "json", None),
    "move": (("POST",), "html", "POST"),
    "complete": (("POST",), "html", "POST"),
    "uncomplete": (("POST",), "html", "POST"),
    "mirror": (("POST", "DELETE"), "json", None),
    # UNVERIFIED: 405 style of nodes-export (only a 401 was ever observed).
    # We assume the same Django require_GET style as /targets.
    "export": (("GET", "HEAD"), "html", "GET, HEAD"),
    "targets": (("GET", "HEAD"), "html", "GET, HEAD"),
}


def dumps(obj: Any) -> bytes:
    """Python ``json.dumps`` defaults: ", " / ": " separators, ensure_ascii (§1.3)."""
    return json.dumps(obj).encode("ascii")


class Response:
    def __init__(
        self,
        status: int,
        body: bytes = b"",
        content_type: str | None = "application/json",
        headers: Iterable[tuple[str, str]] = (),
    ):
        self.status = status
        self.body = body
        self.content_type = content_type
        self.headers = list(headers)

    @classmethod
    def json(cls, status: int, obj: Any, headers: Iterable[tuple[str, str]] = ()) -> Response:
        return cls(status, dumps(obj), "application/json", headers)

    async def send(self, send: Send, head: bool = False) -> None:
        headers = [(b"content-length", str(len(self.body)).encode())]
        if self.content_type is not None:
            headers.append((b"content-type", self.content_type.encode()))
        for k, v in [*self.headers, *FIDELITY_HEADERS]:
            headers.append((k.encode("latin-1"), v.encode("latin-1")))
        await send({"type": "http.response.start", "status": self.status, "headers": headers})
        # HEAD: same headers (content-length of the would-be body), no body.
        await send({"type": "http.response.body", "body": b"" if head else self.body})


def route(path: str) -> tuple[str, str | None] | None:
    """Map a (percent-decoded) path to (route kind, path id).

    One trailing slash is accepted everywhere, without a redirect. ``%2F`` in
    an id has already been decoded to ``/`` by the server, so such ids make
    the route unknown -- exactly as in production (§2.1).
    """
    if path.endswith("/") and len(path) > 1:
        path = path[:-1]
    if not path.startswith("/api/v1/"):
        return None
    parts = path[len("/api/v1/"):].split("/")
    if any(p == "" for p in parts):
        return None
    match parts:
        case ["nodes"]:
            return ("nodes", None)
        case ["nodes-export"]:
            return ("export", None)
        case ["targets"]:
            return ("targets", None)
        case ["nodes", node_id]:
            return ("node", node_id)
        case ["nodes", node_id, ("move" | "complete" | "uncomplete" | "mirror") as kind]:
            return (kind, node_id)
    return None


def _header(scope: Scope, name: bytes) -> str | None:
    for k, v in scope.get("headers", []):
        if k.lower() == name:
            return v.decode("latin-1")
    return None


async def _read_body(receive: Receive) -> bytes:
    chunks = []
    while True:
        msg = await receive()
        if msg["type"] == "http.disconnect":
            break
        chunks.append(msg.get("body", b""))
        if not msg.get("more_body", False):
            break
    return b"".join(chunks)


class _InvalidJson(Exception):
    pass


def parse_body(content_type: str | None, raw: bytes) -> Any:
    """Body parsing (data-model.md §2.4).

    Form encodings are parsed as form fields (every value a string; the last
    duplicate wins). Anything else is parsed with ``json.loads`` whatever the
    Content-Type says. Empty or unparsable -> invalid_json.
    """
    ctype = (content_type or "").split(";")[0].strip().lower()
    if ctype == "application/x-www-form-urlencoded":
        try:
            text = raw.decode("utf-8")
        except UnicodeDecodeError:
            raise _InvalidJson() from None
        return dict(urllib.parse.parse_qsl(text, keep_blank_values=True))
    if ctype == "multipart/form-data":
        return _parse_multipart(content_type or "", raw)
    try:
        return json.loads(raw)
    except (ValueError, UnicodeDecodeError):
        raise _InvalidJson() from None


def _parse_multipart(content_type: str, raw: bytes) -> dict[str, str]:
    msg = BytesParser(policy=HTTP).parsebytes(b"Content-Type: " + content_type.encode("latin-1") + b"\r\n\r\n" + raw)
    out: dict[str, str] = {}
    if not msg.is_multipart():
        return out
    for part in msg.iter_parts():
        name = part.get_param("name", header="content-disposition")
        if not name:
            continue
        payload = part.get_payload(decode=True) or b""
        out[str(name)] = payload.decode(part.get_content_charset() or "utf-8", errors="replace")
    return out


class MockApp:
    """The ASGI callable. One instance owns one :class:`Store`."""

    def __init__(
        self,
        store: Store,
        api_keys: Iterable[str] = ("test-key",),
        clock: Callable[[], float] | None = None,
        config: Config | None = None,
        id_seed: int | None = None,
    ):
        self.store = store
        self.api_keys = set(api_keys)
        self.clock = Clock(base=clock) if clock is not None else Clock()
        ids = SeededIds(id_seed) if id_seed is not None else None
        self.ops = Ops(store, self.clock, config or Config(), id_factory=ids)

    async def __call__(self, scope: Scope, receive: Receive, send: Send) -> None:
        if scope["type"] == "lifespan":
            while True:
                msg = await receive()
                if msg["type"] == "lifespan.startup":
                    await send({"type": "lifespan.startup.complete"})
                elif msg["type"] == "lifespan.shutdown":
                    await send({"type": "lifespan.shutdown.complete"})
                    return
        if scope["type"] != "http":
            return
        method = scope["method"].upper()
        try:
            resp = await self.handle(scope, receive, method)
        except Exception:  # pragma: no cover - defensive
            log.error("unhandled error\n%s", traceback.format_exc())
            err = internal_error()
            resp = Response.json(err.status, err.body)
        await resp.send(send, head=(method == "HEAD"))

    # ------------------------------------------------------------------ dispatch

    def _api_key(self, scope: Scope) -> str | None:
        """§2.3: split on whitespace, need exactly 2 tokens, the 2nd is the key.

        The scheme word is not checked (``BEARER k`` works); ``Bearer k extra``
        is a 401 (golden auth_header_variants).
        """
        auth = _header(scope, b"authorization")
        if auth is None:
            return None
        tokens = auth.split()
        if len(tokens) != 2 or tokens[1] not in self.api_keys:
            return None
        return tokens[1]

    async def handle(self, scope: Scope, receive: Receive, method: str) -> Response:
        path: str = scope["path"]
        if path == "/_mock" or path.startswith("/_mock/"):
            return await self.handle_admin(scope, receive, method, path)

        r = route(path)
        if r is None:
            # 1. Unknown route: HTML 404. Production serves its full 404 page
            # unless the request carried a body, in which case the 404 is
            # empty. [I] fitted to every HTML 404 in .prod-state/requests.jsonl
            # (e.g. bodiless POST .../restore -> page, POST /api/v1/foo with a
            # JSON body -> empty); data-model.md §2.1 says "GET -> page, other
            # methods -> empty", which the log contradicts for bodiless POSTs.
            raw = await _read_body(receive)
            if raw:
                return Response(404, b"", "text/html; charset=utf-8")
            return Response(404, NOT_FOUND_HTML, "text/html; charset=utf-8")
        kind, node_id = r
        allowed, style, allow_header = ROUTES[kind]
        if method not in allowed:
            # 2. Method check (before auth). HEAD and OPTIONS are not allowed
            # on the JSON-style routes either.
            if style == "json":
                return Response.json(405, {"error": "Method not allowed", "allowed_methods": list(allowed)})
            return Response(405, b"", "text/html; charset=utf-8", [("allow", allow_header or "")])

        # 3. Auth.
        api_key = self._api_key(scope)
        if api_key is None:
            if method in ("GET", "HEAD"):
                return Response.json(401, {"errors": AUTH_MESSAGE})
            return Response.json(401, {"code": "invalid_credentials", "message": AUTH_MESSAGE})

        try:
            return Response.json(200, await self.dispatch(kind, node_id, method, scope, receive, api_key))
        except _InvalidJson:
            return Response.json(400, INVALID_JSON)
        except RequestValidationFailed as e:
            return Response.json(400, {"errors": e.errors})
        except ApiError as e:
            return Response.json(e.status, e.body, e.headers.items())
        except Exception:
            # Production answers unexpected failures with this JSON 500 (§2.6).
            # This also covers strings SQLite cannot store, e.g. a JSON lone
            # surrogate ("\ud800") in a name -- UNVERIFIED what production does.
            log.error("internal error\n%s", traceback.format_exc())
            err = internal_error()
            return Response.json(err.status, err.body)

    async def _body(self, scope: Scope, receive: Receive) -> Any:
        raw = await _read_body(receive)
        return parse_body(_header(scope, b"content-type"), raw)

    async def dispatch(
        self, kind: str, node_id: str | None, method: str, scope: Scope, receive: Receive, api_key: str
    ) -> Any:
        ops = self.ops
        match kind, method:
            case "nodes", "GET":
                # Repeated parameter: the last one wins. Other params ignored.
                qs = scope.get("query_string", b"").decode("utf-8", errors="replace")
                parent_id = None
                for k, v in urllib.parse.parse_qsl(qs, keep_blank_values=True):
                    if k == "parent_id":
                        parent_id = v
                # UNVERIFIED (open-questions S4): no parent_id -> the root.
                return ops.list_children(parent_id)
            case "nodes", "POST":
                return ops.create(await self._body(scope, receive))
            case "node", "GET":
                return ops.get_node(node_id)  # type: ignore[arg-type]
            case "node", "POST":
                return ops.update(node_id, await self._body(scope, receive))  # type: ignore[arg-type]
            case "node", "DELETE":
                return ops.delete(node_id)  # type: ignore[arg-type]  # body ignored
            case "move", "POST":
                return ops.move(node_id, await self._body(scope, receive))  # type: ignore[arg-type]
            case "complete", "POST":
                return ops.complete(node_id, True)  # type: ignore[arg-type]  # body never parsed
            case "uncomplete", "POST":
                return ops.complete(node_id, False)  # type: ignore[arg-type]
            case "mirror", "POST":
                return ops.mirror(node_id, await self._body(scope, receive))  # type: ignore[arg-type]
            case "mirror", "DELETE":
                return ops.delete_mirror(node_id)  # type: ignore[arg-type]
            case "export", ("GET" | "HEAD"):
                return ops.export(api_key)
            case "targets", ("GET" | "HEAD"):
                return ops.targets()
        raise AssertionError(f"unrouted {kind} {method}")

    # --------------------------------------------------------------- admin API

    async def handle_admin(self, scope: Scope, receive: Receive, method: str, path: str) -> Response:
        """Test-only routes (no auth). Documented in docs/mock-server.md."""
        ops = self.ops
        sub = path[len("/_mock"):].strip("/")
        try:
            body: Any = {}
            if method in ("POST", "PUT"):
                raw = await _read_body(receive)
                body = json.loads(raw) if raw.strip() else {}
            match sub.split("/"), method:
                case ["reset"], "POST":
                    # {"all": true} also restores the startup config and
                    # zeroes the clock offset.
                    full = bool(body.get("all")) if isinstance(body, dict) else False
                    ops.reset(full=full)
                    if full:
                        self.clock.offset = 0.0
                    return Response.json(200, {"status": "ok"})
                case ["dump"], "GET":
                    return Response.json(200, ops.dump())
                case ["config"], "GET":
                    return Response.json(200, ops.config.to_json())
                case ["config"], "POST":
                    cfg = ops.config
                    if "faithful_cycles" in body:
                        cfg.faithful_cycles = bool(body["faithful_cycles"])
                    if "export_min_interval" in body:
                        cfg.export_min_interval = float(body["export_min_interval"])
                    if "week_start" in body:
                        cfg.week_start = int(body["week_start"]) % 7
                    return Response.json(200, cfg.to_json())
                case ["clock"], "GET":
                    return Response.json(200, {"now": self.clock(), "offset": self.clock.offset})
                case ["clock"], "POST":
                    if "advance" in body:
                        self.clock.offset += float(body["advance"])
                    if "now" in body:
                        self.clock.offset = float(body["now"]) - self.clock.base()
                    if "offset" in body:
                        self.clock.offset = float(body["offset"])
                    return Response.json(200, {"now": self.clock(), "offset": self.clock.offset})
                case ["shortcuts"], "POST":
                    ops.set_shortcut(str(body["key"]), str(body["node_id"]))
                    return Response.json(200, {"status": "ok"})
                case ["shortcuts", key], "DELETE":
                    if not ops.delete_shortcut(urllib.parse.unquote(key)):
                        return Response.json(404, {"code": "not_found", "message": "Shortcut not found"})
                    return Response.json(200, {"status": "ok"})
        except ApiError as e:
            return Response.json(e.status, e.body)
        except (ValueError, KeyError, TypeError) as e:
            return Response.json(400, {"code": "bad_admin_request", "message": str(e)})
        return Response.json(404, {"code": "not_found", "message": f"unknown admin route {method} {path}"})


def create_app(
    db_path: str,
    api_keys: Iterable[str] = ("test-key",),
    clock: Callable[[], float] | None = None,
    config: Config | None = None,
    id_seed: int | None = None,
) -> MockApp:
    return MockApp(Store(db_path), api_keys=api_keys, clock=clock, config=config, id_seed=id_seed)
