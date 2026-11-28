# The same ray query answered by Oberon's collider shapes and by Rapier.
def update(entity, ctx):
    n = entity.state["n"] = entity.state.get("n", 0) + 1
    if n != 2:  # colliders exist once physics has stepped
        return
    a = entity.attributes
    hit = ctx.physics.raycast((0, 1, 0), (0, 0, -2), 50, ignore=entity)
    a["hit"] = hit.entity.name if hit and hit.entity else "none"
    a["dist"] = hit.distance if hit else -1.0
    a["normal"] = hit.normal if hit else (0, 0, 0)
    a["point"] = hit.point if hit else (0, 0, 0)
    a["by_id"] = ctx.scene.by_id(hit.entity.id).name if hit and hit.entity else "none"
    a["miss"] = ctx.physics.raycast((0, 1, 0), (0, 0, 1), 50) is None
    a["short"] = ctx.physics.raycast((0, 1, 0), (0, 0, -1), 2) is None
    a["zero"] = ctx.physics.raycast((0, 1, 0), (0, 0, 0), 50) is None
