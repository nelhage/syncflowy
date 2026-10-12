"""Differential conformance: replay every recorded production scenario against
the mock and diff the normalised transcripts.

Goldens live in tests/conformance/golden/<scenario>.json and are produced by
``syncflowy-conformance record`` (against production). A scenario without a
golden is skipped; a scenario whose required mock options (e.g.
``faithful_cycles``) the mock does not support is skipped too. See
docs/conformance.md.
"""

from __future__ import annotations

import pytest

from syncflowy.conformance.registry import all_scenarios
from syncflowy.conformance.runner import MockPool, check_scenario, golden_path

SCENARIOS = all_scenarios()


@pytest.fixture(scope="module")
def pool():
    with MockPool() as p:
        yield p


@pytest.mark.parametrize("sc", SCENARIOS, ids=[s.name for s in SCENARIOS])
def test_conformance(sc, pool):
    if not golden_path(sc.name).exists():
        pytest.skip("no golden recording (run `syncflowy-conformance record`)")
    r = check_scenario(sc, pool)
    if r.status == "skip":
        pytest.skip(r.detail)
    if r.status != "pass":
        pytest.fail(f"{sc.name}: mock differs from production\n{sc.doc}\n\n{r.detail}", pytrace=False)
