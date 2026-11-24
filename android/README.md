# AUTOMATA on Android

Work in progress: roadmap item E1. This folder builds an APK of a Python
game with [python-for-android] (p4a), using its SDL3 launcher. Today the APK
is a spike that proves the engine runs on Android:
- it draws through SDL's window,
- it takes touches,
- it survives being sent to the background.

`automata-export build --target android` comes later and will use this.

| Path | What it is |
| --- | --- |
| `engine/` | The `automata-android` crate: the engine's Python module (`automata._core`) with an SDL3 window driver |
| `recipes/automata/` | The p4a recipe that builds `engine/` with maturin |
| `pyproject.toml` | maturin settings for that build (placed at the root of a staged copy of the repo) |
| `build_apk.sh` | Stages the repo and runs `p4a apk` |
| `java/` | `AutomataActivity`, the launcher's activity with two small changes (below) |
| `app/main.py` | The spike game: a spinning cube, finger markers, Back to quit |
| `ci/smoke_test.sh` | The emulator test CI runs on the APK |
| `ci/report.py` | Posts a log or a screenshot as a check run on the commit |

## How it fits together

1. p4a's SDL3 launcher starts CPython on SDL's main thread and runs the
   game's `main.py`.
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

Scripts don't change. The same `main.py` runs on desktop with winit.

## Android and SDL details that matter

Things the emulator taught us, each handled in `java/` or
`engine/src/driver.rs`:
- **Re-creation.** Android can destroy and re-create the activity in the
  same process, for example on a configuration change the manifest doesn't
  list. By default SDL treats that as fatal and quits silently
  (`System.exit(0)`, logged as "activity finished"). `AutomataActivity`
  sets `SDL_ANDROID_ALLOW_RECREATE_ACTIVITY` before SDL's `onCreate`, and
  logs which configuration changes caused a re-creation.
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
android/build_apk.sh                 # the spike, x86_64 (emulators)
android/build_apk.sh mygame arm64-v8a  # a folder with main.py, for phones
adb install -r dist/*.apk
```

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
2. **APK:** builds the spike for x86_64.
3. **emulator smoke test:** runs the spike on an API 34 emulator and checks
   that:
   - it starts and draws (`1-running.png`);
   - a tap reaches the game as a touch;
   - Home pauses it (`2-background.png`);
   - returning resumes drawing on a new surface (`3-resumed.png`);
   - Back quits.

   The screenshots and the logcat are uploaded as the `android-smoke`
   artifact. They're also posted on the commit as check runs:
   - `android: logcat`;
   - `android: screenshot …`;
   - `android: stacks`: every thread's stack, saved when a wait times out;
   - `android: clippy log` and `android: APK build log`.

[python-for-android]: https://github.com/kivy/python-for-android
