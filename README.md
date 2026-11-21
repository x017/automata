<p align="center">
  <img src="docs/assets/automata-logo.png" alt="AUTOMATA — dark white sci-fi" width="480">
</p>

# AUTOMATA

**A general-purpose 2D & 3D game engine for independent game development.**

- Native core in **Rust**
- Gameplay scripting in **Python** (via PyO3, Phase 5)
- Rendering on **wgpu**, physics via **Rapier**, audio via **Kira**

The full project specification — vision, architecture, roadmap, and
acceptance criteria — lives in [`build.md`](build.md).

## Status

**Phases 1–6 complete** ✅ — project foundation, windowed/headless runtime,
wgpu rendering, scene/events core, Python bindings, and physics + input.

- Rust workspace: `automata-core` (lifecycle, fixed-timestep loop),
  `automata-scene` (hecs ECS, events, resources), `automata-renderer`
  (wgpu 2D/3D), `automata-input` (keyboard/mouse/gamepad, action mapping),
  `automata-physics` (Rapier 2D/3D, raycasts, character controllers),
  `automata-assets` (glTF / OBJ model import), `automata-python` (PyO3
  bindings)
- Python API: decorator, procedural, and OOP (subclassing) styles —
  see [`docs/api.md`](docs/api.md) and [`examples/python/`](examples/python)
- CI enforces fmt, Clippy (`-D warnings`), tests, docs, and the Python
  test suite on every push

## Oberon, the editor

![Oberon](docs/assets/oberon/camera.png)

```console
cargo run -p oberon --release [level.ron]
```

You build scenes visually and attach Python behaviour scripts, then press
Play. Oberon also has these:
- a **Graphics** tab for tuning the sun, environment, tone mapping, and
  fog, with one-click test scenes
- **scene cameras** with frustums and a live preview
- a built-in code editor with completion and LSP

Start with the [tutorials](docs/wiki/tutorials/README.md) or the
[Oberon manual](docs/wiki/oberon/README.md). The landing page is in
[`docs/site/index.html`](docs/site/index.html).

## Documentation

The whole manual is also a website: open `docs/site/index.html` (the landing
page) or `docs/site/docs/index.html` (the docs). To browse it locally with
working screenshots, serve the `docs/` folder:

```console
python3 -m http.server -d docs 8000   # then open http://localhost:8000/site/
```

- [Wiki home](docs/wiki/README.md): guides for the Python API and Oberon
- [Tutorials](docs/wiki/tutorials/README.md): first scene, coin collector,
  third-person camera, camera rails, lighting & mood, shipping
- [API reference](docs/wiki/api-reference.md)

## Building

```console
python scripts/build.py            # engine + Python wheel + install
python scripts/build.py --all      # also run gates and tests
```

Requires a Rust toolchain, Python 3.11+, and (on Linux) `patchelf` for
wheel repair.


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
│   ├── automata-scene/     # ECS scene graph, events, resources
│   ├── automata-renderer/  # wgpu device, surface, presentation
│   ├── automata-input/     # Keyboard/mouse/gamepad state, action mapping
│   ├── automata-physics/   # Rapier 2D/3D worlds, raycasts, controllers
│   ├── automata-assets/    # glTF / OBJ model loading and caching
│   └── automata-python/    # PyO3 bindings + the `automata` package
├── docs/                   # Architecture, development, and API documentation
├── examples/python/        # Runnable example games (headless-capable)
├── scripts/                # build.py (one-shot build) and env helpers
└── tests/                  # (Cross-crate integration tests, when needed)
```

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT) at your option.
