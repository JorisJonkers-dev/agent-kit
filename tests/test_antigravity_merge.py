"""Antigravity's MCP merge keeps the user's own servers and is idempotent.

The merge runs as a subprocess against a temporary config, the way setup calls
it, so the assertions are about the file on disk rather than about internals.
"""

from __future__ import annotations

import json
import subprocess
import sys
from pathlib import Path
from typing import Any

KIT_ROOT = Path(__file__).resolve().parent.parent
MERGE = KIT_ROOT / "scripts" / "antigravity-merge-mcp.py"
FRAGMENT = KIT_ROOT / "registry" / "generated" / "antigravity" / "mcp_config.json"


def _merge(config: Path, *extra: str) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [sys.executable, str(MERGE), str(FRAGMENT), "--config", str(config), *extra],
        capture_output=True,
        text=True,
        check=False,
    )


def _servers(config: Path) -> dict[str, Any]:
    return json.loads(config.read_text())["mcpServers"]


def test_merge_keeps_the_users_servers_and_settings(tmp_path: Path) -> None:
    config = tmp_path / "mcp_config.json"
    config.write_text(
        json.dumps(
            {
                "theme": "dark",
                "mcpServers": {
                    "mine": {"command": "my-mcp", "args": []},
                    "memory-api": {"serverUrl": "https://stale.example/mcp/"},
                },
            },
        ),
    )
    result = _merge(config)
    assert result.returncode == 0, result.stdout + result.stderr
    data = json.loads(config.read_text())
    assert data["theme"] == "dark"
    servers = data["mcpServers"]
    assert servers["mine"] == {"command": "my-mcp", "args": []}
    assert servers["memory-api"] == json.loads(FRAGMENT.read_text())["mcpServers"]["memory-api"]


def test_a_second_run_changes_nothing(tmp_path: Path) -> None:
    config = tmp_path / "mcp_config.json"
    config.write_text(json.dumps({"mcpServers": {"mine": {"command": "my-mcp"}}}))
    assert _merge(config).returncode == 0
    first = config.read_bytes()
    assert _merge(config).returncode == 0
    assert config.read_bytes() == first


def test_the_first_backup_is_kept(tmp_path: Path) -> None:
    config = tmp_path / "mcp_config.json"
    original = json.dumps({"mcpServers": {"mine": {"command": "my-mcp"}}})
    config.write_text(original)
    assert _merge(config).returncode == 0
    backup = tmp_path / "mcp_config.json.bak"
    assert backup.read_text() == original

    config.write_text(json.dumps({"mcpServers": {"mine": {"command": "changed"}}}))
    assert _merge(config).returncode == 0
    assert backup.read_text() == original


def test_remove_drops_a_server(tmp_path: Path) -> None:
    config = tmp_path / "mcp_config.json"
    config.write_text(json.dumps({"mcpServers": {"retired": {"command": "old-mcp"}, "mine": {"command": "my-mcp"}}}))
    result = _merge(config, "--remove", "retired", "--remove", "drawio")
    assert result.returncode == 0, result.stdout + result.stderr
    servers = _servers(config)
    assert "retired" not in servers
    assert "drawio" not in servers
    assert servers["mine"] == {"command": "my-mcp"}


def test_a_missing_config_file_is_created(tmp_path: Path) -> None:
    config = tmp_path / "gemini" / "config" / "mcp_config.json"
    result = _merge(config)
    assert result.returncode == 0, result.stdout + result.stderr
    assert _servers(config) == json.loads(FRAGMENT.read_text())["mcpServers"]
    assert not (config.parent / "mcp_config.json.bak").exists()
