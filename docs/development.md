# Development Guide

## Prerequisites

- Rust stable (MSRV: **1.86**), with `rustfmt` and `clippy`
- Python 3 (from Phase 5 onward)

Standard setup via [rustup](https://rustup.rs):

```console
rustup component add rustfmt clippy
```

### Offline / sandboxed environments

Some development sandboxes cannot reach `static.rust-lang.org` or
`crates.io`. For those, the repository provides a bootstrap path that
ferries a toolchain and vendored dependencies through GitHub Actions
artifacts:

1. Run the **bootstrap-toolchain** workflow
   (`.github/workflows/bootstrap-toolchain.yml`). It packages the stable
   toolchain (with `rustfmt` and `clippy`) and the `cargo vendor` output as
   workflow artifacts.
2. From the sandbox, fetch and install them:

   ```console
   ./scripts/setup-offline-env.sh
   export PATH="$HOME/.local/automata-rust/stable-x86_64-unknown-linux-gnu/bin:$PATH"
   ```

   The script also writes `.cargo/config.toml` (git-ignored) pointing Cargo
   at `vendor/`, so builds work fully offline.

   **Note for sandboxed sessions:** only git-tracked file contents persist
   across sandbox sessions — the toolchain (`~/.local/automata-rust`),
   `vendor/`, and `.cargo/config.toml` are wiped between sessions. Re-run
   `scripts/setup-offline-env.sh` at the start of every session.

3. Re-run the bootstrap workflow whenever dependencies change (it triggers
   automatically on changes to any `Cargo.toml`).

## Everyday commands

```console
cargo run -p automata-core        # run the engine entry point
cargo test --workspace            # unit + integration + doc tests
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
cargo doc --workspace --no-deps --open
```

All of the above run in CI and must stay green.

### Python bindings

The `automata-python` crate builds the `automata` Python package with
[maturin](https://www.maturin.rs):

```console
python3 -m venv .venv
.venv/bin/pip install maturin pytest
.venv/bin/maturin build --manifest-path crates/automata-python/Cargo.toml
.venv/bin/pip install target/wheels/automata_engine-*.whl
.venv/bin/python -m pytest crates/automata-python/tests/python
.venv/bin/python examples/python/headless_demo.py
```

**Sandbox note:** the sandbox has no `libpython`, so linking any target that
embeds Python fails locally. Workarounds baked into the workflow:

- Build/clippy the bindings with `PYO3_BUILD_EXTENSION_MODULE=1` (this is
  what maturin sets; it skips the libpython link).
- Run `cargo test --workspace --exclude automata-python` locally; the
  crate's Rust unit tests link libpython and therefore run on CI only.
- Python-side behavior is covered locally by the pytest suite against the
  maturin-built wheel, which links without libpython by design.

## Coding conventions

- **Edition 2021**, MSRV 1.86 (bumped deliberately, not casually).
- **No `unsafe`** in engine crates — enforced workspace-wide.
- **Document every public item.** `missing_docs` is a workspace lint and CI
  denies warnings. Document why, not just what.
- **Errors**: libraries define a crate-local `Error` enum with `thiserror`
  plus a `Result<T>` alias. Recoverable conditions are `Result`s, never
  panics. Panics signal programmer error only.
- **Logging**: use `tracing` macros (`info!`, `debug!`, …). Library code
  never prints to stdout/stderr directly.
- **Dependencies**: prefer mature, maintained crates (build.md section 2).
  Declare shared dependencies in the workspace root `[workspace.dependencies]`
  and inherit with `dep.workspace = true`. New dependencies require a stated
  reason in their commit message.
- **Formatting**: `rustfmt.toml` holds the canonical style. Lint policy lives
  in the workspace `[workspace.lints]` tables — exceptions are documented
  there, not scattered across the code.
- **Tests**: unit tests next to the code, integration tests in `tests/`,
  doc tests for public API examples. New functionality ships with tests
  (build.md section 10).

## Working agreements (AI-assisted development)

Per build.md section 10:

- Implement one subsystem at a time; keep the build green after each step.
- Inspect existing code before modifying it; no large uncontrolled rewrites.
- Verify dependency APIs against official documentation.
- Every completed subsystem has a defined public interface and tests.

## Commits and branches

- Imperative commit messages (`Add …`, `Fix …`), referencing the phase or
  subsystem where relevant (`phase 1: …`).
- `main` must always build. Feature work happens on branches and lands via
  pull request with green CI.
