"""Request models, mirroring production's pydantic models.

Production leaks its pydantic model names in ``model_type`` errors
(``Input should be a valid dictionary or instance of CreateItemRequest``), and
its validation errors are pydantic v2 errors trimmed to
``{type, loc, msg, input}`` (data-model.md §2.5, errors-and-http.md §6.2). We
use pydantic v2 with the same class names and the same field declaration
order, so error types, messages, ordering and the "collect every error"
behaviour come out identical without hand-written formatting.

Field declaration orders (observed, followup-2):

=====================  ==========================================  ===========
model                  fields, in order                            required
=====================  ==========================================  ===========
``CreateItemRequest``  name, parent_id, note, layoutMode, position name
``UpdateNodeRequest``  name, note, layoutMode                       none
``MoveNodeRequest``    parent_id, position                          none
``MirrorNodeRequest``  parent_id, position                          parent_id
=====================  ==========================================  ===========

Undeclared keys are ignored and never type-checked (pydantic's default
``extra="ignore"``). ``position`` is an ``Enum`` (not a ``Literal``), because
production reports ``"type": "enum"`` rather than ``literal_error``.
"""

from __future__ import annotations

import enum
from typing import Any

from pydantic import BaseModel, ValidationError


class Position(str, enum.Enum):
    top = "top"
    bottom = "bottom"


class CreateItemRequest(BaseModel):
    name: str
    parent_id: str | None = None
    note: str | None = None
    layoutMode: str | None = None
    position: Position | None = None


class UpdateNodeRequest(BaseModel):
    name: str | None = None
    note: str | None = None
    layoutMode: str | None = None


class MoveNodeRequest(BaseModel):
    parent_id: str | None = None
    position: Position | None = None


class MirrorNodeRequest(BaseModel):
    parent_id: str
    position: Position | None = None


class RequestValidationFailed(Exception):
    """Carries the production-shaped ``errors`` list."""

    def __init__(self, errors: list[dict[str, Any]]):
        super().__init__(errors)
        self.errors = errors


def validate[M: BaseModel](model: type[M], body: Any) -> M:
    """Validate ``body`` (already parsed JSON / form dict) against ``model``.

    Raises :class:`RequestValidationFailed` with error items in production's
    key order ``type, loc, msg, input`` (no ``url``/``ctx``).
    """
    try:
        return model.model_validate(body)
    except ValidationError as e:
        errors = [
            {"type": err["type"], "loc": list(err["loc"]), "msg": err["msg"], "input": err["input"]}
            for err in e.errors(include_url=False, include_context=False)
        ]
        raise RequestValidationFailed(errors) from None


def position_str(p: Position | None) -> str | None:
    return p.value if p is not None else None
