# Guide: Cameras

How the engine picks a view, the two camera types, and the patterns
almost every game needs: follow, orbit, rails, shake, and zoom.

If you build scenes in Oberon, read [Oberon: Cameras](../oberon/cameras.md)
as well. It covers the editor tools (frustums, preview, Look Through) and
the `entity.camera` scripting helper.

## The rules

- A camera is a **component**. Put `at.Camera3D` or `at.Camera2D` on any
  entity.
- The **first entity** with a `Camera3D` is the view for meshes. The first
  with a `Camera2D` is the view for sprites.
- No `Camera3D` means meshes are seen from `(0, 0, 5)` looking at the
  origin. No `Camera2D` means sprites use screen pixels.
- Cameras are plain values. To move one, **assign a new camera** every
  frame. It applies after your update, like every other write.

## Camera3D

```python
at.Camera3D(position=(0, 3.5, 9), target=(0, 1, 0), fov=60.0)
```

| Argument | Meaning |
|---|---|
| `position` | Where the eye is |
| `target` | The point it looks at. "Up" is always world +Y |
| `fov` | Vertical field of view in **degrees** |

The aspect ratio follows the window automatically. Near and far clip are
0.1 and 1000 when built from Python; Oberon lets you edit them.

## Camera2D

```python
at.Camera2D(center=(0, 0), half_height=100.0)
```

`half_height` is how many world units fit between the screen centre and
its top edge. Halve it to zoom in 2×.

## Follow camera

The basic pattern stores the camera state on your game object, eases it
toward a goal every frame, and assigns a new camera.

```python
import math
import automata as at


class Game(at.Game):
    def on_start(self):
        self.cam = [0.0, 4.0, 10.0]
        self.load_scene(build_level())

    def on_update(self, ctx):
        px, py, pz = ctx.scene["player"].transform.position
        goal = (px, py + 3.0, pz + 8.0)
        k = 1.0 - math.exp(-8.0 * ctx.time.delta)   # frame-rate independent
        self.cam = [c + (g - c) * k for c, g in zip(self.cam, goal)]
        ctx.scene["camera"].camera_3d = at.Camera3D(
            position=tuple(self.cam), target=(px, py + 0.6, pz)
        )
```

Why `1 - exp(-rate * dt)` and not `rate * dt`? It gives the same feel at
30 fps and at 240 fps, and it can never overshoot.

## Orbit camera

```python
def on_update(self, ctx):
    turn = float(ctx.input.pressed("e")) - float(ctx.input.pressed("q"))
    self.yaw += turn * 2.0 * ctx.time.delta
    px, py, pz = ctx.scene["player"].transform.position
    ctx.scene["camera"].camera_3d = at.Camera3D(
        position=(px + 9 * math.sin(self.yaw), py + 3.5, pz + 9 * math.cos(self.yaw)),
        target=(px, py + 0.6, pz),
    )
```

To make movement camera-relative, rotate the input by the yaw:

```python
f = float(ctx.input.pressed("w")) - float(ctx.input.pressed("s"))
r = float(ctx.input.pressed("d")) - float(ctx.input.pressed("a"))
s, c = math.sin(self.yaw), math.cos(self.yaw)
vx, vz = r * c - f * s, -r * s - f * c
```

## Camera rail (cutscenes)

Move along a list of keyframes with smooth easing:

```python
SHOTS = [  # (time, position, target)
    (0.0, (-8, 2, 8), (0, 1, 0)),
    (3.0, (8, 3, 6), (0, 1, 0)),
    (6.0, (0, 12, 0.1), (0, 0, 0)),
]

def lerp(a, b, t):
    return tuple(x + (y - x) * t for x, y in zip(a, b))

def smooth(t):
    return t * t * (3 - 2 * t)

def rail(time):
    for (t0, p0, a0), (t1, p1, a1) in zip(SHOTS, SHOTS[1:]):
        if time <= t1:
            k = smooth((time - t0) / (t1 - t0))
            return lerp(p0, p1, k), lerp(a0, a1, k)
    return SHOTS[-1][1], SHOTS[-1][2]

def on_update(self, ctx):
    position, target = rail(ctx.time.elapsed)
    ctx.scene["camera"].camera_3d = at.Camera3D(position=position, target=target)
```

## Screen shake

Shake the *target* a little, not the position. It reads as impact without
making the player feel ill.

```python
import random

def on_update(self, ctx):
    self.trauma = max(0.0, self.trauma - ctx.time.delta * 1.5)
    amount = self.trauma ** 2 * 0.3
    jitter = tuple(random.uniform(-amount, amount) for _ in range(3))
    tx, ty, tz = self.look
    ctx.scene["camera"].camera_3d = at.Camera3D(
        position=self.eye, target=(tx + jitter[0], ty + jitter[1], tz + jitter[2])
    )

# on a hit:  self.trauma = min(1.0, self.trauma + 0.6)
```

## Zoom

Animate `fov`: about 60 normally, 30–40 when aiming, 75+ while sprinting
for a sense of speed.

```python
goal = 35.0 if ctx.input.pressed("z") else 60.0
self.fov += (goal - self.fov) * min(1.0, 10 * ctx.time.delta)
```

## Several cameras

Only the first camera renders. To cut between views, mark each shot with
an empty entity: its position is the eye, and a Vec3 attribute `look`
holds the target. Then copy the shot you want into the main camera:

```python
shot = ctx.scene["shot_door"]
ctx.scene["camera"].camera_3d = at.Camera3D(
    position=shot.transform.position, target=shot.attributes["look"]
)
```

## Behaviour scripts

Scripts attached in Oberon (and run with `at.run_scene`) get a helper that
does the bookkeeping for you:

```python
def update(entity, ctx):
    ctx.camera.follow(ctx.scene["Player"], offset=(0, 3, 8), smooth=8)
    ctx.camera.fov = 70 if ctx.input.pressed("shift") else 60
```

See [Oberon: Cameras](../oberon/cameras.md#driving-a-camera-from-a-script).
