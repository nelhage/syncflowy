"""Scenario registry.

A scenario is a plain function ``fn(ctx)`` decorated with ``@scenario``. Its
name is the function name; its documentation is the docstring. Scenarios live
in ``syncflowy.conformance.scenarios.*`` and are registered on import.
"""

from __future__ import annotations

import fnmatch
import importlib
import pkgutil
from dataclasses import dataclass, field
from typing import Callable


@dataclass(frozen=True)
class Scenario:
    name: str
    fn: Callable
    doc: str
    tags: tuple[str, ...] = ()
    # Keyword arguments that must be passed to ``running_mock`` for the mock to
    # reproduce production here (e.g. ``{"faithful_cycles": True}``). If the
    # mock's ``running_mock`` does not accept them the scenario is skipped on
    # the mock with an explanation, rather than reported as a failure.
    mock_options: dict = field(default_factory=dict)
    module: str = ""

    @property
    def area(self) -> str:
        return self.module.rsplit(".", 1)[-1]


_REGISTRY: dict[str, Scenario] = {}


def scenario(
    fn: Callable | None = None,
    *,
    tags: tuple[str, ...] | list[str] = (),
    mock_options: dict | None = None,
):
    """Register a scenario. Usable bare (``@scenario``) or with arguments."""

    def wrap(f: Callable) -> Callable:
        name = f.__name__
        if name in _REGISTRY and _REGISTRY[name].fn is not f:
            raise ValueError(f"duplicate scenario name {name!r}")
        module = f.__module__
        area = module.rsplit(".", 1)[-1]
        _REGISTRY[name] = Scenario(
            name=name,
            fn=f,
            doc=(f.__doc__ or "").strip(),
            tags=tuple(tags) + (area,),
            mock_options=dict(mock_options or {}),
            module=module,
        )
        return f

    if fn is not None:
        return wrap(fn)
    return wrap


def _load_all() -> None:
    from . import scenarios

    for mod in pkgutil.iter_modules(scenarios.__path__):
        importlib.import_module(f"{scenarios.__name__}.{mod.name}")


def all_scenarios() -> list[Scenario]:
    _load_all()
    return sorted(_REGISTRY.values(), key=lambda s: (s.module, s.name))


def select_scenarios(patterns: list[str] | None) -> list[Scenario]:
    """Select scenarios by glob on the name, or ``@tag`` (area/module name or tag).

    No patterns selects everything.
    """
    scs = all_scenarios()
    if not patterns:
        return scs
    out = []
    for s in scs:
        for p in patterns:
            if p.startswith("@"):
                if p[1:] in s.tags:
                    out.append(s)
                    break
            elif fnmatch.fnmatchcase(s.name, p):
                out.append(s)
                break
    return out
