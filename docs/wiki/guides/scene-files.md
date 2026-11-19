# Scene Files: Saving and Loading Worlds

Every scene can be written to disk as a **`.ron` file** — a plain-text,
hand-editable, git-diffable snapshot of the world. This is the format the
Oberon editor saves, but it works just as well for hand-authored levels,
save games, and test fixtures.

```python
import automata as at

# Save a staged scene...
scene = at.Scene().add(at.Entity("crate").add(at.Transform(position=(0, 5, 0))))
scene.save("scenes/level1.ron")

# ...and load it back later
game = at.Game(title="my game")
game.load_scene(at.Scene.load("scenes/level1.ron"))
game.run(update)
```

## What a scene file looks like

RON (Rust Object Notation): struct-like syntax with comments and trailing
commas. One record per entity; absent components are omitted entirely.

```ron
(
    version: 1,
    entities: [
        (
            name: Some("crate"),
            transform: Some((
                position: (0.0, 5.0, 0.0),
                rotation_euler: (0.0, 0.0, 0.0),   // radians, XYZ order
                scale: (1.0, 1.0, 1.0),
            )),
            mesh_visual: Some((
                primitive: Cube(size: 0.8),
                color: (0.72, 0.52, 0.3, 1.0),
            )),
            rigid_body: Some((
                body_type: Dynamic,
                gravity_scale: 1.0,
                linear_damping: 0.0,
                angular_damping: 0.0,
                enabled: true,
            )),
            collider_3d: Some((
                shape: Cuboid(extents: (0.8, 0.8, 0.8)),
                density: 1.0,
                friction: 0.5,
                restitution: 0.3,
                sensor: false,
                enabled: true,
            )),
        ),
    ],
)
```

Every field is exactly what the matching `at.*` constructor accepts — you
can hand-edit values, add entities, or write whole levels by hand.

## The four entry points

| Call | Input | Use it for |
|---|---|---|
| `scene.to_ron()` | a staged `Scene` | in-memory snapshots (undo, play-mode restore) |
| `scene.save(path)` | a staged `Scene` | writing levels from code |
| `at.Scene.from_ron(text)` | RON text | parsing without touching the filesystem |
| `at.Scene.load(path)` | a file path | loading levels into `game.load_scene(...)` |

And on a **running** game:

| Call | Captures |
|---|---|
| `game.scene_to_ron()` | the **live world** — moved transforms and all |
| `game.save_scene(path)` | same, straight to disk |

The staged/lived distinction matters: `scene.save()` writes what you
*built*; `game.save_scene()` writes what *is* — after physics, scripts,
and events have run. A crate staged at `y=5` and saved after 240 frames is
recorded at its resting position.

## Durability rules

- `version: 1` sits at the top of every file. Loading a file written by a
  **newer** engine raises `ValueError` instead of guessing.
- Unknown fields are ignored; missing fields fall back to defaults. Older
  and newer engines read each other's files as far as the overlapping
  fields allow.
- Malformed files raise `ValueError` with line/column information from the
  RON parser; unreadable paths raise `IOError`.

## Current limitations

- Scenes are **flat** — entity parenting (`Parent`) is not serialized yet.
- `MeshVisual` primitives (`Cube`, `Quad`) serialize by tag; custom
  geometry works but stores raw vertices (`Custom` variant).
- Sprites and textures are not part of the format yet (they arrive with
  the asset pipeline).
- Serialization captures authoring data, not runtime physics state:
  velocities and contacts are rebuilt on load.

## See also

- [Scenes and Entities](scenes-and-entities.md) — the staging model
- [Project Structure](project-structure.md) — where scene files live in a
  growing project
- [Oberon design](../../design/oberon.md) — the editor that writes
  these files
