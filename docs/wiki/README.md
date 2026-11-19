# AUTOMATA Wiki

Welcome to the AUTOMATA documentation wiki — the complete guide to making
2D and 3D games with a Rust engine core and Python gameplay code.

AUTOMATA is **code-first**: games are ordinary Python programs. There is no
editor to learn and no project file format — you write a `Game`, stage
`Entity` objects into a `Scene`, and react to input every frame.

```python
import automata as at


class Hello(at.Game):
    title = "hello automata"
    clear_color = (0.55, 0.78, 0.95, 1.0)  # sky blue

    def on_start(self):
        self.load_scene(
            at.Scene()
            .add(at.Entity("camera").add(at.Camera3D(position=(0, 2, 6), target=(0, 0, 0))))
            .add(
                at.Entity("cube")
                .add(at.Transform(position=(0, 0, 0)))
                .add(at.MeshRenderer.cube(size=1.0, color=(0.95, 0.45, 0.15, 1.0)))
            )
        )

    def on_update(self, ctx):
        ctx.scene["cube"].transform.rotation_euler = (0.0, ctx.time.elapsed, 0.0)
        if ctx.input.just_pressed(at.Key.ESCAPE):
            ctx.quit()


Hello().run()
```

## Where to go

| I want to… | Read |
|---|---|
| Install the engine and open my first window | [Getting Started](getting-started.md) |
| Learn **everything**, step by step, building a real game | [Zero to Hero](zero-to-hero.md) |
| Look up an exact class, method, or parameter | [API Reference](api-reference.md) |
| Solve a specific problem (double jump, pause, spawn waves…) | [Cookbook](cookbook.md) |
| Understand the update loop and the frame model | [Guide: The Game Loop](guides/game-loop.md) |
| Build levels out of entities and components | [Guide: Scenes & Entities](guides/scenes-and-entities.md) |
| Draw things, light them, and move the camera | [Guide: Rendering & Lights](guides/rendering-and-lights.md) |
| Handle keyboard, mouse, and gamepads | [Guide: Input](guides/input.md) |
| Add gravity, collisions, and character movement | [Guide: Physics](guides/physics.md) |
| Pass messages and store game state | [Guide: Events & Resources](guides/events-and-resources.md) |
| Structure a big game with classes | [Guide: OOP Style](guides/oop-style.md) |
| Test my game headlessly in CI | [Guide: Testing](guides/testing.md) |
| Organize a project past one file | [Guide: Project Structure](guides/project-structure.md) |

## The two rules of the frame model

Everything else in this wiki follows from these two rules:

1. **Reads see the start of the frame.** Inside `on_update`, `ctx` is a
   snapshot of the world as the frame began. (One exception: after you
   write a transform, reads *of that entity* return your pending write.)
2. **Writes apply after your callback.** Setting a transform, despawning a
   coin, or sending an event is buffered and becomes visible next frame.

That's it. No locks, no partial states, no data races with the engine.

## Conventions

- Angles are **radians** in `Transform.rotation_euler`, **degrees** in
  `Camera3D(fov=...)`.
- Colors are `(r, g, b, a)` float tuples in the `0.0..=1.0` range.
- Input names are **typed enums** — `at.Key.SPACE`, `at.GamepadButton.SOUTH`
  — but plain strings (`"space"`) work everywhere too.
- Headless mode (`headless=True`) runs the full engine without a window or
  GPU: the same loop, the same physics, no rendering. It's how you test.
