# Guide: OOP Style

Three coding styles, one engine. Pick per project — they mix freely.

| Style | Looks like | Best for |
|---|---|---|
| **OOP** | `class Platformer(at.Game)` | Real games, anything you'll maintain |
| **Decorator** | `@game.update` on an instance | Small toys, sketches |
| **Procedural** | `game.run(tick)` | Throwaway experiments, tests |

This guide is the OOP style in full.

## The Game subclass

```python
class Platformer(at.Game):
    # Engine options as declarative class attributes:
    title = "platformer"
    resolution = (1280, 720)
    headless = False
    resizable = True
    vsync = True
    log_filter = None            # e.g. "debug" or "automata_core=trace"
    target_fps = 60              # None disables frame pacing
    clear_color = (0.55, 0.78, 0.95, 1.0)
    graphics = at.GraphicsQuality.HIGH

    def on_start(self): ...                 # once, before frame 1
    def on_update(self, ctx): ...           # every frame
    def on_coin_pickup(self, payload): ...  # once per "coin_pickup" event

Platformer().run()
```

- Every option defaults to `None` = "engine default"; keyword arguments to
  the constructor **win over** class attributes: `Platformer(headless=True)`.
- Direct construction with an unknown option raises `TypeError`; subclass
  constructors may take their own parameters freely.
- Hooks: `on_start` runs once; `on_update(ctx)` runs every frame; any
  method named `on_<event>` auto-registers as a handler for `<event>`.

## Entity and Scene subclasses

```python
class Player(at.Entity):
    def __init__(self):
        super().__init__("player")
        self.add(at.Transform(position=(-4, 1.4, 0)))
        self.add(at.MeshRenderer.cube(size=0.9, color=ORANGE))


class Level(at.Scene):
    def __init__(self):
        super().__init__()
        self.add(Camera())
        self.add(Player())
```

This is the Django habit applied to games: entity classes are your
**models** (reusable, importable archetypes), scene classes compose them
into levels, and the game class is the **view** wiring input to behavior.

**The constructor rule:** an `Entity`'s first positional argument is its
name. Subclass constructors take keyword parameters
(`Coin(f"coin_{i}", x=x, y=y)` — never `Coin(f"coin_{i}", x, y)`).

## Mixing styles

Everything is the same object underneath:

```python
game = Platformer(headless=True)

@game.update                       # decorator overrides on_update
def tick(ctx): ...

game.run(tick)                     # or procedural: explicit wins
game.run(max_frames=300)           # kwargs still fine
```

Callback resolution order: `run(update=...)` → `@game.update` →
`on_update` override. If none exists, `run` raises `TypeError`.

## IDE support

The wrapper layer is fully typed — annotate your hooks and your editor
autocompletes the whole frame context:

```text
def on_update(self, ctx: at.FrameContext) -> None:
    ctx.scene.      # find, names, spawn, despawn, entity_count
    ctx.input.      # pressed, just_pressed, cursor, gamepads
    ctx.physics.    # grounded, apply_impulse, cast_ray
    ctx.events.     # send, read
    ctx.resources.  # get, set, remove
    ctx.time.       # delta, elapsed, frame
```

Components accepted by `add()` are the `at.Component` union — type
checkers flag unsupported objects before you run.

## See also

- [Project Structure](project-structure.md) — one file becomes a package
- [Events & Resources](events-and-resources.md) — the `on_<event>` contract
- [API Reference](../api-reference.md)
