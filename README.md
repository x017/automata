# AUTOMATA

**A general-purpose 2D & 3D game engine for independent game development.**

- Native core in **Rust**
- Gameplay scripting in **Python** (via PyO3, Phase 5)
- Rendering on **wgpu**, physics via **Rapier**, audio via **Kira**

The full project specification — vision, architecture, roadmap, and
acceptance criteria — lives in [`build.md`](build.md).

## Status

**Phase 2 — Window and Runtime** ✅ (Phase 1: Project Foundation ✅)

- Cargo workspace with shared dependency and lint configuration
- `automata-core`: application lifecycle (`Created → Running → Shutdown`),
  frame timekeeping, `tracing`-based logging, `thiserror`-based error handling
- Windowed runtime driven by the **winit** event loop, plus a headless mode
  for tests/CI (`WindowMode::Headless`)
- `automata-input`: per-frame keyboard/mouse state (`pressed` /
  `just_pressed` / scroll / cursor / modifiers)
- `automata-renderer`: **wgpu** device, surface, and frame presentation
  (clear color) with transient surface-error recovery
- Frame pacing via `ControlFlow::WaitUntil` + configurable `target_fps`
- Testing infrastructure: unit, integration, and doc tests; CI enforces
  fmt, Clippy (`-D warnings`), tests, and docs

Next up: **Phase 3 — Rendering Foundation** (shaders, buffers, textures,
materials, cameras, 2D/3D rendering).

## Quickstart

```console
$ cargo run -p automata-core            # opens a 1280x720 window (Esc quits)
INFO automata_core: automata initialized title="AUTOMATA" version="0.1.0"
INFO automata_core::app: entering main loop mode=Windowed(...)
INFO automata_renderer: GPU adapter selected ...
INFO automata_renderer: surface configured ...

$ cargo run -p automata-core -- --headless   # no display required
```

Run the test suite:

```console
cargo test --workspace
```

## Repository layout

```text
automata/
├── build.md                # Project specification (read this first)
├── crates/
│   ├── automata-core/      # Engine core: lifecycle, time, window runtime
│   ├── automata-input/     # Input event collection and state
│   └── automata-renderer/  # wgpu device, surface, presentation
├── docs/                   # Architecture, development, and API documentation
├── scripts/                # Development environment helpers
└── tests/                  # (Cross-crate integration tests, when needed)
```

The remaining subsystem crates (`automata-scene`, `automata-physics`, …) and
the `python/` package are introduced in later phases, once their boundaries
become meaningful (see `build.md` section 9).

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT) at your option.
