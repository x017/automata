"""Surface a CI log as check-run annotations.

The log-zip endpoint is unreachable from the dev sandbox, but check-run
annotations are readable through the REST API. Prints the tail of
/tmp/<name>.log (ANSI colors stripped) as up to 9 `::notice` workflow
commands of ~20 KB each. GitHub caps annotations at 10 per type per step,
so each log gets its own step.
"""

import pathlib
import re
import sys

name = sys.argv[1]
path = pathlib.Path(f"/tmp/{name}.log")
if not path.exists() or path.stat().st_size == 0:
    sys.exit(0)
text = re.sub(r"\x1b\[[0-9;]*[A-Za-z]", "", path.read_text(errors="replace"))
size = 20_000
text = text[-size * 9 :]
chunks = [text[i : i + size] for i in range(0, len(text), size)]


def enc(s: str) -> str:
    return s.replace("%", "%25").replace("\r", "%0D").replace("\n", "%0A")


for n, chunk in enumerate(chunks):
    print(f"::notice title={name}.log part {n + 1}/{len(chunks)}::{enc(chunk)}")
