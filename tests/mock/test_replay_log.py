"""Replay the production request log (.prod-state/requests.jsonl) against the mock.

Two kinds of replay:

1. **State-independent responses** -- validation errors, invalid_json, 401,
   405 and unknown-route 404s do not depend on what exists, so every such
   logged request is re-sent verbatim and the mock must answer with the same
   status, content-type and (for JSON) the same body, key order included.

2. **Creates** -- every successful logged ``POST /api/v1/nodes`` whose
   created node was later observed (via GET or list) before anything mutated
   it is re-run against a fresh sandbox in the mock; the stored name, note,
   layoutMode and completion, and its multiline children, must match what
   production stored.

The log is local, untracked state; the tests skip if it is absent.
"""

from __future__ import annotations

import bisect
import json
import re
from typing import Any

import pytest

from . import golden
from .conftest import WF

RECORDS = golden.log_records()
UUID_RE = re.compile(r"[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}")
pytestmark = pytest.mark.skipif(not RECORDS, reason="no production request log")
PROD_HOST = "https://workflowy.com"


def _is_prod(r: dict[str, Any]) -> bool:
    """Records from before the ``host`` field are all production; beta-host
    records follow different (mirror-aware) semantics and are not replayed."""
    return r.get("host", PROD_HOST) == PROD_HOST


def _param(r: dict[str, Any], key: str) -> Any:
    params = r.get("params") or {}
    items = params.items() if isinstance(params, dict) else params
    val = None
    for k, v in items:
        if k == key:
            val = v
    return val


def _kind(r: dict[str, Any]) -> str | None:
    resp, status = r["response"], r["status"]
    if status == 400 and isinstance(resp, dict) and ("errors" in resp or resp.get("code") == "invalid_json"):
        return "validation"
    if status == 401:
        return "auth"
    if status == 405:
        return "method"
    if status == 404 and not isinstance(resp, dict):
        return "route"
    return None


STATELESS = [(i, r) for i, r in enumerate(RECORDS) if _is_prod(r) and _kind(r)]


def _send(wf: WF, r: dict[str, Any], auth: bool):
    client = wf.http if auth else wf.anon
    kw: dict[str, Any] = {}
    headers = {k: v for k, v in (r.get("request_headers") or {}).items() if k.lower() == "content-type"}
    if r.get("request_json") is not None:
        kw["content"] = json.dumps(r["request_json"]).encode()
        headers.setdefault("content-type", "application/json")
    elif r.get("request_content") is not None:
        kw["content"] = r["request_content"].encode("utf-8")
    if r.get("params"):
        kw["params"] = r["params"]
    return client.request(r["method"], r["path"], headers=headers, **kw)


@pytest.mark.parametrize("idx,rec", STATELESS, ids=[f"{i}-{r['method']}-{r['status']}" for i, r in STATELESS])
def test_replay_stateless(wf: WF, idx: int, rec: dict[str, Any]) -> None:
    kind = _kind(rec)
    got = _send(wf, rec, auth=(kind != "auth"))
    assert got.status_code == rec["status"]
    want_ct = rec["response_headers"].get("content-type")
    assert got.headers.get("content-type") == want_ct
    if "allow" in rec["response_headers"]:
        assert got.headers.get("allow") == rec["response_headers"]["allow"]
    resp = rec["response"]
    if isinstance(resp, str) and want_ct == "application/json" and resp:
        resp = json.loads(resp)
    if isinstance(resp, (dict, list)):
        assert json.dumps(got.json()) == json.dumps(resp)  # same content and key order
    elif isinstance(resp, str) and resp.startswith("<!DOCTYPE"):
        assert "<title>404</title>" in got.text  # we serve a stub, not the 20 KB page
    elif rec["method"] != "HEAD":
        assert got.text == (resp or "")


# ---------------------------------------------------------------- create replay


def _create_cases() -> list[tuple[int, dict[str, Any], dict[str, Any], list[dict[str, Any]] | None]]:
    creates: dict[str, int] = {}
    for i, r in enumerate(RECORDS):
        if (
            _is_prod(r)
            and r["method"] == "POST"
            and r["path"].rstrip("/") == "/api/v1/nodes"
            and r["status"] == 200
            and isinstance(r.get("request_json"), dict)
            and isinstance(r["request_json"].get("name"), str)
            and isinstance(r["response"], dict)
        ):
            creates[r["response"]["item_id"]] = i

    # uuid -> indices of successful mutating requests that mention it (in the
    # path, e.g. update/move/complete, or in the body, e.g. create/move/mirror
    # into it)
    mentions: dict[str, list[int]] = {}
    for j, r in enumerate(RECORDS):
        if r["method"] in ("POST", "DELETE") and r["status"] == 200:
            text = r["path"] + json.dumps(r.get("request_json"))
            for u in set(UUID_RE.findall(text)):
                mentions.setdefault(u, []).append(j)

    def first_mutation(node_id: str, after: int) -> int:
        idxs = mentions.get(node_id, [])
        k = bisect.bisect_right(idxs, after)
        return idxs[k] if k < len(idxs) else len(RECORDS)

    cases = []
    for item_id, ci in creates.items():
        mut = first_mutation(item_id, ci)
        item = None
        kids: list[dict[str, Any]] | None = None
        for j in range(ci + 1, min(mut, ci + 400)):  # observations come soon after the create
            if item is not None and kids is not None:
                break
            r = RECORDS[j]
            if r["method"] != "GET" or r["status"] != 200 or not isinstance(r["response"], dict):
                continue
            if item is None:
                if "node" in r["response"] and r["response"]["node"]["id"] == item_id:
                    item = r["response"]["node"]
                for n in r["response"].get("nodes", []):
                    if n["id"] == item_id:
                        item = n
            if kids is None and _param(r, "parent_id") == item_id and "nodes" in r["response"]:
                kids = r["response"]["nodes"]
                # only trust multiline children: same timestamp, not separately created,
                # and not mutated before this observation
                if any(
                    k["createdAt"] != k["modifiedAt"] or k["id"] in creates or first_mutation(k["id"], ci) < j
                    for k in kids
                ):
                    kids = None
                    break
        if item is None or item["createdAt"] != item["modifiedAt"]:
            continue
        cases.append((ci, RECORDS[ci]["request_json"], item, kids))
    return cases


CREATE_CASES = _create_cases() if RECORDS else []


def test_create_replay_has_coverage() -> None:
    assert len(CREATE_CASES) > 500


@pytest.mark.parametrize("idx,body,item,kids", CREATE_CASES, ids=[f"create-{c[0]}" for c in CREATE_CASES])
def test_replay_create(wf: WF, idx: int, body: dict[str, Any], item: dict[str, Any], kids: list[dict[str, Any]] | None) -> None:
    sb = wf.sandbox()
    body = {**body, "parent_id": sb}
    r = wf.create_raw(body)
    assert r.status_code == 200, r.text
    got = wf.get(r.json()["item_id"])
    for k in ("name", "note", "completed", "data"):
        assert got[k] == item[k], k
    assert (got["completedAt"] is None) == (item["completedAt"] is None)
    if kids is not None:
        got_kids = wf.children(got["id"])
        want = sorted(kids, key=lambda n: n["priority"])
        assert [(k["name"], k["note"], k["priority"], k["completed"], k["data"]) for k in got_kids] == [
            (k["name"], k["note"], k["priority"], k["completed"], k["data"]) for k in want
        ]
