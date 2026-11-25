# Guide: The Game Loop

The loop is the engine's heartbeat. This guide is the mental model the rest
of the wiki assumes.

## The loop

```python
game = at.Game(title="demo", target_fps=60)

def update(ctx):
    ...  # your code, once per frame

frames = game.run(update)
```

Every frame, in order:

1. **Time advances** — `ctx.time.delta` (seconds since last frame),
   `elapsed`, `frame` (1-based counter) are refreshed.
2. **Events age** — an event sent this frame becomes readable next frame,
   then expires.
3. **Physics steps** — a fixed 60 Hz timestep, as many steps as fit the
   elapsed frame time. Frame rate never changes simulation speed.
4. **Your `update(ctx)` runs** — input snapshot, scene snapshot, buffered
   writes.
5. **Your buffered writes apply** — transforms, velocities, camera changes,
   spawns, despawns all land at once.
6. **The frame renders** (windowed mode) — the scene is recorded into a
   draw list and presented.

## The frame contract

Two rules prevent an entire category of engine bugs:

1. **Reads observe start-of-frame state.** `ctx.scene`, `ctx.input`,
   `ctx.physics` are snapshots taken as the frame began. If physics moves
   the player mid-frame and you write a camera from `player.transform`, you
   see *this* frame's settled position — consistent, never half-updated.
   (Reading an entity you just wrote to returns your pending write.)
2. **Writes apply after your callback returns.** `view.transform.position =
   ...`, `view.despawn()`, `ctx.scene.spawn(...)` are commands in a queue,
   not live mutation. They become visible from the next frame onward.

Consequences:

- Read-your-writes: after writing an entity's transform, reads *of that
  entity* return your pending value; every other entity still reads
  start-of-frame state.
- Safe despawns: despawning an entity that later lines in the same callback
  still reference is harmless — the despawn lands after.
- Events are next-frame mail: a sent event is never visible to the sending
  frame — readers and handlers see it on the following frame, then it's
  gone.

## Ending the loop

| How | When |
|---|---|
| Window closed | windowed mode, automatic |
| `ctx.quit()` | anywhere in your callback |
| `game.quit()` | from outside the callback |
| `max_frames=N` reached | tests, CI, fixed-length demos |

`run` returns the total frame count. A `Game` instance runs **once**;
create a new one to run again.

## Frame pacing

- `target_fps=60` (default) sleeps between frames to hold 60 fps.
- `target_fps=None` runs flat out — for headless tools and benchmarks.
- `vsync=True` (default) additionally syncs presentation to the display.
- Windowed games still render every frame even when idle; there is no
  event-driven redraw mode yet.

## Headless mode

```python
game = at.Game(headless=True)
game.run(update, max_frames=300)
```

The identical loop without window, GPU, or OS input events. Physics,
events, resources, and your logic all run; `MeshRenderer` components are
inert (no GPU exists to materialize them). See [Testing](testing.md).

## In the browser

A web page can't sit in a loop: the browser calls it once per animation
frame. So in a web build `game.run()` starts the game and **returns at
once** (0), and the page runs each frame. Your update, events and scenes
work the same; only code written *after* `game.run()` runs right away
instead of after the game ends. Put cleanup in `game.on_finish(fn)`, which
runs when the game ends there, and at once on desktop (by then `run` has
returned). `game.stepping` tells you which case you're in.

## Errors

- Engine failures (lost GPU device, window errors) raise `automata.Error`.
- Exceptions raised inside your callback stop the loop and **propagate
  unchanged** out of `run` — wrap in try/except if you want cleanup.

## See also

- [Scenes & Entities](scenes-and-entities.md) — what the loop updates
- [Rendering & Lights](rendering-and-lights.md) — what the loop draws
- [Testing](testing.md) — headless patterns
