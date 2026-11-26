# Robot Plaza

A town square full of copies of one **animated robot**. You drive one of
them. The others each show a different way to set up an Animation:

| Robot | What it shows |
| --- | --- |
| **Robot** (you) | An [animation graph](../../../docs/wiki/oberon/animation-graphs.md), `anims/robot.anim.ron`, blends Idle, Walking and Running by speed, plays Jump, and waves with the upper body only, even mid-stride. `scripts/player.py` just sets its parameters. |
| **Greeter** | `scripts/greeter.py` plays Wave once when you walk up, then blends back to Idle. |
| **Dancer** | An Animation set to Dance. No script needed. |
| **Sitter** | Sitting with **Loop** off. One-shot clips hold their last frame. |
| **Slowpoke** | Walking at **Speed** 0.35, on the spot. |

| Key | Action |
| --- | --- |
| W A S D / arrows | Walk |
| Shift | Run |
| Space | Jump |
| F | Wave |
| Q / E | Orbit the camera |

```bash
python examples/oberon/robot_plaza/build_level.py        # rewrites level.ron
cargo run -p oberon --release examples/oberon/robot_plaza/level.ron
pytest examples/oberon/robot_plaza/tests
```

`player.py` never names a clip. Each frame it tells the robot's graph what
the robot is doing, and the graph picks and blends the clips:

```python
anim.set("speed", speed)        # 0 = Idle, 2.2 = Walking, 5 = Running, blended between
anim.set("grounded", grounded)  # Jump goes back to moving once this is True
if jumped:
    anim.trigger("jump")
if waved:
    anim.trigger("wave")        # the Arms layer waves over whatever the legs do
```

Things to try in Oberon:

- Select **Dancer** and pick another clip from the Animation section's
  **Clip** menu. Press the play button next to **Preview** to watch it in the
  scene view without pressing Play, or drag the time bar to scrub through it.
- Set **Slowpoke**'s **Speed** to -1 to make it walk backwards.
- Drop `models/robot.glb` into the scene from the Assets panel. Oberon adds an
  Animation set to its Idle clip.
- Open `anims/robot.anim.ron` (double-click it in Files, or **Edit** next to
  the Robot's **Graph**). Change the Arms layer's mask from `["Torso"]` to
  `["Shoulder.R"]` and only the right arm waves; set the fades to 0 and see
  how much the blends matter.

`models/robot.glb.import.ron` scales the model to 1.8 m and moves its origin
to the middle, so it fits the Character Controller's capsule.

The robot is RobotExpressive by Tomás Laulhé (Quaternius), CC0. See
[CREDITS.md](CREDITS.md).

Guides: [Models & Animation](../../../docs/wiki/oberon/models.md) and [Animation graphs](../../../docs/wiki/oberon/animation-graphs.md).
