# AUTOMATA on Android

Work in progress: roadmap item E1. This folder builds an APK of a Python
game with [python-for-android] (p4a), using its SDL3 launcher. It builds
two kinds of APK:
- **an exported game**, as `automata-export stage` writes it. CI builds
  *Coin Run* this way and plays it on an emulator. Its scenes and scripts
  run unchanged, and its saves and settings go to the app's own storage;
- **a spike** (`app/`) that tests the platform layer: drawing through SDL's
  window, touches, going to the background and coming back.

Coin Run plays with on-screen controls: a stick to walk, a stick to turn
the camera, and a Jump button (`ctx.input.touch_controls`, which shows
them on touch screens; see the wiki's Touch controls guide). Next is
`automata-export build --target android`, which will use this folder.

| Path | What it is |
| --- | --- |
| `engine/` | The `automata-android` crate: the engine's Python module (`automata._core`) with an SDL3 window driver |
| `recipes/automata/` | The p4a recipe that builds `engine/` with maturin |
| `pyproject.toml` | maturin settings for that build (placed at the root of a staged copy of the repo) |
| `build_apk.sh` | Stages the repo and runs `p4a apk` |
| `java/` | `AutomataActivity`, the launcher's activity with two small changes (below) |
| `player/main.py` | The entry point for an exported game: runs the staged folder with `automata.player` |
| `app/main.py` | The spike game: a spinning cube, finger markers, Back to quit |
| `ci/smoke_test.sh` | The emulator test for the spike |
| `ci/coin_run_test.sh` | The emulator test for Coin Run |
| `ci/lib.sh` | What both tests share: waiting on logcat, crash checks, screenshots |
| `ci/report.py` | Posts a log or a screenshot as a check run on the commit |

## How it fits together

1. p4a's SDL3 launcher starts CPython on SDL's main thread and runs the
   app's `main.py`. For an exported game that's `player/main.py`, which runs
   the game folder (`game/` beside it) through `automata.player`, just as
   the desktop launcher does. `game.json` names the start scene or script.
2. `import automata` loads `engine/`, which does three things:
   - sends the engine's logs to logcat (tag `SDL/APP`);
   - installs its SDL driver with `automata_core::platform::install_driver`;
   - fills the module just as the desktop extension does.
3. `game.run()` opens a full-screen SDL window instead of a winit one. The
   driver gives the engine the window's `ANativeWindow`, and wgpu renders to
   it with Vulkan or OpenGL ES.
4. SDL events become the engine's own input events (see
   `automata_input::InputEvent`):
   - fingers become `ctx.input.touches`;
   - Android's Back button becomes the `back_button` key;
   - keyboards, mice and focus work as on desktop.
5. Going to the background lets go of the surface and pauses frames.
   Coming back attaches the new surface. GPU resources (meshes, textures,
   pipelines) are kept, and the time spent away doesn't count as a frame.

Scripts don't change. The same game runs on desktop with winit.

Saves and settings (`ctx.save`, `ctx.settings`) go to the app's private
storage. `automata.user_data_dir` uses p4a's `ANDROID_PRIVATE` there. They
are written in the frame they change, because Android can kill an app in
the background without warning.

## Android and SDL details that matter

Things the emulator taught us, each handled in `java/` or
`engine/src/driver.rs`:
- **Re-creation.** Android can destroy and re-create the activity in the
  same process, for example on a configuration change the manifest doesn't
  list. By default SDL treats that as fatal and quits silently
  (`System.exit(0)`, logged as "activity finished"). It happens even at
  launch: the activity can be re-created while it turns to landscape,
  before SDL has started. `AutomataActivity` allows re-creation with
  `nativeSetenv` as soon as SDL's library is loaded; an environment
  variable alone would only count once SDL had read it. It also logs which
  configuration changes caused a re-creation.
- **Orientation.** The driver holds the screen to the shape of the game's
  window (landscape for 1280×720) with `SDL_ORIENTATIONS`, unless the app
  sets that hint itself.
- **Lifecycle events.** Once SDL has queued the background events, it
  normally blocks every event pump until the app returns, so the queued
  copies arrive late. SDL's docs say to handle them in an event watch. The
  driver does that:
  - a watch notes background/foreground the moment SDL raises them, and the
    loop acts on that note;
  - `SDL_ANDROID_BLOCK_ON_PAUSE=0` is set;
  - while frames are paused, the loop sleeps in 250 ms slices.
- **Focus.** Losing window focus pauses frames too. Android takes the focus
  first when the app is leaving, and also when the notification shade comes
  down.
- **Gamepads.** gilrs has no Android backend yet, so the engine uses gilrs'
  dummy context: no gamepads, no crash. SDL's gamepad support is the
  likely way to add them later.

`engine/` sits outside the main Cargo workspace on purpose. It needs the
`sdl3` crate, which links the launcher's `libSDL3.so`. The workspace, its
offline builds and its lockfile never see that crate. CI resolves
`engine/`'s dependencies from crates.io.

## Building an APK

You need:
- python-for-android `develop` (CI pins the commit in
  `.github/workflows/android.yml`);
- `rustup`;
- JDK 17;
- an Android SDK (`ANDROID_HOME`) with platform 35;
- NDK r28c (`ANDROID_NDK_HOME`).

```sh
# An exported game: stage what a build ships, then package it.
automata-export init my_game --scene level.ron   # once: writes build.ron
automata-export stage my_game --out /tmp/my_game_staged
AUTOMATA_PACKAGE=org.example.mygame AUTOMATA_NAME="My Game" \
  android/build_apk.sh /tmp/my_game_staged arm64-v8a   # for phones

android/build_apk.sh                 # the spike, x86_64 (emulators)
adb install -r dist/org.automata.spike-x86_64.apk
```

`build_apk.sh` takes a staged game (a folder with `game.json`) or any
folder with a `main.py`, which runs as is. The APK lands in `dist/`, named
after its package. Set `AUTOMATA_VERSION` to give it a version. Scripts
ship as source (`--no-byte-compile-python`), because the engine loads
behaviour scripts from their `.py` files.

The first build compiles CPython, SDL3 and the engine, which takes a while.
Later builds reuse p4a's cache.

The engine's log level is `info` by default. Set `AUTOMATA_LOG` (an
`EnvFilter` string) in the environment to change it.

`WGPU_BACKEND=gl` forces OpenGL ES, which helps on emulators with
incomplete Vulkan.

## CI

The `Android` workflow runs three jobs:
1. **check:** `engine/` compiles for `aarch64-linux-android` and passes
   clippy with no warnings.
2. **APK:** builds the spike for x86_64, then *Coin Run*: a copy gets a
   `build.ron` from `automata-export init`, `automata-export stage` copies
   what a build ships, and `build_apk.sh` packages it. The second build
   reuses the first one's Python and engine.
3. **emulator smoke test:** runs both on an API 34 emulator. For the spike
   it checks that:
   - it starts and draws (`1-running.png`);
   - a tap reaches the game as a touch;
   - Home pauses it (`2-background.png`);
   - returning resumes drawing on a new surface (`3-resumed.png`);
   - Back quits.

   For Coin Run it checks that:
   - the level loads and its scripts run (`coin-1-start.png`);
   - keys reach the scripts: F2 changes the graphics preset
     (`coin-2-playing.png`);
   - the setting lands in `settings.json` in the app's storage;
   - after Android kills the app, the next start remembers it;
   - the touch controls show by themselves;
   - holding the left stick forward walks the player into the coin ahead
     (`coin-3-stick.png`; `Score: 1` in the log);
   - the Jump button lights up while it's held (`coin-4-jump.png`,
     measured in the screenshot).

   The touches are injected with `adb shell input motionevent`, placed
   where `touch_controls` lays the controls out on the game's surface.

   The screenshots and the logcat are uploaded as the `android-smoke`
   artifact. They're also posted on the commit as check runs:
   - `android: logcat` and `android: coin run logcat`;
   - `android: screenshot …`;
   - `android: stacks`: every thread's stack, saved when a wait times out;
   - `android: clippy log` and `android: APK build log`.

[python-for-android]: https://github.com/kivy/python-for-android
