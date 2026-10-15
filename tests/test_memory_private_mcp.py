"""memory-private MCP: the private Hindsight bank.

Vibe, the personal-notes agent, has it on. Claude Code and Codex have it
installed but switched off: Claude Code gets it from a --mcp-config file it
loads only when asked, Codex gets it with `enabled = false`.
`memory-private claude|codex` opens one session with it on. Hermes, runners
and cloud sessions never get it.
"""

from __future__ import annotations

import copy
import json
import os
import subprocess
import sys
import tomllib
from pathlib import Path

import pytest

KIT_ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(KIT_ROOT / "scripts"))

import render_registry  # noqa: E402
import tomlkit  # noqa: E402

NAME = "memory-private"
CMD = KIT_ROOT / "installer" / "memory-private"
DISABLE = KIT_ROOT / "scripts" / "codex-disable-mcp.py"


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
    assert server["enabled_for"] == ["vibe"]


def test_no_hermes_or_container_output_carries_it() -> None:
    data = _data()
    for text in (
        render_registry.render_hermes_mcp(data),
        render_registry.render_hermes_local_mcp(data),
        render_registry.render_container_setup_script(data),
    ):
        assert "hindsight-private" not in text
        assert NAME not in text


def test_vibe_has_it_on_with_the_private_token() -> None:
    fragment = tomllib.loads(render_registry.render_vibe_mcp(_data()))
    server = next(s for s in fragment["mcp_servers"] if s["name"] == NAME)
    assert server["transport"] == "streamable-http"
    assert server["url"] == "https://hindsight-private.jorisjonkers.dev/mcp/private/"
    assert server["auth"]["api_key_env"] == "MEMORY_PRIVATE_TOKEN"
    assert server.get("disabled", False) is False


def test_without_enabled_for_vibe_stays_out_of_vibe() -> None:
    data = copy.deepcopy(_data())
    del _server(data)["enabled_for"]
    assert NAME not in render_registry.render_vibe_mcp(data)


def test_a_cloud_run_takes_it_out_of_vibe() -> None:
    script = render_registry.render_setup_script(_data())
    removal = next(line for line in script.splitlines() if "vibe_remove=(" in line and "CLOUD" in line)
    assert f"--remove {NAME}" in removal


def test_claude_never_registers_it_at_user_scope() -> None:
    script = render_registry.render_setup_script(_data())
    assert f"claude mcp add --scope user {NAME}" not in script
    assert f'"claude mcp add {NAME}"' not in script
    assert f"claude mcp remove --scope user {NAME}" in script


def test_codex_registers_it_then_switches_it_off() -> None:
    script = render_registry.render_setup_script(_data())
    add = script.index(f"codex mcp add {NAME} --url https://hindsight-private.jorisjonkers.dev/mcp/private/")
    disable = script.index(f'codex-disable-mcp.py" "${{CODEX_HOME:-${{HOME}}/.codex}}/config.toml" {NAME}')
    assert add < disable


def test_setup_installs_the_claude_session_config() -> None:
    script = render_registry.render_setup_script(_data())
    assert f"install_session_mcp {NAME}" in script


def test_the_claude_session_config_names_the_token_but_never_holds_it() -> None:
    artifacts = render_registry.artifacts(_data())
    text, _ = artifacts[Path(f"registry/generated/claude/{NAME}.mcp.json")]
    server = json.loads(text)["mcpServers"][NAME]
    assert server == {
        "type": "http",
        "url": "https://hindsight-private.jorisjonkers.dev/mcp/private/",
        "headers": {"Authorization": "Bearer ${MEMORY_PRIVATE_TOKEN}"},
    }


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
        {"enabled_for": ["vibe", "hermes"]},
    ],
)
def test_validation_keeps_a_session_only_server_on_the_workstation(change: dict) -> None:
    data = copy.deepcopy(_data())
    _server(data).update(change)
    with pytest.raises(render_registry.RegistryError):
        render_registry.validate(data)


def test_validation_refuses_enabled_for_on_a_server_that_is_on_everywhere() -> None:
    data = copy.deepcopy(_data())
    next(s for s in data["mcp_servers"] if s["name"] == "memory-api")["enabled_for"] = ["vibe"]
    with pytest.raises(render_registry.RegistryError, match="enabled_for"):
        render_registry.validate(data)


def test_the_bundle_ships_the_new_files() -> None:
    publish = (KIT_ROOT / "scripts" / "publish-installer-artifacts.sh").read_text()
    assert f"registry/generated/claude/{NAME}.mcp.json" in publish
    assert "scripts/codex-disable-mcp.py" in publish


# --- codex-disable-mcp.py ---


def _disable(config: Path, name: str = NAME) -> subprocess.CompletedProcess:
    return subprocess.run(
        [sys.executable, str(DISABLE), str(config), name], capture_output=True, text=True, check=False,
    )


def test_disable_turns_off_only_the_named_server(tmp_path: Path) -> None:
    config = tmp_path / "config.toml"
    config.write_text(
        'model = "o3"\n\n[mcp_servers.memory-api]\nurl = "https://a/"\n\n'
        f'[mcp_servers.{NAME}]\nurl = "https://b/"\nbearer_token_env_var = "MEMORY_PRIVATE_TOKEN"\n',
    )
    assert _disable(config).returncode == 0
    doc = tomlkit.parse(config.read_text())
    assert doc["mcp_servers"][NAME]["enabled"] is False
    assert "enabled" not in doc["mcp_servers"]["memory-api"]
    assert doc["model"] == "o3"


def test_disable_is_byte_stable_on_a_second_run(tmp_path: Path) -> None:
    config = tmp_path / "config.toml"
    config.write_text(f'[mcp_servers.{NAME}]\nurl = "https://b/"\n')
    _disable(config)
    once = config.read_text()
    assert _disable(config).returncode == 0
    assert config.read_text() == once


def test_disable_fails_when_the_server_is_missing(tmp_path: Path) -> None:
    config = tmp_path / "config.toml"
    config.write_text('[mcp_servers.other]\nurl = "https://a/"\n')
    result = _disable(config)
    assert result.returncode != 0
    assert NAME in result.stderr


# --- memory-private claude|codex ---


def _stub(bin_dir: Path, name: str) -> None:
    path = bin_dir / name
    path.write_text(
        "#!/usr/bin/env bash\n"
        f'printf "{name}"; printf " %s" "$@"; printf "\\n"\n'
        'printf "DISABLED=%s\\n" "${HINDSIGHT_DISABLED:-unset}"\n',
    )
    path.chmod(0o755)


def _session(tmp_path: Path, *args: str, token: str | None = "smt_x") -> subprocess.CompletedProcess:
    bin_dir = tmp_path / "bin"
    bin_dir.mkdir(exist_ok=True)
    _stub(bin_dir, "claude")
    _stub(bin_dir, "codex")
    # Only the stubs here: a real agent-kit-private on this machine must not stand in.
    path = ":".join(p for p in os.environ["PATH"].split(":") if not Path(p, "agent-kit-private").exists())
    env = {**os.environ, "PATH": f"{bin_dir}:{path}", "XDG_CONFIG_HOME": str(tmp_path / "cfg")}
    env.pop("MEMORY_PRIVATE_TOKEN", None)
    if token is not None:
        env["MEMORY_PRIVATE_TOKEN"] = token
    return subprocess.run(["bash", str(CMD), *args], capture_output=True, text=True, check=False, env=env)


def test_claude_session_loads_the_private_config_and_leaves_the_shared_bank_alone(tmp_path: Path) -> None:
    config = tmp_path / "cfg" / "agent-kit" / f"{NAME}.mcp.json"
    config.parent.mkdir(parents=True)
    config.write_text("{}")
    # `=` forms: both flags are variadic, and a trailing prompt would become a value.
    result = _session(tmp_path, "claude", "--resume")
    assert result.returncode == 0, result.stderr
    assert result.stdout.splitlines() == [
        f"claude --mcp-config={config} --disallowedTools=mcp__memory-api,mcp__memory-mcp,mcp__hindsight --resume",
        "DISABLED=true",
    ]


def test_claude_session_refuses_without_the_installed_config(tmp_path: Path) -> None:
    result = _session(tmp_path, "claude")
    assert result.returncode != 0
    assert "setup-workstation" in result.stderr


def test_codex_session_switches_the_server_on_for_that_run_only(tmp_path: Path) -> None:
    result = _session(tmp_path, "codex", "exec", "hi")
    assert result.returncode == 0, result.stderr
    assert result.stdout.splitlines() == [
        f"codex -c mcp_servers.{NAME}.enabled=true -c mcp_servers.memory-api.enabled=false"
        " -c mcp_servers.memory-mcp.enabled=false -c mcp_servers.hindsight.enabled=false exec hi",
        "DISABLED=true",
    ]


@pytest.mark.parametrize("agent", ["claude", "codex"])
def test_a_session_needs_the_private_token(tmp_path: Path, agent: str) -> None:
    result = _session(tmp_path, agent, token=None)
    assert result.returncode != 0
    assert "MEMORY_PRIVATE_TOKEN" in result.stderr


@pytest.mark.parametrize("agent", ["claude", "codex"])
def test_without_a_token_the_session_goes_through_the_passphrase_gate(tmp_path: Path, agent: str) -> None:
    bin_dir = tmp_path / "bin"
    bin_dir.mkdir(exist_ok=True)
    (bin_dir / "agent-kit-private").write_text('#!/bin/sh\nprintf "gate %s\\n" "$*"\n')
    (bin_dir / "agent-kit-private").chmod(0o755)
    result = _session(tmp_path, agent, "--resume", token=None)
    assert result.returncode == 0, result.stderr
    assert result.stdout.strip() == f"gate exec {CMD} {agent} --resume"
