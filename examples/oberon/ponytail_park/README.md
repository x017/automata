# Ponytail Park

A park with a running track and four copies of one runner. Her **ponytail
and skirt are spring bones**: nothing animates them, they swing on their
own as she moves. Running streams them out behind, stopping throws them
forwards, and then they settle.

| Runner | What it shows |
| --- | --- |
| **Runner** (you) | `scripts/player.py` picks Idle, Walk or Run as you move. 1 waves and 2 cheers: those two clips come from another file. Press R to put the springs back at rest. |
| **Jogger** | `scripts/jogger.py` runs up the track, stops dead and turns round: the clearest swing and settle. |
| **Stroller** | `scripts/stroller.py` walks round the flowerbed. The turn leans the hair and skirt out a little. |
| **Spectator** | Just an Animation set to Idle. Watch the ponytail follow her head as she looks around. |

| Key | Action |
| --- | --- |
| W A S D / arrows | Walk |
| Shift | Run |
| 1 / 2 | Wave / cheer |
| R | Reset the springs |
| Q / E | Orbit the camera |

```bash
python examples/oberon/ponytail_park/build_level.py      # rewrites level.ron
python examples/oberon/ponytail_park/make_runner.py      # rewrites models/runner.glb and models/anims/emotes.glb
cargo run -p oberon --release examples/oberon/ponytail_park/level.ron
pytest examples/oberon/ponytail_park/tests
```

The springs live in the model's import settings,
`models/runner.glb.import.ron`, so every runner gets them:

```ron
springs: (
    chains: [
        (root: "Hair1", stiffness: 0.3, drag: 0.25, gravity: 0.6, radius: 0.03),
        (root: "Skirt", stiffness: 0.6, drag: 0.4, gravity: 0.3, radius: 0.025),
    ],
    colliders: [
        (bone: "Head", offset: (0.0, 0.09, 0.0), radius: 0.13),
        (bone: "Chest", offset: (0.0, 0.1, 0.0), radius: 0.14),
        (bone: "ThighL", offset: (0.0, -0.02, 0.0), radius: 0.07, length: -0.38),
        (bone: "ThighR", offset: (0.0, -0.02, 0.0), radius: 0.07, length: -0.38),
    ],
),
```

`Hair1` starts a chain down the ponytail. `Skirt` is the waistband: it has
eight panels below it, so it stays put and each panel swings on its own.
The head and chest colliders stop the ponytail swinging through her when
she stops, and the thigh capsules kick the skirt out of the way as she runs.

Things to try in Oberon:

- Select the **Jogger**, press Play and open **Springs** in the Mesh
  Renderer's Import block. Set the ponytail's **Drag** to 0.05 and watch it
  swing on for longer, or **Stiffness** to 1 to make it stick out.
- Give the ponytail some **Wind**, say 3 along X.
- Remove the **Chest** collider and watch the ponytail swing through her
  back when she stops.
- Remove the **Skirt** chain, then add it back with **+ Chain**. It's at the
  top of the list under Suggested.

`make_runner.py` builds the runner from boxes, spheres and tubes: a skinned
glTF with 49 bones and three clips (Idle, Walk, Run). The hair and skirt
bones have no animation. Rust tests run this same model through a run and a
stop: `crates/automata-scene/tests/spring_runner.rs`.

### Clips from another file

The wave and the cheer aren't in `runner.glb`. They're in
`models/anims/emotes.glb`, which holds only the skeleton and those two
clips, the way animation packs ship one file per move. The import settings
borrow them:

```ron
clips: [
    (file: "anims/emotes.glb"),
],
```

The file has two clips, so they're called `emotes:Wave` and `emotes:Cheer`,
and `player.py` plays them like any other clip:

```python
anim.play("emotes:Wave", loop=False, fade=0.15, restart=True)
```

In Oberon they're under **Clip files** in the Import block, and in the
Animation component's **Clip** menu.

Guide: [Models → Clips from other files](../../../docs/wiki/oberon/models.md#clips-from-other-files),
[Models → Spring bones](../../../docs/wiki/oberon/models.md#spring-bones).
