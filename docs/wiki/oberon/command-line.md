# The `oberon` command

`oberon` is the editor, and also a command line for everything you don't
need a window for: making projects, playing and testing them, exporting,
and running the project's own Python.

```console
oberon new mygame          # a project, with its own Python in .venv
cd mygame
oberon open                # the editor (plain `oberon` does the same)
oberon run                 # play the main scene in a window
oberon test                # run tests/ headless
code .                     # VS Code: completions and type checks from .venv
```

Every command has `--help`, for example `oberon new --help`. `oberon help`
lists them all.

## Commands

| Command | What it does |
|---|---|
| `oberon [open] [SCENE\|FOLDER]` | Opens the editor. A project folder opens its main scene. |
| `oberon new NAME [--template 3d\|2d\|empty]` | Makes a project folder and its `.venv`. |
| `oberon setup [--upgrade]` | Makes or repairs the project's `.venv`. `--upgrade` moves the project to this Oberon's engine. |
| `oberon run [SCENE] [--headless] [--frames N]` | Plays a scene without the editor. |
| `oberon test [ARGS…]` | Runs `tests/` with pytest. Extra arguments go to pytest. |
| `oberon export [--target linux\|windows\|macos\|android\|web\|all]` | Builds the game. `oberon export check`, `init`, `serve` and `templates` work the same as [`automata-cli`](exporting.md). |
| `oberon bench SCENE [--frames N] [--out stats.json]` | Times a scene in headless Play (see [Performance](../guides/performance.md)). |
| `oberon python [ARGS…]` | Runs the project's own Python. |
| `oberon pip [ARGS…]` | Installs packages into the project's Python. |
| `oberon doctor` | Checks the GPU, the bundled Python, the project's `.venv` and the language server, and says how to fix what's missing. |
| `oberon --version` | Prints the version. |

Commands that work on a project find it from the current folder. The
project is the nearest folder, going upwards, that has a `pyproject.toml`,
a `.venv` or a `build.ron`. The main scene is `[tool.oberon] scene` in
`pyproject.toml`, which is `level.ron` in new projects.

### Exit codes

The commands are made for scripts and CI:
- `0` means success.
- `1` means failure, such as a failed test, a missing project, or a
  `doctor` check marked FAIL.
- `2` means bad usage, such as an unknown option or a missing argument.

`run`, `test`, `python` and `pip` exit with their child process's code, so
`oberon test` fails a CI job exactly when pytest does.

## New projects

`oberon new mygame` makes:

```text
mygame/
  level.ron               the main scene: ground, a player, a camera, a light
  scripts/player.py       walks and jumps, with type hints
  tests/test_level.py     loads the level and runs 60 frames headless
  models/  sounds/        drop assets here
  pyproject.toml          pins automata-engine to this Oberon's version
  pyrightconfig.json      type checking for scripts/ and tests/
  .vscode/                uses .venv, runs pytest, suggests the Python extensions
  build.ron               export settings
  README.md  .gitignore
  .venv/                  the project's Python (not committed)
```

The templates are `3d` (the default), `2d` (a side-view platformer
start: a painted tilemap, a sprite player and a 2D camera that follows it)
and `empty` (only a sun, a sky and a camera). Use `--no-venv` to skip
making `.venv`, then run `oberon setup` later. Use `--python PATH` to make
`.venv` from a different interpreter.

## The project's Python

Every project has its own Python in `.venv`, so projects don't share or
fight over packages. It holds:
- **automata-engine**, the same version as your Oberon
- **basedpyright**, the language server behind completions and type checks
- **pytest**, for `oberon test`

Oberon's download comes with a standalone CPython and these packages as
wheels, next to the executable in `python/` and `wheels/`. `oberon new`
builds `.venv` from them **offline**, so you don't need Python installed and
nothing is downloaded. It uses [uv](https://docs.astral.sh/uv/) when uv is
on your `PATH` (it's faster), and `python -m venv` otherwise.

One interpreter does everything. The project's `.venv` runs:
- Play mode
- `oberon run`
- `oberon test`
- the language server in Oberon's [code editor](code-editor.md)

VS Code and PyCharm pick it up too. **Settings → Python → Interpreter**
still overrides it, for Play, if you set it.

When you open a project whose `.venv` is missing, or whose engine is older
than your Oberon, Play still runs and the Console says what to do. Run
`oberon setup` after cloning a project, and `oberon setup --upgrade` after
updating Oberon.

`oberon pip install requests` adds a package to the project. It works
whether `.venv` was made with pip or with uv. Use your scripts' imports as
usual.

### Environment variables

| Variable | Use |
|---|---|
| `OBERON_PYTHON` | Makes `.venv` from this interpreter instead of the bundled one. |
| `OBERON_WHEELS` | A folder of wheels (the engine, basedpyright, pytest) to install from. |

These are for building Oberon from source, where there's no bundle yet.
To use them, build a wheel with
`maturin build -m crates/automata-python/Cargo.toml -o wheels` and set
`OBERON_WHEELS=$PWD/wheels`.

## In CI

A project made by `oberon new` tests itself with no window and no GPU:

```yaml
- run: oberon setup
- run: oberon test
- run: oberon run --headless --frames 300
- run: oberon export check
```

`oberon doctor` reports a missing GPU as a warning, not a failure, because
`test` and `run --headless` don't need one.

## Making the download

`scripts/bundle_oberon.py` assembles `oberon` with `python/` (from
[python-build-standalone](https://github.com/astral-sh/python-build-standalone))
and `wheels/`. It builds the engine wheel with that same interpreter, so the
wheel's tags match it:

```console
cargo build --release -p oberon
python scripts/bundle_oberon.py --oberon target/release/oberon --out dist/oberon --archive
```

The Export templates workflow does this for Linux and Windows. On each
runner it then hides every other Python and checks the result: `oberon new`
works offline, `doctor`, `test`, `run --headless` and `export check` all
pass, and `scripts/check_completions.py` confirms that the language server
completes `ctx.` with docs and flags a misspelt `entity.animaton`.
