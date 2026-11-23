#!/usr/bin/env python3
"""Checks that a project's language server completes the script API.

    python scripts/check_completions.py mygame

Starts ``mygame/.venv``'s ``basedpyright-langserver`` the way VS Code and
Oberon's code editor do, opens a behaviour script, and asks for completions
after ``ctx.`` and ``entity.`` and for diagnostics on a misspelt
``entity.animaton``. Exits 1, saying what's missing, when completions lack
the API or its docs, or the typo isn't flagged. CI runs it on a fresh
``oberon new`` project (ROADMAP D0).
"""

import json
import os
import subprocess
import sys
import time
from pathlib import Path

SCRIPT = '''from __future__ import annotations

from typing import TYPE_CHECKING

if TYPE_CHECKING:
    from automata.script_api import Context, Entity


def update(entity: Entity, ctx: Context) -> None:
    ctx.
    entity.
    entity.animaton.play("Run")
'''
CTX_LINE, ENTITY_LINE, TYPO_LINE = 9, 10, 11


class Server:
    def __init__(self, command, root):
        self.proc = subprocess.Popen(
            command, cwd=root, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
        )
        self.next_id = 0
        self.notifications = []

    def send(self, message):
        body = json.dumps(message).encode()
        self.proc.stdin.write(f"Content-Length: {len(body)}\r\n\r\n".encode() + body)
        self.proc.stdin.flush()

    def read(self):
        length = None
        while True:
            line = self.proc.stdout.readline()
            if not line:
                raise RuntimeError("the language server exited")
            line = line.strip()
            if not line:
                break
            name, _, value = line.decode().partition(":")
            if name.lower() == "content-length":
                length = int(value)
        return json.loads(self.proc.stdout.read(length))

    def request(self, method, params):
        self.next_id += 1
        self.send({"jsonrpc": "2.0", "id": self.next_id, "method": method, "params": params})
        while True:
            message = self.read()
            if message.get("id") == self.next_id and "method" not in message:
                if "error" in message:
                    raise RuntimeError(f"{method}: {message['error']}")
                return message.get("result")
            if "method" in message and "id" in message:
                # A request from the server (configuration, …): answer it.
                result = None
                if message["method"] == "workspace/configuration":
                    result = [None] * len(message["params"]["items"])
                self.send({"jsonrpc": "2.0", "id": message["id"], "result": result})
            else:
                self.notifications.append(message)

    def notify(self, method, params):
        self.send({"jsonrpc": "2.0", "method": method, "params": params})


def completions(server, uri, line, character):
    result = server.request("textDocument/completion", {
        "textDocument": {"uri": uri},
        "position": {"line": line, "character": character},
    })
    items = result.get("items", []) if isinstance(result, dict) else (result or [])
    return {item["label"]: item for item in items}


def documented(server, item):
    if item.get("documentation"):
        return True
    resolved = server.request("completionItem/resolve", item)
    doc = resolved.get("documentation")
    return bool(doc if isinstance(doc, str) else (doc or {}).get("value"))


def main():
    root = Path(sys.argv[1] if len(sys.argv) > 1 else ".").resolve()
    bin_dir = root / ".venv" / ("Scripts" if os.name == "nt" else "bin")
    exe = bin_dir / ("basedpyright-langserver.exe" if os.name == "nt" else "basedpyright-langserver")
    if not exe.exists():
        sys.exit(f"no language server in {bin_dir} (did `oberon new` install basedpyright?)")
    script = root / "scripts" / "_completion_check.py"
    script.write_text(SCRIPT)
    uri = script.as_uri()
    server = Server([str(exe), "--stdio"], root)
    problems = []
    try:
        server.request("initialize", {
            "processId": os.getpid(),
            "rootUri": root.as_uri(),
            "capabilities": {"textDocument": {
                "completion": {"completionItem": {"documentationFormat": ["markdown", "plaintext"]}},
                "publishDiagnostics": {},
            }},
            "workspaceFolders": [{"uri": root.as_uri(), "name": root.name}],
        })
        server.notify("initialized", {})
        server.notify("textDocument/didOpen", {"textDocument": {
            "uri": uri, "languageId": "python", "version": 1, "text": SCRIPT,
        }})

        ctx = completions(server, uri, CTX_LINE, len("    ctx."))
        for name in ("input", "time", "physics", "publish", "subscribe", "camera", "get"):
            if name not in ctx:
                problems.append(f"`ctx.` doesn't offer {name!r} (got {sorted(ctx)[:12]}…)")
        if "publish" in ctx and not documented(server, ctx["publish"]):
            problems.append("`ctx.publish` completes without its docs")
        entity = completions(server, uri, ENTITY_LINE, len("    entity."))
        for name in ("transform", "animation", "attributes", "character_velocity"):
            if name not in entity:
                problems.append(f"`entity.` doesn't offer {name!r}")

        deadline = time.time() + 60
        flagged = False
        while time.time() < deadline and not flagged:
            for message in server.notifications:
                if message.get("method") == "textDocument/publishDiagnostics" \
                        and message["params"]["uri"] == uri:
                    flagged = any("animaton" in d.get("message", "") for d in message["params"]["diagnostics"])
            if not flagged:
                server.notifications.clear()
                try:
                    server.notifications.append(server.read())
                except RuntimeError:
                    break
        if not flagged:
            problems.append("the misspelt `entity.animaton` isn't flagged")
    finally:
        script.unlink(missing_ok=True)
        try:
            server.request("shutdown", None)
            server.notify("exit", None)
        except Exception:  # noqa: BLE001 - it's going away anyway
            pass
        server.proc.kill()

    if problems:
        print("completion check FAILED:")
        for p in problems:
            print(f"  - {p}")
        sys.exit(1)
    print(f"completion check passed: ctx. offers {len(ctx)} names with docs, "
          f"entity. {len(entity)}, and the typo is flagged")


if __name__ == "__main__":
    main()
