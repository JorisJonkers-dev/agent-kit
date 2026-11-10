"""memory-private MCP: Private Memory, the private Hindsight bank.

Only vibe-private has it, in its own Vibe home, behind the passphrase gate.
The shared Vibe home (vibe, vibe-study), Claude Code, Codex, Hermes, runners
and cloud sessions never get it; an upgrade removes what earlier versions
installed for Claude Code and Codex.
"""

from __future__ import annotations

import copy
import os
import subprocess
import sys
import tomllib
from pathlib import Path

import pytest

KIT_ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(KIT_ROOT / "scripts"))

import render_registry  # noqa: E402

NAME = "memory-private"
CMD = KIT_ROOT / "installer" / "memory-private"


def _data() -> dict:
    return render_registry.load_registry()


def _server(data: dict) -> dict:
    return next(s for s in data["mcp_servers"] if s["name"] == NAME)


def test_registry_points_at_the_private_bank_on_the_workstation_only() -> None:
    server = _server(_data())
    assert server["url_workstation"] == "https://hindsight-private.jorisjonkers.dev/mcp/private/"
    assert server["credential"] == "MEMORY_PRIVATE_TOKEN"
    assert server["session_only"] is True
    assert server["surfaces"] == ["workstation"]
    assert server["cloud"] is False
    assert server["enabled_for"] == ["vibe-private"]


def test_no_hermes_or_container_output_carries_it() -> None:
    data = _data()
    for text in (
        render_registry.render_hermes_mcp(data),
        render_registry.render_hermes_local_mcp(data),
        render_registry.render_container_setup_script(data),
    ):
        assert "hindsight-private" not in text
        assert NAME not in text
        assert "vibe-private" not in text
        assert "vibe-study" not in text


def test_the_shared_vibe_home_never_hears_of_private_memory() -> None:
    fragment = render_registry.render_vibe_mcp(_data())
    assert NAME not in fragment
    assert "hindsight-private" not in fragment
    assert "MEMORY_PRIVATE_TOKEN" not in fragment


def test_vibe_private_has_private_memory_on_with_the_token_named_never_held() -> None:
    fragment = tomllib.loads(render_registry.render_vibe_private_mcp(_data()))
    server = next(s for s in fragment["mcp_servers"] if s["name"] == NAME)
    assert server["transport"] == "streamable-http"
    assert server["url"] == "https://hindsight-private.jorisjonkers.dev/mcp/private/"
    assert server["auth"] == {
        "type": "static",
        "api_key_env": "MEMORY_PRIVATE_TOKEN",
        "api_key_header": "Authorization",
        "api_key_format": "Bearer {token}",
    }
    assert server.get("disabled", False) is False


def test_vibe_private_also_has_shared_memory() -> None:
    fragment = tomllib.loads(render_registry.render_vibe_private_mcp(_data()))
    names = {s["name"] for s in fragment["mcp_servers"]}
    assert "memory-api" in names


def test_vibe_private_has_the_shared_fleet_plus_private_memory_only() -> None:
    data = _data()
    shared = {s["name"] for s in tomllib.loads(render_registry.render_vibe_mcp(data))["mcp_servers"]}
    private = {s["name"] for s in tomllib.loads(render_registry.render_vibe_private_mcp(data))["mcp_servers"]}
    assert private - shared == {NAME}
    assert shared <= private


def test_without_enabled_for_vibe_private_stays_out_of_every_vibe_home() -> None:
    data = copy.deepcopy(_data())
    del _server(data)["enabled_for"]
    assert NAME not in render_registry.render_vibe_mcp(data)
    assert NAME not in render_registry.render_vibe_private_mcp(data)


def test_the_private_fragment_is_a_generated_artifact() -> None:
    assert Path("registry/generated/vibe-private/mcp-servers.toml") in render_registry.artifacts(_data())


def _merge_line(script: str, fragment: str) -> str:
    lines = script.splitlines()
    i = next(i for i, line in enumerate(lines) if fragment in line and "--config" in line)
    return lines[i]


def test_setup_strips_private_memory_from_an_upgraded_shared_vibe_home() -> None:
    script = render_registry.render_setup_script(_data())
    line = _merge_line(script, '"${KIT_ROOT}/registry/generated/vibe/mcp-servers.toml"')
    assert '--config "${VIBE_HOME}/config.toml"' in line
    assert f"--remove {NAME}" in line


def test_setup_merges_private_memory_into_vibe_privates_home() -> None:
    script = render_registry.render_setup_script(_data())
    line = _merge_line(script, '"${KIT_ROOT}/registry/generated/vibe-private/mcp-servers.toml"')
    assert '--config "${VIBE_PRIVATE_HOME}/config.toml"' in line
    assert f"--remove {NAME}" not in line


def test_vibe_privates_home_is_workstation_only_never_built_in_a_cloud_run() -> None:
    script = render_registry.render_setup_script(_data())
    assert '      [ "${CLOUD}" = 1 ] || _vibe_private_home' in script.splitlines()
    assert 'VIBE_PRIVATE_HOME="${HOME}/.vibe-private"' in script.splitlines()


def test_claude_never_gets_private_memory_and_an_upgrade_removes_it() -> None:
    script = render_registry.render_setup_script(_data())
    assert f"claude mcp add --scope user {NAME}" not in script
    assert f'"claude mcp add {NAME}"' not in script
    assert f"claude mcp remove --scope user {NAME}" in script


def test_codex_never_gets_private_memory_and_an_upgrade_removes_it() -> None:
    script = render_registry.render_setup_script(_data())
    assert f"codex mcp add {NAME}" not in script
    assert f"run codex mcp remove {NAME}" in script
    assert "codex-disable-mcp.py" not in script


def test_an_upgrade_deletes_the_retired_claude_session_config() -> None:
    script = render_registry.render_setup_script(_data())
    assert "install_session_mcp" not in script
    assert f'rm -f "${{XDG_CONFIG_HOME:-${{HOME}}/.config}}/agent-kit/{NAME}.mcp.json"' in script


def test_no_claude_session_config_is_generated_any_more() -> None:
    assert not [p for p in render_registry.artifacts(_data()) if "generated/claude" in str(p)]


def test_neither_claude_nor_codex_declares_private_memory_so_converge_removes_it() -> None:
    data = _data()
    assert NAME not in render_registry.declared_servers(data, "claude")
    assert NAME not in render_registry.declared_servers(data, "codex")
    assert NAME not in render_registry.declared_servers(data, "vibe")
    assert NAME in render_registry.declared_servers(data, "vibe-private")


def test_converge_covers_vibe_privates_home() -> None:
    script = render_registry.render_setup_script(_data())
    assert '--vibe-private "${VIBE_PRIVATE_HOME}/config.toml"' in script
    declared = next(line for line in script.splitlines() if "--declared vibe-private=" in line)
    assert NAME in declared


def test_the_expected_claude_servers_leave_it_out() -> None:
    script = render_registry.render_setup_script(_data())
    expected = [line for line in script.splitlines() if "expected_servers" in line and "(" in line]
    assert expected and all(NAME not in line for line in expected)


@pytest.mark.parametrize(
    "change",
    [
        {"surfaces": ["workstation", "hermes"]},
        {"cloud": True},
        {"transport": "stdio", "command": "x"},
        {"enabled_for": ["claude"]},
        {"enabled_for": ["codex"]},
        {"enabled_for": ["hermes"]},
        {"enabled_for": ["vibe"]},
        {"enabled_for": ["vibe-study"]},
        {"enabled_for": ["vibe-private", "claude"]},
    ],
)
def test_validation_keeps_private_memory_on_the_workstation_and_in_vibe_private_alone(change: dict) -> None:
    data = copy.deepcopy(_data())
    _server(data).update(change)
    with pytest.raises(render_registry.RegistryError):
        render_registry.validate(data)


def test_validation_refuses_enabled_for_on_a_server_that_is_on_everywhere() -> None:
    data = copy.deepcopy(_data())
    next(s for s in data["mcp_servers"] if s["name"] == "memory-api")["enabled_for"] = ["vibe"]
    with pytest.raises(render_registry.RegistryError, match="enabled_for"):
        render_registry.validate(data)


def test_the_bundle_ships_vibe_privates_files_and_drops_the_retired_ones() -> None:
    publish = (KIT_ROOT / "scripts" / "publish-installer-artifacts.sh").read_text()
    for path in (
        "registry/generated/vibe-private/mcp-servers.toml",
        "installer/vibe-private",
        "installer/vibe-study",
        "scripts/vibe-split-homes.py",
    ):
        assert f"  {path}\n" in publish
    assert "registry/generated/claude/" not in publish
    assert "codex-disable-mcp.py" not in publish


# --- memory-private no longer opens a Claude or Codex session ---


def _stub(bin_dir: Path, name: str) -> None:
    path = bin_dir / name
    path.write_text(f'#!/usr/bin/env bash\nprintf "{name} ran\\n"\n')
    path.chmod(0o755)


@pytest.mark.parametrize("agent", ["claude", "codex"])
def test_memory_private_refuses_to_open_a_claude_or_codex_session_even_with_the_token(
    tmp_path: Path, agent: str,
) -> None:
    bin_dir = tmp_path / "bin"
    bin_dir.mkdir()
    _stub(bin_dir, agent)
    _stub(bin_dir, "agent-kit-private")
    env = {**os.environ, "PATH": f"{bin_dir}:{os.environ['PATH']}", "MEMORY_PRIVATE_TOKEN": "smt_x"}
    result = subprocess.run(["bash", str(CMD), agent], capture_output=True, text=True, check=False, env=env)
    assert result.returncode == 2
    assert "ran" not in result.stdout
    assert "usage: memory-private on|off|status [DIR]" in result.stderr
