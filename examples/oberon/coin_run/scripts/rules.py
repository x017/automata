"""rules: keeps score, announces the win, and remembers the best run.
Attach to an empty entity.

The best score and the coins found over every run live in ``ctx.save``, so
they're still there next time (in Oberon: ``.oberon/save``; in an exported
game: the player's user-data folder). F2 cycles the graphics preset, which
``ctx.settings`` remembers too.

On a touch screen (a phone) the controls are on the screen: a stick for the
player's actions (player.py), a stick that turns the camera (Q / E) and a
Jump button. They show only there; on a computer the keyboard plays.
"""

PRESETS = ["low", "medium", "high", "ultra", "beautiful"]


def coins(ctx):
    return [e for e in ctx.scene if e.name.startswith("Coin")]


TOUCH_CONTROLS = {
    "sticks": [
        {"side": "left", "keys": ["forward", "left", "back", "right"]},
        {"side": "right", "keys": ["q", "e"]},
    ],
    "buttons": [{"key": "jump", "label": "Jump"}],
}


def start(entity, ctx):
    entity.state["score"] = 0
    touch = ctx.input.touch_controls
    touch.set(**TOUCH_CONTROLS)
    if touch.visible:
        how = "Left stick to move, Jump to jump, right stick to turn."
    else:
        how = "WASD to move, Space to jump, Q/E to turn."
    print(f"Collect {len(coins(ctx))} coins! {how}")
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
