"""A play-script for the demo scene: spins and bobs the cube.

Attach it in Oberon by setting ``runner.set_update(update)`` before Play, or
run the headless comparison directly::

    python tools/oberon/examples/play_script.py
"""

import automata as at


def update(ctx):
    cube = ctx.scene.find("Cube")
    if cube is None or cube.transform is None:
        return
    x, y, z = cube.transform.position
    rx, ry, rz = cube.transform.rotation_euler
    cube.transform.rotation_euler = (rx, ry + ctx.time.delta, rz)
    cube.transform.position = (x, 0.5 + 0.4 * abs(__import__("math").sin(ctx.time.elapsed * 2.0)), z)


if __name__ == "__main__":
    game = at.Game(title="demo play-script", headless=True, target_fps=60)
    game.load_scene(at.Scene.load(__import__("pathlib").Path(__file__).with_name("demo_scene.ron")))
    game.update(update)
    print("ran", game.run(max_frames=120), "frames")
