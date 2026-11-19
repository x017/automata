#!/usr/bin/env python3
"""Convert the bundled Kenney GLB models to centered OBJ for AUTOMATA.

Requires trimesh only for this one-time asset conversion: `python -m pip install trimesh`.
The game itself reads the checked-in OBJ files using AUTOMATA's OBJ loader.
"""
from pathlib import Path

import trimesh

ASSETS = Path(__file__).resolve().parent

for source, target in (
    ("blaster-repeater.glb", "blaster-repeater.obj"),
    ("enemy-flying.glb", "enemy-flying.obj"),
):
    mesh = trimesh.load(ASSETS / source, force="scene").to_geometry()
    mesh.apply_translation(-mesh.bounds.mean(axis=0))
    (ASSETS / target).write_text(mesh.export(file_type="obj"), encoding="utf-8")
    print(f"converted {source} -> {target}")
