# Guide: Project Structure

One file is right until it isn't. When the platformer passes ~300 lines,
split it the Django way: each concern in one obvious home.

## The layout

```
mygame/
├── main.py        # the Game subclass + __main__ entry
├── entities.py    # Entity subclasses: Player, Coin, Platform, Crate...
├── scenes.py      # Scene subclasses: LevelOne, LevelTwo, Menu
├── systems.py     # update helpers: movement, camera, pickups
└── tests/
    └── test_game.py
```

## What goes where

**entities.py** — archetypes, nothing else:

```python
import automata as at

ORANGE = (0.95, 0.45, 0.15, 1.0)


class Player(at.Entity):
    def __init__(self):
        super().__init__("player")
        self.add(at.Transform(position=(-4, 1.4, 0)))
        self.add(at.MeshRenderer.cube(size=0.9, color=ORANGE))
        self.add(at.RigidBody.kinematic_velocity())
        self.add(at.Collider3D.capsule(0.5, 0.4))
        self.add(at.CharacterController(snap_to_ground=0.5))
        self.add(at.CharacterVelocity())
```

**scenes.py** — composition:

```python
class LevelOne(at.Scene):
    def __init__(self):
        super().__init__()
        self.add(Camera())
        self.add(Sun())
        self.add(Ground())
        self.add(Player())
        for x, y in PLATFORMS:
            self.add(Platform(x=x, y=y))
```

**systems.py** — pure functions taking `(game, ctx)`:

```python
def follow_camera(game, ctx):
    px, py, _ = ctx.scene["player"].transform.position
    ...
    ctx.scene["camera"].camera_3d = at.Camera3D(...)
```

**main.py** — wiring and policy:

```python
class MyGame(at.Game):
    title = "my game"
    graphics = at.GraphicsQuality.HIGH

    def on_start(self):
        self.load_scene(LevelOne())

    def on_update(self, ctx):
        movement(self, ctx)
        follow_camera(self, ctx)
```

## The rules of thumb

1. **Entities are data** — no update logic in entity classes; behavior
   lives in systems or the Game hooks.
2. **Scenes compose, never compute** — level layout is a list of `add`
   calls, not procedural generation (put generators in `scenes.py` as
   separate functions if you need them).
3. **Game state on the Game or in resources** — `self.score` for
   game-local state, `ctx.resources` for things tests and tools poke at.
4. **Constants at module top** — speeds, colors, gravity. Your future self
   tunes games by editing exactly one block.

## See also

- [OOP Style](oop-style.md)
- [Testing](testing.md)
