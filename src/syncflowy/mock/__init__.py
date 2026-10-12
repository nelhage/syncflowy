"""A high-fidelity mock of the Workflowy REST API (``/api/v1``), backed by SQLite.

Behaviour follows the reverse-engineered spec in docs/workflowy/data-model.md;
the implementation map and known deviations are in docs/mock-server.md.

Entry points:

* ``syncflowy-mock`` console script (:mod:`syncflowy.mock.cli`)
* :func:`syncflowy.mock.testing.running_mock` for tests
* :func:`syncflowy.mock.app.create_app` for embedding the ASGI app
"""

from .app import MockApp, create_app
from .ops import Config

__all__ = ["Config", "MockApp", "create_app"]
