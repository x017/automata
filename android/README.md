# AUTOMATA on Android

Work in progress: roadmap item E1. This folder builds an APK of a Python
game with [python-for-android] (p4a), using its SDL3 launcher. It builds
three kinds of APK:
- **the export template** (`build_apk.sh template`): Python, the engine
  and the player, with no game. `automata-cli build --target android`
  (and Oberon's Build for Android) adds a game to a copy of it, renames
  and signs it, with no Android tools (see `crates/automata-cli/src/android`).
  CI builds *Coin Run* this way and plays it on an emulator. Its scenes
  and scripts run unchanged, and its saves and settings go to the app's
  own storage;
- **an exported game packaged by p4a**, from the folder
  `automata-cli stage` writes (handy when changing the player itself);
- **a spike** (`app/`) that tests the platform layer: drawing through SDL's
  window, touches, going to the background and coming back.

Coin Run plays with on-screen controls: a stick to walk, a stick to turn
the camera, and a Jump button (`ctx.input.touch_controls`, which shows
them on touch screens; see the wiki's Touch controls guide).

| Path | What it is |
| --- | --- |
| `engine/` | The `automata-android` crate: the engine's Python module (`automata._core`) with an SDL3 window driver |
| `recipes/automata/` | The p4a recipe that builds `engine/` with maturin |
| `pyproject.toml` | maturin settings for that build (placed at the root of a staged copy of the repo) |
| `build_apk.sh` | Stages the repo and runs `p4a apk` |
| `java/` | `AutomataActivity`, the launcher's activity with two small changes (below) |
| `player/main.py` | The entry point for an exported game: unpacks the game built into the APK (or uses a bundled `game/`) and runs it with `automata.player` |
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
   A game built from the template lives in the APK under
   `assets/automata/game/`. `main.py` unpacks it into `game/` the first
   time, and again when `assets/automata/stamp` (a hash of the files)
   changes. The activity passes the APK's path as `AUTOMATA_APK`.
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
- **Re-creation while unpacking.** The first launch unpacks Python for a
  few seconds before SDL's library loads. A re-creation in that window
  leaves two unpack tasks, and both end by calling `finishLoad` on the
  newest activity, so SDL saw a "second" activity before anything could
  allow one, and quit. `finishLoad` now loads the libraries and allows
  re-creation itself before SDL's checks, and runs once per activity.
- **Re-creation while the game runs.** Allowed re-creation means SDL
  quits the game's `main()` and runs it again on the new activity, but
  python-for-android's `main()` runs Python once and then ends the process.
  Bold text, resource overlays (a new system theme after a wallpaper
  change, or one applied just after boot) and the like would close the
  game. So `AutomataActivity` keeps the game running instead: the old
  activity lets go without telling SDL to quit, and the new one adopts
  SDL's running thread, which sees a background/foreground and a new
  surface.
- **Lost lifecycle events.** On a re-creation SDL makes a new lifecycle
  semaphore while its thread may still wait on the old one, so the
  background and foreground events can come late, after the window's
  queued events, or not at all. The old surface was resized after Android
  had destroyed it, and wgpu's error was fatal. The driver now acts on the
  lifecycle watch before every event, and follows the window's
  `ANativeWindow`: a new one replaces the engine's surface, and a new one
  (or the focus) while away means the app is back. The renderer drops a
  surface that reports no support instead of configuring it.
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
# The export template (phones and emulators in one APK):
android/build_apk.sh template arm64-v8a,x86_64
#   → dist/automata-template-android.apk; then, without Android tools:
automata-cli templates install dist/automata-template-android.apk --target android
automata-cli build my_game --target android

# An exported game packaged by p4a: stage what a build ships, then package it.
automata-cli init my_game --scene level.ron   # once: writes build.ron
automata-cli stage my_game --out /tmp/my_game_staged
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

The `Android` workflow runs four jobs:
1. **check:** `engine/` compiles for `aarch64-linux-android` and passes
   clippy with no warnings.
2. **APKs:** builds the spike for x86_64 and the template for
   `arm64-v8a` and `x86_64`. Then it builds *Coin Run* from the template
   the way users do: a copy gets a `build.ron` from `automata-cli init
   --package org.automata.coinrun`, and `automata-cli build --target
   android` signs it with a fresh debug key.
3. **emulator smoke test:** runs both on an API 34 emulator. For the spike
   it checks that:
   - it starts and draws (`1-running.png`);
   - a tap reaches the game as a touch;
   - Home pauses it (`2-background.png`);
   - returning resumes drawing on a new surface (`3-resumed.png`);
   - turning bold text on and off re-creates the activity twice, and the
     game keeps running in the same process and drawing
     (`4-recreated.png`);
   - Back quits.

   For Coin Run it checks that:
   - it installs (the v2 signature holds) as `org.automata.coinrun` 0.1.0,
     and the first start unpacks the game from the APK (the second doesn't);
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
4. **publish:** once the emulator test passes, uploads
   `automata-template-android.apk` to the `export-templates` release,
   where `automata-cli templates download --target android` gets it.

[python-for-android]: https://github.com/kivy/python-for-android
