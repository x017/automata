"""Writes each conformance scenario's `scene.ron` (run after changing one).

    python tests/script_conformance/make_scenes.py
"""

import os

import automata as at

HERE = os.path.dirname(os.path.abspath(__file__))


def scripted(name, script, position=(0.0, 0.0, 0.0), **attrs):
    e = at.Entity(name).add(at.Transform(position=position))
    e.script = script
    for k, v in attrs.items():
        e.attributes[k] = v
    return e


def plain(name, position=(0.0, 0.0, 0.0)):
    return at.Entity(name).add(at.Transform(position=position))


def scenes():
    s = at.Scene()
    player = plain("Player")
    player.attributes["hp"] = 100
    s.add(player)
    for i in range(4):
        s.add(scripted(f"Drone {i + 1}", "drone.py", (float(i), 0.0, -3.0)))
    yield "deferred_writes", s

    s = at.Scene()
    s.add(scripted("Mover", "mover.py"))
    yield "transform", s

    s = at.Scene()
    s.add(scripted("Box", "box.py", int=41, text="there", flag=False, gone=1,
                   start_vec=(4.0, 5.0, 6.0)))
    yield "attributes", s

    s = at.Scene()
    s.add(scripted("Cam", "camera.py", (0.0, 2.0, 10.0))
          .add(at.Camera3D(position=(0.0, 2.0, 10.0), target=(0.0, 0.0, 0.0), fov=60.0)))
    s.add(plain("Target", (4.0, 0.0, 0.0)))
    yield "camera", s

    s = at.Scene()
    s.add(plain("Wall", (0.0, 1.0, -5.0)).add(at.Collider3D.cuboid(1.0, 1.0, 1.0)))
    s.add(scripted("Shooter", "shooter.py", (0.0, 1.0, 0.0)))
    yield "raycast", s

    s = at.Scene()
    s.add(scripted("Doomed", "doomed.py", (2.0, 0.0, 0.0)))
    s.add(scripted("Watcher", "watcher.py"))
    yield "scene", s

    s = at.Scene()
    s.add(scripted("Probe", "probe.py"))
    yield "input", s


if __name__ == "__main__":
    for name, scene in scenes():
        scene.save(os.path.join(HERE, name, "scene.ron"))
        print("wrote", name)
