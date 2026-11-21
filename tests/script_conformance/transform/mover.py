def start(entity, ctx):
    entity.state["n"] = 0
    entity.attributes["starts"] = entity.attributes.get("starts", 0) + 1
    entity.attributes["first_frame"] = ctx.time.frame


def update(entity, ctx):
    entity.state["n"] += 1
    n = entity.state["n"]
    entity.attributes["updates"] = n
    if n <= 3:
        entity.transform.translate((1, 0, 0))
        entity.transform.rotate((0, 0.5, 0))
    if n == 1:
        entity.transform.scale = (2, 2, 2)
        # Reads see this frame's earlier writes: x is already 1 here.
        x = entity.transform.position[0]
        entity.transform.position = (x, 5, 0)
        entity.attributes["x_after_translate"] = x
