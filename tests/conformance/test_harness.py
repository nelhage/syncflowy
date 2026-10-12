"""Harness self-consistency: every scenario, run on two independent mock
instances (different ids, different fake-clock start times), must normalise to
identical transcripts. This checks that the normaliser removes all
run-specific detail (ids, absolute timestamps) and that scenarios are
deterministic. It does not need production goldens."""

from __future__ import annotations

import pytest

from syncflowy.conformance.backend import FakeClock, MockBackend, VettingMockBackend
from syncflowy.conformance.normalize import normalize
from syncflowy.conformance.registry import all_scenarios
from syncflowy.conformance.runner import diff_transcripts, run_scenario

running_mock = pytest.importorskip("syncflowy.mock.testing").running_mock

SCENARIOS = all_scenarios()


@pytest.fixture(scope="module")
def two_mocks():
    clocks = [FakeClock(1_800_000_000.5), FakeClock(1_900_000_123.5)]
    with running_mock(api_key="k", clock=clocks[0], faithful_cycles=True) as u1, \
         running_mock(api_key="k", clock=clocks[1], faithful_cycles=True) as u2:
        # The first backend also applies the production safety vetting, so a
        # scenario that production would refuse fails here, offline.
        bs = [VettingMockBackend(u1, "k", clocks[0]), MockBackend(u2, "k", clocks[1])]
        # Desynchronise the two id/creation sequences.
        bs[1].setup("warmup")
        yield bs
        for b in bs:
            b.close()


@pytest.mark.parametrize("sc", SCENARIOS, ids=[s.name for s in SCENARIOS])
def test_scenario_is_deterministic_on_mock(sc, two_mocks):
    a = normalize(run_scenario(two_mocks[0], sc))
    b = normalize(run_scenario(two_mocks[1], sc))
    assert not diff_transcripts(a, b), diff_transcripts(a, b)
    flat = repr(a)
    assert "<E?" not in flat and "|E" not in flat, "timestamp not attributable to an epoch"
