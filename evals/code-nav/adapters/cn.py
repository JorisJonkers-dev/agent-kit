#!/usr/bin/env python3
"""Command-line front for the code-nav MCP server, for agents with a shell.

usage: cn.py <search|outline|expand|map|symbols|related_repos> [args]
  search <query> [path]      outline <path>      expand <path> <symbol>
  map [path]                 symbols <name>      related_repos <repo>
Environment: CN_BIN (the code-nav binary), CN_ROOT (its one allowed root),
CN_ALLOW (comma-separated subcommands this layer may use).
"""

import json
import os
import subprocess
import sys

ARGS = {
    "search": ("query", "path"),
    "outline": ("path",),
    "expand": ("path", "symbol"),
    "map": ("path",),
    "symbols": ("name",),
    "related_repos": ("repo",),
}


def main(argv):
    if not argv or argv[0] not in ARGS:
        print(__doc__)
        return 2
    tool, vals = argv[0], argv[1:]
    allow = [a for a in os.environ.get("CN_ALLOW", "").split(",") if a]
    if tool not in allow:
        print(f"cn: '{tool}' is not available here; available: {', '.join(allow)}")
        return 2
    names = ARGS[tool]
    arguments = {}
    for name, val in zip(names, vals, strict=False):
        if name == "path" and not val.startswith("/"):
            val = os.path.join(os.environ["CN_ROOT"], val)
        arguments[name] = val
    msgs = [
        {
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-03-26",
                "capabilities": {},
                "clientInfo": {"name": "cn", "version": "1"},
            },
        },
        {"jsonrpc": "2.0", "method": "notifications/initialized"},
        {"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {"name": tool, "arguments": arguments}},
    ]
    try:
        p = subprocess.run(
            [os.environ["CN_BIN"], "--root", os.environ["CN_ROOT"]],
            input="\n".join(json.dumps(m) for m in msgs) + "\n",
            capture_output=True,
            text=True,
            timeout=60,
        )
    except subprocess.TimeoutExpired:
        print("cn: code-nav timed out")
        return 1
    for line in p.stdout.splitlines():
        try:
            r = json.loads(line)
        except ValueError:
            continue
        if r.get("id") == 2:
            res = r.get("result") or {}
            text = "".join(c.get("text", "") for c in res.get("content", []))
            print(text or json.dumps(r.get("error")))
            return 1 if res.get("isError") or r.get("error") else 0
    print(p.stderr)
    return 1


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
