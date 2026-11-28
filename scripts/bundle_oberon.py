#!/usr/bin/env python3
"""Assembles the Oberon download: the editor plus its own Python.

    python scripts/bundle_oberon.py --oberon target/release/oberon --out dist/oberon
    python scripts/bundle_oberon.py --oberon target/release/oberon.exe --out dist/oberon --archive

Produces::

    dist/oberon/
      oberon[.exe]   the editor and command line
      python/        a standalone CPython (python-build-standalone)
      wheels/        automata-engine for that Python, basedpyright, pytest
                     and their dependencies
      README.txt

so that ``oberon new mygame`` makes a project's ``.venv`` offline on a
machine with no Python at all (ROADMAP D0). The engine wheel is built
with the bundled interpreter, so its tags match it.

Run it with any Python 3.10+; it needs ``cargo`` and network access (to
download the interpreter and the tool wheels) unless given
``--python-archive`` and ``--wheel``.
"""

import argparse
import os
import platform
import shutil
import subprocess
import sys
import tarfile
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
# Keep in step with .github/workflows/export.yml.
PBS_TAG = "20261110"
PBS_PYTHON = "3.12.16"
DEV_PACKAGES = ["basedpyright", "pytest"]


def host_triple():
    machine = platform.machine().lower()
    arch = {"amd64": "x86_64", "x86_64": "x86_64", "arm64": "aarch64", "aarch64": "aarch64"}.get(
        machine, machine
    )
    if sys.platform.startswith("win"):
        return f"{arch}-pc-windows-msvc"
    if sys.platform == "darwin":
        return f"{arch}-apple-darwin"
    return f"{arch}-unknown-linux-gnu"


def bundled_python(out):
    if sys.platform.startswith("win"):
        return out / "python" / "python.exe"
    return out / "python" / "bin" / "python3"


def run(cmd, **kwargs):
    print("+", " ".join(str(c) for c in cmd), flush=True)
    subprocess.run([str(c) for c in cmd], check=True, **kwargs)


def download_python(triple, dest):
    asset = f"cpython-{PBS_PYTHON}+{PBS_TAG}-{triple}-install_only_stripped.tar.gz"
    url = (
        "https://github.com/astral-sh/python-build-standalone/releases/download/"
        f"{PBS_TAG}/{asset.replace('+', '%2B')}"
    )
    print(f"downloading {asset}", flush=True)
    with urllib.request.urlopen(url) as response, open(dest, "wb") as f:  # noqa: S310
        shutil.copyfileobj(response, f)
    return dest


def extract_python(archive, out):
    target = out / "python"
    if target.exists():
        shutil.rmtree(target)
    with tarfile.open(archive) as tar:
        if hasattr(tarfile, "data_filter"):
            tar.extractall(out, filter="data")
        else:  # Python < 3.12
            tar.extractall(out)  # noqa: S202 - our own pinned release
    python = bundled_python(out)
    if not python.exists():
        sys.exit(f"{archive} has no python/ folder with {python.relative_to(out)}")
    # Trim what nobody needs to make a venv.
    for sub in ("include", "share"):
        shutil.rmtree(target / sub, ignore_errors=True)
    for path in list(target.rglob("__pycache__")):
        shutil.rmtree(path, ignore_errors=True)
    for name in ("test", "idlelib", "tkinter", "turtledemo"):
        for path in list(target.glob(f"**/lib*/{name}")) + list(target.glob(f"Lib/{name}")):
            shutil.rmtree(path, ignore_errors=True)
    return python


def build_wheel(python, wheels, features):
    run([python, "-m", "pip", "install", "--no-cache-dir", "--quiet", "maturin>=1.7,<2.0"])
    cmd = [
        python, "-m", "maturin", "build", "--release",
        "--manifest-path", ROOT / "crates" / "automata-python" / "Cargo.toml",
        "-i", python, "--out", wheels, "--auditwheel", "skip",
    ]
    if features:
        cmd += ["--features", features]
    run(cmd)
    run([python, "-m", "pip", "uninstall", "--yes", "--quiet", "maturin"])


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--oberon", type=Path, help="the oberon executable (default: build it)")
    parser.add_argument("--out", type=Path, default=ROOT / "dist" / "oberon")
    parser.add_argument("--python-archive", type=Path,
                        help="a python-build-standalone install_only tarball (default: download)")
    parser.add_argument("--wheel", type=Path, help="a prebuilt automata-engine wheel for that Python")
    parser.add_argument("--features", default="audio-cpal", help="engine wheel features")
    parser.add_argument("--archive", action="store_true", help="also pack OUT into a .zip / .tar.gz")
    args = parser.parse_args()

    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=True)
    exe_name = "oberon.exe" if sys.platform.startswith("win") else "oberon"

    oberon = args.oberon
    if oberon is None:
        run(["cargo", "build", "--release", "-p", "oberon"], cwd=ROOT)
        oberon = ROOT / "target" / "release" / exe_name
    shutil.copy2(oberon, out / exe_name)

    archive = args.python_archive or download_python(host_triple(), out / "python.tar.gz")
    python = extract_python(archive, out)
    if args.python_archive is None:
        archive.unlink()
    run([python, "--version"])

    wheels = out / "wheels"
    shutil.rmtree(wheels, ignore_errors=True)
    wheels.mkdir()
    if args.wheel:
        shutil.copy2(args.wheel, wheels / args.wheel.name)
    else:
        build_wheel(python, wheels, args.features)
    # The tools, for exactly this interpreter and platform.
    run([python, "-m", "pip", "download", "--no-cache-dir", "--quiet", "--only-binary=:all:",
         "--dest", wheels, *DEV_PACKAGES])

    version = subprocess.run([out / exe_name, "--version"], capture_output=True, text=True,
                             check=True).stdout.split()[-1]
    (out / "README.txt").write_text(
        f"Oberon {version}: the AUTOMATA editor\n\n"
        "  oberon new mygame     make a project (with its own Python in .venv)\n"
        "  cd mygame\n"
        "  oberon open           the editor\n"
        "  oberon run            play it\n"
        "  oberon test           run its tests\n"
        "  oberon help           everything else\n\n"
        "python/ and wheels/ give new projects their Python without installing\n"
        "anything; keep them next to the oberon executable.\n"
    )
    names = sorted(p.name for p in wheels.iterdir())
    print(f"\nbundled {out}:\n  {exe_name}\n  python/ ({PBS_PYTHON})\n  wheels/ ({len(names)}): "
          + ", ".join(names))

    if args.archive:
        fmt = "zip" if sys.platform.startswith("win") else "gztar"
        made = shutil.make_archive(str(out), fmt, root_dir=out.parent, base_dir=out.name)
        print(f"packed {made}")


if __name__ == "__main__":
    main()
