# Models

Import 3D models made in Blender or any other modelling tool, or
downloaded from an asset store, and give them to your entities. Oberon
reads two formats:

| Format | Files | Notes |
|---|---|---|
| glTF 2.0 | `.glb` (one file) or `.gltf` (+ `.bin`, images) | Recommended. Blender: **File → Export → glTF 2.0**. |
| Wavefront OBJ | `.obj` (+ `.mtl`, images) | Universal and simple. Has no node hierarchy. |

From each glTF material Oberon takes the full PBR set: **base colour** and
its texture, **roughness** and **metallic** (factors and the
metallic-roughness map), the **normal map**, and **emission** (colour, map
and `KHR_materials_emissive_strength`). Textures can be PNG or JPEG.
Spec-gloss materials and `KHR_texture_transform` are converted too. OBJ
files bring base colour and texture, and are drawn as a non-metal with
roughness 0.6. The model draws with the engine's lighting, shadows and fog,
like the built-in cubes.

### Animated models

glTF files keep their **skeleton and animations**: skinned meshes (up to
four bones per vertex, 128 bones per model) and parts moved by animated
nodes, like a robot whose arms are separate meshes. The renderer poses
them on the GPU, and shadows follow the pose.

For now an animated model shows its rest pose in the scene view and in
your game. Playing clips from scripts (`entity.animation.play("run")`) and
previewing them in the Inspector come next. Rust games can already play
clips with the `Animator` component.

## Add a model to the scene

Pick whichever way suits you:

- **Pick it in the Assets tab** (below). Every model in the project is
  shown as a thumbnail.
- **Drag a file from your system file manager** onto the scene view. Oberon
  copies it into the project's `models/` folder and places it where you
  dropped it, standing on the ground.
- **Drag a model from the Files panel** onto the scene view.
- **Double-click a model** in the Files panel to add it where the view is
  looking.
- **GameObject → Model…** (or **+ → Create Model…** in the Hierarchy)
  opens a searchable list of the project's models. Choose **Import from
  file…** at the bottom of the list to pick a file anywhere on disk.

The new entity is named after the file (`oak_tree.glb` becomes **Oak
tree**). It shows the whole model: every mesh and material in the file
draws as one entity, so you move, rotate and scale it as one object.

## The Assets tab

**Window → Assets** opens the Assets tab next to the Console. It shows every
model in the project as a rendered thumbnail, so you can find the crate you
want without reading file names.

![The Assets tab showing Kenney's FPS models](../../assets/oberon/assets.png)

| Control | What it does |
|---|---|
| **Models** / **Sounds** | Switch between model thumbnails and sound waveforms. The count is the number of files. |
| **Filter** | Type part of a name or folder (`walls`, `enemy`) to narrow the grid. |
| **Import…** | Copy a model or sound from anywhere on disk into the project. |

Tiles work like files in the Files panel:

- **Drag** a model tile onto the scene to place it there, or onto the
  Inspector to give it to the selected entity.
- **Double-click** a tile to add the model where the view is looking.
- **Right-click** a tile for the same menu as in the Files panel (*Add to
  Scene*, *Use on <entity>*, …).

Thumbnails render on the CPU, so they work without a GPU too. They're
cached, and they redraw when the file or its import settings change.

### What importing copies

Only the project folder ships with an exported game, so files from outside
the project are copied in:

- A self-contained file (`.glb`, or a `.gltf` / `.obj` that uses no other
  files) goes to `models/<name>`.
- A model with companion files (`.bin`, `.mtl`, textures) gets its own
  folder, `models/<name>/`. The companions keep their paths relative to the
  model, so the model's internal references keep working.
- A model that's already inside the project is used where it is.

## Give a model to an existing entity

- Select the entity and drag a model from the Files panel onto the
  **Inspector**.
- Or open the Mesh Renderer's **Primitive** menu and choose **Model…**.
- Or right-click a model in the Files panel and choose **Use on
  <entity>**.

If the entity has a collider, it's refitted to the new model (see
[Colliders](#colliders)).

## The Inspector

With a model selected, the **Mesh Renderer** section shows:

| Row | What it does |
|---|---|
| **Primitive** | `Model`. Switch to Cube, Sphere or Quad to go back to a primitive. |
| **File** | The model's path relative to the scene. It turns red if the file is missing or can't be read. Double-click it to show the file in the Files panel. |
| *summary* | Parts (one per material), triangle count, size in metres, and whether it's textured. If loading failed, this line shows the error instead. |
| **Change…** | Pick a different model. |
| **Add Collider / Fit Collider** | Adds a collider shaped for the model, or refits the existing one. |
| **Tint** | Multiplies every material's colour. Leave it white to see the model as authored. |
| **Surface**, **Roughness**, **Metallic**, **Emission** | Leave them at Default to keep the file's materials. Setting one overrides it on every part. See [Surfaces](interface.md#surfaces). |

## Import settings

Models from different tools and stores rarely agree on size, facing or
origin. Instead of fixing every entity that uses a model, fix the model
once. Its **Import** block in the Inspector (under the Mesh Renderer) holds
three settings:

| Setting | What it does | Quick buttons |
|---|---|---|
| **Scale** | Multiplies the model's size. Use it for kits authored in centimetres or at toy scale. | **×0.01**, **×10** |
| **Rotation** | Turns the model, in degrees about X, Y and Z, before the entity's own rotation. | **+90°** about Y |
| **Pivot** | Where the entity's position sits on the model: **File** (the file's own origin), **Bottom** (centre of the base, so it stands on the ground) or **Centre**. | |

Type a value or use a quick button, and every entity using that file
updates at once, along with its thumbnail. **Reset** goes back to the file
as it is. The settings apply wherever the model is used: in Play, under
`run_scene`, and in exported games.

Settings are saved next to the model as `<file>.import.ron`. For example,
`models/blaster.glb.import.ron`:

```ron
(
    scale: 1.0,
    rotation: (0.0, 180.0, 0.0),
    pivot: File,
)
```

Commit these files with your models. A model without one is used exactly
as the file describes it. Fitted colliders use the model's size after the
import settings, so press **Fit Collider** after changing them.

## Colliders

**Add Collider** picks a shape from the model:

| The model | Shape |
|---|---|
| Fills at least 90% of its bounding box (crates, slabs) | **Box**, fitted to the bounds |
| Anything else, with no rigid body or a static one | **Mesh**: the model's own triangles |
| Anything else, on a dynamic or kinematic body | **Convex Hull**: shrink-wrapped around the model |

The collider section's **Shape** menu shows the choice; pick another to
override it. Mesh and Convex Hull appear in the menu when the entity shows a
model, and a **Model** row shows which file the shape comes from. Shapes
built from a model follow the entity's scale and the model's import
settings by themselves.

A box doesn't scale with the entity. Models are rarely centred on their
origin (a tree's origin is at its roots), so a fitted box also gets an
**offset** that centres it on the mesh. After you scale a model, press
**Fit Collider** to resize the box. Fit keeps the shape you picked. Swapping
the model picks the shape again, since the old one was chosen for the old
model.

A Mesh collider on a **Dynamic** body shows an error. A triangle mesh has
no inside, so it can't give the body mass, and it plays as its convex hull.
Pick Convex Hull, or make the body Static or Kinematic. **Build Game**
refuses to export a scene with that mistake. For a character or a round
object, use Capsule or Sphere and adjust the size.

## Editing a model while Oberon is open

Oberon watches model files. Re-export from Blender and the scene view
updates within half a second, with no reload. The Console says *Model files
changed on disk; reloaded*. Texture and `.mtl` changes are picked up too.

## Scene files and paths

The scene stores only the path, relative to the scene's folder:

```ron
mesh_visual: Some((
    primitive: Model(
        path: "models/tree.glb",
    ),
    color: (1.0, 1.0, 1.0, 1.0),
)),
collider_3d: Some((
    shape: Cuboid(extents: (1.5, 2.4, 1.5)),
    offset: (0.0, 1.2, 0.0),
    // …
)),
```

A Mesh or Convex Hull collider stores the model path, not the triangles:

```ron
collider_3d: Some((
    shape: Mesh(model: "models/platform.glb"),
    // …
)),
```

Paths always use `/`, so scenes move between Windows and Linux unchanged.
Keep models inside the project folder. **Build → Build Game** refuses to
export a scene that references a missing model or one outside the project,
and warns if a model's textures are missing.

## From Python

```python
import automata as at

tree = (
    at.Entity("Tree")
    .add(at.Transform(position=(2, 0, -3), scale=(1.5, 1.5, 1.5)))
    .add(at.MeshRenderer.model("models/tree.glb"))
    .add(at.RigidBody.fixed())
    .add(at.Collider3D.cuboid(2.2, 3.6, 2.2).offset(0, 1.8, 0))
)

m = at.MeshRenderer.model("models/crate.obj", color=(1, 0.8, 0.8, 1))
(lo, hi) = m.bounds      # model-space box, e.g. for sizing a collider
```

Relative paths resolve against the scene's folder when the scene came from
`Scene.load` / `run_scene`. Otherwise they resolve against your script's
folder, and you can call `at.set_asset_root(path)` to choose a folder
yourself. A missing file raises `FileNotFoundError`. See the
[API reference](../api-reference.md#visuals).

## Try it

`examples/oberon/fps/` uses ten of Kenney's CC0 models, three of them with
import settings. See the [FPS tutorial](../tutorials/fps.md).

`examples/models/` has a two-material glTF tree with an embedded texture,
a textured OBJ crate, and `scene.ron` placing them. Open it:

```console
cargo run -p oberon --release examples/models/scene.ron
```

Press Play and the top crate drops onto the bottom one. Their offset box
colliders cover the meshes, not the pivots. `make_models.py` regenerates
the files.

## Troubleshooting

| Symptom | Fix |
|---|---|
| File row red, *File not found* | The model moved or the scene was saved elsewhere. **Change…** to re-pick it. |
| *…mesh compression, which isn't supported* | Re-export without mesh compression. These glTF extensions aren't supported. |
| Model is huge or tiny | Modelling tools disagree on units. Set **Import → Scale** (×0.01 for centimetres), or export from Blender with **Apply Unit** checked. glTF is always metres. |
| Model faces the wrong way | glTF's forward is +Z. Press **Import → Rotation → +90°** until it faces the right way. |
| Model floats or sinks when placed | Its origin isn't at its base. Set **Import → Pivot** to **Bottom**. |
| Looks inside-out | The mesh's faces point inward. Recalculate normals in the modelling tool (Blender: **Mesh → Normals → Recalculate Outside**). |
| Untextured but should be | Only the base colour texture is read. Check the image is PNG or JPEG and that the path in the `.mtl` / `.gltf` exists. |
