"""Surface a CI log as check-run annotations.

The log-zip endpoint is unreachable from the dev sandbox, but check-run
annotations are readable through the REST API. Prints the tail of
/tmp/<name>.log (ANSI colors stripped) as up to 8 `::notice` workflow
commands of ~20 KB each. GitHub caps annotations at 10 per type per step,
so each log gets its own step.

The API returns only the first 4 KB of each annotation, so what failed
comes first, on its own: each panic or failed test with a few lines after
it and the test binary it ran in, cargo's `failures:` lists and compiler
errors.
"""

import pathlib
import re
import sys

name = sys.argv[1]
path = pathlib.Path(f"/tmp/{name}.log")
if not path.exists() or path.stat().st_size == 0:
    sys.exit(0)
text = re.sub(r"\x1b\[[0-9;]*[A-Za-z]", "", path.read_text(errors="replace"))


def enc(s: str) -> str:
    return s.replace("%", "%25").replace("\r", "%0D").replace("\n", "%0A")


FAILED = re.compile(r"panicked at|\bFAILED\b|^failures:|^error(\[|:)|^\s+\S+ \(line \d+\)$")
binary = ""
shown_binary = ""
failures: list[str] = []
lines = text.splitlines()
for i, line in enumerate(lines):
    if line.lstrip().startswith("Running "):
        binary = line.strip()
    elif FAILED.search(line):
        if binary != shown_binary:
            failures.append(f"[{binary}]")
            shown_binary = binary
        extra = 6 if "panicked at" in line else 1
        failures.extend(lines[i : i + extra])
if failures:
    summary = "\n".join(failures)[:4000]
    print(f"::notice title={name}.log failures::{enc(summary)}")

size = 20_000
text = text[-size * 8 :]
chunks = [text[i : i + size] for i in range(0, len(text), size)]
for n, chunk in enumerate(chunks):
    print(f"::notice title={name}.log part {n + 1}/{len(chunks)}::{enc(chunk)}")
