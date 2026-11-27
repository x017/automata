"""sky: the backdrop drifts slower than the level (parallax).

It keeps most of the camera's movement (``FOLLOW``), so the hills seem far
away. The image is big enough to fill the view wherever the camera goes.
"""

from __future__ import annotations

from typing import TYPE_CHECKING

if TYPE_CHECKING:
    from automata.script_api import Context, Entity

FOLLOW = 0.85


def start(entity: Entity, ctx: Context) -> None:
    entity.state["home"] = entity.transform.position


def update(entity: Entity, ctx: Context) -> None:
    camera = ctx.scene.find("Camera")
    if camera is None:
        return
    hx, hy, hz = entity.state["home"]
    cx, cy, _ = camera.transform.position
    entity.transform.position = (
        hx + (cx - hx) * FOLLOW,
        hy + (cy - hy) * FOLLOW,
        hz,
    )
