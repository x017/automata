# Robot Plaza

A town square full of copies of one **animated robot**. You drive one of
them. The others each show a different way to set up an Animation:

| Robot | What it shows |
| --- | --- |
| **Robot** (you) | `scripts/player.py` blends between Idle, Walking, Running, Jump and Wave as you move. |
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

The heart of `player.py` is a few lines. It works out which clip fits what
the robot is doing, then asks for it every frame. Asking for the clip that's
already playing does nothing, so the clip only changes when the choice does,
and `fade` blends the two over a fraction of a second:

```python
clip, loop = choose_clip(grounded, speed, anim.clip, anim.playing, keys.just_pressed("f"))
anim.play(clip, loop=loop, fade=FADES[clip])
```

Things to try in Oberon:

- Select **Dancer** and pick another clip from the Animation section's
  **Clip** menu. Press the play button next to **Preview** to watch it in the
  scene view without pressing Play, or drag the time bar to scrub through it.
- Set **Slowpoke**'s **Speed** to -1 to make it walk backwards.
- Drop `models/robot.glb` into the scene from the Assets panel. Oberon adds an
  Animation set to its Idle clip.
- In `player.py`, change the `FADES` to 0 and see how much the blends matter.

`models/robot.glb.import.ron` scales the model to 1.8 m and moves its origin
to the middle, so it fits the Character Controller's capsule.

The robot is RobotExpressive by Tomás Laulhé (Quaternius), CC0. See
[CREDITS.md](CREDITS.md).

Guide: [Models & Animation](../../../docs/wiki/guides/models-and-animation.md).
