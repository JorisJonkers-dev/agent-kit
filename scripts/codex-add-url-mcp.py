#!/usr/bin/env python3
"""Register one Codex MCP server by URL, without signing in.

``codex mcp add <name> --url <url>`` starts Codex's OAuth flow as soon as the
server advertises OAuth, and waits for a browser callback, so setup cannot use
it for an ``auth: oauth`` server. This writes ``[mcp_servers.<name>]`` with the
URL only and keeps everything else in the file; setup then checks that
``codex mcp get <name>`` finds it. The user signs in later with
``codex mcp login <name>``.

Usage::

    uv run --no-project --with tomlkit python scripts/codex-add-url-mcp.py <config.toml> <name> <url>
"""

from __future__ import annotations

import sys
from pathlib import Path

import tomlkit


def add(config: Path, name: str, url: str) -> int:
    doc = tomlkit.parse(config.read_text()) if config.is_file() else tomlkit.document()
    servers = doc.get("mcp_servers")
    if servers is None:
        servers = tomlkit.table(is_super_table=True)
        doc["mcp_servers"] = servers
    server = tomlkit.table()
    server["url"] = url
    servers[name] = server
    config.parent.mkdir(parents=True, exist_ok=True)
    config.write_text(tomlkit.dumps(doc))
    return 0


def main(argv: list[str]) -> int:
    if len(argv) != 3:
        print("usage: codex-add-url-mcp.py <config.toml> <name> <url>", file=sys.stderr)
        return 2
    return add(Path(argv[0]), argv[1], argv[2])


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
