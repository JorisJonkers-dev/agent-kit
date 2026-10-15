#!/usr/bin/env python3
"""Merge a rendered ``[[mcp_servers]]`` or ``[[hooks]]`` fragment into Mistral Vibe's config.

Vibe reads its MCP servers from ``~/.vibe/config.toml`` (``$VIBE_HOME``) and has
no ``mcp add`` command, so the setup script merges the generated fragment
(``registry/generated/vibe/mcp-servers.toml``) here instead. A server named in
the fragment replaces any entry of that name; every other server, setting and
comment in the file is kept. ``--key hooks`` does the same for the
``[[hooks]]`` in ``hooks.toml``. The result is read back and compared, so the
caller can trust the merge landed rather than the exit code.

Usage::

    uv run --no-project --with tomlkit python scripts/vibe-merge-mcp.py \\
        <fragment.toml> [--key mcp_servers|hooks] [--config PATH] [--remove NAME ...]
"""

from __future__ import annotations

import argparse
import os
import shutil
import sys
from pathlib import Path

import tomlkit


def _default_config() -> Path:
    return Path(os.environ.get("VIBE_HOME") or Path.home() / ".vibe") / "config.toml"


def merge(fragment: Path, config: Path, remove: list[str], key: str = "mcp_servers") -> int:
    wanted = tomlkit.parse(fragment.read_text()).get(key, [])
    managed = {str(s["name"]) for s in wanted}
    dropped = set(remove)

    doc = tomlkit.parse(config.read_text()) if config.is_file() else tomlkit.document()
    current = doc.get(key)
    kept = [s for s in (current or []) if str(s.get("name")) not in managed | dropped]

    # Rebuilt from plain values: entries keep no whitespace trivia, so a re-run
    # writes byte-identical output instead of growing blank lines.
    servers = tomlkit.aot()
    for entry in [*kept, *(w for w in wanted if str(w["name"]) not in dropped)]:
        servers.append(tomlkit.item(entry.unwrap()))
    if key in doc:
        del doc[key]
    if len(servers):
        doc[key] = servers

    # The first backup is the user's own config from before agent-kit; keep it.
    backup = config.with_name(config.name + ".bak")
    if config.is_file() and not backup.exists():
        shutil.copy2(config, backup)
    config.parent.mkdir(parents=True, exist_ok=True)
    config.write_text(tomlkit.dumps(doc).lstrip("\n"))

    back = {str(s["name"]): s for s in tomlkit.parse(config.read_text()).get(key, [])}
    missing = [str(s["name"]) for s in wanted if str(s["name"]) not in dropped and back.get(str(s["name"])) != s]
    lingering = sorted(dropped & set(back))
    if missing or lingering:
        print(f"vibe-merge-mcp: FAILED, not as written: {missing}, still present: {lingering}", file=sys.stderr)
        return 1
    print(f"vibe-merge-mcp: {len(managed - dropped)} managed {key} entries in {config}")
    return 0


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Merge rendered Vibe MCP servers or hooks into its config")
    parser.add_argument("fragment", type=Path)
    parser.add_argument("--key", choices=["mcp_servers", "hooks"], default="mcp_servers")
    parser.add_argument("--config", type=Path, default=None)
    parser.add_argument("--remove", action="append", default=[], metavar="NAME")
    args = parser.parse_args(argv)
    config = args.config or _default_config().with_name("hooks.toml" if args.key == "hooks" else "config.toml")
    return merge(args.fragment, config, args.remove, args.key)


if __name__ == "__main__":
    raise SystemExit(main())
