"""Tweens and timers: both runners move and fire them on the same frames."""


def opened(entity):
    entity.attributes["opened"] = True


def quarter(entity, ctx):
    entity.attributes["rang_on"] = ctx.time.frame
    # The position tween was made first, so it has already moved this frame.
    entity.attributes["y_at_quarter"] = entity.transform.position[1]


def tick(entity):
    entity.attributes["ticks"] += 1


def start(entity, ctx):
    ctx.tween(entity, "position", (0, 3, 0), 0.5, ease="linear", on_done=lambda: opened(entity))
    ctx.after(0.25, quarter, entity, ctx)
    ctx.every(0.1, tick, entity, times=3)
    ctx.tween(entity, "score", 10, 0.5, ease="out_cubic")
    ctx.tween(entity, "scale", (2, 2, 2), 0.25, ease="in_out_sine", yoyo=True, loop=2)
    entity.state["fade"] = ctx.tween_value(1.0, 0.0, 0.2, ease="linear")


def update(entity, ctx):
    entity.attributes["fade"] = entity.state["fade"].value
