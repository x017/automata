#!/usr/bin/env python3
"""What a web build's download is made of, and whether it fits the budget.

    web/ci/web_size.py SITE [--before TEMPLATE.tar.gz] [--budget-mb MB]
                       [--budget-gz MB] [--stats STATS.json --budget-start S]

Prints:
- every file of SITE (a web build or the web template), raw and gzipped
  (what a web server sends; the wheel and the zips are compressed already),
  next to the same file in an older template (`--before`, the published
  `automata-template-web.tar.gz`) when given;
- inside the engine wheel and Python's standard library: the largest parts;
- the start-up timings from a smoke test's stats.json (`--stats`).

Fails when the download is over `--budget-mb` (raw) or `--budget-gz`
(gzipped), or the first frame came later than `--budget-start` seconds
after navigation.
"""

from __future__ import annotations

import argparse
import gzip
import io
import json
import sys
import tarfile
import zipfile
from collections import Counter
from pathlib import Path

MB = 1e6


def gz(data: bytes) -> int:
    return len(gzip.compress(data, compresslevel=9, mtime=0))


def site_files(site: Path) -> dict[str, bytes]:
    return {
        p.relative_to(site).as_posix(): p.read_bytes()
        for p in sorted(site.rglob("*"))
        if p.is_file()
    }


def template_files(archive: Path) -> dict[str, bytes]:
    out = {}
    with tarfile.open(archive) as tar:
        for m in tar.getmembers():
            if m.isfile():
                out[m.name.removeprefix("./")] = tar.extractfile(m).read()
    return out


def mb(n: float) -> str:
    return f"{n / MB:7.2f}"


def table(files: dict[str, bytes], before: dict[str, bytes] | None) -> list[str]:
    lines = []
    head = f"  {'file':<58} {'MB':>7} {'gz MB':>7}"
    if before is not None:
        head += f" {'before':>8} {'change':>8}"
    lines.append(head)
    names = sorted(set(files) | set(before or {}), key=lambda n: -len(files.get(n, b"")))
    for name in names:
        data = files.get(name)
        row = f"  {name:<58} {mb(len(data)) if data is not None else '      -'} {mb(gz(data)) if data is not None else '      -'}"
        if before is not None:
            old = before.get(name)
            # Wheel names carry the version; compare the wheel with the wheel.
            if old is None and name.endswith(".whl"):
                old = next((d for n, d in before.items() if n.endswith(".whl")), None)
            if old is None:
                row += f" {'(new)':>8}"
            else:
                change = (len(data) if data is not None else 0) - len(old)
                row += f" {mb(len(old))} {change / MB:+8.2f}"
        lines.append(row)
    return lines


def inside_zip(data: bytes, title: str, group) -> list[str]:
    z = zipfile.ZipFile(io.BytesIO(data))
    parts: Counter[str] = Counter()
    raw: Counter[str] = Counter()
    for i in z.infolist():
        key = group(i.filename)
        parts[key] += i.compress_size
        raw[key] += i.file_size
    lines = [f"{title} ({len(z.infolist())} files), largest parts (MB compressed, raw):"]
    for key, size in parts.most_common(10):
        lines.append(f"  {mb(size)} {mb(raw[key])}  {key}")
    return lines


def wheel_part(name: str) -> str:
    if name.endswith(".so"):
        return name + " (the engine: Rust, wgpu, naga, physics, audio)"
    if "/__pycache__/" in name:
        return "automata/__pycache__/ (compiled Python)"
    if name.endswith(".py"):
        return "automata/*.py (Python source)"
    return name.split("/")[0] + "/…"


def stdlib_part(name: str) -> str:
    top = name.split("/")[0]
    return top.removesuffix(".pyc").removesuffix(".py")


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("site", type=Path)
    ap.add_argument("--before", type=Path)
    ap.add_argument("--budget-mb", type=float)
    ap.add_argument("--budget-gz", type=float)
    ap.add_argument("--stats", type=Path)
    ap.add_argument("--budget-start", type=float)
    args = ap.parse_args()

    files = site_files(args.site)
    before = None
    if args.before and args.before.is_file():
        try:
            before = template_files(args.before)
        except (tarfile.TarError, OSError) as e:
            print(f"(couldn't read {args.before}: {e})")
    total = sum(len(d) for d in files.values())
    total_gz = sum(gz(d) for d in files.values())
    out = [f"download: {total / MB:.2f} MB ({total_gz / MB:.2f} MB gzipped), {len(files)} files"]
    if before is not None:
        b_total = sum(len(d) for n, d in before.items())
        b_gz = sum(gz(d) for d in before.values())
        out.append(f"the published template: {b_total / MB:.2f} MB ({b_gz / MB:.2f} MB gzipped); "
                   "a build adds game.zip and player.json")
    out += [""] + table(files, before) + [""]
    wheel = next((d for n, d in files.items() if n.endswith(".whl")), None)
    if wheel:
        out += inside_zip(wheel, "engine wheel", wheel_part) + [""]
    stdlib = files.get("pyodide/python_stdlib.zip")
    if stdlib:
        compiled = sum(n.endswith(".pyc") for n in zipfile.ZipFile(io.BytesIO(stdlib)).namelist())
        out += inside_zip(stdlib, f"python_stdlib.zip, {compiled} modules compiled", stdlib_part) + [""]

    failed = []
    if args.budget_mb is not None:
        out.append(f"budget: {args.budget_mb:.2f} MB raw, {total / MB:.2f} MB used")
        if total > args.budget_mb * MB:
            failed.append(f"the download is {total / MB:.2f} MB, over the {args.budget_mb} MB budget")
    if args.budget_gz is not None:
        out.append(f"budget: {args.budget_gz:.2f} MB gzipped, {total_gz / MB:.2f} MB used")
        if total_gz > args.budget_gz * MB:
            failed.append(f"the download is {total_gz / MB:.2f} MB gzipped, over the {args.budget_gz} MB budget")
    if args.stats:
        try:
            timings = json.loads(args.stats.read_text())["stats"]["timings"]
        except (OSError, KeyError, TypeError, ValueError) as e:
            timings = None
            failed.append(f"no start-up timings in {args.stats}: {e}")
        if timings:
            out.append("start-up (ms since navigation): " + ", ".join(f"{k} {v}" for k, v in timings.items()))
            first = timings.get("first_frame")
            if args.budget_start is not None:
                out.append(f"budget: first frame within {args.budget_start:.1f} s")
                if first is None or first > args.budget_start * 1000:
                    failed.append(f"the first frame came at {first} ms, over the {args.budget_start} s budget")
    out += [f"FAILED: {f}" for f in failed] or ["within budget"]
    print("\n".join(out))
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
