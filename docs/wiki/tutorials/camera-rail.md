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
3. Repeat for `Shot 2` and `Shot 3`.

> Quick way to place a marker: frame the shot, then press **+ → Camera**.
> It lands at your eye position. Copy its **Eye** into the Empty's
> position and its **Look At** into `look`. Then delete the temporary
> camera.

## 2. The rail script

Replace `scripts/follow_camera.py`, or make a new script on the camera:

```python
import math

SHOT_TIME = 2.5   # seconds per segment
BLEND = 1.0       # seconds to blend into the follow camera


def smoothstep(t):
    t = min(1.0, max(0.0, t))
    return t * t * (3.0 - 2.0 * t)


def lerp(a, b, t):
    return tuple(x + (y - x) * t for x, y in zip(a, b))


def shots(ctx):
    found = sorted((e for e in ctx.scene if e.name.startswith("Shot ")), key=lambda e: e.name)
    return [(e.transform.position, e.attributes["look"]) for e in found]


def rail(keys, time):
    """Position and target `time` seconds into the flyover, or None when done."""
    segment = int(time // SHOT_TIME)
    if segment >= len(keys) - 1:
        return None
    k = smoothstep((time - segment * SHOT_TIME) / SHOT_TIME)
    (p0, a0), (p1, a1) = keys[segment], keys[segment + 1]
    return lerp(p0, p1, k), lerp(a0, a1, k)


def start(entity, ctx):
    entity.state["keys"] = shots(ctx)
    entity.state["yaw"] = 0.0


def update(entity, ctx):
    cam = entity.camera
    keys = entity.state["keys"]
    t = ctx.time.elapsed
    on_rail = rail(keys, t) if len(keys) >= 2 else None

    if on_rail is not None:
        cam.position, cam.target = on_rail
        return

    # After the flyover: follow, easing in over BLEND seconds.
    done_at = (len(keys) - 1) * SHOT_TIME if len(keys) >= 2 else 0.0
    blend = smoothstep((t - done_at) / BLEND)
    yaw = entity.state["yaw"]
    offset = (8.0 * math.sin(yaw), 3.5, 8.0 * math.cos(yaw))
    cam.follow(ctx.scene["Player"], offset=offset, smooth=1.0 + 7.0 * blend)
```

Play. The camera glides through the shots and then settles in behind the
player.

## How it works

- **Keyframes are data in the scene.** Moving a Shot entity in the editor
  changes the flyover without touching code. Designers can tweak it.
- **`smoothstep`** eases each segment in and out. Linear interpolation
  would jerk at every keyframe; smoothstep has zero speed at both ends.
- **The hand-off** reuses `follow`, but ramps its `smooth` from 1 (lazy)
  to 8 (normal) over `BLEND` seconds. The camera drifts from the last
  shot to its follow position instead of cutting.
- Everything keys off `ctx.time.elapsed`, so the shot takes the same time
  on any machine.

## Variations

**Skippable intro:** jump straight to the hand-off when the player presses
a key.

```python
if ctx.input.just_pressed("space"):
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
cam.fov = math.degrees(2 * math.atan(width / (2 * distance)))
```

**Next:** [Lighting & mood](lighting-and-mood.md).
