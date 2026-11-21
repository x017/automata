#!/usr/bin/env python3
"""Runs the performance baseline and prints a Markdown table.

    python examples/bench/run.py                      # everything
    python examples/bench/run.py --quick              # 100 and 1,000 only
    python examples/bench/run.py --runners run_scene,oberon --sizes 100,1000

The scenes come from ``build_scenes.py`` (written to ``examples/bench/scenes``).
Each one runs headless (no window, no rendering) and unpaced, so a frame
costs exactly what the scripts and the engine take, in three runners:

* ``oberon``: Oberon's Play mode (``oberon --bench``). The simulation runs
  in Rust and the scripts in a separate Python process over the bridge.
* ``run_scene``: ``automata.run_scene``, the scripts in the same process.
* ``export``: the scene exported with ``automata-export build`` and run with
  ``--headless`` (the game's launcher starts ``run_scene``).

Every runner advances the game by exactly 1/60 s a frame, so physics takes
one step per frame however long the frame takes (no catch-up steps):
the numbers are the cost of one 60 Hz frame. Frame time is split into **script** (Python running ``update`` on every
entity) and **engine** (the rest: physics, the simulation, syncing, and for
Oberon the trip over the bridge). The release binaries are built with
cargo first unless you pass ``--no-build``. Run it with the Python that has
``automata`` installed (``pip install automata-engine`` or a local wheel).
The table, the machine and the date are printed; ``--out`` also saves the
raw numbers as JSON.
"""

import argparse
import datetime
import json
import os
import platform
import shutil
import statistics
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
sys.path.insert(0, HERE)

RUNNERS = ("oberon", "run_scene", "export")
EXE = ".exe" if os.name == "nt" else ""
TARGET = "windows" if os.name == "nt" else "linux"


def log(*parts):
    print(*parts, file=sys.stderr, flush=True)


def cargo_build(bins):
    """Builds the release binaries the chosen runners need."""
    if "oberon" in bins:
        # Headless: no audio device needed, so no ALSA/WASAPI to link.
        run(["cargo", "build", "--release", "-p", "oberon", "--no-default-features"])
    rest = [b for b in bins if b != "oberon"]
    if rest:
        cmd = ["cargo", "build", "--release"]
        for b in rest:
            cmd += ["-p", b]
        run(cmd)


def run(cmd, env=None, quiet=False):
    log("$", " ".join(cmd))
    out = subprocess.run(
        cmd,
        cwd=ROOT,
        env=env,
        stdout=subprocess.PIPE if quiet else None,
        stderr=subprocess.PIPE if quiet else None,
        text=True,
    )
    if out.returncode != 0:
        if quiet:
            log(out.stdout[-4000:], out.stderr[-4000:])
        raise SystemExit(f"failed ({out.returncode}): {' '.join(cmd)}")
    return out


def binary(name):
    return os.path.join(ROOT, "target", "release", name + EXE)


def stats_env(path, opts):
    env = dict(os.environ)
    env["AUTOMATA_FRAME_STATS"] = path
    env["AUTOMATA_FRAME_STATS_WARMUP"] = str(opts.warmup)
    env["AUTOMATA_FRAME_STATS_SECONDS"] = str(opts.seconds)
    return env


def read_stats(path):
    with open(path, encoding="utf-8") as f:
        return json.load(f)


def run_oberon(scene, out, opts):
    run(
        [
            binary("oberon"),
            "--bench",
            scene,
            "--frames",
            str(opts.frames),
            "--warmup",
            str(opts.warmup),
            "--seconds",
            str(opts.seconds),
            "--python",
            sys.executable,
            "--out",
            out,
        ],
        quiet=True,
    )


def run_run_scene(scene, out, opts):
    # One tick more than warmup + frames: a frame is timed at the next tick.
    frames = opts.warmup + opts.frames + 1
    code = (
        "import automata as at, sys; "
        f"at.run_scene(sys.argv[1], headless=True, max_frames={frames}, fixed_delta=1 / 60)"
    )
    run([sys.executable, "-c", code, scene], env=stats_env(out, opts), quiet=True)


def run_export(project, out, opts, template, bundled):
    cmd = [binary("automata-export"), "build", project, "--target", TARGET]
    if template:
        cmd += ["--template", template]
    built = run(cmd, quiet=True).stdout
    exe = next(
        (line.split(" ", 1)[1].strip() for line in built.splitlines() if line.startswith("built ")),
        None,
    )
    if not exe:
        raise SystemExit(f"automata-export printed no executable:\n{built}")
    frames = opts.warmup + opts.frames + 1
    env = stats_env(out, opts)
    if not bundled:
        env["AUTOMATA_PYTHON"] = sys.executable  # the Python with automata installed
    run(
        [exe, "--headless", "--frames", str(frames), "--fixed-delta", str(1 / 60)],
        env=env,
        quiet=True,
    )


def export_template(work):
    """The installed export template, else a development one made from the
    freshly built player. Returns ``(--template dir or None, description,
    bundled Python?)``."""
    listing = subprocess.run(
        [binary("automata-export"), "templates", "list"],
        capture_output=True,
        text=True,
        cwd=ROOT,
    ).stdout
    for line in listing.splitlines():
        words = line.split()
        if words[:2] == [TARGET, "installed"]:
            if "development" in line:
                return None, "installed development template (this Python)", False
            return None, "installed template (bundled Python)", True
    folder = os.path.join(work, "template")
    os.makedirs(folder, exist_ok=True)
    shutil.copy2(binary("automata-player"), os.path.join(folder, "automata-player" + EXE))
    return folder, "development template built from source (this Python)", False


def percentile(values, q):
    ordered = sorted(values)
    if not ordered:
        return 0.0
    k = (len(ordered) - 1) * q
    lo = int(k)
    hi = min(lo + 1, len(ordered) - 1)
    return ordered[lo] + (ordered[hi] - ordered[lo]) * (k - lo)


def summarize(raw):
    frame = raw["frame_ms"]
    script = raw["script_ms"]
    avg = statistics.fmean(frame) if frame else 0.0
    script_avg = statistics.fmean(script) if script else 0.0
    return {
        "frames": len(frame),
        "avg": avg,
        "p95": percentile(frame, 0.95),
        "script": script_avg,
        "engine": max(avg - script_avg, 0.0),
        "fps": 1000.0 / avg if avg > 0 else 0.0,
    }


def machine():
    cpu = platform.processor() or platform.machine()
    try:
        with open("/proc/cpuinfo", encoding="utf-8") as f:
            for line in f:
                if line.startswith("model name"):
                    cpu = line.split(":", 1)[1].strip()
                    break
    except OSError:
        pass
    return {
        "cpu": cpu,
        "cores": os.cpu_count(),
        "os": f"{platform.system()} {platform.release()}",
        "python": platform.python_version(),
        "date": datetime.date.today().isoformat(),
    }


def table(rows):
    lines = [
        "| Scene | Entities | Runner | Avg frame | p95 frame | Script | Engine | FPS |",
        "|---|---:|---|---:|---:|---:|---:|---:|",
    ]
    for r in rows:
        s = r["summary"]
        if s is None:
            lines.append(f"| {r['kind']} | {r['size']:,} | {r['runner']} | failed | | | | |")
            continue
        lines.append(
            f"| {r['kind']} | {r['size']:,} | {r['runner']} | {s['avg']:.2f} ms | "
            f"{s['p95']:.2f} ms | {s['script']:.2f} ms | {s['engine']:.2f} ms | {s['fps']:.0f} |"
        )
    return "\n".join(lines)


def main():
    parser = argparse.ArgumentParser(description="AUTOMATA performance baseline")
    parser.add_argument("--sizes", default="100,1000,5000")
    parser.add_argument("--kinds", default="scripts,physics")
    parser.add_argument("--runners", default=",".join(RUNNERS))
    parser.add_argument("--frames", type=int, default=300, help="frames to measure")
    parser.add_argument("--warmup", type=int, default=30, help="frames to skip first")
    parser.add_argument(
        "--seconds", type=float, default=30.0, help="stop measuring a run after this long"
    )
    parser.add_argument("--quick", action="store_true", help="100 and 1,000 entities, 120 frames")
    parser.add_argument("--no-build", action="store_true", help="use the binaries in target/release")
    parser.add_argument("--out", help="also write the raw numbers to this JSON file")
    opts = parser.parse_args()
    if opts.quick:
        opts.sizes, opts.frames = "100,1000", 120
    sizes = [int(s) for s in opts.sizes.split(",") if s]
    kinds = [k for k in opts.kinds.split(",") if k]
    runners = [r for r in opts.runners.split(",") if r]
    for r in runners:
        if r not in RUNNERS:
            raise SystemExit(f"unknown runner {r!r} (choose from {', '.join(RUNNERS)})")

    import build_scenes

    scenes = build_scenes.write(os.path.join(HERE, "scenes"), kinds=kinds, sizes=sizes)
    if not opts.no_build:
        bins = []
        if "oberon" in runners:
            bins.append("oberon")
        if "export" in runners:
            bins += ["automata-export", "automata-player"]
        cargo_build(bins)

    rows = []
    notes = []
    with tempfile.TemporaryDirectory(prefix="automata-bench-") as work:
        template, bundled = None, False
        if "export" in runners:
            template, how, bundled = export_template(work)
            notes.append(f"export runner: {how}")
        for kind, size, folder in scenes:
            scene = os.path.join(folder, "bench.ron")
            for runner in runners:
                out = os.path.join(work, f"{kind}-{size}-{runner}.json")
                log(f"== {kind}-{size} · {runner}")
                try:
                    if runner == "oberon":
                        run_oberon(scene, out, opts)
                    elif runner == "run_scene":
                        run_run_scene(scene, out, opts)
                    else:
                        run_export(folder, out, opts, template, bundled)
                    raw = read_stats(out)
                    summary = summarize(raw)
                except (SystemExit, OSError, ValueError) as err:
                    log(f"   failed: {err}")
                    raw, summary = None, None
                rows.append(
                    {"kind": kind, "size": size, "runner": runner, "summary": summary, "raw": raw}
                )
                if summary:
                    log(f"   {summary['avg']:.2f} ms/frame over {summary['frames']} frames")

    info = machine()
    print(table(rows))
    print()
    print(
        f"{info['date']} · {info['cpu']} ({info['cores']} threads) · {info['os']} · "
        f"Python {info['python']} · headless, {opts.frames} frames after {opts.warmup} warm-up"
    )
    for note in notes:
        print(f"{note}")
    if opts.out:
        with open(opts.out, "w", encoding="utf-8") as f:
            json.dump({"machine": info, "options": vars(opts), "runs": rows}, f, indent=1)
        log(f"wrote {opts.out}")
    return 1 if any(r["summary"] is None for r in rows) else 0


if __name__ == "__main__":
    sys.exit(main())
