#!/usr/bin/env python3
"""Build the AUTOMATA engine and its Python bindings in one command.

    python scripts/build.py              # engine + Python wheel + install
    python scripts/build.py --release    # optimized builds
    python scripts/build.py --check      # fmt / clippy / doc gates
    python scripts/build.py --test       # cargo tests + pytest
    python scripts/build.py --all        # everything, in order
    python scripts/build.py --skip-rust --test   # re-test without rebuilding

Stages
------
rust   : cargo build --workspace
python : maturin wheel for crates/automata-python + pip install
check  : cargo fmt --check, clippy -D warnings, rustdoc -D warnings
test   : cargo test --workspace, then pytest crates/automata-python/tests

Environment
-----------
Python builds use the active virtualenv ($VIRTUAL_ENV) when set, otherwise
the repo's `.venv` (created on demand, with maturin and pytest installed).
On Linux, wheel repair needs `patchelf` on PATH.
"""

import argparse
import glob
import os
import shutil
import subprocess
import sys
import sysconfig
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
VENV = ROOT / ".venv"


def fail(message: str) -> "SystemExit":
    print(f"error: {message}", file=sys.stderr)
    return SystemExit(1)


def run(cmd: list[str], **kwargs) -> None:
    printable = " ".join(cmd)
    print(f"\n=== {printable}", flush=True)
    try:
        subprocess.run(cmd, cwd=ROOT, check=True, **kwargs)
    except FileNotFoundError:
        raise fail(f"{cmd[0]!r} not found on PATH")
    except subprocess.CalledProcessError as err:
        raise fail(f"{printable} failed with exit code {err.returncode}")


def cargo() -> str:
    found = shutil.which("cargo")
    if found is None:
        raise fail(
            "cargo not found. Install Rust via https://rustup.rs "
            "or add your toolchain's bin directory to PATH."
        )
    return found


def venv_python() -> Path:
    """The interpreter used for the Python bindings and pytest."""
    active = os.environ.get("VIRTUAL_ENV")
    if active:
        return Path(active) / "bin" / "python"
    candidate = VENV / "bin" / "python"
    if not candidate.exists():
        print("\n=== creating .venv")
        run([sys.executable, "-m", "venv", str(VENV)])
        run([str(candidate), "-m", "pip", "install", "-q", "maturin", "pytest"])
    return candidate


def stage_rust(release: bool) -> None:
    cmd = [cargo(), "build", "--workspace"]
    if release:
        cmd.append("--release")
    run(cmd)


def stage_python(release: bool) -> None:
    python = venv_python()
    if not (VENV / "bin" / "maturin").exists() and not os.environ.get("VIRTUAL_ENV"):
        run([str(python), "-m", "pip", "install", "-q", "maturin", "pytest"])
    # Force a fresh extension artifact: auditwheel's repair step rewrites the
    # cached .so in place, and repairing an already-repaired artifact fails
    # ("required library libudev-<hash>.so.1 could not be located") on any
    # build where the Rust code did not change.
    run([cargo(), "clean", "-p", "automata-python"])
    build = [str(python), "-m", "maturin", "build",
             "--manifest-path", "crates/automata-python/Cargo.toml"]
    if release:
        build.append("--release")
    run(build)
    wheels = sorted(
        glob.glob(str(ROOT / "target" / "wheels" / "automata_engine-*.whl")),
        key=os.path.getmtime,
    )
    if not wheels:
        raise fail("maturin reported success but no wheel appeared in target/wheels")
    run([str(python), "-m", "pip", "install", "-q", "--force-reinstall", wheels[-1]])
    print(f"\ninstalled {Path(wheels[-1]).name}")


def stage_check() -> None:
    run([cargo(), "fmt", "--all", "--", "--check"])
    run([cargo(), "clippy", "--workspace", "--all-targets", "--", "-D", "warnings"])
    env = dict(os.environ, RUSTDOCFLAGS="-D warnings")
    run([cargo(), "doc", "--workspace", "--no-deps"], env=env)


def stage_test() -> None:
    run([cargo(), "test", "--workspace"])
    run([str(venv_python()), "-m", "pytest", "crates/automata-python/tests/python", "-q"])


def main() -> None:
    parser = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    parser.add_argument("--release", action="store_true",
                        help="optimized build (slower to compile)")
    parser.add_argument("--check", action="store_true",
                        help="run the fmt/clippy/doc gates")
    parser.add_argument("--test", action="store_true",
                        help="run the Rust and Python test suites")
    parser.add_argument("--all", action="store_true",
                        help="rust + python + check + test")
    parser.add_argument("--skip-rust", action="store_true",
                        help="skip the cargo build stage")
    parser.add_argument("--skip-python", action="store_true",
                        help="skip the wheel build/install stage")
    args = parser.parse_args()

    if not sysconfig.get_platform().startswith(("linux", "win", "macosx")):
        print(f"warning: untested platform {sysconfig.get_platform()}")

    build_rust = not args.skip_rust
    build_python = not args.skip_python
    check = args.check or args.all
    test = args.test or args.all

    if build_rust:
        stage_rust(args.release)
    if build_python:
        stage_python(args.release)
    if check:
        stage_check()
    if test:
        stage_test()
    print("\ndone.")


if __name__ == "__main__":
    main()
