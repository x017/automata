#!/usr/bin/env python3
"""Writes the Shader Lab: three showpieces wearing custom WGSL material
shaders, seen through a post effect shader.

    python examples/oberon/shader_lab/build_level.py   # then open it in Oberon
    cargo run -p oberon --release examples/oberon/shader_lab/level.ron

The shaders are plain ``.wgsl`` files in ``shaders/``. Three are material
shaders: a ``.mat.ron`` names one with ``shader:`` and passes it numbers
with ``shader_params:``. The other three are post effects; the scene's
"Look" entity carries a Post Shader naming one. Open a shader in Oberon,
change a number and save: the scene changes at once. Make a mistake and the
Console says which line, while the scene keeps the standard look until it's
fixed.

This script writes the materials and the scene; the shaders are hand-written.
"""

import os

import automata as at

HERE = os.path.dirname(os.path.abspath(__file__))

HEADER = (
    "// A material. Entities that use it share its colour, surface and\n"
    '// textures: material: "PATH" on a Mesh Renderer. Texture paths are\n'
    "// relative to this file's folder.\n"
)

# name -> fields, in the order Oberon's material editor writes them. Each
# shader's header comment says what its numbers do.
MATERIALS = {
    "hologram": {
        "color": (1.0, 1.0, 1.0, 1.0),
        "shader": "../shaders/hologram.wgsl",
        # colour (p0.rgb), scanlines per metre (p0.a), flicker (p1.x)
        "shader_params": [0.25, 0.85, 1.0, 7.0, 0.5],
    },
    "dissolve": {
        "color": (0.85, 0.82, 0.78, 1.0),
        "roughness": 0.6,
        "shader": "../shaders/dissolve.wgsl",
        # amount, speed, noise size (p0.xyz); edge colour and brightness (p1)
        "shader_params": [0.0, 0.6, 3.0, 0.0, 1.0, 0.45, 0.1, 6.0],
    },
    "lava": {
        "color": (1.0, 1.0, 1.0, 1.0),
        "shader": "../shaders/lava.wgsl",
        # flow speed, pattern size, glow brightness
        "shader_params": [1.0, 4.0, 3.0],
    },
}

# Post effects, by shader: the numbers each one starts with (``p0`` then
# ``p1``, as in the shader's header). ``LOOK`` is the one the scene uses.
POST_EFFECTS = {
    "crt": [0.08, 0.35, 1.5, 0.6],
    "ink": [0.85, 0.12, 4.0],
    "underwater": [3.0, 1.5, 0.0, 0.0, 0.45, 0.85, 0.9, 0.6],
}
LOOK = "crt"

# Showpieces on plinths: (name, material, shape, x).
SHOWPIECES = [
    ("Hologram", "hologram", "sphere", -2.0),
    ("Dissolving Crate", "dissolve", "cube", 0.0),
    ("Lava Ball", "lava", "sphere", 2.0),
]


def _value(v):
    if isinstance(v, bool):
        return "true" if v else "false"
    if isinstance(v, str):
        return f'"{v}"'
    if isinstance(v, tuple):
        return "(" + ", ".join(_value(x) for x in v) + ")"
    if isinstance(v, list):
        return "[" + ", ".join(_value(x) for x in v) + "]"
    return repr(float(v))


def material_text(name):
    """The ``.mat.ron`` file for ``MATERIALS[name]``."""
    fields = MATERIALS[name]
    body = "".join(f"    {k}: {_value(v)},\n" for k, v in fields.items())
    return HEADER.replace("PATH", f"materials/{name}.mat.ron") + "(\n" + body + ")\n"


def write_assets(folder=HERE):
    """Writes ``materials/`` under ``folder`` (and copies ``shaders/`` there
    when it's another folder)."""
    mats = os.path.join(folder, "materials")
    os.makedirs(mats, exist_ok=True)
    for name in MATERIALS:
        with open(os.path.join(mats, f"{name}.mat.ron"), "w", encoding="utf-8") as f:
            f.write(material_text(name))
    if os.path.abspath(folder) != HERE:
        shaders = os.path.join(folder, "shaders")
        os.makedirs(shaders, exist_ok=True)
        for name in os.listdir(os.path.join(HERE, "shaders")):
            with open(os.path.join(HERE, "shaders", name), encoding="utf-8") as src:
                text = src.read()
            with open(os.path.join(shaders, name), "w", encoding="utf-8") as dst:
                dst.write(text)


def mat(name):
    return f"materials/{name}.mat.ron"


def build(folder=HERE, look=LOOK):
    """The scene (materials and shaders must already be under ``folder``)."""
    at.set_asset_root(folder)
    scene = at.Scene()
    scene.add(at.Entity("Environment").add(at.Environment.preset("night")))
    scene.add(
        at.Entity("Moon").add(
            at.DirectionalLight(direction=(0.4, -1.0, -0.5), color=(0.7, 0.78, 1.0),
                                ambient=0.35)
        )
    )
    # The post effect: the first entity with a Post Shader draws it.
    scene.add(
        at.Entity("Look").add(at.PostShader(f"shaders/{look}.wgsl", params=POST_EFFECTS[look]))
    )
    scene.add(
        at.Entity("Floor")
        .add(at.Transform(position=(0.0, -0.05, 0.0), scale=(10.0, 0.1, 8.0)))
        .add(at.MeshRenderer.cube(size=1.0, color=(0.16, 0.17, 0.2, 1.0), roughness=0.8))
    )
    scene.add(
        at.Entity("Back Wall")
        .add(at.Transform(position=(0.0, 2.0, -3.2), scale=(10.0, 4.0, 0.2)))
        .add(at.MeshRenderer.cube(size=1.0, color=(0.22, 0.23, 0.27, 1.0), roughness=0.9))
    )
    for name, material, shape, x in SHOWPIECES:
        scene.add(
            at.Entity(f"{name} Plinth")
            .add(at.Transform(position=(x, 0.35, -0.6), scale=(0.9, 0.7, 0.9)))
            .add(at.MeshRenderer.cube(size=1.0, color=(0.3, 0.3, 0.33, 1.0), roughness=0.9))
        )
        make = at.MeshRenderer.sphere if shape == "sphere" else at.MeshRenderer.cube
        scene.add(
            at.Entity(name)
            .add(at.Transform(position=(x, 1.2, -0.6), rotation_euler=(0.0, 0.5, 0.0)))
            .add(make(size=0.9, material=mat(material)))
        )
    scene.add(
        at.Entity("Glow")
        .add(at.Transform(position=(2.0, 1.4, 0.4)))
        .add(at.PointLight(color=(1.0, 0.45, 0.15), intensity=6.0, range=5.0))
    )
    scene.add(
        at.Entity("Main Camera")
        .add(at.Transform(position=(0.0, 2.2, 5.2)))
        .add(at.Camera3D(position=(0.0, 2.2, 5.2), target=(0.0, 1.0, -0.6)))
    )
    return scene


if __name__ == "__main__":
    write_assets()
    path = os.path.join(HERE, "level.ron")
    build().save(path)
    print(f"wrote {path}")
