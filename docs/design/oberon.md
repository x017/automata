# Oberon — Design Document

**Status:** proposal for review · **No code yet**

Oberon is the visual editor for AUTOMATA: a standalone desktop application
in the style of Unity's editor — viewport, hierarchy, inspector, play
controls — for building scenes, placing entities, and tweaking component
values without touching code. It is **not** an in-game overlay; it is a
separate app that drives the engine.

```
┌──────────────────────────────────────────────────────────────┐
│  File  Edit  Assets                              ▶ ⏸ ⏭      │
├────────────┬────────────────────────────────┬────────────────┤
│            │                                │                │
│  Hierarchy │           Viewport             │   Inspector    │
│            │   (live engine render)         │                │
│  ▸ scene   │                                │  Transform     │
│    ▸ floor │                                │  MeshRenderer  │
│    ▸ sun   │                                │  RigidBody     │
│    ▾ player│                                │  Collider3D    │
│            │                                │                │
├────────────┴────────────────────────────────┴────────────────┤
│  Console / Project                                           │
└──────────────────────────────────────────────────────────────┘
```

## 1. What Oberon is — and is not

| Is | Is not |
|---|---|
| A Python desktop app built with **nexa-ui** | An in-game debug overlay |
| A scene authoring + inspection tool | A code editor (VS Code owns that, like Unity) |
| A client of the **public engine API** | A fork or privileged build of the engine |
| The place you *arrange* a game | The place you *program* a game |

The engine stays code-first. Oberon produces and consumes **scene files**;
game logic remains Python in the user's editor of choice.

## 2. Brand

Oberon follows AUTOMATA's identity: **dark white sci-fi**.

- Near-black background, off-white foreground (matching the logo's
  paper-white on true black), one accent used sparingly.
- Thin lines, orbital motifs, generous spacing. nexa theming does the
  heavy lifting (`accent`, dark palette).
- The logo's gear-in-the-A reads as the inspector's component gear; the
  orbital ring is the play-mode indicator. Brand assets live in
  `docs/assets/`.

## 3. Architecture

### 3.1 Process model: in-process embedding

Oberon runs the engine **in the same Python process** as the UI:

```
┌─ Oberon (Python process) ─────────────────────────┐
│  nexa-ui event loop (main thread)                 │
│    ├─ hierarchy panel  ← reads scene names/tree   │
│    ├─ inspector panel  ← reads/writes components  │
│    └─ viewport widget  ← RGBA bytes each frame    │
│         ▲                                          │
│  automata engine (Rust core via PyO3)              │
│    ├─ game.step(dt) — advance one frame           │
│    └─ game.render_frame(w, h) → bytes (offscreen) │
└───────────────────────────────────────────────────┘
```

One tick of the nexa loop in edit mode:

1. `game.step(dt)` — physics + buffered writes apply (game logic paused).
2. `rgba = game.render_frame(viewport_size)` — offscreen wgpu render,
   read back to CPU bytes.
3. A nexa image widget displays `rgba`.

Why the viewport is an image feed and not GPU interop: nexa's core is a
C99 immediate-mode UI with OpenGL 3.3 + a CPU software rasterizer, while
automata renders through wgpu. Sharing GPU textures between those two
stacks is not realistic; the RGBA handoff through CPU memory is simple,
portable, and fast enough at editor scale (§11 has the numbers). The
zero-readback fallback — the engine opening its own native window beside
the Oberon window — stays available if the smoke test says otherwise.

Why in-process, and not an editor/game subprocess pair:

- **Frame readback is a memcpy**, not a socket stream — the viewport is
  just a texture handoff.
- **The inspector reads live ECS state** through the existing `EntityView`
  handles; no state-sync protocol to design or debug.
- **Edits are ordinary engine calls** through the existing buffered-write
  path — the editor can never desync from the engine's frame model,
  because it *is* the frame.

Accepted cost: a hard crash in the engine can take the UI down. The engine
already isolates Python callback exceptions (they propagate out of
`step()` and Oberon shows them in the console panel). A subprocess mode is
a possible later hardening, not MVP scope.

### 3.2 Why not an overlay / why not a code editor

Recorded decisions (both stand):

- In-game overlay editors fight the game for input, screen space, and the
  render pass. A separate window is the Unity model and the right model.
- An in-engine code editor with LSP is not worth building: VS Code with
  the Python extension is strictly better. Oberon opens files in the
  user's `$EDITOR` instead.

## 4. Engine work required (the embedding hooks)

Oberon consumes only public Python API. Four engine additions unlock it,
in order:

### 4.1 Scene serialization — RON *(decided)*

Scenes persist as **RON** (Rust Object Notation): text, comments, trailing
commas, struct-like syntax, clean git diffs. Same choice Bevy made.

```ron
Scene(
  entities: [
    ( name: Some("sun"), components: [
        DirectionalLight(direction: (-0.45, -1.0, -0.3),
                         color: (1.0, 0.96, 0.88),
                         ambient: 0.35, shadows: true),
    ]),
    // ...
  ],
)
```

- `serde` derives on every component type; a `version` field at the root
  for forward compatibility.
- Python API: `at.Scene.save(path)` / `at.Scene.load(path)`, plus
  `scene.to_ron()` / `at.Scene.from_ron(text)` for in-memory snapshots
  (play-mode restore, §6).
- CBOR remains the future **shipping/packing** format (binary, compact,
  fast) — free to add later since serde separates data model from format.
  Authoring stays text; git never sees binary scenes.

### 4.2 Manual stepping

Today `game.run()` owns the loop. The editor needs to drive it:

```python
game = at.Game(headless=True, graphics=at.GraphicsQuality.HIGH)
game.load_scene(scene)
game.step(dt)          # advance exactly one frame: physics, scripts, writes
```

`step` is the same machinery as `run(max_frames=1)`, minus loop ownership:
no `target_fps` sleeping, no quit checks — Oberon owns pacing. Edit mode
steps *without* the game's update callback (simulation paused, writes
apply); play mode steps *with* it.

### 4.3 Offscreen frame readback

The renderer already renders offscreen (used by the GPU test suite,
including readback to CPU). Expose it:

```python
rgba: bytes = game.render_frame((width, height))  # RGBA8, row-major
```

Oberon feeds these bytes to a nexa image widget. 1280×720×4 bytes ≈
3.5 MB/frame — trivial locally; the risk item is nexa's image-widget
update cost (§11).

### 4.4 Full component read-model + writable visuals

The inspector must read *and* write everything it shows. Today readable:
name, transform, camera_3d, character_velocity. Missing:

- **Reads:** `MeshRenderer` (kind, size, color), `DirectionalLight`
  (direction, color, ambient, shadows), `RigidBody` (type, damping),
  `Collider*` (shape, friction, restitution, sensor).
- **Writes:** mesh color, light properties, collider modifiers. (The wiki
  currently says "material swaps are a spawn-a-new-entity job (for now)" —
  this milestone is what retires that caveat.)

Same buffered-write semantics as transforms: edits apply after the current
frame, exactly like gameplay writes.

## 5. Panels

| Panel | Contents | nexa building block |
|---|---|---|
| **Hierarchy** | entity tree (name, visibility glyph), add/remove/rename | `tree_table` |
| **Viewport** | live offscreen render; editor camera; click-to-select; drag-to-move | image widget + canvas overlay |
| **Inspector** | selected entity's components as typed editors (vec3 fields, color swatch, enum dropdowns, checkboxes) | property-inspector component |
| **Console** | engine logs + Python stdout/stderr from play mode | `LogBuffer` |
| **Project** (later) | scene files, scripts, assets in the project folder | `file_browser` |

Layout is a nexa `dockspace` — dockable, rearrangeable panels come free
with the toolkit. Data flows from the engine side: hierarchy from
`ctx.scene.names()` / `EntityView` walks, inspector from the component
read-model (§4.4), selection rays from `physics.cast_ray`.

Editor camera: a viewport-only fly/orbit camera (middle-drag orbit,
scroll zoom, F-focus on selection). It overrides the render camera in edit
mode; the game's own `Camera3D` entity takes over in play mode.

## 6. Play mode semantics (Unity's rules)

- **Edit mode:** simulation paused. Edits write through the buffered path
  and are *persistent* — they're authoring.
- **Press ▶:** Oberon serializes the scene to an in-memory RON snapshot,
  then steps *with* the game's update callback. Viewport switches to the
  game camera. Console starts streaming.
- **Press ■:** the in-memory snapshot is restored. **Play-mode changes are
  discarded** — this is Unity's single most important editor rule and we
  adopt it verbatim.
- **⏸ / ⏭:** pause keeps the world frozen but rendered; step advances one
  frame of game logic.

## 7. Editing interactions

- **Select:** click in viewport → ray from editor camera through the
  clicked pixel → `physics.cast_ray` → entity id → hierarchy + inspector
  focus. (Ray picking already exists in the physics API.)
- **Move:** drag on the viewport → ray-cast onto a drag plane → buffered
  transform write. Rotation/scale via inspector fields first; viewport
  gizmos (translate/rotate/scale handles) come after MVP.
- **Spawn:** add menu → entity archetype (empty, cube, light, camera) →
  appears at the drag-plane point in front of the camera.
- **Delete:** `EntityView.despawn()` (already buffered and safe).
- **Undo/redo:** command stack over inspector edits + spawn/delete
  (post-MVP; the buffered-write model makes commands trivially
  representable).

## 8. Code changes during play

**No hot reload — decided.** Oberon does not watch files. Code changes
apply the manual way: stop (■) and play (▶) again. Stopping restores the
pre-play snapshot (§6), so a stop/play cycle is a clean, complete reload
of both code and scene.

## 9. Project & repo model

A game project is a plain folder:

```
mygame/
├── main.py            # the Game subclass — entry point
├── scenes/
│   └── level1.ron     # authored in Oberon, diffed in git
└── assets/            # meshes/textures/audio (when those land)
```

Oberon opens a folder, lists its scenes, and knows how to run `main.py`
in play mode. No project file format beyond the folder — consistent with
the code-first philosophy.

**Repo home:** recommendation — separate repository (`x017/oberon`), same
relationship nexa has: Oberon depends on the published `automata_engine`
wheel. The engine repo gains only the §4 hooks, all of which are also
game-useful API. *(User's call — a `tools/oberon/` directory in this repo
is the alternative.)*

## 10. Milestones

| # | Deliverable | Where |
|---|---|---|
| M0 | RON scene save/load: serde on components, `Scene.save/load/to_ron/from_ron`, version field, tests, wiki chapter | engine |
| M1 | `game.step(dt)` + `game.render_frame(size)` (offscreen RGBA readback to Python) | engine |
| M2 | Component read-model + writable mesh color / light / collider params | engine |
| M3 | Oberon skeleton: nexa window, dark-white theme, live viewport, hierarchy, transform inspector | oberon |
| M4 | Click-select via ray, drag-move, spawn/delete | oberon |
| M5 | Play/pause/step with snapshot-restore, console panel | oberon |
| M6 | Save/load scenes from the UI; project folder model | oberon |
| later | Gizmos, undo/redo, multi-select, asset browser, prefabs from `Entity` subclasses, subprocess hardening | oberon |

M0–M2 are engine work on this repo (games benefit independently — save
files, photo mode, debug cameras). M3+ live in the Oberon app.

## 11. Risks & open questions

- **nexa image-widget throughput** for a per-frame ~3.5 MB RGBA buffer
  (1280×720×4). nexa's rasterizer is CPU-based, so the frame is a memcpy
  budget, not a texture upload: ~3.5 MB/frame at 60 Hz ≈ 210 MB/s, well
  inside modern memory bandwidth. M3 smoke-tests the real path before any
  other Oberon work; the fallback is the engine's existing windowed mode
  in a separate native window (zero readback, less integrated feel).
- **GIL contention** between nexa's loop and a stepped frame. MVP runs
  everything on the UI thread (a small scene steps in well under a frame
  budget); a worker-thread mode is the upgrade path.
- **Editor camera vs game camera** confusion. Rule: edit mode always uses
  the editor camera; play mode always starts from the game's camera
  entity. The viewport header shows which is active.
- **RON hand-editing errors.** Parse errors must report file/line
  precisely (ron does this); Oberon surfaces them in the console panel
  instead of failing to open the scene.

## 12. What Oberon deliberately won't do

- No code editing (VS Code exists).
- No in-game overlay mode.
- No visual scripting / node graphs (far future, separate decision).
- No asset *creation* (meshes/textures come from Blender etc.; Oberon
  arranges, it doesn't model).
