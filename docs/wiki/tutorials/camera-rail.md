# Tutorial: Camera rail & intro shot

**Goal:** when Play starts, the camera flies over the level for a few
seconds, showing off the coins, and then hands control to the
third-person follow camera smoothly.

**You'll learn:**
- keyframes
- easing curves
- using empty entities as editable shot markers
- blending between two camera behaviours

## 1. Mark the shots in the editor

Hard-coding positions means you can't see them. Use entities instead:

1. **+ → Empty**, renamed `Shot 1`. An Empty only needs a position, so
   move it to where the camera should be for the opening shot.
2. Add an attribute `look` of type **Vec3**, set to the point the shot
   should look at.
3. Repeat for `Shot 2` and `Shot 3`. Shots play in name order. Put the
   last one where the follow camera starts (behind the player, at the
   camera's own position), so the hand-off is seamless.

> Quick way to place a marker: frame the shot, then press **+ → Camera**.
> It lands at your eye position. Copy its **Eye** into the Empty's
> position and its **Look At** into `look`. Then delete the temporary
> camera.

## 2. The rail script

An entity runs one script, so select **Main Camera**, remove its Script
component, and attach a new one, `scripts/camera_rail.py`. The end of the
rail script brings the orbiting follow camera back.

```python
"""camera_rail: an intro flyover through the Shot markers, then a follow camera."""

import math

SHOT_TIME = 2.5  # seconds per segment
BLEND = 1.0  # seconds to blend into the follow camera
DISTANCE = 8.0
HEIGHT = 3.5
TURN = 2.0  # radians per second, Q / E orbit after the intro


def smoothstep(t):
    t = min(1.0, max(0.0, t))
    return t * t * (3.0 - 2.0 * t)


def lerp(a, b, t):
    return tuple(x + (y - x) * t for x, y in zip(a, b))


def shots(ctx):
    found = sorted((e for e in ctx.scene if e.name.startswith("Shot ")), key=lambda e: e.name)
    return [(tuple(e.transform.position), tuple(e.attributes["look"])) for e in found]


def rail(keys, time):
    """Position and target `time` seconds into the flyover, or None when done."""
    if len(keys) < 2:
        return None
    segment = int(time // SHOT_TIME)
    if segment >= len(keys) - 1:
        return None
    k = smoothstep((time - segment * SHOT_TIME) / SHOT_TIME)
    (p0, a0), (p1, a1) = keys[segment], keys[segment + 1]
    return lerp(p0, p1, k), lerp(a0, a1, k)


def start(entity, ctx):
    entity.state["keys"] = shots(ctx)
    entity.state["yaw"] = 0.0
    entity.state["done_at"] = None


def update(entity, ctx):
    cam = entity.camera
    t = ctx.time.elapsed

    # Skippable intro: Space jumps straight to the hand-off.
    if entity.state["keys"] and ctx.input.just_pressed("space"):
        entity.state["keys"] = []

    on_rail = rail(entity.state["keys"], t)
    if on_rail is not None:
        cam.position, cam.target = on_rail
        return

    # After the flyover: follow, easing in over BLEND seconds.
    if entity.state["done_at"] is None:
        entity.state["done_at"] = t
    blend = smoothstep((t - entity.state["done_at"]) / BLEND)
    yaw = entity.state["yaw"] + ctx.input.axis("q", "e") * TURN * ctx.time.delta
    entity.state["yaw"] = yaw
    offset = (DISTANCE * math.sin(yaw), HEIGHT, DISTANCE * math.cos(yaw))
    cam.follow(ctx.scene["Player"], offset=offset, smooth=1.0 + 7.0 * blend)
```

Play. The camera glides through the shots and then settles in behind the
player. Press Space during the flyover to skip it.

The finished level has three shots and this script, in
[`scripts/camera_rail.py`](../../../examples/oberon/coin_run/scripts/camera_rail.py).
To add Steps 3 and 4 from the previous tutorial (look-ahead and sprint
zoom) after the hand-off, copy `look_ahead` and `sprint_zoom` from
`follow_camera.py` and call them after `cam.follow`.

## How it works

- **Keyframes are data in the scene.** Moving a Shot entity in the editor
  changes the flyover without touching code. Designers can tweak it.
- **`smoothstep`** eases each segment in and out. Linear interpolation
  would jerk at every keyframe; smoothstep has zero speed at both ends.
- **The hand-off** reuses `follow`, but ramps its `smooth` from 1 (lazy)
  to 8 (normal) over `BLEND` seconds. The camera drifts from the last
  shot to its follow position instead of cutting. `done_at` remembers
  when the rail ended, so skipping blends the same way.
- Everything keys off `ctx.time.elapsed`, so the shot takes the same time
  on any machine.

## Variations

**Skip with any key:** the script skips on Space. To skip on the first
movement key instead:

```python
if entity.state["keys"] and any(ctx.input.just_pressed(k) for k in ("w", "a", "s", "d", "space")):
    entity.state["keys"] = []
```

**Catmull-Rom curves:** with four or more shots, a spline passes through
every key without stopping at each one. Replace `lerp` and `smoothstep`
with:

```python
def catmull(p0, p1, p2, p3, t):
    t2, t3 = t * t, t * t * t
    return tuple(
        0.5 * (2 * b + (-a + c) * t + (2 * a - 5 * b + 4 * c - d) * t2 + (-a + 3 * b - 3 * c + d) * t3)
        for a, b, c, d in zip(p0, p1, p2, p3)
    )
```

**Dolly zoom (the "Vertigo" shot):** move the camera back while narrowing
the field of view, keeping the subject the same size:

```python
width = 4.0                              # metres of subject to keep in frame
distance = math.dist(cam.position, ctx.scene["Player"].transform.position)
cam.fov = math.degrees(2 * math.atan(width / (2 * distance)))
```

**Next:** [Lighting & mood](lighting-and-mood.md).
