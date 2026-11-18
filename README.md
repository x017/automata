# AUTOMATA

**A general-purpose 2D & 3D game engine for independent game development.**

- Native core in **Rust**
- Gameplay scripting in **Python** (via PyO3, Phase 5)
- Rendering on **wgpu**, physics via **Rapier**, audio via **Kira**

The full project specification — vision, architecture, roadmap, and
acceptance criteria — lives in [`build.md`](build.md).

## Status

**Phase 1 — Project Foundation** ✅

- Cargo workspace with shared dependency and lint configuration
- `automata-core`: application lifecycle (`Created → Running → Shutdown`),
  frame timekeeping, `tracing`-based logging, `thiserror`-based error handling
- Testing infrastructure: unit, integration, and doc tests
- Coding conventions: [`docs/development.md`](docs/development.md)
- CI: formatting, Clippy (`-D warnings`), tests, docs

Next up: **Phase 2 — Window and Runtime** (`winit`, `wgpu`, real event loop).

## Quickstart

```console
$ cargo run -p automata-core
INFO automata_core: automata initialized title="AUTOMATA" version="0.1.0"
INFO automata_core: entering main loop
...
INFO automata_core: automata shut down frames=120 elapsed_seconds=2.0...
```

Run the test suite:

```console
cargo test --workspace
```

## Repository layout

```text
automata/
├── build.md            # Project specification (read this first)
├── crates/
│   └── automata-core/  # Engine core: lifecycle, time, logging, errors
├── docs/               # Architecture, development, and API documentation
├── scripts/            # Development environment helpers
└── tests/              # (Cross-crate integration tests, when needed)
```

Subsystem crates (`automata-renderer`, `automata-scene`, …) and the
`python/` package are introduced in later phases, once their boundaries
become meaningful (see `build.md` section 9).

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT) at your option.
