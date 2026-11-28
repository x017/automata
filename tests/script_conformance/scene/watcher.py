def update(entity, ctx):
    n = entity.state["n"] = entity.state.get("n", 0) + 1
    a = entity.attributes
    if n == 1:
        # A despawn lands at the end of the frame, whoever ran first.
        a["found_during_frame"] = ctx.scene.find("Doomed") is not None
        a["contains_self"] = "Watcher" in ctx.scene
        a["missing"] = ctx.scene.find("Nobody") is None
        try:
            ctx.scene["Nobody"]
            a["key_error"] = False
        except KeyError:
            a["key_error"] = True
        a["same_handle"] = ctx.scene["Watcher"] is entity
        a["by_id_self"] = ctx.scene.by_id(entity.id) is entity
    if n == 3:
        a["gone_later"] = ctx.scene.find("Doomed") is None
        a["state_kept"] = entity.state["n"]
