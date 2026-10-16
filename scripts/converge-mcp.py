#!/usr/bin/env python3
"""Converge the MCP servers of every agent surface to the registry and the keep file.

A surface is one Claude profile's ``.claude.json``, Codex's ``config.toml`` or
Vibe's ``config.toml``. The setup script renders the declared and retired names
from the registry. A retired server is removed; any other server the surface's
declared set does not name is removed unless the keep file names it.

``--check`` prints each removal and changes nothing. A normal run removes the
servers, reads every surface again and fails if a removed name is still there.

Usage::

    uv run --no-project --with pyyaml --with tomlkit python scripts/converge-mcp.py \\
        --keep PATH --claude-home DIR [--claude DIR ...] [--codex PATH] [--vibe PATH] \\
        --declared SURFACE=NAME,... --retired NAME,... [--check]
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import tomllib
from dataclasses import dataclass, field
from pathlib import Path

import tomlkit
import yaml

DECLARED_SOURCES = ("plugin:",)


@dataclass
class Surface:
    label: str
    kind: str
    path: Path
    env: dict[str, str] = field(default_factory=dict)

    def names(self) -> set[str]:
        if not self.path.is_file():
            return set()
        if self.kind == "claude":
            return set(json.loads(self.path.read_text()).get("mcpServers") or {})
        data = tomllib.loads(self.path.read_text())
        if self.kind == "codex":
            return set(data.get("mcp_servers") or {})
        return {str(s.get("name")) for s in data.get("mcp_servers") or []}

    def remove(self, name: str) -> None:
        if self.kind == "claude":
            cmd = ["claude", "mcp", "remove", "--scope", "user", name]
            subprocess.run(cmd, env=self.env, capture_output=True, check=False)
        elif self.kind == "codex":
            subprocess.run(["codex", "mcp", "remove", name], capture_output=True, check=False)
        else:
            doc = tomlkit.parse(self.path.read_text())
            kept = tomlkit.aot()
            for server in doc.get("mcp_servers") or []:
                if str(server.get("name")) != name:
                    kept.append(server)
            del doc["mcp_servers"]
            if len(kept):
                doc["mcp_servers"] = kept
            self.path.write_text(tomlkit.dumps(doc))


def _tilde(path: Path) -> str:
    home = Path.home()
    return f"~/{path.relative_to(home)}" if path.is_relative_to(home) else str(path)


def _claude(profile: Path, claude_home: Path) -> Surface:
    env = dict(os.environ)
    # The primary profile without CLAUDE_CONFIG_DIR keeps .claude.json in HOME.
    if profile == claude_home and not env.get("CLAUDE_CONFIG_DIR"):
        env.pop("CLAUDE_CONFIG_DIR", None)
        path = Path.home() / ".claude.json"
    else:
        env["CLAUDE_CONFIG_DIR"] = str(profile)
        path = profile / ".claude.json"
    return Surface(f"claude {_tilde(profile)}", "claude", path, env)


def _keep(path: Path) -> set[str]:
    if not path.is_file():
        return set()
    data = yaml.safe_load(path.read_text()) or {}
    keep = data.get("keep") if isinstance(data, dict) else None
    if keep is None:
        return set()
    if not isinstance(keep, list) or not all(isinstance(n, str) for n in keep):
        raise SystemExit(f"converge-mcp: {path} must hold a top-level `keep:` list of names; nothing removed")
    return set(keep)


def _names(value: str) -> set[str]:
    return {n for n in value.split(",") if n}


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Converge MCP servers to the registry and the keep file")
    parser.add_argument("--keep", type=Path, required=True)
    parser.add_argument("--claude-home", type=Path, required=True)
    parser.add_argument("--claude", type=Path, action="append", default=[])
    parser.add_argument("--codex", type=Path)
    parser.add_argument("--vibe", type=Path)
    parser.add_argument("--declared", action="append", default=[], metavar="SURFACE=NAME,...")
    parser.add_argument("--retired", default="")
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args(argv)

    keep = _keep(args.keep)
    retired = _names(args.retired)
    declared = {kind: _names(names) for kind, _, names in (d.partition("=") for d in args.declared)}

    surfaces = [_claude(p, args.claude_home) for p in args.claude]
    if args.codex:
        surfaces.append(Surface("codex", "codex", args.codex))
    if args.vibe:
        surfaces.append(Surface("vibe", "vibe", args.vibe))

    removals: list[tuple[Surface, str, str]] = []
    for surface in surfaces:
        for name in sorted(surface.names()):
            if name in retired:
                removals.append((surface, name, "retired"))
            elif name not in declared.get(surface.kind, set()) and not name.startswith(DECLARED_SOURCES) \
                    and name not in keep:
                removals.append((surface, name, "undeclared"))

    for surface, name, reason in removals:
        if args.check:
            print(f"would remove {name} ({surface.label}, {reason})")
        else:
            surface.remove(name)
            print(f"removed {name} ({surface.label}, {reason})")

    lingering = [(s, n) for s, n, _ in removals if not args.check and n in s.names()]
    for surface, name in lingering:
        print(f"{name} is still present ({surface.label})")
    return 1 if lingering else 0


if __name__ == "__main__":
    sys.exit(main())
