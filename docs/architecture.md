# Architecture

This document describes the engine's architecture as it is actually
implemented. The target design is specified in `build.md` section 3; this
file tracks what exists and why.

## Layers

```text
┌─────────────────────────────────────┐
│ Python gameplay code (Phase 5)      │
├─────────────────────────────────────┤
│ automata-python (PyO3 bindings)     │
├─────────────────────────────────────┤
│ renderer · scene · input · physics  │
│ audio · assets (Phases 2–7)         │
├─────────────────────────────────────┤
│ automata-core (this phase)          │
└─────────────────────────────────────┘
```

Each layer may only depend on layers below it. Python never sees Rust
ownership or lifetime details; the binding layer translates between the two
worlds.

## Crates

| Crate | Status | Responsibility |
|---|---|---|
| `automata-core` | ✅ Phase 1–2 | Application lifecycle, main loop, timekeeping, logging, error types, windowed/headless runtime |
| `automata-input` | ✅ Phase 2 | Input event collection, keyboard/mouse state |
| `automata-renderer` | ✅ Phase 2 (context) | wgpu device, surface, frame presentation; drawing in Phase 3 |
| `automata-scene` | ✅ Phase 4 | Scenes, entities, ECS, transforms |
| `automata-physics` | Phase 6 | Rapier 2D/3D integration |
| `automata-nav` | ✅ C4 | Navigation meshes baked from static colliders, path finding, Nav Agents (steering, avoidance, jump links) |
| `automata-audio` | Phase 7 | Kira-based audio |
| `automata-assets` | ✅ Models | glTF 2.0 / OBJ model loading (meshes, base colour, textures), a hot-reloading cache, asset-root path resolution |
| `automata-python` | ✅ Phase 5 | PyO3 bindings and Python API |

New crates are introduced only when a subsystem boundary becomes meaningful —
not ahead of time.

## Application lifecycle (Phase 1–2)

```text
Created ──run()──▶ Running ──quit requested / window closed──▶ Shutdown
```

- **Created** — `Application::new(EngineConfig)` initializes engine-wide
  services (logging) and returns. No window or GPU device exists yet.
- **Running** — `Application::run(update)` invokes `update` once per frame
  with an `AppContext` (timing, input, quit control). Two drivers implement
  the same contract:
  - `WindowMode::Windowed` — the winit event loop drives frames
    (`resumed` creates the window + GPU context; `about_to_wait` runs the
    frame and paces itself via `ControlFlow::WaitUntil`; `RedrawRequested`
    presents). OS input events feed `InputState` between frames.
  - `WindowMode::Headless` — a sleep-paced loop without window, GPU, or
    input. Used by tests, CI, and tooling.
- **Shutdown** — reached after a quit request or window close completes the
  current frame. An application cannot run twice; `run` returns
  `Error::InvalidState`.

Fatal errors inside event-loop callbacks (which cannot return `Result`) are
captured by the driver and reported by `run` after the loop exits.

## Rendering (Phase 2 scope)

`automata-renderer::RenderContext` owns the wgpu surface, device, and queue.
It is deliberately minimal — device/surface management, resizing, and
presenting a clear color. Key wgpu 30 API notes (verified against vendored
sources):

- `Instance::new(InstanceDescriptor::new_without_display_handle())`
- `request_adapter`/`request_device` return `Result`s
- `DeviceDescriptor` requires explicit `experimental_features` and `trace`
- `Surface::get_current_texture()` returns the `CurrentSurfaceTexture` enum
  (Success/Suboptimal/Timeout/Occluded/Outdated/Lost/Validation), not a
  `Result` — transient variants are handled internally
- Presentation is `Queue::present(surface_texture)`
- `RenderPassColorAttachment` requires a `depth_slice` field

Phase 3 adds shaders, buffers, textures, materials, and cameras here.

## Python bridge (Phase 5)

`automata-python` wraps the engine in the `automata` Python package (PyO3,
maturin-built). The design keeps the FFI boundary free of live borrows:

- `Game` owns the `Application`. `run(update)` borrows it for the whole
  loop; each frame the driver builds a `FrameContext` from **owned
  snapshots** (time, input, scene) plus **shared write buffers** (scene
  commands, outgoing events, quit flag).
- The Python callback sees start-of-frame state; its writes are applied
  immediately after it returns and become visible the next frame. A context
  object that escapes its callback holds stale owned data — no dangling
  references are possible, and no `unsafe` is required.
- Rust errors surface as `automata.Error`; exceptions raised by the Python
  callback stop the loop and propagate unchanged to the caller of `run`.
- The crate sets `unsafe_code = "deny"` instead of the workspace-wide
  `forbid`: PyO3 macro expansions contain `unsafe impl` blocks that a
  `forbid` cannot tolerate. Hand-written binding code contains no unsafe.

## Cross-cutting conventions

- **Errors** — each crate defines an `Error` enum with `thiserror` and a
  `Result<T>` alias. No panics for recoverable conditions.
- **Logging** — `tracing` everywhere; the subscriber is installed once by
  `Application::new`, honoring `RUST_LOG`.
- **Unsafe** — forbidden in engine crates (`unsafe_code = "forbid"`).
- **Docs** — every public item carries documentation; CI enforces it.
