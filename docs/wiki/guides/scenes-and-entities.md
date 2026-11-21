# Guide: Scenes & Entities

AUTOMATA is an ECS engine with a Pythonic face: **entities** are bags of
**components**, and a **scene** is the set of entities alive in your world.
You never touch the ECS directly — you stage plain Python builders and the
engine materializes them.

## Staging a scene

```python
scene = (
    at.Scene()
    .add(at.Entity("floor").add(at.Transform(scale=(40, 1, 40))).add(at.RigidBody.fixed()))
    .add(at.Entity("crate").add(at.Transform(position=(2, 1, 0))).add(at.RigidBody.dynamic()))
)
game.load_scene(scene)   # replaces the current scene wholesale
```

- `Entity("name")` — the name is optional but is how you look the entity up
  at runtime. Methods chain: `.add(component)` returns the entity.
- `Scene().add(entity)` returns the scene — build the whole level in one
  expression, or subclass (below).
- `load_scene` clears the old world and spawns the new one. Call it before
  `run`, or inside `on_update` to switch levels.

## The components

| Component | Purpose |
|---|---|
| `Transform(position, rotation_euler, scale)` | Where it is, how it's turned, how big |
| `MeshRenderer.cube(size, color)` / `.quad(...)` | A visible 3D primitive |
| `Camera3D(position, target, fov)` / `Camera2D(center, half_height)` | The frame's eye (first entity wins) |
| `DirectionalLight(direction, color, ambient, shadows)` | The frame's sun (first entity wins) |
| `RigidBody.dynamic()/.fixed()/.kinematic_velocity()/.kinematic_position()` | How physics moves it |
| `Collider3D.cuboid(x,y,z)` / `.ball(r)` / `.capsule(h,r)` / `.mesh(model)` / `.convex_hull(model)` | Its 3D collision shape |
| `Collider2D.cuboid(x,y)` / `.ball(r)` / `.capsule(h,r)` | Its 2D collision shape |
| `CharacterController(...)` | Kinematic character solver (slopes, steps, snap) |
| `CharacterVelocity(velocity)` | The per-frame drive input for that solver |

Unknown component types are rejected with a `TypeError` listing the
supported set.

## Entity classes: reusable archetypes

Subclass `Entity` to bundle a reusable thing. This is the Django-models
habit: define once, place everywhere.

```python
class Coin(at.Entity):
    def __init__(self, name, x, y):
        super().__init__(name)
        self.add(at.Transform(position=(x, y, 0.0), scale=(0.4, 0.4, 0.4)))
        self.add(at.MeshRenderer.cube(color=(1.0, 0.85, 0.25, 1.0)))


class Level(at.Scene):
    def __init__(self):
        super().__init__()
        for i, (x, y) in enumerate(COIN_SPOTS):
            self.add(Coin(f"coin_{i}", x=x, y=y))
```

> **Constructor rule:** the first positional argument of an `Entity`
> constructor is always the entity *name*. Give subclass constructors
> keyword parameters (`Coin(f"coin_{i}", x=x, y=y)`), or a positional like
> `2.0` is interpreted as a name and rejected.

## At runtime: SceneView and EntityView

Inside `update`, `ctx.scene` is a snapshot with a command buffer:

```python
scene = ctx.scene
scene.entity_count            # entities alive at frame start
scene.names()                 # all named entities
scene.find("player")          # EntityView or None
scene["player"]               # EntityView or KeyError
for view in scene: ...        # iterate everything

scene.spawn(at.Entity("drop").add(...))   # appears next frame
scene.despawn(view.id)                    # gone next frame (children too)
```

`EntityView` is the per-entity handle:

| Member | Read | Write |
|---|---|---|
| `.id` / `.name` | yes | — |
| `.transform` | `TransformRef` (live) | assign a `Transform`, or set `.position` / `.rotation_euler` / `.scale` on the ref |
| `.character_velocity` | `(vx, vy, vz)` | assign a tuple — the character drive |
| `.camera_3d` | `Camera3D` copy | assign a `Camera3D` — follow cameras |
| `.despawn()` | — | queues removal |

All writes buffer and apply after your callback (see
[The Game Loop](game-loop.md#the-frame-contract)).

## Patterns

**Find once, cache the id.** `scene["player"]` does a name scan; fine
per-frame for a handful of lookups. For hot loops over many entities,
collect views once per frame with `for view in scene`.

**Spawn during play.** `ctx.scene.spawn(entity)` is how projectiles, drops,
and waves appear. The entity exists from the next frame.

**Level switching.** `self.load_scene(Level2())` inside `on_update` or an
`on_<event>` handler swaps the world when the frame's callback returns: the
next frame sees only the new level. Other writes to the old level's
entities that frame are dropped, and anything you `spawn` that frame lands
in the new level. (In plain-function style, `ctx.scene.load(Level2())` does
the same.)

## See also

- [Rendering & Lights](rendering-and-lights.md) — cameras, meshes, suns
- [Physics](physics.md) — bodies and colliders in depth
- [OOP Style](oop-style.md) — `Game`/`Scene`/`Entity` subclassing rules
- [API Reference](../api-reference.md)
