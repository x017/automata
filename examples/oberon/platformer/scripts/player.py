"""player: runs, jumps, collects coins, stomps slimes and reaches the flag.

A/D or the arrow keys run; Space (or W / up) jumps. Hold the jump for a
higher one. Jumping still works for a moment after running off a ledge
("coyote time") and a press just before landing counts ("jump buffer"):
two small forgivenesses that make a platformer feel fair.

The sprite sheet's clips (idle, run, jump, fall) follow what the body is
doing, and it faces the way it runs. Touching a slime from the side, or
falling off the level, sends you back to the start; landing on a slime
squashes it.
"""

from __future__ import annotations

import math
from typing import TYPE_CHECKING

if TYPE_CHECKING:  # types for your editor; nothing is imported when it runs
    from automata.script_api import Context, Entity

SPEED = 7.0  # cells per second
ACCEL = 60.0  # how fast it gets up to speed (cells/s per second)
JUMP = 13.5  # take-off speed: about 3 cells high
GRAVITY = 28.0
FALL_MAX = 18.0
CUT = 0.45  # letting go of jump early keeps this much of the upward speed
COYOTE = 0.1  # seconds after leaving a ledge a jump still counts
BUFFER = 0.12  # seconds before landing a jump press is remembered
STOMP_BOUNCE = 10.0
PICKUP = 0.7  # how close a coin has to be
TOUCH = (0.75, 0.8)  # half the box (x, y) a slime hurts within
FALL_OUT = -3.0  # below this the player has fallen off the level


def jump_held(keys) -> bool:
    return keys.pressed("space") or keys.pressed("w") or keys.pressed("arrow_up")


def jump_pressed(keys) -> bool:
    return keys.just_pressed("space") or keys.just_pressed("w") or keys.just_pressed("arrow_up")


def start(entity: Entity, ctx: Context) -> None:
    s = entity.state
    s["spawn"] = entity.transform.position
    s["vx"] = 0.0
    s["vy"] = 0.0
    s["since_ground"] = 0.0
    s["since_press"] = 99.0
    s["jumped"] = False  # only jumps you pressed for get cut short
    s["won"] = False
    s["deaths"] = 0
    s["total"] = sum(1 for e in ctx.scene if e.name.startswith("Coin"))
    entity.attributes["coins"] = 0
    entity.attributes["deaths"] = 0
    entity.attributes["won"] = False


def approach(v: float, target: float, step: float) -> float:
    if v < target:
        return min(v + step, target)
    return max(v - step, target)


def respawn(entity: Entity, s: dict) -> None:
    entity.transform.position = s["spawn"]
    s["vx"] = s["vy"] = 0.0
    s["deaths"] += 1
    entity.attributes["deaths"] = s["deaths"]


def update(entity: Entity, ctx: Context) -> None:
    s = entity.state
    keys = ctx.input
    dt = ctx.time.delta

    x_in = keys.axis("a", "d") + keys.axis("arrow_left", "arrow_right")
    x_in = max(-1.0, min(1.0, x_in)) if not s["won"] else 0.0

    # Running: ease toward the wanted speed (quicker to turn than to start).
    target = x_in * SPEED
    turning = s["vx"] * target < 0
    s["vx"] = approach(s["vx"], target, ACCEL * dt * (2.0 if turning else 1.0))

    # Jumping, with coyote time, a jump buffer and a variable height.
    grounded = entity.grounded
    s["since_ground"] = 0.0 if grounded else s["since_ground"] + dt
    s["since_press"] = 0.0 if jump_pressed(keys) and not s["won"] else s["since_press"] + dt
    vy = s["vy"]
    if s["since_press"] <= BUFFER and s["since_ground"] <= COYOTE and vy <= 0.0:
        vy = JUMP
        s["since_press"] = s["since_ground"] = 99.0
        s["jumped"] = True
    elif grounded and vy <= 0.0:
        vy = -2.0  # keep in touch with the ground
        s["jumped"] = False
    else:
        if vy > 0.0 and s["jumped"] and not jump_held(keys):
            vy *= CUT ** (dt * 60.0) if vy > JUMP * 0.3 else 1.0
        vy = max(vy - GRAVITY * dt, -FALL_MAX)
    s["vy"] = vy
    entity.character_velocity = (s["vx"], vy, 0.0)

    # The sprite: face the way it runs, and pick the clip.
    sprite = entity.sprite
    if sprite is not None:
        if abs(x_in) > 0.01:
            sprite.flip_x = x_in < 0
        if not grounded and s["since_ground"] > COYOTE:
            sprite.play("jump" if vy > 0 else "fall")
        elif abs(s["vx"]) > 0.5:
            sprite.play("run")
        else:
            sprite.play("idle")

    px, py, _ = entity.transform.position
    if py < FALL_OUT:
        respawn(entity, s)
        return

    for e in ctx.scene:
        name = e.name
        if name.startswith("Coin"):
            cx, cy, _ = e.transform.position
            if math.hypot(cx - px, cy - py) < PICKUP:
                e.despawn()
                entity.attributes["coins"] = int(entity.attributes.get("coins", 0)) + 1
        elif name.startswith("Slime"):
            sx, sy, _ = e.transform.position
            if abs(sx - px) < TOUCH[0] and abs(sy - py) < TOUCH[1]:
                if vy < 0 and py > sy + 0.3:
                    e.despawn()  # landed on it
                    s["vy"] = STOMP_BOUNCE
                    s["jumped"] = False
                else:
                    respawn(entity, s)
                    return
        elif name == "Flag" and not s["won"]:
            fx, fy, _ = e.transform.position
            if abs(fx - px) < 0.8 and abs(fy - py) < 1.5:
                s["won"] = True
                entity.attributes["won"] = True
                print(f"Made it! {entity.attributes['coins']} of {s['total']} coins.")

    draw_hud(entity, ctx)


def draw_hud(entity: Entity, ctx: Context) -> None:
    s = entity.state
    hud = ctx.hud
    w, h = hud.size
    coins = int(entity.attributes.get("coins", 0))
    hud.image("sprites/coin_icon.png", 36, 36, size=(32, 32))
    hud.text(f"{coins} / {s['total']}", 58, 36, size=28, shadow=True)
    if s["won"]:
        hud.rect(w / 2, h / 2, 520, 120, (0.1, 0.08, 0.2, 0.75))
        hud.text("You made it!", w / 2, h / 2 - 18, size=44, align="center", shadow=True)
        hud.text(
            f"{coins} of {s['total']} coins",
            w / 2,
            h / 2 + 28,
            size=26,
            color=(1.0, 0.85, 0.3, 1.0),
            align="center",
            shadow=True,
        )
