# Guide: Physics

Rapier-powered rigid bodies in 3D and 2D, a character controller, and
raycasts — stepped at a fixed 60 Hz so gameplay is identical at any frame
rate.

## Bodies

```python
at.RigidBody.dynamic()              # gravity, forces, collisions — crates, balls
at.RigidBody.fixed()                # never moves — floors, walls, platforms
at.RigidBody.kinematic_velocity()   # you set velocity; physics resolves collisions
at.RigidBody.kinematic_position()   # you set position; it pushes dynamic bodies
```

Fine-tuning: `at.RigidBody(body_type="dynamic", gravity_scale=1.0,
linear_damping=0.0, angular_damping=0.0, enabled=True)`.

## Colliders

```python
at.Collider3D.cuboid(x, y, z)   # FULL sizes — match Transform.scale exactly
at.Collider3D.ball(radius)
at.Collider3D.capsule(half_height, radius)
at.Collider3D.mesh("models/platform.glb")         # the model's own triangles
at.Collider3D.convex_hull("models/crate.glb")     # shrink-wrapped around it

at.Collider2D.cuboid(x, y)
at.Collider2D.ball(radius)
at.Collider2D.capsule(half_height, radius)
```

Chainable material modifiers: `.friction(0.8)`, `.restitution(0.3)`
(bounciness), `.density(2.0)`, `.sensor(True)` (detect overlap without
blocking movement — pickups, triggers).

> **Collider sizes are full sizes.** `Transform(scale=(40, 1, 40))` pairs
> with `Collider3D.cuboid(40, 1, 40)`. Get this wrong and things float or
> sink by exactly half.

### Colliders from models

`mesh` and `convex_hull` take a model file (relative to the scene, like
`MeshRenderer.model`) and build the shape from it when the scene starts,
after the model's import settings and at the entity's scale. The scene
stores only the path.

- **Mesh** is exact: rays and characters hit the real triangles, so
  ramps, arches and bevelled platforms behave as they look. Use it for
  static scenery (no rigid body, or a static or kinematic one). A triangle
  mesh has no inside, so it can't give a dynamic body mass: on a dynamic
  body it plays as its convex hull, and Oberon and **Build Game** flag it.
- **Convex hull** wraps the model like shrink-film: no dents or holes. Use
  it for dynamic bodies a box or capsule fits badly.
- Boxes, balls and capsules are still the cheapest. Prefer them when they
  fit.

> **2D and 3D are separate worlds.** A `Collider2D` never touches a
> `Collider3D`. Pick one per game; per entity, use the matching family.

## The per-frame toolbox

```python
ctx.physics.grounded(player)                       # standing on something?
ctx.physics.velocity(entity)                       # current (vx, vy, vz)
ctx.physics.set_velocity(entity, (0, 5, 0))
ctx.physics.apply_impulse(entity, (0, 300, 0))     # one-shot kick
ctx.physics.apply_force(entity, (10, 0, 0))        # continuous push
ctx.physics.gravity() / ctx.physics.set_gravity((0, -9.81, 0))
ctx.physics.body_count()

key = ctx.physics.cast_ray(origin, direction, max_distance=100.0, exclude=player)
hit = ctx.physics.ray_result(key)   # (entity_id, point, normal, distance) or None
```

`ctx.physics2d` mirrors the essentials for the 2D world (`apply_impulse`,
`grounded`, `body_count`).

## The character controller

The platformer recipe — tight movement that still collides and climbs
slopes:

```python
player = (
    at.Entity("player")
    .add(at.Transform(position=(0, 1.4, 0)))
    .add(at.MeshRenderer.cube(size=0.9, color=ORANGE))
    .add(at.RigidBody.kinematic_velocity())       # 1. we own the velocity
    .add(at.Collider3D.capsule(0.5, 0.4))         # 2. capsule slides over edges
    .add(at.CharacterController(snap_to_ground=0.5))  # 3. kinematic solver
    .add(at.CharacterVelocity())                  # 4. per-frame drive input
)
```

```python
def on_update(self, ctx):
    player = ctx.scene["player"]
    dx = self.controls.axis("move_x", ctx.input)
    if ctx.physics.grounded(player):
        self.vy = -1.0                             # stay snapped to ground
        if self.controls.just_pressed("jump", ctx.input):
            self.vy = JUMP_SPEED
    else:
        self.vy -= GRAVITY * ctx.time.delta
    player.character_velocity = (dx * WALK_SPEED, self.vy, 0.0)
```

`CharacterController` options (keyword-only): `snap_to_ground`,
`autostep`, `max_slope_climb_angle`, `min_slope_slide_angle`,
`offset`, `slide`, and `mode="normal"` (or `"autostep"`-friendly setups).

## Common pitfalls

- **Player falls through the floor:** collider sizes don't match the
  visual scale (use full sizes), or the floor lacks `RigidBody.fixed()`.
- **Kinematic character never moves:** you set `Transform.position` instead
  of `character_velocity` — the solver only sees the velocity component.
- **Dynamic body jitters against a wall:** that's restitution/friction;
  try `friction(0.2)` on the wall or clamp velocity yourself.

## See also

- [Scenes & Entities](scenes-and-entities.md) — component basics
- [Cookbook](../cookbook.md#movement) — coyote time, jump buffering
- [API Reference](../api-reference.md#physics-ctxphysics)
