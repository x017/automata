def update(entity, ctx):
    n = entity.state["n"] = entity.state.get("n", 0) + 1
    cam = ctx.camera
    if n == 1:
        cam.position = (0, 3, 8)
        cam.look_at(ctx.scene["Target"])
        cam.fov = 500
        # entity.camera and ctx.camera are the same camera.
        entity.attributes["fov_now"] = entity.camera.fov
    if n == 2:
        entity.attributes["pos"] = cam.position
        entity.attributes["target"] = cam.target
        entity.attributes["fov"] = cam.fov
        entity.attributes["target_has_camera"] = ctx.scene["Target"].camera is not None
