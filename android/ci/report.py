#!/usr/bin/env python3
"""Publishes a CI report as a GitHub check run, readable through the API.

    report.py NAME --log FILE [--lines N] [--grep REGEX]
    report.py NAME --image FILE

`--log` puts the end of a log in the check run's text (only lines matching
`--grep`, when given). `--image` puts a small JPEG there as base64
(make it small first: the text holds at most 65535 characters). Needs
GITHUB_TOKEN, GITHUB_REPOSITORY and GITHUB_SHA, and `checks: write`.
"""

import argparse
import base64
import json
import os
import re
import sys
import urllib.request

LIMIT = 65000


def log_text(path, lines, pattern):
    with open(path, encoding="utf-8", errors="replace") as f:
        rows = f.read().splitlines()
    if pattern:
        rows = [r for r in rows if re.search(pattern, r)]
    # Escape sequences make the text unreadable; drop them.
    rows = [re.sub(r"\x1b\[[0-9;]*m", "", r) for r in rows[-lines:]]
    text = "\n".join(rows)
    return "```\n" + text[-(LIMIT - 10):] + "\n```"


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("name")
    parser.add_argument("--log")
    parser.add_argument("--image")
    parser.add_argument("--lines", type=int, default=400)
    parser.add_argument("--grep")
    parser.add_argument("--conclusion", default="neutral")
    args = parser.parse_args()

    if args.image:
        if not os.path.exists(args.image):
            print(f"no {args.image}; nothing to report")
            return
        with open(args.image, "rb") as f:
            data = base64.b64encode(f.read()).decode()
        if len(data) > LIMIT:
            sys.exit(f"{args.image} is too big for a check run ({len(data)} base64 chars)")
        summary, text = f"base64 JPEG, {len(data)} characters", data
    elif args.log:
        if not os.path.exists(args.log):
            print(f"no {args.log}; nothing to report")
            return
        summary, text = f"last lines of {os.path.basename(args.log)}", log_text(
            args.log, args.lines, args.grep
        )
    else:
        sys.exit("give --log or --image")

    body = {
        "name": args.name,
        "head_sha": os.environ["GITHUB_SHA"],
        "status": "completed",
        "conclusion": args.conclusion,
        "output": {"title": args.name, "summary": summary, "text": text},
    }
    request = urllib.request.Request(
        f"https://api.github.com/repos/{os.environ['GITHUB_REPOSITORY']}/check-runs",
        data=json.dumps(body).encode(),
        headers={
            "Authorization": f"Bearer {os.environ['GITHUB_TOKEN']}",
            "Accept": "application/vnd.github+json",
        },
        method="POST",
    )
    with urllib.request.urlopen(request) as response:
        print(f"reported {args.name}: HTTP {response.status}")


if __name__ == "__main__":
    main()
