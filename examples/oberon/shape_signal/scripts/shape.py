"""A shape on its pedestal: it turns slowly and lifts (spinning fast)
while the Game script's ``lit`` attribute names it. It holds still while a
screen that pauses the game is up."""


def start(entity, ctx):
    entity.state["base"] = entity.transform.position[1]
    entity.state["lift"] = 0.0


def update(entity, ctx):
    if ctx.ui.screens.paused:
        return
    s = entity.state
    lit = ctx.scene["Game"].attributes.get("lit") == entity.name
    s["lift"] += ((1.0 if lit else 0.0) - s["lift"]) * min(1.0, ctx.time.delta * 9.0)
    x, _, z = entity.transform.position
    entity.transform.position = (x, s["base"] + 0.9 * s["lift"], z)
    speed = entity.attributes.get("speed", 0.6) + 7.0 * s["lift"]
    entity.transform.rotate((0.0, speed * ctx.time.delta, 0.0))
