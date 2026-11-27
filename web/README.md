# AUTOMATA on the web

Work in progress: roadmap item E3 (plan: `NEXT_PLANS.md` §7). This folder
builds the engine for the browser: CPython runs in [Pyodide], the engine is
a Pyodide extension module, and wgpu draws with WebGL2 (its OpenGL ES
backend under Emscripten). Python games run unchanged. Pyodide comes with
every export; nothing is loaded from a CDN.

Phase 1 (the spike) is in: the engine runs *stepped*, the wheel builds in
CI, and a spinning cube runs in headless Chromium. Phase 2 (input and
lifecycle) is in: keys, mouse and pointer lock, the wheel, touch, gamepads,
resizing, hidden tabs, full screen, and sound through an AudioWorklet, all
driven in CI with fake input. Phase 3 (real games) is in: the player runs
an exported game from `game.zip`, keeps saves and settings in IndexedDB,
and CI plays Coin Run (`game.zip` at the top of the repo). The exporter
(`automata-cli build --target web`) comes next.

| Path | What it is |
| --- | --- |
| `engine/` | The `automata-web` crate: the engine's Python module (`automata._core`) with a step driver |
| `pyproject.toml` | maturin settings for that build (placed at the root of a staged copy of the repo) |
| `player/index.html`, `player/player.js` | The page: loads Pyodide, the engine wheel and the game, runs a frame per animation frame, passes input on |
| `player/audio-worklet.js` | The sound output: plays what the engine mixed each frame |
| `spike/main.py` | The test game: a cube that answers keys, mouse, touch, gamepads, pointer lock, and beeps |
| `ci/build_wheel.sh` | Builds the wheel with pyodide-build (cross-build environment, Emscripten, Rust toolchain) |
| `ci/package.sh` | Puts the site together in `web/dist/site/` (and a zip of it): Pyodide's runtime files, the page, the wheel, the game (a script, a staged game folder or a `game.zip`) |
| `ci/smoke.mjs` | Loads the site in headless Chromium (Playwright), waits for frames, measures, takes screenshots |
| `ci/input_checks.mjs` | Drives the test game with fake keys, mouse, touches and a fake gamepad, and checks what it saw |
| `ci/coin_run.mjs` | Plays Coin Run from `game.zip`: keys, a coin, the setting and the save after a reload, touch controls |
| `ci/check_screenshot.py` | Fails unless the cube (or, with `--any`, some scene) is on the screenshot |

## How it fits together

1. The page (`player.js`) loads Pyodide from `pyodide/` beside it, hands it
   the canvas (`pyodide.canvas.setCanvas3D`), installs the engine wheel
   (`pyodide.loadPackage`), mounts the save folder (below) and starts the
   game that `player.json` names: `"game": "game.zip"`, an exported game
   folder unpacked to `/game` and run with `automata.player` as the
   desktop and Android players do, or `"main": "main.py"`, one script run
   as `__main__` (the test game).
2. `import automata` loads `engine/`, which installs its step driver with
   `automata_python::install_step_driver` and fills the module just as the
   desktop extension does (`automata._core.platform` is `"web"`).
3. A browser page can't sit in a loop, so `game.run()` doesn't: with a step
   driver installed, it starts a *stepped run*
   (`automata_core::Application::start_stepped`) and returns 0 at once.
   `Game.stepping` is then true; `Game.on_finish(callback)` runs cleanup
   when the game ends (`automata.run_scene` stops scene scripts that way).
4. The page calls `automata._core.web_frame(width, height)` from each
   `requestAnimationFrame`, with the canvas's drawing size. The driver
   attaches the canvas on the first frame (Emscripten's EGL draws to
   Pyodide's canvas), resizes when the size changes, feeds queued input,
   then runs and draws the frame (`Application::step`, the same `Host`
   operations a desktop or Android driver uses). It returns `False` once
   the game has quit; an exception from the game is raised there.

The engine's own crates build for `wasm32-unknown-emscripten` as they are,
with three differences: there's no winit (no built-in window driver),
gamepads come from the page (`navigator.getGamepads()` snapshots fed to
`automata_input::Gamepads::feed`) instead of gilrs, whose web backend needs
wasm-bindgen, and sound goes out through the page (below) instead of cpal,
which has no Emscripten host.

## Input and the page's life

`player.js` listens to the page and hands everything to the engine
between frames through `automata._core.web_*` functions, which queue
the same `InputEvent`s the desktop and Android drivers make:

| The page sees | The engine gets |
| --- | --- |
| `keydown` / `keyup` | `web_key(code, down)`: keys by `KeyboardEvent.code` (`"KeyW"`: the key in the W place on any layout; `KeyCode::from_code`), key repeat as presses. Keys the engine knows don't scroll the page; Ctrl/Cmd shortcuts and F-keys stay the browser's |
| Shift, Ctrl, Alt, Meta | `web_modifiers(bits)` when they change |
| `mousedown` / `mouseup`, `mousemove`, `wheel` | `web_mouse_button`, `web_cursor` (canvas pixels, so high-DPI screens match the drawing size), `web_mouse_motion` (`movementX/Y`, which keeps coming while the pointer is locked), `web_scroll` (about one per wheel notch, +y away from you) |
| Pointer events from fingers (and pens) | `web_touch(id, phase, x, y)`; the first finger also acts as the left button, as on Android |
| `navigator.getGamepads()`, polled every frame | `web_gamepads([...])`: standard mapping, Y axes flipped to up-is-positive |
| `blur`, the tab hidden | `web_focus_lost()`: everything held lets go; sound pauses |
| The tab shown again | `web_resume()`: the time away isn't counted as a frame (and no frame ever counts for more than 0.25 s, on any platform) |
| The canvas's size × `devicePixelRatio` | `web_frame(width, height)` resizes the surface |

**Pointer lock.** After each frame the page reads `web_cursor_locked()`
(the game's `ctx.screen.lock_cursor()`). Browsers grant a lock only right
after the player pressed something, so the page asks then (with raw mouse
movement where the browser has it) and lets go as soon as the game does.
Escape is the browser's way out of a lock; the next click locks again
while the game wants it.

**Full screen.** Also only from a click: the page has a small button in
the corner (F11 works too). Games just see the new size.

## Games, saves and settings

A game for the web is the folder every export ships (scenes, scripts,
assets and `game.json`), zipped: `automata-cli stage PROJECT --zip
game.zip`. The page fetches it, unpacks it into Pyodide's file system
(`pyodide.unpackArchive`), sets the page title from `game.json` and calls
`automata.player.main(["/game"])`, so a scene game starts its
`start_scene` with its scripts and a code-first game runs its `main`
script, exactly as exported desktop builds do.

Saves and settings (`ctx.save`, `ctx.settings`) go where
`automata.script_api.user_data_dir` says, which in the browser is
`/data/<game name>` (`WEB_DATA_DIR`). The page mounts Emscripten's IDBFS
there before the game starts, so the files live in the site's IndexedDB:
it loads them first (`FS.syncfs(true)`), and copies changes back after
each write. The game's file store tells the page through
`automata.script_api.store_hooks` (called with the path after every write
or removal); the page syncs 50 ms later, coalescing writes made
meanwhile, and again when the page is hidden or closed. Without IndexedDB
(some private windows) saves last until the page closes and
`window.automata.storage.error` says why.

Touch controls show by themselves on a touch screen: the page sets
`AUTOMATA_TOUCH_CONTROLS=1` when the main pointer is coarse (phones and
tablets), or as `?touch=1` / `?touch=0` in the URL says.

`game.zip` at the top of the repo is Coin Run exported this way
(`scripts/game_zip.sh` rebuilds it; CI fails when it no longer matches
`examples/oberon/coin_run`), next to `game.apk`, the same game for
Android as built and emulator-tested by the Android workflow.

## Sound

kira mixes as on desktop, but into the `stream` backend
(`automata_audio::stream`): each frame the engine renders that frame's
worth of sound (48 kHz stereo) where the game runs, and the page takes it
with `web_audio()` and posts it to an AudioWorklet (`audio-worklet.js`).
The worklet plays it after a short buffer (50 ms, or one and a half
frames' worth when frames are slower), fills gaps with silence, and
keeps the delay under 250 ms by dropping the oldest sound. Rendering
follows frame time, so sound positions and states behave as on desktop,
and while the tab is hidden (no frames) sound waits. Browsers start sound
only after the first click, key or tap; the page resumes it then.

Desktop tests can run games stepped too: `automata._core._use_stepped_runs(True)`
installs a windowless step driver, and `automata._core.step_running_game()`
runs a frame (see `crates/automata-python/tests/python/test_stepped.py`).

## First measurements

From the spike in CI (Pyodide 314.0.8, headless Chromium, WebGL2 on
SwiftShader, a 960×540 canvas):

| | |
| --- | --- |
| Download | 17.12 MB in 11 files, 9.97 MB gzipped |
| `pyodide.asm.wasm` (CPython) | 9.60 MB, 3.59 MB gzipped |
| The engine wheel | 3.58 MB (already compressed) |
| `python_stdlib.zip` | 2.55 MB |
| Pyodide ready / engine imported / game started / first frame | 1.6 s / 1.9 s / 2.1 s / 2.7 s after navigation |
| Frame rate | ~9.5 fps (a CPU rasterizer; real GPUs run far faster) |

Phase 5 sets the budget; trimming the standard library and the wheel are
the obvious levers.

## WebGL2 notes

The renderer works within WebGL2's limits: the surface allows only
`COLOR_TARGET` (no `COPY_DST`), passes that only `textureLoad` scene depth
bind it as unfilterable float (GLSL can't load from depth textures), and
the point shadow array gets a spare layer on wasm so wgpu's GL backend
doesn't make it a cube-map array, which WebGL2 lacks. The Beautiful
preset's soft sun shadows find their blockers with depth compares only
(GLSL can't read depth texels either), so every preset builds on WebGL2.
`every_shader_translates_for_webgl2` (in `automata-renderer`) translates
every entry point of every shader to GLSL ES 3.00 as wgpu does in the
browser, so desktop CI catches what WebGL2 can't do, and the smoke test
switches the test game to Beautiful and back (`beautiful.png` in the
artifact).

## Building

Only CI builds it (`.github/workflows/web.yml`); locally you'd need the
same tools. With Python 3.14, `pip install pyodide-build==0.39.4`, rustup
and Node:

```sh
web/ci/build_wheel.sh            # web/dist/wheel/automata_engine-…-pyemscripten_…whl
web/ci/package.sh                # web/dist/site/ with the test game
web/ci/package.sh game.zip       # …or Coin Run (or any staged folder or script)
cd web/dist/site && python -m http.server 8000
```

Then open <http://localhost:8000/>. `build_wheel.sh` installs Pyodide's
cross-build environment, its Emscripten (5.0.3 for Pyodide 314) and the
Rust toolchain Pyodide's ABI names (`pyodide config get rust_toolchain`).

CI then serves the site, loads it in headless Chromium with WebGL2 on
SwiftShader, waits for 180 frames, drives the test game with fake input
(`input_checks.mjs`: keys and Shift, letting go on blur, the cursor in
canvas pixels, a click, the wheel, a tap, a fake standard gamepad plugged
in, pushed, pressed and pulled out, a beep reaching the page and the
worklet, pointer lock asked for and given back, a hidden tab, the
Beautiful preset, a resize),
and checks the screenshot. It posts check runs: `web: smoke` (frames,
frame rate, timings, every input check, download size per file, the
console) and `web: screenshot`. `INPUT=0` skips the input checks.

Then it packages Coin Run from `game.zip` and plays it (`coin_run.mjs`,
the browser twin of the Android emulator test): it must start with
keyboard controls, F2 must change the graphics preset, the setting must
reach IndexedDB, walking forward should pick up a coin (soft: SwiftShader
is slow), and after a reload F2 must go on from the saved preset (and the
best run be remembered). With `?touch=1` the touch controls must show. It
posts `web: coin run` and `web: coin run screenshot`.

[Pyodide]: https://pyodide.org
