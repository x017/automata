# Input with nothing held, plus the parts of ctx every runner offers.
def update(entity, ctx):
    a = entity.attributes
    if "done" in a:
        return
    a["axis"] = ctx.input.axis("left", "right")
    a["shift"] = ctx.input.pressed("shift")
    a["space_down"] = ctx.input.just_pressed("space")
    a["click"] = ctx.input.mouse_pressed("left")
    a["dx"] = ctx.input.mouse_delta[0]
    ctx.input.lock_mouse(True)
    a["locked_now"] = ctx.input.mouse_locked
    ctx.input.lock_mouse(False)
    a["unlocked_now"] = not ctx.input.mouse_locked
    a["delta_positive"] = ctx.time.delta > 0
    a["elapsed_ok"] = ctx.time.elapsed >= 0
    w, h = ctx.hud.size
    a["hud_center_ok"] = ctx.hud.center == (w / 2.0, h / 2.0)
    ctx.hud.rect(10, 10, 4, 4, (1, 1, 1, 1))
    ctx.log("conformance probe ran")
    a["done"] = True
