#!/usr/bin/env python3
"""Writes the Material Workshop: a room dressed entirely with shared
material files.

    python examples/oberon/workshop/build_level.py   # then open it in Oberon
    cargo run -p oberon --release examples/oberon/workshop/level.ron

Every surface names a ``materials/*.mat.ron`` file instead of carrying its
own colour: brick walls, a tiled floor, a stack of crates that all share one
wood material, brushed steel, copper and glossy paint on turntables, a
glowing neon sign and a pixel-art grass block. Open a material in Oberon
and every object wearing it changes as you edit; drag a material from the
Files panel onto an object to dress it.

The textures are drawn by ``textures.py`` (standard library only), so the
whole project rebuilds from these two scripts.
"""

import importlib.util
import os

import automata as at

HERE = os.path.dirname(os.path.abspath(__file__))

# The texture generator sits next to this file (loaded by path, so this
# works however the script is run).
_spec = importlib.util.spec_from_file_location("workshop_textures", os.path.join(HERE, "textures.py"))
textures = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(textures)

HEADER = (
    "// A material. Entities that use it share its colour, surface and\n"
    '// textures: material: "PATH" on a Mesh Renderer. Texture paths are\n'
    "// relative to this file's folder.\n"
)

# name -> fields, in the order Oberon's material editor writes them.
MATERIALS = {
    "brick": {
        "color": (1.0, 1.0, 1.0, 1.0),
        "roughness": 1.0,
        "base_map": "../textures/brick.png",
        "normal_map": "../textures/brick_normal.png",
        "roughness_metal_map": "../textures/brick_rm.png",
        "tiling": (8.0, 5.0),
    },
    "floor_tiles": {
        "color": (1.0, 1.0, 1.0, 1.0),
        "roughness": 1.0,
        "base_map": "../textures/tiles.png",
        "normal_map": "../textures/tiles_normal.png",
        "roughness_metal_map": "../textures/tiles_rm.png",
        "tiling": (6.0, 5.0),
    },
    "crate": {
        "color": (1.0, 1.0, 1.0, 1.0),
        "roughness": 0.8,
        "base_map": "../textures/planks.png",
        "normal_map": "../textures/planks_normal.png",
    },
    "brushed_steel": {
        "color": (0.86, 0.87, 0.9, 1.0),
        "roughness": 1.0,
        "metallic": 1.0,
        "roughness_metal_map": "../textures/brushed_rm.png",
    },
    "copper": {
        "color": (0.95, 0.6, 0.42, 1.0),
        "roughness": 1.0,
        "metallic": 1.0,
        "roughness_metal_map": "../textures/brushed_rm.png",
        "tiling": (2.0, 1.0),
    },
    "glossy_paint": {
        "color": (0.8, 0.08, 0.06, 1.0),
        "roughness": 0.15,
    },
    "neon": {
        "color": (0.04, 0.04, 0.05, 1.0),
        "roughness": 0.4,
        "emissive": (0.25, 0.9, 1.0),
        "emissive_strength": 4.0,
        "emissive_map": "../textures/neon.png",
    },
    "grass_block": {
        "color": (1.0, 1.0, 1.0, 1.0),
        "roughness": 0.9,
        "base_map": "../textures/grass_block.png",
        "pixelated": True,
    },
}

# Objects that share the crate material: (name, position, size, yaw).
CRATES = [
    ("Crate", (-3.6, 0.5, -3.6), 1.0, 0.1),
    ("Crate 2", (-2.45, 0.5, -3.9), 1.0, -0.25),
    ("Crate 3", (-3.1, 1.5, -3.7), 1.0, 0.35),
    ("Crate 4", (-4.6, 0.4, -2.4), 0.8, 0.6),
    ("Crate 5", (3.9, 0.35, 0.4), 0.7, -0.4),
]

# Showpieces on turntables: (name, material, x, turn speed).
SHOWPIECES = [
    ("Steel Ball", "brushed_steel", -1.6, 0.5),
    ("Copper Ball", "copper", 0.0, -0.4),
    ("Paint Ball", "glossy_paint", 1.6, 0.3),
]


def _value(v):
    if isinstance(v, bool):
        return "true" if v else "false"
    if isinstance(v, str):
        return f'"{v}"'
    if isinstance(v, tuple):
        return "(" + ", ".join(_value(x) for x in v) + ")"
    return repr(float(v))


def material_text(name):
    """The ``.mat.ron`` file for ``MATERIALS[name]``."""
    fields = MATERIALS[name]
    body = "".join(f"    {k}: {_value(v)},\n" for k, v in fields.items())
    return HEADER.replace("PATH", f"materials/{name}.mat.ron") + "(\n" + body + ")\n"


def write_assets(folder=HERE):
    """Writes ``textures/`` and ``materials/`` under ``folder``."""
    textures.build(os.path.join(folder, "textures"))
    mats = os.path.join(folder, "materials")
    os.makedirs(mats, exist_ok=True)
    for name in MATERIALS:
        with open(os.path.join(mats, f"{name}.mat.ron"), "w", encoding="utf-8") as f:
            f.write(material_text(name))


def mat(name):
    return f"materials/{name}.mat.ron"


def build(folder=HERE):
    """The scene (materials must already be on disk under ``folder``)."""
    at.set_asset_root(folder)
    scene = at.Scene()
    scene.add(at.Entity("Environment").add(at.Environment.preset("daylight")))
    scene.add(
        at.Entity("Sun").add(
            at.DirectionalLight(direction=(0.45, -1.0, -0.55), color=(1.0, 0.95, 0.86),
                                ambient=0.45)
        )
    )
    scene.add(
        at.Entity("Floor")
        .add(at.Transform(position=(0.0, -0.05, 0.0), scale=(12.0, 0.1, 10.0)))
        .add(at.MeshRenderer.cube(size=1.0, material=mat("floor_tiles")))
    )
    scene.add(
        at.Entity("Back Wall")
        .add(at.Transform(position=(0.0, 2.0, -5.15), scale=(12.0, 4.0, 0.3)))
        .add(at.MeshRenderer.cube(size=1.0, material=mat("brick")))
    )
    scene.add(
        at.Entity("Side Wall")
        .add(at.Transform(position=(-6.15, 2.0, 0.0), scale=(0.3, 4.0, 10.6)))
        .add(at.MeshRenderer.cube(size=1.0, material=mat("brick")))
    )
    for name, pos, size, yaw in CRATES:
        scene.add(
            at.Entity(name)
            .add(at.Transform(position=pos, rotation_euler=(0.0, yaw, 0.0)))
            .add(at.MeshRenderer.cube(size=size, material=mat("crate")))
        )
    for name, material, x, speed in SHOWPIECES:
        # Plain stone plinths: an object's own colour still works beside
        # material files.
        scene.add(
            at.Entity(f"{name} Plinth")
            .add(at.Transform(position=(x, 0.35, -1.4), scale=(0.9, 0.7, 0.9)))
            .add(at.MeshRenderer.cube(size=1.0, color=(0.32, 0.32, 0.35, 1.0), roughness=0.9))
        )
        ball = (
            at.Entity(name)
            .add(at.Transform(position=(x, 1.15, -1.4)))
            .add(at.MeshRenderer.sphere(size=0.9, material=mat(material)))
        )
        ball.script = "scripts/turntable.py"
        ball.attributes["speed"] = speed
        scene.add(ball)
    scene.add(
        at.Entity("Grass Block")
        .add(at.Transform(position=(3.6, 0.5, -3.6), rotation_euler=(0.0, -0.5, 0.0)))
        .add(at.MeshRenderer.cube(size=1.0, material=mat("grass_block")))
    )
    scene.add(
        at.Entity("Neon Sign")
        .add(at.Transform(position=(2.2, 2.55, -4.98)))
        .add(at.MeshRenderer.quad(size=1.6, material=mat("neon")))
    )
    scene.add(
        at.Entity("Main Camera")
        .add(at.Transform(position=(0.6, 2.7, 6.2)))
        .add(at.Camera3D(position=(0.6, 2.7, 6.2), target=(-0.2, 1.1, -2.0)))
    )
    return scene


if __name__ == "__main__":
    write_assets()
    path = os.path.join(HERE, "level.ron")
    build().save(path)
    print(f"wrote {path}")
