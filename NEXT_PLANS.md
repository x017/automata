# Next plans

Decisions and priorities agreed in planning conversations, in the order
we intend to tackle them. Each section records **what was decided**,
**why**, and **what's still open**, so work can resume without
re-discussing settled points.

Status: 🟢 decided · 🟡 under discussion · ⚪ not discussed yet

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

## 2. One scripting API instead of two 🟢 decided (not started)

Today Oberon's bridge (`crates/oberon/src/bridge.py`, ~860 lines, standard
library only) and `run_scene` (`automata/scripting.py`, ~570 lines) each
implement `entity` / `ctx` separately. They also **behave differently**:
- Oberon applies script writes after all scripts run; `run_scene` applies
  them immediately.
- Some helpers (`transform.translate`, `input.axis`) exist on one side only.

### Decisions

| Question | Decision |
|---|---|
| Approach | **One shared API layer + a shared test suite.** `entity` / `ctx` are written once, in a standard-library-only Python module, on top of a small backend interface. |
| Backends | **Bridge backend** (Oberon's JSON frames) and **engine backend** (the Rust extension under `run_scene`). Both are thin: read/write transform and attributes, ray cast, sounds, HUD, input, despawn. |
| When writes become visible | **Deferred (Oberon's model) everywhere.** Scripts read the start of the frame plus their own writes; all writes apply after every script has run. Results don't depend on script order. |
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

## 3. Text in the in-game HUD ⚪

`ctx.hud` draws images and rectangles but not text. The FPS shows a health
bar because it can't write "100%". Small effort, big payoff: scores,
timers, labels, menus.

## 4. Skeletal animation ⚪

glTF models already load, but skins and animations are ignored. This is
the biggest gap for real 3D games, since most downloaded characters are
animated.

## 5. Static mesh colliders ⚪

Only box, ball and capsule colliders exist. Uneven level geometry (such as
Kenney's platforms) has to be approximated with boxes. A triangle-mesh
collider for static scenery would fix that.

## 6. Performance baseline ⚪

The per-frame JSON bridge and per-entity Python calls have no measured
limit. Add a benchmark scene (100 / 1,000 / 5,000 scripted entities) for
Oberon Play, `run_scene` and exported builds, publish the numbers, and
track them in CI.

## Process note

CI was red for weeks before anyone noticed (GPU tests only run on CI's
software renderer). A failing PR check should block further work until it's
fixed.
