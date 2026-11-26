# Robot Plaza

A town square full of copies of one **animated robot**. You drive one of
them. The others each show a different way to set up an Animation:

| Robot | What it shows |
| --- | --- |
| **Robot** (you) | An [animation graph](../../../docs/wiki/oberon/animation-graphs.md), `anims/robot.anim.ron`, blends Idle, Walking and Running by speed, plays Jump, and waves with the upper body only, even mid-stride. `scripts/player.py` just sets its parameters. It wears a top hat (three blocks with **Attach To Bone** on its Head) and has **Feet On Ground** on, so walk it up the steps and down the ramp. |
| **Greeter** | `scripts/greeter.py` turns its head to watch you once you're within 8 m (`animation.look_at`), plays Wave once when you walk up, then blends back to Idle. |
| **Dancer** | An Animation set to Dance. No script needed. |
| **Sitter** | Sitting with **Loop** off. One-shot clips hold their last frame. |
| **Slowpoke** | Walking at **Speed** 0.35, on the spot. |
| **Stepper** | Idle, with one foot on the bottom step and **Feet On Ground** on: that foot stands on the step and its knee bends. Untick it to see the foot sink in. |

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
- Select **Hat Crown** and drag it with the gizmo: it stays on the head, and
  its Transform shows the offset from the Head bone. Pick another **Bone**
  (`Palm2.R`) and the hat moves to the hand. Press the play button next to
  the Robot's **Preview** and the hat bobs along with it.
- Open `anims/robot.anim.ron` (double-click it in Files, or **Edit** next to
  the Robot's **Graph**). Change the Arms layer's mask from `["Torso"]` to
  `["Shoulder.R"]` and only the right arm waves; set the fades to 0 and see
  how much the blends matter.

`models/robot.glb.import.ron` scales the model to 1.8 m and moves its origin
to the middle, so it fits the Character Controller's capsule.

The robot is RobotExpressive by Tomás Laulhé (Quaternius), CC0. See
[CREDITS.md](CREDITS.md).

Guides: [Models & Animation](../../../docs/wiki/oberon/models.md), [Animation graphs](../../../docs/wiki/oberon/animation-graphs.md) and [Attachments & IK](../../../docs/wiki/oberon/attachments-and-ik.md).
