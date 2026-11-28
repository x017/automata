"""slime: hops back and forth along its ledge.

Attributes (Inspector): range (Float) — how far it wanders either side of
where it starts, in cells.

It turns round at the ends of its range, facing the way it goes. The player
script decides what touching it does (a stomp from above squashes it).
"""

from __future__ import annotations

import math
from typing import TYPE_CHECKING

if TYPE_CHECKING:
    from automata.script_api import Context, Entity

SPEED = 1.5  # cells per second


def start(entity: Entity, ctx: Context) -> None:
    entity.state["home"] = entity.transform.position
    entity.state["dir"] = -1.0


def update(entity: Entity, ctx: Context) -> None:
    hx, hy, hz = entity.state["home"]
    reach = float(entity.attributes.get("range", 2.0))
    x, _, _ = entity.transform.position
    d = entity.state["dir"]
    x += d * SPEED * ctx.time.delta
    if abs(x - hx) >= reach:
        x = hx + math.copysign(reach, x - hx)
        entity.state["dir"] = -d
    entity.transform.position = (x, hy, hz)
    sprite = entity.sprite
    if sprite is not None:
        # The art faces left.
        sprite.flip_x = entity.state["dir"] > 0
