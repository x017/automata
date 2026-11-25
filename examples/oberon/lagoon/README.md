# Lagoon

A small island in a turquoise sea, built to show off Water: depth colour,
refraction and shoreline foam on Beautiful graphics, plus water physics.
The crates and buoys are ordinary rigid bodies. The water's buoyancy holds
them up and its waves rock them. When Play starts, a crate drops into the
sea and rings spread out from the splash, while a motor boat cruises round
the island leaving a wake. A swimmer waits at the end of the pier: press
Tab, run, jump in with a splash, and dive into a blue-green world with
sunlight dancing on the reef.

| Input | Action |
| --- | --- |
| Drag (either mouse button), A / D, Q / E, arrow keys | Orbit the island |
| W / S | Zoom |
| Tab | Take control of the swimmer (Tab again: back to the island view) |
| W / A / S / D, Shift, Space | Swimmer: walk (relative to the camera), run, jump. In the water Space swims up |
| C or Ctrl | Swimmer: dive |
| G | Cycle the graphics quality (Low → Medium → High → Beautiful) |
| Esc | Quit |

## Run it

```sh
# In the editor: open the scene, pick Graphics → Beautiful, press Play (Ctrl+P)
cargo run -p oberon --release examples/oberon/lagoon/level.ron

# Without the editor
python examples/oberon/lagoon/main.py                   # Beautiful
python examples/oberon/lagoon/main.py --graphics high
python examples/oberon/lagoon/main.py --headless        # 300 frames, no window
```

## How the floating works

Every entity with a `Water` component is also a volume of water for physics.
Each step, an awake dynamic body that reaches into it is pushed up by
`water density × gravity × submerged volume` and slowed by the water's `drag`.
Water's density defaults to 2.0 and colliders' to 1.0, so a default box floats
half under. In this scene:

| Thing | Collider | Density | Floats |
| --- | --- | --- | --- |
| Crates | convex hull of `models/crate.obj` | 1.0 | half under |
| Buoys | box | 0.8 | 40% under |

The boat is different: a kinematic body that `scripts/motor_boat.py` steers
round an ellipse. Each frame it reads the water height under its bow,
stern and sides to sit on the swell and pitch and roll with it.

Give something a density above 2 and it sinks, slowly, because the water
drags on it. Bodies follow the waves the GPU draws: big bodies are sampled at
eight points, so they pitch and roll on the swell. Things that settle on calm
water (the pool) fall asleep and cost nothing.

Scripts can ask for the surface height too, for swimming or riding the
waves, and make splashes of their own:

```python
h = ctx.water.height(x, z)           # None where there's no water
ctx.water.splash(entity, 2.0)        # rings and foam, strength 0 to 10
```

## Ripples and wakes

A ripple simulation runs around the camera (64 m across). Whatever crosses
the surface stirs it: the crate's splash sends out rings, the buoys ring
gently as they bob, and the boat pushes ripples aside into a V-shaped wake
with a foam trail. They show on every graphics preset. See
[Physics → Ripples and wakes](../../../docs/wiki/guides/physics.md#ripples-and-wakes).

## Swimming and diving

`scripts/swimmer.py` drives a character (a kinematic body with a
`CharacterController`). On land it walks, runs and jumps. Once the water
is up to its chest it swims instead: it floats with its head out, C dives
and Space swims back up. While it's in control, `scripts/orbit_camera.py`
follows close behind it, and when it dives the camera follows it under.

Underwater, the world turns blue-green with distance, the sun throws
moving caustics on the reef, sand and pier posts (Medium graphics and up),
and the sound goes muffled until you surface. Jumping in hard enough
throws up a splash sized by how fast you hit the water. See
[Rendering → Under the surface](../../../docs/wiki/guides/rendering-and-lights.md#under-the-surface).

## Export it

```sh
automata-cli build examples/oberon/lagoon --target all
```

## What's inside

| Path | What |
| --- | --- |
| `level.ron` | The scene: the sea and pool, the island in sand slabs and ramps, rocks, trees, the pier and the floating things. |
| `scripts/orbit_camera.py` | Orbits the island under the mouse and keys, drifts round on its own when idle, and follows the swimmer while it's in control. |
| `scripts/motor_boat.py` | Steers the boat round the island on the waves. |
| `scripts/swimmer.py` | The swimmer: walks, jumps, swims and dives (Tab to take control). |
| `models/` | The crate and tree models. |
| `main.py` | Runs the scene with the quality switch (G) and a HUD. |
| `build_level.py` | Rebuilds `level.ron` from Python. Run it after changing the layout in code. |
| `tests/` | Headless tests: `pytest examples/oberon/lagoon/tests`. |
