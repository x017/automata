"""follow_camera: a third-person camera that trails the robot (Q / E orbit)."""

import math

DISTANCE = 6.0
HEIGHT = 2.6
SMOOTH = 5.0
TURN = 2.0  # radians per second


def start(entity, ctx):
    entity.state["yaw"] = 0.0


def update(entity, ctx):
    player = ctx.scene["Robot"]
    yaw = entity.state["yaw"] + ctx.input.axis("q", "e") * TURN * ctx.time.delta
    entity.state["yaw"] = yaw
    cam = entity.camera
    cam.follow(player, offset=(DISTANCE * math.sin(yaw), HEIGHT, DISTANCE * math.cos(yaw)),
               smooth=SMOOTH)
    x, y, z = player.transform.position
    cam.look_at((x, y + 0.4, z))
