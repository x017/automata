"""follow_camera: a third-person camera that trails the player (Q / E orbit)."""

import math

DISTANCE = 4.5
HEIGHT = 1.8
SMOOTH = 5.0
TURN = 2.0  # radians per second


def start(entity, ctx):
    entity.state["yaw"] = 0.0


def update(entity, ctx):
    player = ctx.scene.find("Runner")
    if player is None:
        return
    yaw = entity.state["yaw"] + ctx.input.axis("q", "e") * TURN * ctx.time.delta
    entity.state["yaw"] = yaw
    cam = entity.camera
    cam.follow(player, offset=(DISTANCE * math.sin(yaw), HEIGHT, DISTANCE * math.cos(yaw)),
               smooth=SMOOTH)
    x, y, z = player.transform.position
    cam.look_at((x, y + 1.1, z))
