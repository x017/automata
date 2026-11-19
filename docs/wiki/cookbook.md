# Cookbook

Small recipes for problems every game hits. Each is complete enough to
paste into the platformer and adapt.

## Movement

### Double jump

```python
def on_start(self):
    self.jumps_left = 2

def on_update(self, ctx):
    player = ctx.scene["player"]
    dx = self.controls.axis("move_x", ctx.input)
    if ctx.physics.grounded(player):
        self.jumps_left = 2
        self.vy = -1.0
    else:
        self.vy -= GRAVITY * ctx.time.delta

    if self.controls.just_pressed("jump", ctx.input) and self.jumps_left > 0:
        self.vy = JUMP_SPEED
        self.jumps_left -= 1
    player.character_velocity = (dx * WALK, self.vy, 0.0)
```

### Coyote time (jump shortly after leaving a ledge)

```python
COYOTE = 0.12  # seconds

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
def on_update(self, ctx):
    self.next_wave -= ctx.time.delta
    if self.next_wave <= 0.0:
        self.next_wave = 5.0
        self.wave += 1
        for i in range(self.wave):
            ctx.scene.spawn(Enemy(x=8.0 + i * 1.5))   # appears next frame
```

### Pickup by proximity (no physics sensors needed)

```python
def on_update(self, ctx):
    px, py, _ = ctx.scene["player"].transform.position
    for name, (cx, cy) in list(self.coins.items()):
        if abs(px - cx) < 0.8 and abs(py - cy) < 1.0:
            ctx.scene.find(name).despawn()
            del self.coins[name]
            ctx.events.send("coin_pickup", {"score": self.score + 1})
```

## Flow control

### Pause

```python
def on_update(self, ctx):
    if ctx.input.just_pressed(at.Key.ESCAPE):
        self.paused = not self.paused
    if self.paused:
        return  # world freezes; camera/menus still drawable if you return late
    # ... game logic below only runs unpaused
```

(Physics keeps stepping — for a full freeze, skip writing velocities while
paused so characters stay put; dynamic bodies keep settling.)

### Switching levels

```python
def on_level_complete(self, payload):
    self.level += 1
    self.load_scene(make_level(self.level))   # applies immediately
```

## Input

### Rebindable controls screen (core loop)

```python
# entering "listen" mode
self.listening_for = "jump"

def on_update(self, ctx):
    if self.listening_for:
        for key in at.Key:
            if ctx.input.just_pressed(key):
                self.controls.rebind(self.listening_for, [at.Binding.key(key)])
                self.listening_for = None
                break
```

## See also

- [Zero to Hero](zero-to-hero.md) — the guided path
- [API Reference](api-reference.md)
