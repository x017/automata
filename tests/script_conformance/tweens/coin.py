def start(entity, ctx):
    ctx.tween(entity, "scale", (0, 0, 0), 0.2, ease="in_back", on_done=entity.despawn)
