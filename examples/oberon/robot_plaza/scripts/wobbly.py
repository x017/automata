"""wobbly: falls over when you bump into it, then gets back up.

``entity.ragdoll.on(push=...)`` turns the animation off and hands every
bone to physics, with a shove in the direction you were walking. When the
body has stopped moving (``ragdoll.resting``) and had a moment on the
ground, ``ragdoll.off()`` stands it up where it fell and blends back into
its animation over the Ragdoll's **Blend Time**.
"""

import math

BUMP = 0.9  # metres between you and it that count as a bump
SHOVE = 220.0  # newton-seconds: about a 3 m/s push on a 70 kg robot
LIE_STILL = 1.5  # seconds on the ground before it gets up


def start(entity, ctx):
    entity.state["down"] = 0.0


def push_from(player, entity):
    """A shove away from ``player``, a little upwards so it tips over."""
    (px, _, pz), (ex, _, ez) = player.transform.position, entity.transform.position
    dx, dz = ex - px, ez - pz
    length = math.hypot(dx, dz) or 1.0
    return (dx / length * SHOVE, SHOVE * 0.15, dz / length * SHOVE)


def update(entity, ctx):
    rag = entity.ragdoll
    if rag is None:
        return
    if rag.limp:
        if rag.resting:
            entity.state["down"] += ctx.time.delta
        if entity.state["down"] > LIE_STILL:
            rag.off()
            entity.state["down"] = 0.0
        return
    if rag.blending:
        return  # still getting up
    player = ctx.scene.find("Robot")
    if player is None:
        return
    if math.dist(player.transform.position, entity.transform.position) < BUMP:
        rag.on(push=push_from(player, entity))
        entity.attributes["falls"] = entity.attributes.get("falls", 0) + 1
