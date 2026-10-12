"""Validation error bodies (data-model.md §2.5; errors-and-http.md §6.2; followup-2)."""

from __future__ import annotations

import json
from typing import Any

import pytest

from .conftest import WF, rand_uuid

STR = "Input should be a valid string"
ENUM = "Input should be 'top' or 'bottom'"


def err(type_: str, loc: list[Any], msg: str, input_: Any) -> dict[str, Any]:
    return {"type": type_, "loc": loc, "msg": msg, "input": input_}


def assert_errors(r, expected: list[dict[str, Any]]) -> None:
    assert r.status_code == 400, r.text
    assert r.headers["content-type"] == "application/json"
    # exact bytes, including key order type, loc, msg, input
    assert r.content == json.dumps({"errors": expected}).encode()


# --------------------------------------------------------------- create


def test_create_all_errors_in_declaration_order(wf: WF) -> None:
    body = {"position": "q", "layoutMode": 3, "note": 2, "name": 1, "parent_id": 1}
    assert_errors(
        wf.create_raw(body),
        [
            err("string_type", ["name"], STR, 1),
            err("string_type", ["parent_id"], STR, 1),
            err("string_type", ["note"], STR, 2),
            err("string_type", ["layoutMode"], STR, 3),
            err("enum", ["position"], ENUM, "q"),
        ],
    )


def test_create_missing_name_input_is_whole_body(wf: WF) -> None:
    assert_errors(
        wf.create_raw({"parent_id": 1}),
        [err("missing", ["name"], "Field required", {"parent_id": 1}), err("string_type", ["parent_id"], STR, 1)],
    )
    assert_errors(wf.create_raw({}), [err("missing", ["name"], "Field required", {})])


@pytest.mark.parametrize("value", [None, 42, True, ["a", "b"], {"x": 1}, 1.5])
def test_create_name_types(wf: WF, value: Any) -> None:
    assert_errors(wf.create_raw({"name": value}), [err("string_type", ["name"], STR, value)])


@pytest.mark.parametrize("value", ["middle", "TOP", "Bottom", "", 0, 1, 5, -1, True, "1", ["top"]])
def test_create_position_enum(wf: WF, value: Any) -> None:
    assert_errors(wf.create_raw({"name": "x", "position": value}), [err("enum", ["position"], ENUM, value)])


@pytest.mark.parametrize("pid", [None, ""])
def test_create_parent_id_nullable_and_empty_pass_validation(wf: WF, pid: Any) -> None:
    assert_errors(wf.create_raw({"name": 1, "parent_id": pid}), [err("string_type", ["name"], STR, 1)])


def test_create_validation_before_parent_lookup(wf: WF) -> None:
    pid = rand_uuid()
    assert_errors(
        wf.create_raw({"parent_id": pid}), [err("missing", ["name"], "Field required", {"parent_id": pid})]
    )
    assert_errors(
        wf.create_raw({"parent_id": pid, "name": "x", "position": "middle"}),
        [err("enum", ["position"], ENUM, "middle")],
    )


def test_create_ignores_undeclared_keys(wf: WF) -> None:
    sb = wf.sandbox()
    r = wf.create_raw(
        {"parent_id": sb, "name": "x", "priority": "bad", "completed": 7, "id": 3, "data": [], "foo": {}}
    )
    assert r.status_code == 200


@pytest.mark.parametrize("body", [[1, 2], "hello", None, 5, [{"name": "x"}]])
def test_create_model_type(wf: WF, body: Any) -> None:
    assert_errors(
        wf.create_raw(content=json.dumps(body).encode()),
        [err("model_type", [], "Input should be a valid dictionary or instance of CreateItemRequest", body)],
    )


# --------------------------------------------------------------- update


def test_update_errors(wf: WF) -> None:
    nid = rand_uuid()
    assert_errors(
        wf.update_raw(nid, {"layoutMode": 3, "note": ["x"], "name": 123, "parent_id": 5, "position": "zzz"}),
        [
            err("string_type", ["name"], STR, 123),
            err("string_type", ["note"], STR, ["x"]),
            err("string_type", ["layoutMode"], STR, 3),
        ],
    )


@pytest.mark.parametrize("body", [[{"note": "array"}], "hello", None, [1]])
def test_update_model_type(wf: WF, body: Any) -> None:
    assert_errors(
        wf.update_raw(rand_uuid(), content=json.dumps(body).encode()),
        [err("model_type", [], "Input should be a valid dictionary or instance of UpdateNodeRequest", body)],
    )


def test_update_nulls_pass(wf: WF) -> None:
    r = wf.update_raw(rand_uuid(), {"name": None, "note": None, "layoutMode": None})
    assert r.status_code == 404  # validation passed, then lookup failed


# --------------------------------------------------------------- move


def test_move_errors(wf: WF) -> None:
    assert_errors(
        wf.move_raw(rand_uuid(), {"position": "q", "parent_id": 123}),
        [err("string_type", ["parent_id"], STR, 123), err("enum", ["position"], ENUM, "q")],
    )


@pytest.mark.parametrize("body", [{}, {"parent_id": None}, {"parent_id": ""}])
def test_move_parent_optional(wf: WF, body: dict[str, Any]) -> None:
    r = wf.move_raw(rand_uuid(), body)
    assert r.status_code == 404 and r.json()["message"] == "Item not found"


@pytest.mark.parametrize("body", [[1], "s", None])
def test_move_model_type(wf: WF, body: Any) -> None:
    assert_errors(
        wf.move_raw(rand_uuid(), content=json.dumps(body).encode()),
        [err("model_type", [], "Input should be a valid dictionary or instance of MoveNodeRequest", body)],
    )


# --------------------------------------------------------------- mirror


def test_mirror_errors(wf: WF) -> None:
    assert_errors(wf.mirror_raw(rand_uuid(), {}), [err("missing", ["parent_id"], "Field required", {})])
    assert_errors(wf.mirror_raw(rand_uuid(), {"parent_id": None}), [err("string_type", ["parent_id"], STR, None)])
    assert_errors(wf.mirror_raw(rand_uuid(), {"parent_id": 5}), [err("string_type", ["parent_id"], STR, 5)])
    assert_errors(
        wf.mirror_raw(rand_uuid(), {"position": "q"}),
        [err("missing", ["parent_id"], "Field required", {"position": "q"}), err("enum", ["position"], ENUM, "q")],
    )


def test_mirror_empty_parent_passes_validation(wf: WF) -> None:
    r = wf.mirror_raw(rand_uuid(), {"parent_id": ""})
    assert r.status_code == 404 and r.json()["message"] == "Item not found"


@pytest.mark.parametrize("body", [[1], "s", None])
def test_mirror_model_type(wf: WF, body: Any) -> None:
    assert_errors(
        wf.mirror_raw(rand_uuid(), content=json.dumps(body).encode()),
        [err("model_type", [], "Input should be a valid dictionary or instance of MirrorNodeRequest", body)],
    )


def test_validation_beats_all_lookups(wf: WF) -> None:
    sb = wf.sandbox()
    # real node, self-mirror (semantic error) + bad position -> enum wins
    assert_errors(wf.mirror_raw(sb, {"parent_id": sb, "position": "middle"}), [err("enum", ["position"], ENUM, "middle")])
    assert_errors(
        wf.move_raw(rand_uuid(), {"parent_id": rand_uuid(), "position": "middle"}),
        [err("enum", ["position"], ENUM, "middle")],
    )


def test_form_values_are_strings(wf: WF) -> None:
    sb = wf.sandbox()
    # form fields are always strings, so "position=1" is an enum error with input "1"
    assert_errors(wf.create_raw(data={"parent_id": sb, "name": "x", "position": "1"}), [err("enum", ["position"], ENUM, "1")])
