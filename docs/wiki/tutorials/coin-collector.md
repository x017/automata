# Tutorial: Coin collector

**Goal:** turn your first scene into a game. Coins spin and bob, the player
collects them, a score counts up, and you win when they're all gone.

**You'll learn:**
- behaviour scripts (`start` / `update`)
- attributes
- reading other entities
- the Console
- hot reload

**Start from:** the scene from [Your first scene](first-scene.md). Open
it with Ctrl+O. Or open the finished version,
`examples/oberon/coin_run/level.ron`, and follow along.

## 1. A coin

1. **+ → Cube**, renamed `Coin 1`.
2. Scale it to `0.5, 0.5, 0.12`: a flat disc-ish block.
3. Colour it gold: `#FFC733`.
4. Set its Y position to `1` so it floats at chest height.

## 2. The coin script

With the coin selected, choose **Add Component → Script…** and type
`scripts/coin.py`. Oberon creates the file from a template and opens it in
the code editor. Replace the contents with:

```python
"""coin: spins, bobs, and is collected when the player touches it."""

import math

PICKUP_RADIUS = 1.1


def start(entity, ctx):
    entity.state["home"] = entity.transform.position
    entity.attributes["collected"] = False


def update(entity, ctx):
    if entity.attributes.get("collected"):
        return
    hx, hy, hz = entity.state["home"]
    t = ctx.time.elapsed
    entity.transform.position = (hx, hy + 0.15 * math.sin(t * 3.0 + hx), hz)
    entity.transform.rotation_euler = (0.0, t * 2.5, 0.0)

    px, py, pz = ctx.scene["Player"].transform.position
    if math.dist((px, py, pz), (hx, hy, hz)) < PICKUP_RADIUS:
        entity.attributes["collected"] = True
        entity.transform.position = (hx, -100.0, hz)  # out of sight
        print(f"+{entity.attributes.get('value', 1)}")
```

Save it (Ctrl+S), press Esc to return to the scene, and press **Play**.
The coin spins and bobs. Walk into it, and it disappears with `+1` in the
Console.

What's going on:

- `start` runs once. We remember the coin's starting position in
  `entity.state`, a private dict that lasts for the whole Play session.
- `update` runs every frame. `ctx.time.elapsed` drives the motion, so it
  runs at the same speed at any frame rate.
- `ctx.scene["Player"]` finds another entity by name.
- `entity.attributes["collected"]` is a value **other scripts can read**.
  That's how the score will know.

## 3. Hot reload

Keep Play running. In the code editor, change `t * 2.5` to `t * 8.0` and
save. Within half a second the Console says `Reloaded coin.py`, and the
coin spins faster. You don't need to restart.

Now break it on purpose: on that same line, change `entity.transform` to
`entity.transfrom` and save. The Console shows the error with a
**coin.py:22** link. That coin pauses; the rest of
the game keeps running. Fix the typo and save, and it resumes.

## 4. Many coins, different values

Stop. Select the coin and press **Ctrl+D** five times. The copies share
`scripts/coin.py`; each one gets its own `entity` and `state`. Spread them
out, and put one somewhere hard to reach, such as on top of a pillar.

Give that hard-to-reach coin a bigger reward:

1. In the Inspector, choose **Attributes → + Add attribute**.
2. Name: `value`, type **Int**, value `5`.

The script already reads `entity.attributes.get('value', 1)`, so coins
without the attribute are worth 1.

## 5. Keeping score

Score belongs to the game, not to any coin. Choose **+ → Empty** and name
it `Rules`. Attach a new script, `scripts/rules.py`:

```python
"""rules: keeps score and announces the win."""


def coins(ctx):
    return [e for e in ctx.scene if e.name.startswith("Coin")]


def start(entity, ctx):
    entity.state["score"] = 0
    print(f"Collect {len(coins(ctx))} coins! WASD to move, Space to jump, Q/E to turn.")


def update(entity, ctx):
    all_coins = coins(ctx)
    score = sum(c.attributes.get("value", 1) for c in all_coins if c.attributes.get("collected"))
    if score != entity.state["score"]:
        entity.state["score"] = score
        entity.attributes["score"] = score
        print(f"Score: {score}")
        if all_coins and all(c.attributes.get("collected") for c in all_coins):
            print("You found every coin!")
```

Play and collect everything. The Console counts up and celebrates at the
end.

> **Why doesn't each coin add to the score itself?** Every script reads
> the start-of-frame state, and writes apply after all scripts run. If two
> coins were collected in the same frame, each would read `score = 3` and
> write `4`. Having **one writer per value** avoids that. Coins report
> their own state, and the rules script adds it up.

## 6. Your own player script (optional)

The built-in Player controller is fine in Oberon. A script makes the game
behave the same outside the editor too (see
[From Oberon to Python](oberon-to-python.md)). Attach `scripts/player.py`
to the Player. Use the version in
[`examples/oberon/coin_run/scripts/player.py`](../../../examples/oberon/coin_run/scripts/player.py):
it walks relative to the camera, sprints, jumps, and has gravity. A
character with a script skips the built-in controller.

Its controls are *actions*: `start` binds names to default keys, and
`update` asks about the names. A settings screen can then let players
change the keys (`ctx.settings.rebind("jump", "k")`), and the game
remembers it:

```python
def start(entity, ctx):
    entity.state["vy"] = 0.0
    ctx.settings.bind("forward", "w", "arrow_up")
    ctx.settings.bind("jump", "space")
    ...
```

The heart of `update`:

```python
vy = entity.state["vy"]
if ctx.physics.grounded(entity):
    vy = JUMP if keys.just_pressed("jump") else -2.0
else:
    vy -= GRAVITY * ctx.time.delta
entity.state["vy"] = vy
entity.character_velocity = (vx, vy, vz)
```

## 7. Remember the best run

Scores reset every time you press Play. To keep something between runs,
put it in `ctx.save`. It works like a dict that's written to disk at the
end of the frame. In Oberon it lives in your project's `.oberon/save`
folder. In an exported game it lives in the player's own user-data
folder. Here's the finished `scripts/rules.py`:

```python
"""rules: keeps score, announces the win, and remembers the best run.
Attach to an empty entity.

The best score and the coins found over every run live in ``ctx.save``, so
they're still there next time (in Oberon: ``.oberon/save``; in an exported
game: the player's user-data folder). F2 cycles the graphics preset, which
``ctx.settings`` remembers too.
"""

PRESETS = ["low", "medium", "high", "ultra", "beautiful"]


def coins(ctx):
    return [e for e in ctx.scene if e.name.startswith("Coin")]


def start(entity, ctx):
    entity.state["score"] = 0
    print(f"Collect {len(coins(ctx))} coins! WASD to move, Space to jump, Q/E to turn.")
    if "best" in ctx.save:
        print(f"Welcome back! Best run: {ctx.save['best']}, "
              f"coins found so far: {ctx.save.get('coins_ever', 0)}.")


def update(entity, ctx):
    all_coins = coins(ctx)
    score = sum(c.attributes.get("value", 1) for c in all_coins if c.attributes.get("collected"))
    if score != entity.state["score"]:
        ctx.save["coins_ever"] = ctx.save.get("coins_ever", 0) + score - entity.state["score"]
        entity.state["score"] = score
        entity.attributes["score"] = score
        print(f"Score: {score}")
        if score > ctx.save.get("best", 0):
            ctx.save["best"] = score
        if all_coins and all(c.attributes.get("collected") for c in all_coins):
            print("You found every coin!")

    if ctx.input.just_pressed("f2"):
        preset = PRESETS[(PRESETS.index(ctx.settings.graphics) + 1) % len(PRESETS)]
        ctx.settings.graphics = preset
        print(f"Graphics: {preset}")

    ctx.hud.text(f"Coins {score}   Best {max(score, ctx.save.get('best', 0))}", 16, 16, size=22)
```

Play twice. The second time, the Console says "Welcome back!" and the
HUD shows your best. Press **F2** to cycle the graphics preset.
`ctx.settings` remembers it, along with the volume and any rebound keys,
in the same folder. To start fresh, delete `.oberon/save` (or call
`ctx.save.clear()`).

## What you learned

- A behaviour script has `start(entity, ctx)` and `update(entity, ctx)`.
- `entity.state` is private memory; `entity.attributes` is shared, shown
  in the Inspector, and saved.
- Find other entities with `ctx.scene["Name"]` or by iterating
  `ctx.scene`.
- Save to hot-reload; errors pause only the script that failed.
- Have one writer per value.
- `ctx.save` keeps values between runs; `ctx.settings` keeps the player's
  volume, graphics and keys.

**Next:** [Third-person camera](third-person-camera.md). Make the camera
chase the player.
