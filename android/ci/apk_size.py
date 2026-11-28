#!/usr/bin/env python3
"""What an APK's size is made of, optionally next to an older build.

    android/ci/apk_size.py APK [--before OLD_APK] [--top N] [--budget MB]

Prints five tables:
- what each processor type (ABI) adds, compressed: its `lib/<abi>/`
  files. With `--budget`, the script fails when one ABI adds more;
- the APK by part: each native library per ABI, the app's files
  (`assets/private.tar`), the Java code and the rest, compressed (what a
  download costs). python-for-android packs each ABI's Python (standard
  library, C modules, site-packages with the engine) as a tar.gz named
  `lib/<abi>/libpybundle.so`;
- what's inside those archives (uncompressed): the standard library zip,
  Python's C modules, the engine and so on, per ABI;
- the largest standard library packages and the largest files in the
  archives, to see what trimming would save.
"""

from __future__ import annotations

import argparse
import io
import re
import sys
import tarfile
import zipfile
from collections import Counter

MB = 1e6


def part_of(name: str) -> str:
    """The table row an APK entry counts towards."""
    if name.startswith("lib/") and name.endswith(".so"):
        return name
    if name.startswith("assets/"):
        return name if name.endswith((".tar", ".tar.gz", ".zip")) else "assets/(other)"
    if name.endswith(".dex"):
        return "classes*.dex"
    if name.startswith("res/") or name == "resources.arsc":
        return "resources"
    return "(other)"


def inner_part(name: str, abi: str) -> str:
    """The row a file inside one of python-for-android's archives counts
    towards; `abi` is set for a `libpybundle.so`."""
    rest = name.removeprefix("./")
    m = re.match(r"_python_bundle(?:__([^/]+))?/(.*)", rest)
    if m:
        abi, rest = m.group(1) or abi, m.group(2)
    elif not abi:
        return "app files (main.py, ...)"
    prefix = f"[{abi}] " if abi else ""
    if rest == "stdlib.zip":
        return prefix + "stdlib.zip"
    if rest.startswith("modules/"):
        return prefix + "C modules (lib-dynload)"
    if re.match(r"site-packages/automata/_core[^/]*\.so$", rest):
        return prefix + "engine (_core.so)"
    if rest.startswith("site-packages/automata/"):
        return prefix + "automata package (.pyc)"
    if rest.startswith("site-packages/"):
        return prefix + "site-packages (other)"
    return prefix + "(other)"


class Apk:
    """Sizes read from one APK."""

    def __init__(self, path: str) -> None:
        self.parts: Counter[str] = Counter()  # compressed, in the APK
        self.raw: Counter[str] = Counter()  # uncompressed
        self.inner: Counter[str] = Counter()  # inside assets archives
        self.stdlib: Counter[str] = Counter()  # top-level stdlib packages
        self.files: Counter[str] = Counter()  # files inside assets archives
        self.abis: Counter[str] = Counter()  # compressed, per lib/<abi>/
        self.total = 0
        with open(path, "rb") as f:
            data = f.read()
        self.total = len(data)
        with zipfile.ZipFile(io.BytesIO(data)) as z:
            for info in z.infolist():
                part = part_of(info.filename)
                self.parts[part] += info.compress_size
                if info.filename.startswith("lib/") and info.filename.count("/") >= 2:
                    self.abis[info.filename.split("/")[1]] += info.compress_size
                else:
                    self.abis["(shared: Java, resources, app files)"] += info.compress_size
                self.raw[part] += info.file_size
                if info.filename.startswith("assets/") and part != "assets/(other)":
                    self._read_archive(z.read(info), "")
                elif info.filename.endswith("/libpybundle.so"):
                    self._read_archive(z.read(info), info.filename.split("/")[1])

    def _read_archive(self, blob: bytes, abi: str) -> None:
        try:
            tar = tarfile.open(fileobj=io.BytesIO(blob), mode="r:*")
        except tarfile.TarError:
            return
        with tar:
            for member in tar:
                if not member.isfile():
                    continue
                self.inner[inner_part(member.name, abi)] += member.size
                where = f"[{abi}] " if abi else ""
                self.files[where + member.name.removeprefix("./")] += member.size
                if member.name.endswith("stdlib.zip") and not self.stdlib:
                    extracted = tar.extractfile(member)
                    if extracted is not None:
                        self._read_stdlib(extracted.read())

    def _read_stdlib(self, blob: bytes) -> None:
        with zipfile.ZipFile(io.BytesIO(blob)) as z:
            for info in z.infolist():
                top = info.filename.split("/")[0]
                if "/" not in info.filename:
                    top = "(single modules)"
                self.stdlib[top] += info.file_size


def mb(n: float) -> str:
    return f"{n / MB:8.2f}"


def delta(new: float, old: float | None) -> str:
    if old is None:
        return ""
    d = (new - old) / MB
    return f"{d:+8.2f}" if abs(d) >= 0.005 else "       ="


def table(title: str, new: Counter[str], old: Counter[str] | None, top: int) -> list[str]:
    keys = set(new) | set(old or {})
    rows = sorted(keys, key=lambda k: -max(new.get(k, 0), (old or {}).get(k, 0)))[:top]
    head = f"{title:<44} {'MB':>8}" + (f" {'before':>8} {'change':>8}" if old is not None else "")
    lines = [head, "-" * len(head)]
    for k in rows:
        line = f"{k:<44} {mb(new.get(k, 0))}"
        if old is not None:
            line += f" {mb(old.get(k, 0))} {delta(new.get(k, 0), old.get(k, 0))}"
        lines.append(line)
    total_new = sum(new.values())
    line = f"{'total':<44} {mb(total_new)}"
    if old is not None:
        total_old = sum(old.values())
        line += f" {mb(total_old)} {delta(total_new, total_old)}"
    lines.append(line)
    return lines


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("apk")
    parser.add_argument("--before", help="an older APK to compare with")
    parser.add_argument("--top", type=int, default=30)
    parser.add_argument("--budget", type=float, help="fail when one ABI adds more MB than this")
    args = parser.parse_args()

    new = Apk(args.apk)
    old = None
    if args.before:
        try:
            old = Apk(args.before)
        except (OSError, zipfile.BadZipFile) as e:
            print(f"(no comparison: {e})")
    out = [f"APK: {mb(new.total).strip()} MB" + (
        f" (before: {mb(old.total).strip()} MB, {delta(new.total, old.total).strip()} MB)"
        if old else "")]
    out.append("")
    out += table("per processor type (compressed)", new.abis, old.abis if old else None, args.top)
    over = []
    if args.budget is not None:
        over = [abi for abi, n in new.abis.items() if not abi.startswith("(") and n > args.budget * MB]
        out.append(f"budget: {args.budget:g} MB per processor type: "
                   + (f"OVER for {', '.join(sorted(over))}" if over else "ok"))
    out.append("")
    out += table("APK part (compressed, as downloaded)", new.parts, old.parts if old else None, args.top)
    out.append("")
    out += table("inside the Python archives (uncompressed)", new.inner, old.inner if old else None, args.top)
    out.append("")
    out += table("stdlib.zip, largest packages (uncompressed)", new.stdlib,
                 old.stdlib if old else None, args.top)
    out.append("")
    out += table("largest files in the Python archives (uncompressed)", new.files,
                 old.files if old else None, args.top)
    print("\n".join(out))
    return 1 if over else 0


if __name__ == "__main__":
    sys.exit(main())
