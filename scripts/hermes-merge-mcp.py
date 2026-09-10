#!/usr/bin/env python3
"""Merge a rendered ``mcp_servers:`` fragment into the local Hermes config.

Hermes reads its MCP servers from ``~/.hermes/config.yaml`` under the
``mcp_servers`` key (the same schema the gateway's ``mcp-servers.yaml`` uses).
Running ``hermes mcp add`` is interactive -- it probes the server, lists its
tools and prompts to enable/select -- so an idempotent setup script cannot use
it. This helper does what ``hermes mcp add`` does to the config file, without
the interaction: merge a workstation-flavoured ``mcp_servers:`` block into the
target config, preserving every other key.

Usage::

    uv run python scripts/hermes-merge-mcp.py <fragment.yaml> [--config PATH]

``<fragment.yaml>`` is the generated, workstation-flavoured ``mcp_servers:``
block (see ``render_hermes_local_mcp`` in ``render_registry.py``). The target
defaults to ``$HERMES_CONFIG`` or ``~/.hermes/config.yaml`` and is backed up to
``<target>.bak`` before writing. The merged ``mcp_servers`` section is read
back and compared to the fragment's so the caller can trust the merge landed --
an exit code alone is not evidence the value is present.
"""

from __future__ import annotations

import argparse
import copy
import shutil
import sys
from pathlib import Path
from typing import Any

import yaml


def _load_config(path: Path) -> dict[str, Any]:
    if not path.is_file():
        return {}
    text = path.read_text()
    if not text.strip():
        return {}
    data = yaml.safe_load(text)
    return data if isinstance(data, dict) else {}


def merge_fragment(fragment: dict[str, Any], target: dict[str, Any]) -> dict[str, Any]:
    """Return a copy of ``target`` with the fragment's ``mcp_servers`` merged in."""
    result = copy.deepcopy(target)
    servers = result.setdefault("mcp_servers", {})
    if not isinstance(servers, dict):
        raise ValueError("existing config has a non-mapping 'mcp_servers' key")
    for name, entry in (fragment.get("mcp_servers") or {}).items():
        servers[name] = entry
    return result


def remove_names(names: list[str], target: dict[str, Any]) -> dict[str, Any]:
    """Return a copy of ``target`` with the named ``mcp_servers`` removed."""
    result = copy.deepcopy(target)
    servers = result.get("mcp_servers")
    if not isinstance(servers, dict):
        return result
    for name in names:
        servers.pop(name, None)
    return result


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("fragment", help="rendered mcp_servers fragment (.yaml)")
    parser.add_argument("--config", help="target config (default: $HERMES_CONFIG or ~/.hermes/config.yaml)")
    parser.add_argument("--remove", action="append", default=[], metavar="NAME",
                        help="remove this MCP server from the target config (repeatable)")
    args = parser.parse_args(argv)

    fragment_path = Path(args.fragment)
    if not fragment_path.is_file():
        print(f"fragment not found: {fragment_path}", file=sys.stderr)
        return 1
    fragment = yaml.safe_load(fragment_path.read_text())
    if not isinstance(fragment, dict) or "mcp_servers" not in fragment:
        print("fragment must be a mapping with an 'mcp_servers' key", file=sys.stderr)
        return 1

    config_path = Path(args.config) if args.config else _default_config_path()

    deployed = set((fragment.get("mcp_servers") or {}).keys())
    config_path.parent.mkdir(parents=True, exist_ok=True)
    target = _load_config(config_path)
    merged = remove_names(args.remove, target) if args.remove else merge_fragment(fragment, target)

    # Back up the previous config so a bad edit is reversible.
    if config_path.is_file():
        shutil.copy2(config_path, config_path.with_suffix(config_path.suffix + ".bak"))

    changed = merged != target
    config_path.write_text(
        yaml.safe_dump(merged, sort_keys=False, allow_unicode=True),
        encoding="utf-8",
    )

    # Verify the VALUE: read back and confirm every fragment server is present.
    reread = _load_config(config_path)
    if args.remove:
        present = set((reread.get("mcp_servers") or {}).keys())
        still = set(args.remove) & present
        if still:
            print(f"verification failed; still present after remove: {sorted(still)}", file=sys.stderr)
            return 1
        verb = "removed" if changed else "already absent"
        print(f"hermes: {verb} MCP server(s) {sorted(args.remove)} from {config_path}")
        return 0
    present = set((reread.get("mcp_servers") or {}).keys())
    missing = deployed - present
    if missing:
        print(f"verification failed; missing after merge: {sorted(missing)}", file=sys.stderr)
        return 1
    verb = "merged" if changed else "already present; no change"
    print(f"hermes: {verb} {len(deployed)} MCP server(s) into {config_path}")
    return 0


def _default_config_path() -> Path:
    import os

    env = os.environ.get("HERMES_CONFIG", "").strip()
    if env:
        return Path(env)
    return Path.home() / ".hermes" / "config.yaml"


if __name__ == "__main__":
    sys.exit(main())