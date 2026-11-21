"""follow_camera: a smooth third-person camera with Q / E orbit."""

import math

DISTANCE = 8.0
HEIGHT = 3.5
TURN = 2.0  # radians per second


def start(entity, ctx):
    entity.state["yaw"] = 0.0


def update(entity, ctx):
    turn = float(ctx.input.pressed("e")) - float(ctx.input.pressed("q"))
    yaw = entity.state["yaw"] + turn * TURN * ctx.time.delta
    entity.state["yaw"] = yaw
    offset = (DISTANCE * math.sin(yaw), HEIGHT, DISTANCE * math.cos(yaw))
    entity.camera.follow(ctx.scene["Player"], offset=offset, smooth=6.0)
