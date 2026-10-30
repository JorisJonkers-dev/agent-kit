#!/usr/bin/env python3
"""Merge a rendered ``mcpServers`` fragment into Antigravity's global MCP config.

Antigravity reads its MCP servers from ``~/.gemini/config/mcp_config.json`` and
has no ``mcp add`` command, so the setup script merges the generated fragment
(``registry/generated/antigravity/mcp_config.json``) here instead. A server
named in the fragment replaces any entry of that name; every other server and
every other top-level key in the file is kept. The result is read back and
compared, so the caller can trust the merge landed rather than the exit code.

Usage::

    python scripts/antigravity-merge-mcp.py <fragment.json> [--config PATH] [--remove NAME ...]
"""

from __future__ import annotations

import argparse
import json
import shutil
import sys
from pathlib import Path
from typing import Any


def _default_config() -> Path:
    return Path.home() / ".gemini" / "config" / "mcp_config.json"


def merge(fragment: Path, config: Path, remove: list[str]) -> int:
    wanted: dict[str, Any] = json.loads(fragment.read_text()).get("mcpServers") or {}
    dropped = set(remove)

    doc: dict[str, Any] = json.loads(config.read_text()) if config.is_file() else {}
    current: dict[str, Any] = doc.get("mcpServers") or {}
    # Rebuilt in the file's own order, so a re-run writes byte-identical output.
    merged = {name: entry for name, entry in current.items() if name not in wanted and name not in dropped}
    merged.update({name: entry for name, entry in wanted.items() if name not in dropped})
    doc["mcpServers"] = merged

    # The first backup is the user's own config from before agent-kit; keep it.
    backup = config.with_name(config.name + ".bak")
    if config.is_file() and not backup.exists():
        shutil.copy2(config, backup)
    config.parent.mkdir(parents=True, exist_ok=True)
    config.write_text(json.dumps(doc, indent=2) + "\n")

    back = json.loads(config.read_text()).get("mcpServers") or {}
    missing = [name for name in wanted if name not in dropped and back.get(name) != wanted[name]]
    lingering = sorted(dropped & set(back))
    if missing or lingering:
        print(f"antigravity-merge-mcp: FAILED, not as written: {missing}, still present: {lingering}",
              file=sys.stderr)
        return 1
    print(f"antigravity-merge-mcp: {len(set(wanted) - dropped)} managed servers in {config}")
    return 0


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Merge rendered Antigravity MCP servers into its config")
    parser.add_argument("fragment", type=Path)
    parser.add_argument("--config", type=Path, default=None)
    parser.add_argument("--remove", action="append", default=[], metavar="NAME")
    args = parser.parse_args(argv)
    return merge(args.fragment, args.config or _default_config(), args.remove)


if __name__ == "__main__":
    raise SystemExit(main())
