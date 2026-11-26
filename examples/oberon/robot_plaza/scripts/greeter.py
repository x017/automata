"""greeter: watches the player, and waves once when they walk up.

``animation.look_at(player)`` turns the head (and a little of the neck)
towards the player, as far as it can without looking over its shoulder;
``look_at(None)`` eases it back. ``play(..., loop=False)`` runs a clip once;
``animation.playing`` turns False when it's done, and the next ``play``
blends back to the idle clip.
"""

import math

REACH = 3.0  # metres
WATCH = 8.0  # metres


def start(entity, ctx):
    entity.state["near"] = False


def update(entity, ctx):
    player = ctx.scene.find("Robot")
    if player is None:
        return
    distance = math.dist(player.transform.position, entity.transform.position)
    near = distance < REACH
    anim = entity.animation
    watching = distance < WATCH
    if watching != anim.looking:
        anim.look_at(player if watching else None)
    if near and not entity.state["near"]:
        # Turn to the player and wave.
        (px, _, pz), (ex, _, ez) = player.transform.position, entity.transform.position
        entity.transform.rotation_euler = (0.0, math.atan2(px - ex, pz - ez), 0.0)
        anim.play("Wave", loop=False, fade=0.2)
        entity.attributes["waves"] = entity.attributes.get("waves", 0) + 1
    elif anim.clip == "Wave" and not anim.playing:
        anim.play("Idle", fade=0.4)
    entity.state["near"] = near
