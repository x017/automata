# Four drones hit the player once, in the same frame. Every write is
# visible to the scripts that run after it, so the hits add up.
def update(entity, ctx):
    if entity.state.get("done"):
        return
    entity.state["done"] = True
    ctx.scene["Player"].attributes["hp"] -= 5
