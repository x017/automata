# Guide: Events & Resources

Two small systems that replace a lot of architecture: **events** for things
that *happen*, **resources** for things that *are*.

## Events

```python
ctx.events.send("coin_pickup", {"score": 5})   # any name, any payload
for payload in ctx.events.read("coin_pickup"):  # readable next frame
    ...
```

- Payloads are arbitrary Python objects.
- Events live for **one frame**: the one after they're sent. Send in frame
  N, read in frame N+1 — then they're gone. Nothing queues longer than
  that — events are notifications, not a log.
- Any part of the game can send; any part can read. Physics collisions,
  pickups, "level complete", UI clicks — all just names and payloads.

### Handlers

OOP style registers methods by name automatically:

```python
class Platformer(at.Game):
    def on_coin_pickup(self, payload):     # called once per event
        self.score = payload["score"]
```

Decorator style:

```python
@game.on_event("coin_pickup")
def handle(payload):
    ...
```

Handlers run right after the update callback each frame, once per matching
event readable that frame — i.e. the frame after it was sent.

## Resources

```python
def on_update(self, ctx):
    score = ctx.resources.get("score") or 0   # None when unset
    ctx.resources.set("score", score + 1)
    ctx.resources.remove("score")             # delete
    "score" in ctx.resources                  # membership
    ctx.resources.names()                     # list keys
```

Resources are the game's shared whiteboard: score, current level number,
a seeded RNG, a reference to your save-file object. If you'd otherwise make
it a module-level global, put it in resources instead — tests can then run
many games without cross-contamination.

## Events vs resources

| | Events | Resources |
|---|---|---|
| Lifetime | next frame only | until removed |
| Question it answers | "what just happened?" | "what is true right now?" |
| Examples | hit, pickup, death, level_complete | score, lives, inventory, settings |

## See also

- [The Game Loop](game-loop.md) — the two-frame rule in context
- [OOP Style](oop-style.md) — handler auto-registration rules
- [API Reference](../api-reference.md#framecontext-ctx)
