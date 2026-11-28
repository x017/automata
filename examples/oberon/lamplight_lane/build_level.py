#!/usr/bin/env python3
"""Writes Lamplight Lane: a street at nightfall, built from prefabs.

    python examples/oberon/lamplight_lane/build_level.py   # then open it in Oberon
    cargo run -p oberon --release examples/oberon/lamplight_lane/level.ron

Every lamp, fence panel and crate in the scene is an *instance* of a
``prefabs/*.prefab`` file and stays linked to it:

* ``lamp_post.prefab`` is four entities: an iron post and arm, a glass
  globe that glows for the look, and the Point Light that lights the street.
* ``lamp_post_warm.prefab`` is a **variant** of it. It stores only what it
  changes (an amber globe and light), so editing the post in the base
  prefab reaches the warm lamps too.
* ``fence.prefab`` is one panel, laid out ten times along a line, as
  Edit ▸ Duplicate Along… does.
* ``crate.prefab`` is a crate with an ``hp`` attribute. The *Heavy Crate*
  overrides its colour and ``hp``. The Inspector lists those two overrides,
  with Revert buttons, and Apply writes them into the prefab.

Edit a prefab file (or Apply from an instance) and every instance follows.
Each instance keeps its overrides.
"""

import os

import automata as at

HERE = os.path.dirname(os.path.abspath(__file__))

IRON = (0.08, 0.085, 0.09, 1.0)
COBBLE = (0.34, 0.32, 0.3, 1.0)
PAVING = (0.46, 0.44, 0.41, 1.0)
WOOD = (0.47, 0.33, 0.19, 1.0)
CRATE = (0.62, 0.45, 0.25, 1.0)
HEAVY = (0.3, 0.33, 0.36, 1.0)

COOL = (0.82, 0.9, 1.0)   # the base lamp's light
AMBER = (1.0, 0.62, 0.28)  # the warm variant's

FENCE_STEP = 2.1  # metres from one fence panel to the next
FENCE_COUNT = 10


def box(name, position, scale, color, **surface):
    return (
        at.Entity(name)
        .add(at.Transform(position=position, scale=scale))
        .add(at.MeshRenderer.cube(size=1.0, color=color, **surface))
    )


def lamp_post():
    """The base lamp: post, arm, glowing globe and the light itself (the
    root is the post, standing on the ground at the origin; the globe hangs
    over the street, +Z, so the post doesn't shade its own light)."""
    post = box("Lamp Post", (0.0, 1.6, 0.0), (0.12, 3.2, 0.12), IRON, metallic=0.8, roughness=0.45)
    arm = box("Lamp Arm", (0.0, 3.14, 0.32), (0.07, 0.07, 0.7), IRON, metallic=0.8, roughness=0.45)
    globe = (
        at.Entity("Lamp Globe")
        .add(at.Transform(position=(0.0, 2.9, 0.62)))
        .add(at.MeshRenderer.sphere(size=0.4, color=(0.7, 0.85, 1.0, 1.0), emissive=COOL,
                                    emissive_strength=1.2, glow_light=0.0))
    )
    light = (
        at.Entity("Lamp Light")
        .add(at.Transform(position=(0.0, 2.9, 0.62)))
        .add(at.PointLight(color=COOL, intensity=5.0, range=10.0, radius=0.12))
    )
    return [post, arm, globe, light]


def crate():
    entity = box("Crate", (0.0, 0.4, 0.0), (0.8, 0.8, 0.8), CRATE, roughness=0.8)
    entity.attributes["hp"] = 3
    return entity


def fence_panel():
    return box("Fence", (0.0, 0.5, 0.0), (2.0, 1.0, 0.08), WOOD, roughness=0.85)


def write_prefabs(folder):
    """Writes the prefab files into ``folder/prefabs`` (the base lamp, its
    warm variant, the fence panel and the crate). Prefab links are relative
    to the asset root, so it points at ``folder`` meanwhile."""
    at.set_asset_root(folder)
    try:
        _write_prefabs(folder)
    finally:
        at.set_asset_root(None)


def _write_prefabs(folder):
    prefabs = os.path.join(folder, "prefabs")
    os.makedirs(prefabs, exist_ok=True)
    at.Prefab(lamp_post()).save(os.path.join(prefabs, "lamp_post.prefab"))
    at.Prefab(fence_panel()).save(os.path.join(prefabs, "fence.prefab"))
    at.Prefab(crate()).save(os.path.join(prefabs, "crate.prefab"))

    # The variant: instances of the base lamp with their own colours,
    # saved as a prefab. Each entity stays linked to the base and lists
    # only what it changes.
    base = at.Prefab.load(os.path.join(prefabs, "lamp_post.prefab"))
    post, arm, globe, light = base.instantiate_all()
    post.name = "Warm Lamp Post"
    globe.add(at.MeshRenderer.sphere(size=0.4, color=(1.0, 0.5, 0.15, 1.0), emissive=AMBER,
                                     emissive_strength=1.0, glow_light=0.0))
    light.add(at.PointLight(color=AMBER, intensity=5.5, range=10.0, radius=0.12))
    at.Prefab([post, arm, globe, light]).save(os.path.join(prefabs, "lamp_post_warm.prefab"))


def build(folder=HERE):
    """The scene, its instances linked to the prefabs in ``folder``
    (written first by :func:`write_prefabs`)."""
    at.set_asset_root(folder)
    try:
        return _build(folder)
    finally:
        at.set_asset_root(None)


def _build(folder):
    def prefab(name):
        return at.Prefab.load(os.path.join(folder, "prefabs", name))

    lamp, warm = prefab("lamp_post.prefab"), prefab("lamp_post_warm.prefab")
    fence, box_prefab = prefab("fence.prefab"), prefab("crate.prefab")

    scene = at.Scene()
    scene.add(
        at.Entity("Low Sun").add(
            at.DirectionalLight(direction=(-0.8, -0.35, -0.35), color=(0.42, 0.3, 0.38),
                                ambient=0.06)
        )
    )
    scene.add(at.Entity("Environment").add(at.Environment.preset("night")))
    scene.add(box("Street", (0.0, -0.1, 0.0), (32.0, 0.2, 7.0), COBBLE, roughness=0.9))
    scene.add(box("Pavement", (0.0, -0.05, -5.0), (32.0, 0.3, 3.0), PAVING, roughness=0.85))
    scene.add(box("Garden", (0.0, -0.1, -12.0), (32.0, 0.2, 11.0), (0.2, 0.3, 0.14, 1.0),
                  roughness=0.95))

    # Lamps along the kerb, cool and warm by turns. Each lamp is four
    # linked entities moved together.
    for i, x in enumerate((-12.0, -6.0, 0.0, 6.0, 12.0)):
        source = warm if i % 2 else lamp
        parts = source.instantiate_all(position=(x, 1.6, -3.9))
        parts[0].name = f"{'Warm ' if i % 2 else ''}Lamp {i + 1}"
        for part in parts:
            scene.add(part)

    # The fence behind the pavement: one panel, duplicated along a line.
    start = -FENCE_STEP * (FENCE_COUNT - 1) / 2
    for i in range(FENCE_COUNT):
        scene.add(fence.instantiate(position=(start + i * FENCE_STEP, 0.5, -6.6),
                                    name=f"Fence {i + 1}"))

    # Crates by the third lamp. The heavy one overrides colour and hp.
    for i, (x, y, z) in enumerate([(1.2, 0.4, -4.4), (2.1, 0.4, -4.6), (1.65, 1.2, -4.5)]):
        scene.add(box_prefab.instantiate(position=(x, y, z), name=f"Crate {i + 1}"))
    heavy = box_prefab.instantiate(position=(-1.3, 0.4, -4.5), name="Heavy Crate")
    heavy.add(at.MeshRenderer.cube(size=1.0, color=HEAVY, roughness=0.8))
    heavy.attributes["hp"] = 10
    scene.add(heavy)

    scene.add(
        at.Entity("Main Camera")
        .add(at.Transform(position=(-17.0, 4.6, 7.5)))
        .add(at.Camera3D(position=(-17.0, 4.6, 7.5), target=(-1.5, 1.0, -4.5)))
    )
    return scene


def write(folder=HERE):
    """Writes the prefabs and ``level.ron`` into ``folder``; returns the
    scene file's path."""
    write_prefabs(folder)
    scene = build(folder)
    path = os.path.join(folder, "level.ron")
    at.set_asset_root(folder)
    try:
        scene.save(path)
    finally:
        at.set_asset_root(None)
    return path


if __name__ == "__main__":
    print(f"wrote {write()}")
