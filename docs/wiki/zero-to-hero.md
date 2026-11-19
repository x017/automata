# Zero to Hero: The Complete AUTOMATA Tutorial

A twelve-module masterclass. You start with an empty window and end with
**Coin Runner** — a playable 3D platformer with physics, shadows, collectibles,
a follow camera, and a headless test suite. Every module is a complete,
runnable program.

Work through the modules in order; each one adds exactly one concept.

- [Module 0 — The smallest game](#module-0--the-smallest-game)
- [Module 1 — Scenes, entities, components](#module-1--scenes-entities-components)
- [Module 2 — Cameras: the art of seeing](#module-2--cameras-the-art-of-seeing)
- [Module 3 — Transforms: move, rotate, scale](#module-3--transforms-move-rotate-scale)
- [Module 4 — Input and action maps](#module-4--input-and-action-maps)
- [Module 5 — Physics: bodies and colliders](#module-5--physics-bodies-and-colliders)
- [Module 6 — The character controller](#module-6--the-character-controller)
- [Module 7 — Lights, shadows, and quality](#module-7--lights-shadows-and-quality)
- [Module 8 — Events and resources](#module-8--events-and-resources)
- [Module 9 — OOP architecture](#module-9--oop-architecture)
- [Module 10 — Headless testing](#module-10--headless-testing)
- [Module 11 — Capstone: Coin Runner](#module-11--capstone-coin-runner)

---

## Module 0 — The smallest game

**Concept:** a `Game` owns the engine; `run` drives the loop.

```python
import automata as at

game = at.Game(title="module 0")
game.run(lambda ctx: None)
```

Run it. A window opens and stays open until you close it. That is the whole
engine contract:

1. Create a `Game` (options like `title`, `resolution`, `clear_color`,
   `graphics` live here).
2. Call `run(update)` — your `update(ctx)` callback runs **once per frame**.
3. The loop ends when the window closes, `ctx.quit()` is called, or
   `max_frames` is reached. `run` returns the frame count.

A game runs **once**. After `run` returns, the engine has shut down; build a
new `Game` to run again.

**Exercise:** pass `max_frames=120` to `run` and print the return value.

---

## Module 1 — Scenes, entities, components

**Concept:** a scene is a bag of entities; an entity is a bag of components;
components are plain data.

```python
import automata as at

scene = (
    at.Scene()
    .add(at.Entity("camera").add(at.Camera3D(position=(0, 2, 6), target=(0, 0, 0))))
    .add(
        at.Entity("cube")
        .add(at.Transform(position=(0, 0, 0)))
        .add(at.MeshRenderer.cube(size=1.0, color=(0.95, 0.45, 0.15, 1.0)))
    )
)

game = at.Game(title="module 1")
game.load_scene(scene)
game.run(lambda ctx: None)
```

An orange cube. The anatomy:

- `at.Entity("cube")` — a game object. The **name** is how you find it later
  (`ctx.scene["cube"]`). Names are optional but you want them for anything
  you'll look up.
- `.add(component)` — attaches data. Chainable. The components that matter
  for a visible 3D thing: `Transform` (where it is) and
  `MeshRenderer.cube(size, color)` (what it looks like).
- `game.load_scene(scene)` — materializes the staged scene into the running
  world, replacing whatever was there.

**The golden rule of 3D visibility:** a scene draws nothing until it has a
`Camera3D` entity *and* an entity with both `Transform` and `MeshRenderer`.

**Exercise:** add a second cube at `(2, 0, 0)` with a different color.

---

## Module 2 — Cameras: the art of seeing

**Concept:** the first entity with a camera component becomes the frame's
camera. Cameras are plain data — and writable per frame.

```python
import automata as at

game = at.Game(title="module 2")
game.load_scene(
    at.Scene()
    .add(at.Entity("cam").add(at.Camera3D(position=(0, 2, 6), target=(0, 0, 0), fov=60.0)))
    .add(at.Entity("cube").add(at.Transform()).add(at.MeshRenderer.cube()))
)


def update(ctx):
    # Orbit: the camera circles the cube, always looking at it.
    import math

    t = ctx.time.elapsed
    ctx.scene["cam"].camera_3d = at.Camera3D(
        position=(6.0 * math.sin(t), 2.0, 6.0 * math.cos(t)),
        target=(0, 0, 0),
    )


game.run(update)
```

Facts to internalize:

- `Camera3D(position, target, fov)` — position in world space, the point it
  looks at, vertical field of view in degrees (default 60).
- `view.camera_3d = at.Camera3D(...)` inside update **replaces the camera
  next frame**. This is how follow cameras, orbit cameras, and cutscene
  cameras all work.
- Without any camera, meshes render through a fallback camera at
  `(0, 0, 5)` looking at the origin — handy for debugging, never ship it.
- There's also `Camera2D(center, half_height)` for orthographic 2D views.

**Exercise:** make the camera bob up and down (`2.0 + math.sin(t)`) instead
of orbiting.

---

## Module 3 — Transforms: move, rotate, scale

**Concept:** `Transform` is position + rotation + scale; writes you make in
`update` are buffered and applied right after the callback.

```python
import math

import automata as at

game = at.Game(title="module 3")
game.load_scene(
    at.Scene()
    .add(at.Entity("cam").add(at.Camera3D(position=(0, 3, 7), target=(0, 0.5, 0))))
    .add(
        at.Entity("cube")
        .add(at.Transform(position=(0, 0.5, 0), scale=(1.0, 1.0, 1.0)))
        .add(at.MeshRenderer.cube(size=1.0, color=(0.4, 0.7, 0.9, 1.0)))
    )
)


def update(ctx):
    cube = ctx.scene["cube"]
    cube.transform.rotation_euler = (0.0, ctx.time.elapsed * 1.5, 0.0)  # spin
    cube.transform.scale = (1.0, 1.0 + 0.3 * abs(math.sin(ctx.time.elapsed)), 1.0)


game.run(update)
```

Key ideas:

- `view.transform` returns a live handle. Reads see **start-of-frame**
  values — except when you've written to that same entity this frame, in
  which case you read back your own pending write ("read your writes").
  Either way the world itself updates right after your callback returns,
  so the next frame sees the new values.
- Rotation is stored as a **quaternion**; reading `rotation_euler` back
  converts, so expect float drift (`0.6999999`, not `0.7`). Never compare
  rotations with `==` — write them, don't round-trip them.
- `rotation_euler` is `(x, y, z)` in **radians**, applied in XYZ order.
- `scale` multiplies the mesh — `MeshRenderer.cube(size=1)` with
  `scale=(40, 1, 40)` is how you make a floor out of a unit cube.
- `ctx.time` gives you `delta` (seconds since last frame), `elapsed`
  (seconds since start), and `frame` (counter starting at 1).

**Exercise:** pulse the cube's color instead of scale… trick question —
color lives on `MeshRenderer`, which is fixed at spawn. Change *size*,
*position*, or *rotation*; material swaps are a spawn-a-new-entity job (for
now).

---

## Module 4 — Input and action maps

**Concept:** poll hardware directly for toys; bind named actions for games.

```python
import automata as at

game = at.Game(title="module 4")
game.load_scene(
    at.Scene()
    .add(at.Entity("cam").add(at.Camera3D(position=(0, 4, 8), target=(0, 0, 0))))
    .add(at.Entity("cube").add(at.Transform()).add(at.MeshRenderer.cube()))
)

controls = at.ActionMap()
controls.bind_axis("move_x", at.AxisBinding.key_pair(at.Key.A, at.Key.D))
controls.bind_axis("move_x", at.AxisBinding.key_pair(at.Key.ARROW_LEFT, at.Key.ARROW_RIGHT))
controls.bind_axis("move_x", at.AxisBinding.gamepad_axis(at.GamepadAxis.LEFT_STICK_X))
controls.bind("quit", at.Binding.key(at.Key.ESCAPE))

x = 0.0


def update(ctx):
    global x
    dx = controls.axis("move_x", ctx.input)  # -1.0 ..= 1.0
    x += dx * 4.0 * ctx.time.delta
    ctx.scene["cube"].transform.position = (x, 0.0, 0.0)
    if controls.just_pressed("quit", ctx.input):
        ctx.quit()


game.run(update)
```

The layers:

- **Raw polling** — `ctx.input.pressed(at.Key.SPACE)`,
  `ctx.input.just_pressed(...)`, `ctx.input.mouse_just_pressed(...)`,
  `ctx.input.cursor`. Fine for experiments.
- **ActionMap** — bind *names* to *controls*, query the names. Three facts
  worth remembering:
  1. `bind` and `bind_axis` **stack**: A/D, arrows, and the left stick all
     drive `move_x` above. Values sum and clamp to −1..1.
  2. Axes are analog in, analog out (`gamepad_axis`), or digital-as-analog
     (`key_pair` gives −1/0/+1).
  3. Bindings are mutable at runtime (`rebind`, `unbind`, `unbind_axis`) —
     a remapping screen is ordinary game code.
- **Enums vs strings** — `at.Key.SPACE` and `"space"` are the same value.
  The enums (`Key`, `MouseButton`, `GamepadButton`, `GamepadAxis`) exist so
  your editor autocompletes and the type checker catches typos. Prefer them.

**Exercise:** bind jump to both `Key.SPACE` and `GamepadButton.SOUTH`, and
print `"jump!"` when it's just-pressed.

---

## Module 5 — Physics: bodies and colliders

**Concept:** `RigidBody` says *how* a thing moves, `Collider3D` says *what*
it hits. Physics steps at a fixed 60 Hz regardless of frame rate.

```python
import automata as at

scene = (
    at.Scene()
    .add(at.Entity("cam").add(at.Camera3D(position=(0, 4, 9), target=(0, 1, 0))))
    .add(  # the floor: fixed body + big flat collider
        at.Entity("floor")
        .add(at.Transform(position=(0, 0, 0), scale=(20, 1, 20)))
        .add(at.MeshRenderer.cube(size=1.0, color=(0.35, 0.65, 0.30, 1.0)))
        .add(at.RigidBody.fixed())
        .add(at.Collider3D.cuboid(20.0, 1.0, 20.0))
    )
    .add(  # a crate: dynamic body falls, bounces a little, rests
        at.Entity("crate")
        .add(at.Transform(position=(0, 5, 0)))
        .add(at.MeshRenderer.cube(size=0.8, color=(0.72, 0.52, 0.30, 1.0)))
        .add(at.RigidBody.dynamic())
        .add(at.Collider3D.cuboid(0.8, 0.8, 0.8).restitution(0.3))
    )
)

game = at.Game(title="module 5")
game.load_scene(scene)
game.run(lambda ctx: None)
```

Run it and watch the crate drop onto the floor.

Body types:

| Factory | Behavior |
|---|---|
| `RigidBody.dynamic()` | Full simulation: gravity, forces, collisions |
| `RigidBody.fixed()` | Never moves; the world itself (floors, walls) |
| `RigidBody.kinematic_velocity()` | Moves at a velocity *you* set (characters) |
| `RigidBody.kinematic_position()` | Teleports along a path; pushes dynamics |

Collider shapes: `Collider3D.cuboid(x, y, z)` (full sizes, matching
`Transform.scale`), `.ball(radius)`, `.capsule(half_height, radius)`.
Modifiers chain: `.friction(0.8)`, `.restitution(0.3)`, `.density(2.0)`,
`.sensor(True)` (detects without blocking).

The `ctx.physics` toolbox: `apply_impulse`, `apply_force`, `velocity`,
`set_velocity`, `grounded`, `cast_ray`, `set_gravity`. There's a parallel
`ctx.physics2d` world with its own `Collider2D` shapes — 2D and 3D never
mix; pick one per game.

**Exercise:** drop five crates from different heights with different
restitutions.

---

## Module 6 — The character controller

**Concept:** a kinematic body + `CharacterController` + a per-frame
`CharacterVelocity` gives you tight, game-feel movement that still collides.

```python
import automata as at

WALK, GRAVITY, JUMP = 5.0, 20.0, 9.5

scene = (
    at.Scene()
    .add(at.Entity("cam").add(at.Camera3D(position=(0, 3.5, 10), target=(0, 1, 0))))
    .add(
        at.Entity("floor")
        .add(at.Transform(scale=(30, 1, 8)))
        .add(at.MeshRenderer.cube(color=(0.35, 0.65, 0.30, 1.0)))
        .add(at.RigidBody.fixed())
        .add(at.Collider3D.cuboid(30, 1, 8))
    )
    .add(
        at.Entity("player")
        .add(at.Transform(position=(-4, 1.4, 0)))
        .add(at.MeshRenderer.cube(size=0.9, color=(0.95, 0.45, 0.15, 1.0)))
        .add(at.RigidBody.kinematic_velocity())
        .add(at.Collider3D.capsule(0.5, 0.4))
        .add(at.CharacterController(snap_to_ground=0.5))
        .add(at.CharacterVelocity())
    )
)

controls = at.ActionMap()
controls.bind_axis("move_x", at.AxisBinding.key_pair(at.Key.A, at.Key.D))
controls.bind("jump", at.Binding.key(at.Key.SPACE))

vy = 0.0


def update(ctx):
    global vy
    player = ctx.scene["player"]
    dx = controls.axis("move_x", ctx.input)

    if ctx.physics.grounded(player):
        vy = -1.0  # keep contact so snap-to-ground works on slopes
        if controls.just_pressed("jump", ctx.input):
            vy = JUMP
    else:
        vy -= GRAVITY * ctx.time.delta

    player.character_velocity = (dx * WALK, vy, 0.0)


game = at.Game(title="module 6")
game.load_scene(scene)
game.run(update)
```

The recipe, line by line:

1. `RigidBody.kinematic_velocity()` — physics resolves collisions, but *you*
   own the velocity.
2. `Collider3D.capsule(...)` — capsules slide over edges instead of
   snagging; the standard character shape.
3. `CharacterController(snap_to_ground=0.5)` — Rapier's kinematic solver:
   slopes, autostep, ground snapping.
4. Every frame: compute `vy` yourself (gravity + jump), read `dx` from the
   action map, write `player.character_velocity = (dx * WALK, vy, 0)`.
5. `ctx.physics.grounded(player)` is your "can I jump?" query.

**Exercise:** add a double jump (track jumps used since last grounded).

---

## Module 7 — Lights, shadows, and quality

**Concept:** one `DirectionalLight` entity is the sun; `clear_color` is the
sky; `graphics` is the quality preset.

```python
import automata as at


class MyGame(at.Game):
    title = "module 7"
    clear_color = (0.55, 0.78, 0.95, 1.0)      # sky blue background
    graphics = at.GraphicsQuality.HIGH          # shadow map resolution


scene = (
    at.Scene()
    .add(at.Entity("cam").add(at.Camera3D(position=(0, 3.5, 9), target=(0, 1, 0))))
    .add(
        at.Entity("sun").add(
            at.DirectionalLight(
                direction=(-0.45, -1.0, -0.3),   # where the light travels
                color=(1.0, 0.96, 0.88),         # warm afternoon
                ambient=0.35,                    # fill light in shadow
                shadows=True,                    # shadow-mapped shadows
            )
        )
    )
    # ... floor, cubes from module 6 ...
)
```

Everything renders with Blinn-Phong shading: diffuse from the sun, a subtle
specular highlight, and ambient so shadow sides aren't pitch black.

- **Shadows** are automatic for every mesh when the sun has `shadows=True`.
- **Quality presets:** `LOW` disables shadows (fastest, flat look),
  `MEDIUM`/`HIGH`/`ULTRA` use 1024/2048/4096 shadow maps.
- **Mood is data:** `clear_color` + sun `color` + `ambient` is your whole
  color grade. Golden hour = warm color + low ambient + long direction.

**Exercise:** make a night scene (dark blue `clear_color`, pale blue sun,
`ambient=0.15`).

---

## Module 8 — Events and resources

**Concept:** `ctx.events` passes named messages (delivered next frame);
`ctx.resources` stores arbitrary Python objects across frames.

```python
class CoinGame(at.Game):
    def on_start(self):
        self.load_scene(Level())
        self.score = 0

    def on_update(self, ctx):
        # ... when the player grabs a coin:
        ctx.events.send("coin_pickup", {"score": self.score + 1})

    def on_coin_pickup(self, payload):      # auto-registered by name
        self.score = payload["score"]
        print(f"score: {self.score}")
```

- `ctx.events.send(name, payload)` / `ctx.events.read(name)` — payloads are
  any Python object. An event sent this frame becomes readable on the next
  frame — handlers, physics follow-ups, everything — then it expires.
  Events are next-frame mail, not a log.
- In OOP style, a method named `on_<name>` is automatically called once per
  matching `<name>` event. Decorator style: `@game.on_event("name")`.
- `ctx.resources.set/get/remove` — a typed-by-convention bag for things
  like `score`, `current_level`, or a shared RNG. It's a dictionary with
  engine lifetime.

**Events vs resources:** events are *things that happened* (transient);
resources are *things that are* (persistent).

**Exercise:** keep `score` in `ctx.resources` instead of an attribute, and
print it from a second handler.

---

## Module 9 — OOP architecture

**Concept:** subclass `Game`, `Scene`, and `Entity`; engine options are
class attributes; hooks are methods. This is the style for anything past a
toy.

```python
class Player(at.Entity):
    def __init__(self):
        super().__init__("player")
        self.add(at.Transform(position=(-4, 1.4, 0)))
        self.add(at.MeshRenderer.cube(size=0.9, color=(0.95, 0.45, 0.15, 1.0)))
        self.add(at.RigidBody.kinematic_velocity())
        self.add(at.Collider3D.capsule(0.5, 0.4))
        self.add(at.CharacterController(snap_to_ground=0.5))
        self.add(at.CharacterVelocity())


class Level(at.Scene):
    def __init__(self):
        super().__init__()
        self.add(Camera())
        self.add(Sun())
        self.add(Ground())
        self.add(Player())


class Platformer(at.Game):
    title = "platformer"
    target_fps = 60
    clear_color = SKY
    graphics = at.GraphicsQuality.HIGH

    def on_start(self):
        self.load_scene(Level())
        self.controls = at.ActionMap()
        ...

    def on_update(self, ctx): ...
    def on_coin_pickup(self, payload): ...
```

Why this scales (the Django lesson):

- **Declarative class attributes** for engine options — readable at a
  glance, overridable per subclass (`class DebugPlatformer(Platformer):
  graphics = at.GraphicsQuality.LOW`).
- **Entity subclasses are archetypes** — `Player()` is a reusable,
  importable unit, like a model class. Scenes compose them.
- **Hooks have names, not wiring** — `on_start`, `on_update`,
  `on_<event>`; you never register anything by hand.
- Keyword arguments beat class attributes when both are given:
  `Platformer(headless=True)` wins over the class body.

**The one constructor rule:** subclass constructors may take their own
parameters, but pass them as **keywords** — the first positional argument
of an `Entity` constructor is always the entity name
(`Crate(position=(2, 1, 0))`, not `Crate((2, 1, 0))`).

**Exercise:** refactor module 6 into `Player`/`Level`/`Platformer` classes.

---

## Module 10 — Headless testing

**Concept:** `headless=True` runs the real loop — physics, events, logic —
with no window or GPU. That makes your game a pytest target.

```python
import pytest

from mygame import Platformer  # the Game subclass from your game module


def test_player_lands_on_the_ground():
    class Probe(Platformer):
        """The real game, plus a test probe in its update hook."""

        headless = True

        def on_update(self, ctx):
            super().on_update(ctx)  # real game logic runs unchanged
            if ctx.time.frame == 120:
                self.landed_y = ctx.scene["player"].transform.position[1]
                ctx.quit()

    game = Probe()
    game.run()
    assert game.landed_y == pytest.approx(1.0, abs=0.05)
```

The probe subclasses the game, wraps `on_update`, and inspects the world
after 120 frames. (The same wrap works without subclassing:
`game.run(update=my_probe)` — an explicit callback wins over `on_update`;
call `Platformer.on_update(game, ctx)` inside it to keep the real logic.)

Patterns that work well:

- **Drive inputs through physics**, not synthetic key events: in headless
  mode there's no keyboard, so test *logic* (jump impulse when grounded)
  and *outcomes* (ball rests at y = 1.0), not raw input plumbing.
- `max_frames=N` bounds every test.
- `MeshRenderer` is inert headless — scenes are safe to load unchanged.
- The engine's own suite (`crates/automata-python/tests/python`) is 50+
  examples of this style.

**Exercise:** test that collecting a coin despawns it (spawn near the
player, run 60 frames, assert `ctx.scene.find("coin_0")` is `None`).

---

## Module 11 — Capstone: Coin Runner

The complete game — everything from modules 0–10 in one file. This is
`examples/python/oop_platformer.py` in the repo; run it:

```console
python examples/python/oop_platformer.py            # windowed
python examples/python/oop_platformer.py --headless # CI
```

What it demonstrates, line by line:

| Feature | Where it came from |
|---|---|
| `class Platformer(at.Game)` with class-attribute options | Module 9 |
| `Sun`, `Camera`, `Ground`, `Player`, `Platform`, `Coin`, `Crate` entity classes | Modules 1, 5, 6, 7 |
| A smooth **follow camera** written per frame | Module 2 |
| A/D + arrows + gamepad stick all driving `move_x` | Module 4 |
| Grounded-jump gravity loop | Module 6 |
| Spinning coins, proximity pickup, `coin_pickup` events | Modules 3, 8 |
| Sky `clear_color`, warm shadowed sun, quality preset | Module 7 |
| `--headless` flag for CI | Module 10 |

```python
class Platformer(at.Game):
    title = "oop platformer"
    headless = "--headless" in sys.argv
    target_fps = 60
    clear_color = SKY
    graphics = at.GraphicsQuality.HIGH

    def on_update(self, ctx):
        player = ctx.scene["player"]
        px, py, _ = player.transform.position
        dx = self.controls.axis("move_x", ctx.input)

        if ctx.physics.grounded(player):
            self.vy = -1.0
            if self.controls.just_pressed("jump", ctx.input):
                self.vy = JUMP_SPEED
        else:
            self.vy -= GRAVITY * ctx.time.delta
        player.character_velocity = (dx * WALK_SPEED, self.vy, 0.0)

        # follow camera
        blend = min(1.0, 8.0 * ctx.time.delta)
        self.cam_x += (px - self.cam_x) * blend
        self.cam_y += (max(py + 1.6, 2.6) - self.cam_y) * blend
        ctx.scene["camera"].camera_3d = at.Camera3D(
            position=(self.cam_x, self.cam_y, 10.0),
            target=(self.cam_x, self.cam_y - 1.4, 0.0),
        )

        # coins: spin + collect
        for name, (cx, cy) in list(self.coins.items()):
            coin = ctx.scene.find(name)
            if coin is None:
                continue
            coin.transform.rotation_euler = (0.0, ctx.time.elapsed * 2.5, 0.0)
            if abs(px - cx) < 0.8 and abs(py - cy) < 1.0:
                coin.despawn()
                del self.coins[name]
                self.score += 1
                ctx.events.send("coin_pickup", {"score": self.score, "total": len(COINS)})
```

### Where next

- [Cookbook](cookbook.md) — double jump, coyote time, moving platforms,
  spawn waves, pause, scene switching.
- [Project Structure](guides/project-structure.md) — when one file becomes
  five.
- [API Reference](api-reference.md) — every class, every parameter.

You are no longer a beginner. Ship something small and weird.
