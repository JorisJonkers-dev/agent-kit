"""MCP servers whose tools Claude Code runs without a prompt.

A server with ``claude_allow_tools: true`` gets ``mcp__<server>__*`` in each
Claude profile's ``permissions.allow``. Setup joins the rules to the user's own
and changes nothing else, the same way it joins the private-bank deny rules.
"""

from __future__ import annotations

import copy
import json
import os
import subprocess
import sys
from pathlib import Path

import pytest

KIT_ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(KIT_ROOT / "scripts"))

import render_registry  # noqa: E402
import yaml  # noqa: E402


def _registry() -> dict:
    return yaml.safe_load((KIT_ROOT / "registry" / "estate-tooling.yaml").read_text())


def _fn(script: str, name: str) -> str:
    return f"{name}() {{" + script.split(f"\n{name}() {{")[1].split("\n}\n")[0] + "\n}\n"


def _server(data: dict, name: str) -> dict:
    return next(s for s in data["mcp_servers"] if s["name"] == name)


def _merge(tmp_path: Path, settings: dict | None, check: bool = False) -> Path:
    script = render_registry.render_setup_script(_registry())
    profile = tmp_path / "claude"
    profile.mkdir(exist_ok=True)
    path = profile / "settings.json"
    if settings is not None:
        path.write_text(json.dumps(settings))
    body = (
        f"set -u\nCHECK_ONLY={int(check)}\nCLAUDE_PROFILE_DIRS=({profile})\n"
        "log() { :; }\nok() { :; }\nwarn() { echo \"warn $*\" >&2; }\nfail() { echo \"fail $*\" >&2; }\n"
        + _fn(script, "claude_mcp_allows") + "claude_mcp_allows\n"
    )
    result = subprocess.run(["/bin/bash", "-c", body], capture_output=True, text=True, check=False,
                            env={**os.environ})
    assert result.returncode == 0 and not result.stderr, result.stderr
    return path


def test_overleaf_tools_are_allowed() -> None:
    assert "mcp__overleaf__*" in render_registry.claude_allow_rules(_registry())


def test_an_unflagged_server_gets_no_rule() -> None:
    data = copy.deepcopy(_registry())
    _server(data, "overleaf")["claude_allow_tools"] = False
    assert "mcp__overleaf__*" not in render_registry.claude_allow_rules(data)


def test_the_rules_join_the_users_own_and_change_nothing_else(tmp_path: Path) -> None:
    mine = {"model": "opus", "permissions": {"allow": ["Bash(ls)"], "deny": ["Bash(rm -rf:*)"]}}
    data = json.loads(_merge(tmp_path, mine).read_text())
    assert data["model"] == "opus"
    assert data["permissions"]["deny"] == ["Bash(rm -rf:*)"]
    assert data["permissions"]["allow"] == ["Bash(ls)", *render_registry.claude_allow_rules(_registry())]


def test_a_missing_settings_file_is_created(tmp_path: Path) -> None:
    data = json.loads(_merge(tmp_path, None).read_text())
    assert data == {"permissions": {"allow": render_registry.claude_allow_rules(_registry())}}


def test_a_second_run_changes_nothing(tmp_path: Path) -> None:
    path = _merge(tmp_path, {})
    once = path.read_text()
    _merge(tmp_path, None)
    assert path.read_text() == once


def test_check_mode_writes_nothing(tmp_path: Path) -> None:
    assert not _merge(tmp_path, None, check=True).exists()


def test_setup_applies_them_after_the_profiles_exist() -> None:
    script = render_registry.render_setup_script(_registry())
    main = script.split(_fn(script, "claude_mcp_allows"))[1]
    assert main.index('log "claude profiles"') < main.index("\nclaude_mcp_allows\n")


def test_claude_allow_tools_must_be_a_bool() -> None:
    data = copy.deepcopy(_registry())
    _server(data, "overleaf")["claude_allow_tools"] = "yes"
    with pytest.raises(render_registry.RegistryError, match="claude_allow_tools must be true or false"):
        render_registry.validate(data)


def test_claude_allow_tools_needs_the_workstation_surface() -> None:
    data = copy.deepcopy(_registry())
    _server(data, "overleaf")["surfaces"] = ["hermes"]
    with pytest.raises(render_registry.RegistryError, match="needs the workstation surface"):
        render_registry.validate(data)
