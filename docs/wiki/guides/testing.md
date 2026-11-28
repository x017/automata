# Guide: Testing

Your game is a Python program; test it like one. Headless mode runs the
real engine — physics, events, resources, your logic — with no window and
no GPU.

## The pattern

```python
import automata as at


def test_scene_loads():
    game = at.Game(headless=True)
    game.load_scene(at.Scene().add(at.Entity("box").add(at.Transform())))
    seen = {}

    def update(ctx):
        seen["names"] = ctx.scene.names()
        ctx.quit()

    game.run(update)
    assert seen["names"] == ["player-spawn"]
```

Headless rules:

- `headless=True` + `max_frames=N` bounds every run.
- `MeshRenderer` is inert (no GPU) — stage visuals anyway; they're ignored.
- **No OS input events arrive.** Test outcomes and logic, not key plumbing:
  instead of "pressing space makes the player jump," test "when grounded
  and `jump` is triggered, `vy` becomes positive" — or drive physics
  directly (`ctx.physics.apply_impulse`) and assert the landing.

## Probing a Game subclass

Wrap the real update hook rather than replacing it:

```python
from mygame import Platformer


def test_player_settles_on_ground():
    class Probe(Platformer):
        headless = True

        def on_update(self, ctx):
            super().on_update(ctx)
            if ctx.time.frame == 120:
                self.player_y = ctx.scene["player"].transform.position[1]
                ctx.quit()

    game = Probe()
    game.run()
    assert game.player_y == pytest.approx(1.0, abs=0.05)
```

An explicit `run(update=fn)` also wins over `on_update`; call
`Platformer.on_update(game, ctx)` inside it to keep the real logic.

## What to test

| Layer | Example assertion |
|---|---|
| Scene staging | names present, `entity_count` right |
| Physics | ball rests at expected height after N frames |
| Game logic | collecting a coin despawns it and bumps score |
| Events | sending `hit` triggers your handler |
| Action maps | bindings stack; `axis_names()` correct |
| API misuse | bad colors raise `ValueError`, unknown keys raise `KeyError` |

## CI

The repo's own CI runs the Python suite headless on every push — your
game's tests work the same way:

```console
python -m pytest tests/ -q
```

GPU rendering is covered separately by the engine's offscreen tests (they
run under lavapipe in CI); your game tests never need a GPU.

## See also

- [The Game Loop](game-loop.md#headless-mode)
- The engine's own suite: `crates/automata-python/tests/python/test_automata.py`
