#!/usr/bin/env python3
"""Merge a rendered MCP fragment into OpenCode 2's global ``opencode.json``.

OpenCode 2 reads its MCP servers from ``mcp.servers`` in
``~/.config/opencode/opencode.json``. ``opencode mcp add`` writes one server at
a time, so the setup script merges the generated fragment
(``registry/generated/opencode/opencode.json``) here instead, in one pass that
also drops retired servers. A server named in the fragment replaces any entry
of that name, except that an ``enabled`` flag the user set on it is kept; every
other server, every other key under ``mcp`` and every other top-level key in
the file is kept. The result is read back and compared, so the caller can trust
the merge landed rather than the exit code.

Usage::

    python scripts/opencode-merge-mcp.py <fragment.json> [--config PATH] [--remove NAME ...]
"""

from __future__ import annotations

import argparse
import json
import os
import shutil
import sys
from pathlib import Path
from typing import Any

# Keys of a managed server that are the user's to set.
USER_KEYS = ("enabled",)
SCHEMA = "https://opencode.ai/config.json"


def _default_config() -> Path:
    base = os.environ.get("XDG_CONFIG_HOME") or str(Path.home() / ".config")
    return Path(base) / "opencode" / "opencode.json"


def _managed(entry: dict[str, Any], current: Any) -> dict[str, Any]:
    """The fragment's entry, carrying over what the user set on the one it replaces."""
    if not isinstance(current, dict):
        return entry
    return {**entry, **{key: current[key] for key in USER_KEYS if key in current}}


def _servers(doc: dict[str, Any]) -> dict[str, Any]:
    mcp = doc.get("mcp")
    servers = mcp.get("servers") if isinstance(mcp, dict) else None
    return servers if isinstance(servers, dict) else {}


def merge(fragment: Path, config: Path, remove: list[str]) -> int:
    wanted = _servers(json.loads(fragment.read_text()))
    dropped = set(remove)

    text = config.read_text() if config.is_file() else ""
    try:
        doc: dict[str, Any] = json.loads(text) if text.strip() else {}
    except json.JSONDecodeError as error:
        # opencode.json may hold comments when someone wrote it by hand; a merge
        # that rewrote it would drop them, so it stops instead.
        print(f"opencode-merge-mcp: FAILED, {config} is not plain JSON ({error}); nothing changed",
              file=sys.stderr)
        return 1
    current = _servers(doc)
    managed = {name: _managed(entry, current.get(name)) for name, entry in wanted.items() if name not in dropped}
    # Rebuilt in the file's own order, so a re-run writes byte-identical output.
    merged = {name: entry for name, entry in current.items() if name not in wanted and name not in dropped}
    merged.update(managed)
    block = doc.get("mcp")
    mcp: dict[str, Any] = block if isinstance(block, dict) else {}
    doc["mcp"] = {**mcp, "servers": merged}
    doc.setdefault("$schema", SCHEMA)

    # The backup is the user's own config from before agent-kit, so it is taken
    # only from a file no merge has touched (none of the managed servers in it),
    # and never overwritten.
    backup = config.with_name(config.name + ".bak")
    untouched = not set(wanted) & set(current)
    if config.is_file() and untouched and not backup.exists():
        shutil.copy2(config, backup)
    config.parent.mkdir(parents=True, exist_ok=True)
    # Written beside it and renamed, so a starting OpenCode never reads half a file.
    staged = config.with_name(config.name + ".agent-kit-tmp")
    staged.write_text(json.dumps(doc, indent=2) + "\n")
    os.replace(staged, config)

    back = _servers(json.loads(config.read_text()))
    missing = [name for name, entry in managed.items() if back.get(name) != entry]
    lingering = sorted(dropped & set(back))
    if missing or lingering:
        print(f"opencode-merge-mcp: FAILED, not as written: {missing}, still present: {lingering}",
              file=sys.stderr)
        return 1
    print(f"opencode-merge-mcp: {len(managed)} managed servers in {config}")
    return 0


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Merge rendered OpenCode MCP servers into its config")
    parser.add_argument("fragment", type=Path)
    parser.add_argument("--config", type=Path, default=None)
    parser.add_argument("--remove", action="append", default=[], metavar="NAME")
    args = parser.parse_args(argv)
    return merge(args.fragment, args.config or _default_config(), args.remove)


if __name__ == "__main__":
    raise SystemExit(main())
