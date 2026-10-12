"""Remove the recursive mirror left by exp07 (mirror of A placed under A's child A1)."""
from common import c, show
show("REMOVE mirror eb5a168a", c.request("DELETE", "/api/v1/nodes/eb5a168a-a284-4187-9e03-ef2f1b371ab4/mirror"))
show("LIST A1", c.list("c36b4c86-00da-4a65-850e-6f4a13e218d7"))
