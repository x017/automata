"""greeter: waves once when the player walks up, then goes back to idling.

``play(..., loop=False)`` runs a clip once; ``animation.playing`` turns
False when it's done, and the next ``play`` blends back to the idle clip.
"""

import math

REACH = 3.0  # metres


def start(entity, ctx):
    entity.state["near"] = False


def update(entity, ctx):
    player = ctx.scene.find("Robot")
    if player is None:
        return
    near = math.dist(player.transform.position, entity.transform.position) < REACH
    anim = entity.animation
    if near and not entity.state["near"]:
        # Turn to the player and wave.
        (px, _, pz), (ex, _, ez) = player.transform.position, entity.transform.position
        entity.transform.rotation_euler = (0.0, math.atan2(px - ex, pz - ez), 0.0)
        anim.play("Wave", loop=False, fade=0.2)
        entity.attributes["waves"] = entity.attributes.get("waves", 0) + 1
    elif anim.clip == "Wave" and not anim.playing:
        anim.play("Idle", fade=0.4)
    entity.state["near"] = near
