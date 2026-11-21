# Tutorial: Third-person camera

**Goal:** a camera that chases the player smoothly, orbits with Q / E,
looks ahead when moving, and widens its view while sprinting. You'll build
it in four small steps, playing after each one.

**Start from:** the [coin collector](coin-collector.md) scene, which needs
a **Player** and a **Camera** (see [Cameras](../oberon/cameras.md)).

Select the camera, choose **Add Component → Script…**, and name it
`scripts/follow_camera.py`.

## Step 1: Follow

```python
def update(entity, ctx):
    entity.camera.follow(ctx.scene["Player"])
```

Play. The camera keeps 3 m above and 8 m behind (+Z) the player, always
looking at them, and catches up smoothly when they move.

`follow(entity, offset=(0, 3, 8), smooth=8.0, look_height=0.6)` does three
things every frame:

1. Computes the goal: the player's position plus `offset`.
2. Moves part of the way there. That fraction is `1 − e^(−smooth·dt)`,
   which feels the same at any frame rate. Higher `smooth` means
   snappier, and `0` snaps instantly.
3. Aims at the player, `look_height` metres above their origin.

Try `smooth=2` for a lazy cinematic follow, or `offset=(0, 10, 4)` for a
top-down view.

## Step 2: Orbit with Q / E

The offset doesn't have to be fixed. Rotate it around the player with a
yaw angle kept in `entity.state`:

```python
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
```

Play and hold Q. The camera swings around the player.

WASD stays camera-relative as you orbit:
- The **built-in** player controller reads the camera's facing
  automatically.
- If you wrote your own `player.py` from the coin tutorial, it does the
  same from `ctx.camera.position` and `ctx.camera.target`.

## Step 3: Look ahead

A camera that stares at the player's back hides what's coming. Aim a
little **ahead** of the player in the direction they're moving. You can
work out the velocity from the position change between frames:

```python
LOOK_AHEAD = 0.35  # seconds of travel to lead by


def update(entity, ctx):
    player = ctx.scene["Player"]
    # ... yaw and offset as before ...
    entity.camera.follow(player, offset=offset, smooth=6.0)

    p = player.transform.position
    last = entity.state.get("last", p)
    dt = max(ctx.time.delta, 1e-4)
    vel = [(a - b) / dt for a, b in zip(p, last)]
    entity.state["last"] = p

    lead = entity.state.get("lead", (0.0, 0.0, 0.0))
    goal = (vel[0] * LOOK_AHEAD, 0.0, vel[2] * LOOK_AHEAD)
    k = min(1.0, 4.0 * ctx.time.delta)
    lead = tuple(l + (g - l) * k for l, g in zip(lead, goal))
    entity.state["lead"] = lead
    entity.camera.look_at((p[0] + lead[0], p[1] + 0.6, p[2] + lead[2]))
```

`follow` aims at the player first; calling `look_at` afterwards replaces
the aim. The lead is smoothed separately, so starting and stopping don't
jolt the view.

## Step 4: Sprint zoom

A wider field of view feels faster. Ease the FOV toward a goal:

```python
    goal_fov = 72.0 if ctx.input.pressed("shift") else 60.0
    cam = entity.camera
    cam.fov += (goal_fov - cam.fov) * min(1.0, 6.0 * ctx.time.delta)
```

## Tuning

| Feel | Settings |
|---|---|
| Tight action | `DISTANCE=6`, `HEIGHT=2.5`, `smooth=10` |
| Relaxed exploration | `DISTANCE=10`, `HEIGHT=4.5`, `smooth=4`, `LOOK_AHEAD=0.6` |
| Top-down | `offset=(0, 14, 6)`, no orbit |
| Side-scroller | Keep Z fixed (see [Cameras recipes](../oberon/cameras.md#recipes)) |

Use **Look Through** on the camera during Edit mode to check the starting
framing, since the first frame of Play starts from where the camera is
placed.

## The finished script

[`examples/oberon/coin_run/scripts/follow_camera.py`](../../../examples/oberon/coin_run/scripts/follow_camera.py)
is Step 2. Steps 3 and 4 are left for you to add.

**Next:** [Camera rail & intro shot](camera-rail.md).
