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
│  Console              (Files sits under the Hierarchy)       │
└──────────────────────────────────────────────────────────────┘
```

## 1. What Oberon is — and is not

| Is | Is not |
|---|---|
| A Python desktop app built with **nexa-ui** | An in-game debug overlay |
| A scene authoring + inspection tool, and a Python IDE (§3.2.1) | A replacement for a general-purpose editor |
| A client of the **public engine API** | A fork or privileged build of the engine |
| The place you *arrange* a game | The place you *program* a game |

The engine stays code-first. Oberon produces and consumes **scene files**;
game logic remains Python in the user's editor of choice.

## 2. Brand

Oberon follows AUTOMATA's identity: **dark white sci-fi**.

- **AUTOMATA Black** (`crates/oberon/src/theme.rs`): neutral, untinted
  blacks — background 5, panels 12, raised controls 22, the 3D viewport
  pure black — with the logo's warm paper-white (242, 240, 235) as both
  text and accent. No blue tint anywhere; color is reserved for meaning
  (axis colors, warnings, errors, success, syntax).
- Thin 1 px borders, 5 px radii, generous spacing. Primary actions are
  filled paper-white with black text; everything else is outline or
  ghost.
- UX conveniences: toolbar tooltips (0.45 s), a segmented Move / Rotate /
  Scale control with key hints, a pill play-state badge, a **command
  palette** (Ctrl+Shift+P or the toolbar Search box — fuzzy search over
  every menu command, scene entities, project scripts and settings pages),
  **Settings** (Ctrl+, or the gear: Appearance / Script Editor / Python /
  Files; changes apply instantly and persist to `prefs.ron`), and
  interface zoom (Ctrl+= / Ctrl+- / Ctrl+0, 60–250 %).
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
- ~~An in-engine code editor with LSP is not worth building.~~ **Revised
  (2026-11):** Oberon is a full IDE. It gets a native Rust code editor
  (tabs, Python highlighting, find/replace) with an LSP client
  (basedpyright / pylsp / jedi-language-server over hand-written JSON-RPC)
  rather than a webview or a Code-OSS fork.

#### 3.2.1 The script editor *(shipped)*

Scripts open as document tabs next to the **Scene** tab over the center area
(double-click a `.py` in Project, *Edit Script* in the Inspector, or click a
`File "…", line N` traceback in the Console). While a script tab is visible
the 3D view isn't rendered.

- Model (`crates/oberon/src/code/`): `Buffer` (line vector, char columns,
  grouped undo/redo, Python-aware auto-indent / dedent / auto-close),
  an incremental line-state Python highlighter (`syntax.rs`; triple-quoted
  strings span lines), and the system clipboard (`clipboard.rs`: X11
  `CLIPBOARD` via x11rb, `wl-copy`/`wl-paste` on Wayland, in-process
  fallback).
- View (`ui/code_view.rs`): gutter with line numbers and diagnostic marks,
  current-line, selection, word-occurrence and bracket-match highlights,
  diagnostic squiggles with hover tooltips, find/replace and go-to-line
  bars, completion popup, a status line, and a Save / Don't Save / Cancel
  bar when closing a modified tab. Mouse: click, drag, Shift-click,
  double-click word, triple-click / gutter line, Ctrl-click definition.
- Keys: Ctrl+S / Ctrl+Shift+S (save / save all), Ctrl+W, Ctrl+Tab,
  Ctrl+Z/Y, Ctrl+X/C/V (whole line with no selection), Ctrl+D, Ctrl+/,
  Ctrl+Shift+K, Ctrl+L, Ctrl+] / Ctrl+[, Tab / Shift+Tab, Alt+Up/Down,
  Ctrl+Enter, Ctrl+Backspace/Delete, Ctrl+F / Ctrl+H / Ctrl+G, F3 /
  Shift+F3, Ctrl+Space (complete), F12 (definition), Ctrl+Shift+I (hover),
  Esc (back to the scene shortcuts). Keys the editor doesn't use (Ctrl+P,
  Ctrl+Q, F1) stay global.
- Saving is all the running game needs: the bridge's mtime hot reload
  (§8) picks the file up. Files changed on disk reload automatically when
  the tab is clean. Quitting with modified scripts asks first.
- Language features (`lsp.rs`): the first opened script starts a language
  server — `language_server` in `~/.config/oberon/prefs.ron` (a command
  line, `"auto"` or `"off"`), else the first installed of
  basedpyright-langserver, pyright-langserver, pylsp, jedi-language-server
  (searched in the project's `.venv`/`venv`, the configured Python's
  directory, `~/.local/bin`, then `PATH`). JSON-RPC over stdio with
  `Content-Length` framing, read on a background thread and polled once per
  frame; full-text document sync; char ↔ UTF-16 column conversion.
  Diagnostics (squiggles, gutter marks, tab and status counts), completion
  (on `.`, after two word chars, or Ctrl+Space), hover (resting the mouse
  on a word for 0.6 s, or Ctrl+Shift+I) and go-to-definition (F12 /
  Ctrl+click, across files). With no server installed, completion falls
  back to words from the open files plus Python keywords and builtins.
  `tests/lsp_e2e.rs` exercises a real server when one is installed.
- Script API completion (`code/api.rs`): behaviour scripts never import
  automata, so no language server can type `entity` / `ctx`. Oberon models
  the bridge API (`Context`, `Input`, `Time`, `Physics`, `Scene`, `Entity`,
  `Transform`, `Attributes`) itself: inside any `def f(entity, ctx)` hook it
  resolves chains such as `ctx.scene["Player"].transform.`, local
  assignments (`player = ctx.scene.find("Player")`) and `for e in
  ctx.scene:` loops, and completes members with signatures; key names in
  `ctx.input.pressed("…")` / `axis("…", "…")`; entity names in
  `ctx.scene["…"]` / `find("…")`; and attribute names in
  `entity.attributes["…"]` / `.get("…")`, taken from the open scene. Hover
  documents API members. These answers never go to the server; everything
  else does.
- Settings (Script Editor page): font size, highlight current line,
  indentation guides, show whitespace, auto-close brackets and quotes,
  suggest while typing, trim trailing whitespace on save, save scripts
  before Play.

## 4. Engine work required (the embedding hooks)

Oberon consumes only public Python API. Four engine additions unlock it,
in order:

> **Status for implementers:** M0 is shipped; M1–M2 are **contracts, not
> yet code**. Scaffold against them with mocks — `render_frame` can be a
> generated gradient `bytes` object, `step` a no-op — and the real engine
> drops in without Oberon changes.
>
> **Dev setup for the Oberon repo:** `pip install nexa-ui` (tested:
> 1.4.0), and the automata wheel built from this repo —
> `python scripts/build.py` then `pip install target/wheels/*.whl`. Pin
> both in `pyproject.toml`.

### 4.1 Scene serialization — RON *(decided, shipped)*

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
| **Files** | project folder as a tree under the Hierarchy (shipped; native, not nexa) | `file_browser` |

Layout is a nexa `dockspace` — dockable, rearrangeable panels come free
with the toolkit. Data flows from the engine side: hierarchy from
`ctx.scene.names()` / `EntityView` walks, inspector from the component
read-model (§4.4), selection rays from `physics.cast_ray`.

Editor camera: a viewport-only fly/orbit camera (middle-drag orbit,
scroll zoom, F-focus on selection). It overrides the render camera in edit
mode; the game's own `Camera3D` entity takes over in play mode.

### 5.1 As shipped (native Rust UI)

The panels above were the nexa-era plan. The shipped editor draws its own
UI (tiny-skia), and the right dock has two tabs:

- **Inspector**: components, attributes, Add Component.
- **Graphics**: a test bench for the frame look. It has:
  - **Renderer**: quality preset (an editor preference) and live
    ms / fps / mesh / shadow-map stats.
  - **Sun**: edits the first Directional Light via azimuth/elevation.
  - **Environment**: presets, hemisphere ambient, background, exposure,
    tone mapping, fog.
  - **Camera**: FOV, Look Through, Align to View, preview toggle.
  - **Test Scenes**: color chart, shadows, fog depth, 400-cube stress.
    Entities are named `Test …`, and adding or removing them is one undo
    step.

  Each group names the entity it edits and has a Select shortcut. Nothing
  is hidden editor state.

**Scene cameras.**
- Every `Camera3D` draws a frustum in the viewport.
- The selected camera gets a live preview inset (an offscreen render
  through the scene camera, bottom-right, 16:9).
- New cameras spawn at the current editor view.
- Moving a camera entity moves its eye and target together.
- **Play renders through the first scene camera.** Player input becomes
  relative to it, and scripts drive it with `entity.camera` /
  `ctx.camera` (`follow`, `look_at`). Without a camera, Play falls back
  to the editor view following the player.

User docs: `docs/wiki/oberon/`.

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

**Revised (2026-11): scripts hot-reload through the Python bridge.**

Behaviour scripts (`start/update/stop(entity, ctx)` modules attached in
the Inspector) never run inside the editor process. On Play, Oberon starts
the configured interpreter (`prefs.ron` → `python`, default `python3`) on
`bridge.py`, a pure-stdlib runner compiled into the editor and written to
`<project>/.oberon/` together with `_automata_script_api.py`. That second
file is `automata/script_api.py`, the one implementation of `entity`/`ctx`,
which `run_scene` also uses. `bridge.py` is only the driver: the protocol,
the Console redirect and ray casts against the sent collider shapes.
`tests/script_conformance/` runs the same scenes through both runners. The
two sides exchange JSON lines:

| Direction | Message | Contents |
|---|---|---|
| → runner | `frame` | dt, time, held keys, every entity's transform / character velocity / grounded / attributes / script |
| → runner | `stop` | call `stop` hooks and exit |
| ← runner | `ready` | Python version |
| ← runner | `log` | `print` output (Console) |
| ← runner | `error` | summary, script file + line, user-frame traceback |
| ← runner | `reloaded` | a script file changed and was re-imported |
| ← runner | `frame` | per-entity writes: transform, velocity, attributes |

Frames are pipelined, not awaited: the next frame is sent only after the
previous reply, so a slow script lowers the script tick rate instead of
freezing the editor (a 2 s stall is reported). The runner checks script
mtimes every ~0.4 s and re-imports changed files, keeping each entity's
`state`; a failing script reports once and pauses until its file changes.
Stop still restores the pre-play snapshot (§6). `automata.run_scene()`
runs the same scripts in the engine.

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
| M0 ✅ | RON scene save/load: serde on components, `Scene.save/load/to_ron/from_ron`, `Game.save_scene/scene_to_ron` (live world), version field, tests, [wiki chapter](../wiki/guides/scene-files.md) | engine |
| M1 | `game.step(dt)` + `game.render_frame(size)` (offscreen RGBA readback to Python) | engine |
| M2 | Component read-model + writable mesh color / light / collider params | engine |
| M3 | Oberon skeleton: nexa window, dark-white theme, live viewport, hierarchy, transform inspector. (Viewport path already proven: ~970 fps headless at 720p.) Bonus: `render_to_png` gives Oberon deterministic CI tests from day one. | oberon |
| M4 | Click-select via ray, drag-move, spawn/delete | oberon |
| M5 | Play/pause/step with snapshot-restore, console panel | oberon |
| M6 | Save/load scenes from the UI; project folder model | oberon |
| later | Gizmos, undo/redo, multi-select, asset browser, prefabs from `Entity` subclasses, subprocess hardening | oberon |

M0–M2 are engine work on this repo (games benefit independently — save
files, photo mode, debug cameras). M3+ live in the Oberon app.

## 11. Risks & open questions

- ~~nexa image-widget throughput~~ — **measured and resolved
  (2026-11-19).** `image_create(w, h, rgba_bytes)` takes automata's exact
  readback format; `SoftwareRenderer(threads=4)` + manual `ctx.frame()`
  ran **~970 fps** at 1280×720 with the worst-case path (destroy+create
  the full texture every frame), pixel-verified via `render_to_png`.
  Gotcha recorded for M3: `ui.run(headless=True)` *paces* frames (idle
  semantics) — throughput benchmarks must drive `ctx.frame()` manually.
  Escape hatch, if ever needed: nexa's `CustomRenderer`/`DrawData` lets an
  external wgpu renderer composite the UI (zero-copy direction). The
  separate-window fallback remains available but is not needed.
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

## 13. nexa integration notes (verified against nexa-ui 1.4.0)

Hard-won facts from the feasibility pass — read before writing M3:

- **Drive frames manually.** `ui.run(headless=True)` paces frames for idle
  CPU savings; a live viewport needs the manual loop:
  `ui.UI(width, height, renderer=...)`, `with ctx.frame(): ...` per tick.
  The Oberon tick order is: pump nexa input → `game.step(dt)` (edit mode:
  paused) → `game.render_frame(size)` → `ctx.image_destroy/create/image`.
- **Image path:** `ctx.image_create(w, h, rgba_bytes)` →
  `ctx.image(tex, width=, height=)`; destroy the old texture after
  creating the new one. ~970 fps at 720p headless — not a bottleneck.
- **Widget map:** layout = `dockspace`; hierarchy = `tree_table`;
  inspector = `inspector` + `inspector_group`; console = `LogBuffer`;
  project panel = `file_browser`; gizmo/overlay hit areas = `ctx.canvas`
  with its `paint(ui, rect)` callback.
- **Theming (dark white sci-fi):** `ThemeMode.DARK`, near-black surfaces,
  off-white foreground (logo palette: paper white on true black), one
  `Accent` used sparingly; OKLCH token overrides for the exact grays.
- **Testing:** headless `SoftwareRenderer` + `ctx.render_to_png(path)`
  gives deterministic PNG snapshot tests in CI from day one.
- **Idle cost:** `ctx.request_redraw()` only when dirty — the editor
  sleeps at 0% CPU when nothing moves.
