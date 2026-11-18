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
| `automata-core` | ✅ Phase 1 | Application lifecycle, main loop, timekeeping, logging, error types |
| `automata-renderer` | Phase 3 | wgpu-based 2D/3D rendering |
| `automata-scene` | Phase 4 | Scenes, entities, ECS, transforms |
| `automata-input` | Phase 2/6 | Keyboard, mouse, gamepad, action mapping |
| `automata-physics` | Phase 6 | Rapier 2D/3D integration |
| `automata-audio` | Phase 7 | Kira-based audio |
| `automata-assets` | Phase 7 | Resource loading, caching, handles |
| `automata-python` | Phase 5 | PyO3 bindings and Python API |

New crates are introduced only when a subsystem boundary becomes meaningful —
not ahead of time.

## Application lifecycle (Phase 1)

```text
Created ──run()──▶ Running ──quit requested──▶ Shutdown
```

- **Created** — `Application::new(EngineConfig)` initializes engine-wide
  services (logging) and returns. No window or GPU device exists yet.
- **Running** — `Application::run(update)` invokes `update` once per frame
  with an `AppContext` (timing + quit control). Frame pacing follows
  `EngineConfig::target_fps`.
- **Shutdown** — reached after a quit request completes the current frame.
  An application cannot run twice; `run` returns `Error::InvalidState`.

The Phase 1 loop is headless. Phase 2 keeps this public API but drives the
loop from `winit`'s event loop instead of a sleep-based cycle.

## Cross-cutting conventions

- **Errors** — each crate defines an `Error` enum with `thiserror` and a
  `Result<T>` alias. No panics for recoverable conditions.
- **Logging** — `tracing` everywhere; the subscriber is installed once by
  `Application::new`, honoring `RUST_LOG`.
- **Unsafe** — forbidden in engine crates (`unsafe_code = "forbid"`).
- **Docs** — every public item carries documentation; CI enforces it.
