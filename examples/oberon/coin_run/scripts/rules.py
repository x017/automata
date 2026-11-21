"""rules: keeps score and announces the win. Attach to an empty entity."""


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
