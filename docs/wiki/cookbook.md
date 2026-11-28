# Cookbook

Small recipes for problems every game hits. Each is complete enough to
paste into the [capstone platformer](zero-to-hero.md#module-11--capstone-coin-runner)
(`examples/python/oop_platformer.py`) and adapt. The names (`WALK_SPEED`,
`GRAVITY`, `JUMP_SPEED`, `self.controls`, `self.cam_x`) are the capstone's.

**All of them, pasted in:**
[`examples/python/cookbook_platformer.py`](../../examples/python/cookbook_platformer.py)
is the capstone with every recipe on this page working together, and
[`examples/python/tests/test_cookbook_platformer.py`](../../examples/python/tests/test_cookbook_platformer.py)
tests them headless:

```console
python examples/python/cookbook_platformer.py
pytest examples/python/tests
```

When recipes combine, they share state. The finished file merges the three
movement recipes into one `move` method, for example.

## Movement

### Double jump

```python
def on_start(self):
    super().on_start()          # the capstone loads the level and controls
    self.jumps_left = 2

def on_update(self, ctx):
    player = ctx.scene["player"]
    dx = self.controls.axis("move_x", ctx.input)
    if ctx.physics.grounded(player) and self.vy <= 0.0:
        self.jumps_left = 2
        self.vy = -1.0
    else:
        self.vy -= GRAVITY * ctx.time.delta

    if self.controls.just_pressed("jump", ctx.input) and self.jumps_left > 0:
        self.vy = JUMP_SPEED
        self.jumps_left -= 1
    player.character_velocity = (dx * WALK_SPEED, self.vy, 0.0)
```

`self.vy <= 0.0` matters: right after a jump the player can still touch
the ground for a frame, and refilling `jumps_left` then would allow a
third jump.

### Coyote time (jump shortly after leaving a ledge)

```python
COYOTE = 0.12  # seconds

def on_start(self):
    super().on_start()
    self.coyote = 0.0

def on_update(self, ctx):
    if ctx.physics.grounded(ctx.scene["player"]):
        self.coyote = COYOTE
    else:
        self.coyote = max(0.0, self.coyote - ctx.time.delta)
    can_jump = self.coyote > 0.0
    # ... allow jump while can_jump
```

### Jump buffering (jump pressed just before landing still works)

```python
BUFFER = 0.12

def on_start(self):
    super().on_start()
    self.jump_buffer = 0.0

def on_update(self, ctx):
    self.jump_buffer = BUFFER if self.controls.just_pressed("jump", ctx.input) \
        else max(0.0, self.jump_buffer - ctx.time.delta)
    if ctx.physics.grounded(ctx.scene["player"]) and self.jump_buffer > 0.0:
        self.vy = JUMP_SPEED
        self.jump_buffer = 0.0
```

## Cameras

### Orbit camera

```python
import math

def on_update(self, ctx):
    t = ctx.time.elapsed * 0.6
    ctx.scene["camera"].camera_3d = at.Camera3D(
        position=(8.0 * math.sin(t), 3.0, 8.0 * math.cos(t)),
        target=(0, 1, 0),
    )
```

### Screen shake

```python
import random

def on_start(self):
    super().on_start()
    self.shake = 0.0

def on_hit(self, payload):
    self.shake = 0.25  # seconds of shake

def on_update(self, ctx):
    self.shake = max(0.0, self.shake - ctx.time.delta)
    mag = 0.3 * (self.shake / 0.25)
    ox, oy = random.uniform(-mag, mag), random.uniform(-mag, mag)
    ctx.scene["camera"].camera_3d = at.Camera3D(
        position=(self.cam_x + ox, self.cam_y + oy, 10.0),
        target=(self.cam_x, self.cam_y - 1.4, 0.0),
    )
```

## Spawning & timing

### Spawn waves on a timer

```python
def on_start(self):
    super().on_start()
    self.next_wave, self.wave = 5.0, 0

def on_update(self, ctx):
    self.next_wave -= ctx.time.delta
    if self.next_wave <= 0.0:
        self.next_wave = 5.0
        self.wave += 1
        for i in range(self.wave):
            ctx.scene.spawn(Crate(position=(2.0 + i * 1.5, 8.0, 0.0)))   # appears next frame
```

`Crate` is the capstone's; any `Entity` subclass works.

### Pickup by proximity (no physics sensors needed)

```python
def on_update(self, ctx):
    px, py, _ = ctx.scene["player"].transform.position
    for name, (cx, cy) in list(self.coins.items()):
        coin = ctx.scene.find(name)
        if coin is None:  # spawned this frame; it exists from the next
            continue
        if abs(px - cx) < 0.8 and abs(py - cy) < 1.0:
            coin.despawn()
            del self.coins[name]
            self.score += 1
            ctx.events.send("coin_pickup", {"score": self.score})
```

## Flow control

### Pause

```python
def on_start(self):
    super().on_start()
    self.paused = False

def on_update(self, ctx):
    if ctx.input.just_pressed(at.Key.ESCAPE):
        self.paused = not self.paused
    if self.paused:
        ctx.scene["player"].character_velocity = (0.0, 0.0, 0.0)
        return  # the game logic below only runs unpaused
    # ... game logic
```

Physics keeps stepping while paused. A character's velocity stays at
whatever you last wrote, so set it to zero or the player keeps sliding.
Dynamic bodies (crates) keep settling.

### Switching levels

```python
def on_level_complete(self, payload):
    self.level += 1
    self.load_scene(make_level(self.level))   # the next frame is the new level
```

`make_level(n)` is your own function returning a `Scene`. Send
`level_complete` when the last coin goes, and reset per-level state (the
coin table, timers) after loading.

## Input

### Rebindable controls screen (core loop)

```python
def on_start(self):
    super().on_start()
    self.listening_for = None

def on_update(self, ctx):
    if self.listening_for is None and ctx.input.just_pressed(at.Key.F1):
        self.listening_for = "jump"          # entering "listen" mode
        return
    if self.listening_for:
        for key in at.Key:
            if key != at.Key.F1 and ctx.input.just_pressed(key):
                self.controls.rebind(self.listening_for, [at.Binding.key(key)])
                self.listening_for = None
                break
        return                               # swallow input while listening
    # ... normal game logic
```

## See also

- [Zero to Hero](zero-to-hero.md) — the guided path
- [API Reference](api-reference.md)
