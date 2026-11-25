# AUTOMATA on the web

Work in progress: roadmap item E3 (plan: `NEXT_PLANS.md` §7). This folder
builds the engine for the browser: CPython runs in [Pyodide], the engine is
a Pyodide extension module, and wgpu draws with WebGL2 (its OpenGL ES
backend under Emscripten). Python games run unchanged. Pyodide comes with
every export; nothing is loaded from a CDN.

Phase 1 (the spike) is in: the engine runs *stepped*, the wheel builds in
CI, and a spinning cube runs in headless Chromium. Input, audio, real games
and the exporter come in the next phases.

| Path | What it is |
| --- | --- |
| `engine/` | The `automata-web` crate: the engine's Python module (`automata._core`) with a step driver |
| `pyproject.toml` | maturin settings for that build (placed at the root of a staged copy of the repo) |
| `player/index.html`, `player/player.js` | The page: loads Pyodide, the engine wheel and the game, then runs a frame per animation frame |
| `spike/main.py` | The spike game: a spinning cube and a status line |
| `ci/build_wheel.sh` | Builds the wheel with pyodide-build (cross-build environment, Emscripten, Rust toolchain) |
| `ci/package.sh` | Puts the site together in `web/dist/site/`: Pyodide's runtime files, the page, the wheel, the game |
| `ci/smoke.mjs` | Loads the site in headless Chromium (Playwright), waits for frames, measures, takes screenshots |
| `ci/check_screenshot.py` | Fails unless the cube is on the screenshot |

## How it fits together

1. The page (`player.js`) loads Pyodide from `pyodide/` beside it, hands it
   the canvas (`pyodide.canvas.setCanvas3D`), installs the engine wheel
   (`pyodide.loadPackage`) and runs the game's script as `__main__`.
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
with two differences: there's no winit (no built-in window driver), and
gamepads come from the page (`navigator.getGamepads()` snapshots fed to
`automata_input::Gamepads::feed`) instead of gilrs, whose web backend needs
wasm-bindgen. Audio has no browser output yet (cpal has no Emscripten
host): phase 2 adds an AudioWorklet.

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
preset's soft sun shadows still `textureLoad` the shadow cascades, so
Beautiful won't build its pipelines on WebGL2 until that's reworked (phase
2); the default High and the other presets do.

## Building

Only CI builds it (`.github/workflows/web.yml`); locally you'd need the
same tools. With Python 3.14, `pip install pyodide-build==0.39.4`, rustup
and Node:

```sh
web/ci/build_wheel.sh            # web/dist/wheel/automata_engine-…-pyemscripten_…whl
web/ci/package.sh                # web/dist/site/ (or: package.sh path/to/game.py)
cd web/dist/site && python -m http.server 8000
```

Then open <http://localhost:8000/>. `build_wheel.sh` installs Pyodide's
cross-build environment, its Emscripten (5.0.3 for Pyodide 314) and the
Rust toolchain Pyodide's ABI names (`pyodide config get rust_toolchain`).

CI then serves the site, loads it in headless Chromium with WebGL2 on
SwiftShader, waits for 180 frames, and checks the screenshot. It posts
check runs: `web: smoke` (frames, frame rate, timings, download size per
file, the console) and `web: screenshot`.

[Pyodide]: https://pyodide.org
