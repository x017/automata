"""AUTOMATA in the browser: the web spike (roadmap E3, phase 1).

A spinning cube on a floor, drawn by the engine with WebGL2 inside
Pyodide. CI loads it in headless Chromium, waits for frames, takes a
screenshot and reads the `automata-spike` lines it prints (the console).
"""

import automata as at

print(f"automata-spike start platform={getattr(at._core, 'platform', 'desktop')}")

game = at.Game(title="AUTOMATA", clear_color=(0.10, 0.12, 0.18, 1.0))
game.load_scene(
    at.Scene()
    .add(at.Entity("camera").add(at.Camera3D(position=(0.0, 2.6, 6.0), target=(0.0, 0.6, 0.0))))
    .add(at.Entity("sun").add(at.DirectionalLight(direction=(-0.4, -1.0, -0.35), ambient=0.35)))
    .add(
        at.Entity("floor")
        .add(at.Transform(position=(0.0, -0.5, 0.0), scale=(12.0, 1.0, 12.0)))
        .add(at.MeshRenderer.cube(size=1.0, color=(0.30, 0.33, 0.40, 1.0)))
    )
    .add(
        at.Entity("cube")
        .add(at.Transform(position=(0.0, 0.8, 0.0)))
        .add(at.MeshRenderer.cube(size=1.2, color=(0.95, 0.55, 0.20, 1.0)))
    )
)
state = {"spin": 0.0}


@game.update
def update(ctx):
    state["spin"] += ctx.time.delta * 1.2
    ctx.scene["cube"].transform.rotation_euler = (0.25, state["spin"], 0.0)
    width, height = ctx.screen.size
    ctx.screen.text(
        f"AUTOMATA on the web   {width}x{height}   frame {ctx.time.frame}",
        24.0, 40.0, size=30.0, shadow=(0.0, 0.0, 0.0, 0.8),
    )
    if ctx.time.frame % 60 == 0:
        print(f"automata-spike frame={ctx.time.frame} size={width}x{height}")


game.run()
print(f"automata-spike started stepping={game.stepping}")
