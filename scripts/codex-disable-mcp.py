#!/usr/bin/env python3
"""Switch off one Codex MCP server, leaving it registered.

Codex has ``mcp add`` and ``mcp remove`` but no way to disable a server, so the
setup script sets ``enabled = false`` on its ``[mcp_servers.<name>]`` table
here. Codex then lists it as disabled, and ``codex -c
mcp_servers.<name>.enabled=true`` turns it on for one run. Everything else in
the file is kept, and the value is read back before reporting success.

Usage::

    uv run --no-project --with tomlkit python scripts/codex-disable-mcp.py <config.toml> <name>
"""

from __future__ import annotations

import sys
from pathlib import Path

import tomlkit


def disable(config: Path, name: str) -> int:
    doc = tomlkit.parse(config.read_text()) if config.is_file() else tomlkit.document()
    server = (doc.get("mcp_servers") or {}).get(name)
    if server is None:
        print(f"codex-disable-mcp: {config} has no [mcp_servers.{name}]", file=sys.stderr)
        return 1
    if server.get("enabled") is not False:
        server["enabled"] = False
        config.write_text(tomlkit.dumps(doc))
    back = tomlkit.parse(config.read_text())["mcp_servers"][name].get("enabled")
    if back is not False:
        print(f"codex-disable-mcp: {name} did not read back as disabled", file=sys.stderr)
        return 1
    return 0


def main(argv: list[str]) -> int:
    if len(argv) != 2:
        print("usage: codex-disable-mcp.py <config.toml> <name>", file=sys.stderr)
        return 2
    return disable(Path(argv[0]), argv[1])


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
