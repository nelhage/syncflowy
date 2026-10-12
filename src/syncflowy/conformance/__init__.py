"""Differential conformance suite: run identical scenarios against production
Workflowy and the mock server, normalise the transcripts, and diff them.

See ``docs/conformance.md`` for the design. The main pieces:

* :mod:`.backend`   -- ``ProdBackend`` (via ``ProdClient``) and ``MockBackend``
  (plain HTTP to a mock base URL), plus ``FakeClock`` for the mock.
* :mod:`.context`   -- ``Ctx``, the object a scenario function receives; it
  records every request/response in order.
* :mod:`.registry`  -- the ``@scenario`` decorator and scenario lookup.
* :mod:`.normalize` -- turns a raw transcript into a backend-independent one
  (symbolic ids, epoch-relative timestamps).
* :mod:`.runner`    -- running scenarios, golden file I/O, diffing.
* :mod:`.cli`       -- the ``syncflowy-conformance`` command.
* :mod:`.scenarios` -- the scenarios themselves, one module per area.
"""

from .registry import Scenario, all_scenarios, scenario, select_scenarios

__all__ = ["Scenario", "all_scenarios", "scenario", "select_scenarios"]
