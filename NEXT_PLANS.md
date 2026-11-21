# Next plans

Decisions and priorities agreed in planning conversations, in the order
we intend to tackle them. Each section records **what was decided**,
**why**, and **what's still open**, so work can resume without
re-discussing settled points.

Status: ✅ done · 🟢 decided · 🟡 under discussion · ⚪ not discussed yet

---

## 1. Android export 🟢 decided (not started)

### Decisions

| Question | Decision |
|---|---|
| How Python runs on the device | **Bundle real CPython** (officially supported on Android since 3.13, PEP 738). Scripts run unchanged, with the same behaviour as in Oberon and on desktop. |
| First goal | **Sideloading**: an APK you install on your own phone, debug-signed. Google Play (AAB, release signing, store requirements) comes later. |
| Touch input | **Both**: built-in on-screen controls (virtual joystick + buttons mapped to existing keys, so current games work) **and** a touch API (`ctx.input.touches`) for custom input. |
| Next platform after Android | **Web**. |

### Why CPython and not the alternatives

- **Translating Python to Rust** would only work for a subset of Python
  (`entity.state` holds anything, duck typing, the standard library). It
  would make scripts behave differently on the phone than in the editor,
  which undermines the "same scene and scripts everywhere, with tests that
  mean something" promise. It's months of work before a first game runs. It
  stays on the table as an **optional speed-up for hot scripts later**, not
  as the way onto Android.
- **A small interpreter** (RustPython, PocketPy) gives small apps and easy
  builds, but isn't fully compatible with desktop Python. That brings the
  same "works in Oberon, breaks on the phone" problem, in a smaller form.

### What changes architecturally

On desktop, **Python hosts the engine**: the player runs Python, which
imports the Rust extension. On Android the system starts a native
activity, so **Rust hosts Python**: the app initializes CPython in-process,
then runs `automata.player` against the bundled `game/` folder. The
Android activity handle must reach the engine's event loop before Python
calls `run_scene`.

Other Android work:
- **Lifecycle:** pause/resume, and the drawing surface destroyed and
  recreated at any time.
- **Rendering:** Vulkan / OpenGL ES through wgpu.
- **Audio:** cpal's Android backend.
- **Requirements:** 16 KB memory pages (needed for apps targeting
  Android 15+).

### Delivery model: a template, like Windows and Linux

1. CI builds a **prebuilt Android template APK** (engine + CPython),
   published with the other export templates.
2. `automata-export build --target android` (and Oberon's Build menu)
   copies the template, adds `game/`, and signs it. **Users need no Android
   SDK or NDK** (the same model Godot uses).

### Phases (each tested in CI on an Android emulator)

1. **Spike:** render a scene from Rust on Android, without Python
   (renderer, lifecycle, input).
2. **Embedded CPython:** run Coin Run on a device or emulator.
3. **Touch:** `ctx.input.touches`, the virtual joystick and buttons, and
   control settings in `build.ron`.
4. **Template + export:** `--target android` in `automata-export` and
   Oberon, with debug signing.
5. **Size and polish:** trim the standard library (target: well under
   +20 MB), check startup time.

### Open questions

- APK size budget, and which standard library modules to drop.
- Signing: bundle a tool or implement APK signing (v2) in Rust.
- How the FPS-style mouse look maps to touch (drag-to-look area?).

### Web, next: a note for later

Option A extends to the web: **Pyodide is CPython compiled to
WebAssembly**, so scripts would keep desktop behaviour there too. The
engine side needs wgpu's WebGPU/WebGL2 backends, a browser event loop
(no blocking main loop), and asset loading over HTTP. This needs its own
discussion before starting.

---

## 2. One scripting API instead of two ✅ done

**Done.** `automata/script_api.py` is the one implementation. Oberon's
`bridge.py` and `scripting.py` are now only its drivers, and
`tests/script_conformance/` runs through both runners in CI. Details are in
*Outcome* below.

Before, Oberon's bridge (`crates/oberon/src/bridge.py`, ~860 lines, standard
library only) and `run_scene` (`automata/scripting.py`, ~570 lines) each
implemented `entity` / `ctx` separately. They also **behaved differently**:
- Oberon applied script writes after all scripts ran; `run_scene` applied
  them immediately.
- Some helpers (`transform.translate`, `input.axis`) existed on one side only.

### Decisions

| Question | Decision |
|---|---|
| Approach | **One shared API layer + a shared test suite.** `entity` / `ctx` are written once, in a standard-library-only Python module, on top of a small backend interface. |
| Backends | **Bridge backend** (Oberon's JSON frames) and **engine backend** (the Rust extension under `run_scene`). Both are thin: read/write transform and attributes, ray cast, sounds, HUD, input, despawn. |
| When writes become visible | **Deferred (Oberon's model) everywhere.** Scripts read the start of the frame plus every write made earlier in it (one shared handle per entity); all writes reach the simulation after every script has run. |
| Play without the Automata package | **Keep zero-setup.** Oberon embeds the shared module, so Play works with any `python3`. No install step, and no version mismatch between Oberon and an installed package. |

### Plan

1. Move the user-facing API into a shared module (e.g.
   `automata/script_api.py`). Oberon `include_str!`s it next to
   `bridge.py` and writes both into `.oberon/`.
2. Reduce `bridge.py` and `scripting.py` to their backends plus
   loading, hot reload and error handling.
3. Switch `run_scene` to deferred writes, and document the frame model once
   for both (Scripting page).
4. **Conformance suite:** small script scenarios (move, ray cast, sound,
   despawn, HUD, write timing, attributes, hot reload) run through both
   runners in CI with the same expected results.
5. Keep the existing coin_run and FPS tests green throughout; they act as
   the end-to-end check.
6. Make the formerly "Oberon only" helpers available everywhere and remove
   those notes from the docs.

### Outcome

- **`crates/automata-python/python/automata/script_api.py`** (standard
  library only): `entity`, `ctx`, script loading, hot reload and error
  reports, built on a `Backend` interface. `ScriptHost.frame(records)` returns
  a `FrameOutput` (writes, sounds, HUD, cursor lock) for the driver to apply.
- **Oberon:** `bridge.rs` `include_str!`s the module and writes it as
  `.oberon/_automata_script_api.py` next to `bridge.py`. `bridge.py` (~300
  lines) keeps the JSON protocol, the Console redirect and the ray casts
  against collider shapes. Play still needs only a plain `python3`.
- **Engine:** `scripting.py` turns `EntityView`s into records and applies
  the output through the engine's buffered writes. `EntityView.attributes`
  gained a setter (a whole-map replacement). Engine-only members (`ctx.events`,
  `ctx.resources`, `ctx.quit()`, `ctx.raw`, the rest of `ctx.physics`) come
  through `Backend.extra`. In Oberon they raise an error that says they need
  `run_scene`.
- **Conformance:** seven scenarios (write timing, transforms and
  read-your-writes, attribute types, cameras, ray casts, scene lookups and
  despawn, input and ctx basics). They run in `oberon`'s
  `script_conformance_scenarios_match_expectations` (real Play loop and
  bridge) and in `tests/python/test_script_conformance.py` (real engine).
  Hot reload and error reports are unit-tested in `test_script_api.py`.
  Sounds and HUD stay covered by the end-to-end FPS tests.
- **Side effect:** `run_scene` no longer looks entities up by name once per
  scripted entity, which was O(n²). With 1000 scripted entities a headless
  frame went from 268 ms to 14 ms. The bridge went from 13.2 to 15.0 ms
  (JSON both ways included). Item #6 will measure properly.
- Entities spawned at runtime with a script now run it under `run_scene`
  too (handles are keyed by id, not name).

## 3. Game UI system 🟢 decided (phase 1 ✅ done)

**Decision: a full game-UI system**: text, widgets, custom fonts, layout and
focus, and a UI editor in Oberon. It's large, so it ships in phases; the
first phase is small and comes early (see *Order* below).

Building blocks already in place: `ab_glyph` (font rasterizing) is a
workspace dependency, Oberon bundles the Geist font, and `ctx.hud` already
draws images and rectangles in both Oberon (tiny-skia compositing) and the
engine (wgpu sprites).

### Phases

1. **Text** (quick win): `ctx.hud.text("100%", x, y, size=24,
   color=…, align="left" | "center" | "right")` with the built-in font,
   optional shadow/outline. Works in Oberon Play, `run_scene` and exports.
   The engine draws from a glyph atlas texture; Oberon draws through its
   canvas. The FPS gets its "100%" health label.

   ✅ **Done.** `automata_renderer::text` holds the built-in font (Geist
   Medium, moved out of Oberon's assets), the line layout, and a
   `TextAtlas`: glyphs are rasterized once into a 1024² texture and drawn
   as sprites. Oberon's Play HUD already went through the engine renderer,
   so it uses the same atlas instead of its tiny-skia canvas, and text is
   identical in Play, `run_scene` and exports. `ctx.hud.text` lives in the
   shared script API, so both runners validate the same way. `shadow` and
   `outline` take a colour or `True`. Multi-line text works. Under
   `run_scene`, `ctx.screen.measure_text` gives text's size. Oberon has no
   Python-side font metrics, so measuring there waits for phase 4
   (layout). The FPS shows its health percentage and an "ARENA CLEAR"
   banner.
2. **Widgets** (immediate mode, drawn from scripts each frame):
   `if ctx.ui.button("Play", x, y, w, h): …`, labels, progress bars,
   panels, images, toggles, sliders. Hover/pressed states. Clicks on UI are
   consumed, so pressing a button doesn't also fire the blaster. Mouse and
   touch (Android).
3. **Custom fonts:** `.ttf` / `.otf` from the project folder
   (`font="fonts/Pixel.ttf"`). They appear in the Assets tab, and export
   checks for missing fonts.
4. **Layout and focus:**
   - anchors (top-left, centre, bottom-right…) and sizes relative to the
     screen, so the UI fits any resolution, with UI scale for high-DPI and
     phones;
   - rows, columns and stacks;
   - keyboard / gamepad focus navigation;
   - screens (main menu, pause menu, HUD) that can be shown and hidden.
5. **UI editor in Oberon:** compose screens visually into UI documents
   (e.g. `hud.ui.ron`) on a canvas view with anchors, styles and a theme.
   Scripts bind to them by name:
   `ctx.ui.screen("hud").set("health", 50)`,
   `ctx.ui.screen("menu").clicked("play")`.

### Design notes and open questions

- **Immediate-mode API for scripts** (phases 1–4) fits the deferred frame
  model and both runner backends. **UI documents** (phase 5) are retained
  data that the same renderer draws.
- Theming: one default style matching AUTOMATA's look, overridable per
  document.
- Text features to settle: wrapping, rich text (colour spans), right-to-left
  text (probably out of scope at first).

---

## 4. Skeletal animation 🟢 decided (not started)

**Decision: everything**: clips, crossfades, Inspector preview, animation
events, root motion, and a visual animation state machine in Oberon.

### Phases

1. **Import and skinning:**
   - Read glTF skins (joints, inverse bind matrices) and animation channels
     (translation / rotation / scale; linear, step and cubic-spline
     interpolation).
   - GPU skinning in the renderer: four joint weights per vertex, a joint
     matrix buffer per skinned entity.
   - Shadows and the Oberon viewport use the posed mesh.
2. **Playback:**
   - An Animation component: default clip, speed, play on start.
   - Script API: `entity.animation.play("run", loop=True, fade=0.2,
     speed=1.0)`, `.stop()`, `.playing`, `.time`.
   - Crossfading between clips.
   - In the Inspector: the clip list and a preview scrubber, so animations
     play in the editor without pressing Play.
3. **Events and root motion:**
   - Per-clip events stored in the model's import settings
     (`.import.ron`), e.g. `("run", 0.3, "footstep")`, delivered to
     scripts as an `on_animation_event(entity, ctx, name)` callback.
   - Optional root motion: the root bone's movement drives the Character
     Controller instead of sliding the feet.
4. **State machine:**
   - An animation graph asset (e.g. `player.anim.ron`) holding:
     - states (clip, speed, loop) and 1D blend spaces (idle / walk / run
       by speed);
     - transitions with conditions on parameters (`speed > 0.1`, triggers
       like `jump`) and fade times.
   - Scripts only set parameters: `entity.animation.set("speed", v)`,
     `entity.animation.trigger("jump")`.
   - **Oberon graph editor:** a node canvas for states and transitions,
     with the active state highlighted live during Play.

### Notes and open questions

- Needs the shared scripting API (#2) first: the animation API goes into
  the shared layer once.
- Oberon's bridge protocol needs animation parameters and state in the
  per-frame messages.
- Example and test assets: a CC0 animated character (Kenney has some).
- Later: attachment points ("hold the blaster in the right hand bone"),
  2D blend spaces, additive layers, IK.

---

## 5. Mesh colliders 🟢 decided (not started)

**Decision: triangle meshes and convex hulls, chosen automatically.** Rapier
supports both natively.

- **Shapes:**
  - **Mesh** (triangle mesh): for scenery that never moves (no rigid body,
    or Fixed). Exact; ray casts hit the real triangles.
  - **Convex Hull:** works for moving objects (Dynamic / Kinematic).
- **Automatic choice on Add Collider / Fit Collider:**
  - model nearly fills its bounding box → **Box** (cheapest);
  - otherwise, static → **Mesh**;
  - otherwise, moving → **Convex Hull**.

  The Inspector shows the chosen shape, with a dropdown to override it.
- **Source data:** the scene stores a reference to the model, not the
  triangles. Colliders are built from the model after its import settings,
  and **the entity's scale is applied**. That also fixes "physics ignores
  scale" for these shapes.
- **Python:** `at.Collider3D.mesh("models/platform.glb")`,
  `at.Collider3D.convex_hull("models/crate.glb")`.
- **Validation:** a Mesh collider on a Dynamic body is an error in the
  Inspector and in export checks.
- **FPS example:** switch the platforms from approximate boxes to mesh
  colliders.

---

## 6. Performance baseline 🟢 decided (not started)

**Decision: run on demand and publish the numbers in the docs** (no CI
tracking or failures).

- `examples/bench/`: a generated scene with **100 / 1,000 / 5,000**
  scripted entities. Each script moves its entity, reads an attribute and
  casts a ray; some scenes have physics bodies.
- Measured in **Oberon Play** (headless harness), **`run_scene`** and an
  **exported build**: average and 95th-percentile frame time, split into
  script time and engine time.
- `python examples/bench/run.py` prints a Markdown table.
- **Docs:** a *Performance* guide with the table, the machine and date, and
  practical limits ("keep scripted entities under N for 60 FPS"). Rerun
  before releases.
- Suggestion: take one quick measurement **before** the shared-API
  refactor (#2), so we know the refactor didn't slow things down.

---

## Order

Agreed: **quick wins first.**

1. **Shared scripting API** (#2). Everything else touches the script API.
2. **HUD text** (#3, phase 1).
3. **Mesh colliders** (#5).
4. **Performance baseline** (#6).
5. **Skeletal animation** (#4, phases 1–4).
6. **Game UI**, the rest (#3, phases 2–5).
7. **Android** (#1), then the web.

Placing UI phases 2–5 after animation is a proposal, since you chose
the full UI system after picking this order. Swap them if menus and buttons
matter sooner.

## Process note

CI was red for weeks before anyone noticed (GPU tests only run on CI's
software renderer). A failing PR check should block further work until it's
fixed.
