"""What keeps Claude Code out of the private bank's files, and the retired retain hook.

Claude Code gets deny rules for Vibe's home (its session logs hold the private
conversations) and the gated token, for its file tools and for Bash. Vibe's
every-turn retain hook is retired: Vibe saves only what matters, by tool.
"""

from __future__ import annotations

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


def _script() -> str:
    return render_registry.render_setup_script(_registry())


def _fn(script: str, name: str) -> str:
    return f"{name}() {{" + script.split(f"\n{name}() {{")[1].split("\n}\n")[0] + "\n}\n"


def test_the_rules_cover_vibe_and_the_gate_for_file_tools_and_bash() -> None:
    rules = _registry()["claude_private_deny"]
    for rule in ("Read(~/.vibe/**)", "Edit(~/.vibe/**)", "Bash(*.vibe*)",
                 "Read(~/.config/agent-kit/private/**)", "Edit(~/.config/agent-kit/private/**)",
                 "Bash(*agent-kit/private*)", "Bash(*agent-kit-private*)"):
        assert rule in rules


def _merge(tmp_path: Path, settings: dict | None, cloud: bool = False) -> Path:
    script = _script()
    profile = tmp_path / "claude"
    profile.mkdir(exist_ok=True)
    path = profile / "settings.json"
    if settings is not None:
        path.write_text(json.dumps(settings))
    body = (
        f"set -u\nCLOUD={int(cloud)}\nCHECK_ONLY=0\nCLAUDE_PROFILE_DIRS=({profile})\nfailures=0\nwarnings=0\n"
        "todo_failures=()\ntodo_warnings=()\n"
        "log() { :; }\nok() { :; }\nwarn() { echo \"warn $*\" >&2; }\nfail() { echo \"fail $*\" >&2; }\n"
        + _fn(script, "claude_private_denies") + "claude_private_denies\n"
    )
    result = subprocess.run(["/bin/bash", "-c", body], capture_output=True, text=True, check=False,
                            env={**os.environ})
    assert result.returncode == 0 and not result.stderr, result.stderr
    return path


def test_the_rules_join_the_users_own_and_change_nothing_else(tmp_path: Path) -> None:
    mine = {"model": "opus", "permissions": {"allow": ["Bash(ls)"], "deny": ["Bash(rm -rf:*)"]}}
    path = _merge(tmp_path, mine)
    data = json.loads(path.read_text())
    rules = _registry()["claude_private_deny"]
    assert data["model"] == "opus"
    assert data["permissions"]["allow"] == ["Bash(ls)"]
    assert data["permissions"]["deny"] == ["Bash(rm -rf:*)", *rules]


def test_a_second_run_changes_nothing(tmp_path: Path) -> None:
    path = _merge(tmp_path, {})
    once = path.read_text()
    _merge(tmp_path, None)
    assert path.read_text() == once


@pytest.mark.parametrize("settings", [None, {}])
def test_a_cloud_run_leaves_settings_alone(tmp_path: Path, settings: dict | None) -> None:
    path = _merge(tmp_path, settings, cloud=True)
    assert (not path.exists()) if settings is None else json.loads(path.read_text()) == {}


def test_setup_applies_them_after_the_profiles_exist() -> None:
    script = _script()
    main = script.split(_fn(script, "claude_private_denies"))[1]
    assert main.index('log "claude profiles"') < main.index("\nclaude_private_denies\n")


# --- the every-turn retain hook is retired ---


def test_the_retain_hook_is_gone_from_the_kit() -> None:
    assert not (KIT_ROOT / "installer" / "vibe-retain-private").exists()
    publish = (KIT_ROOT / "scripts" / "publish-installer-artifacts.sh").read_text()
    paths = publish.split("KIT_PATHS=(")[1].split(")")[0].split()
    assert "installer/vibe-retain-private" not in paths
    assert "installer/agent-kit-private" in paths


def test_setup_removes_the_retired_hook_and_its_command(tmp_path: Path) -> None:
    script = _script()
    assert "vibe_hooks_remove=(--remove agent-kit-retain-private)" in script
    assert "install_kit_command vibe-retain-private" not in script
    assert "retire_kit_command vibe-retain-private" in script
    home = tmp_path / "home"
    (home / ".local" / "bin").mkdir(parents=True)
    managed = home / ".local" / "bin" / "vibe-retain-private"
    managed.write_text("#!/bin/sh\n# managed by agent-kit setup-workstation.sh\n")
    body = (f"HOME={home}\nCHECK_ONLY=0\nlog() {{ :; }}\nok() {{ :; }}\n" + _fn(script, "retire_kit_command")
            + "retire_kit_command vibe-retain-private\n")
    subprocess.run(["/bin/bash", "-c", body], check=True)
    assert not managed.exists()


def test_a_command_setup_did_not_write_is_not_retired(tmp_path: Path) -> None:
    script = _script()
    home = tmp_path / "home"
    (home / ".local" / "bin").mkdir(parents=True)
    mine = home / ".local" / "bin" / "vibe-retain-private"
    mine.write_text("#!/bin/sh\necho mine\n")
    body = (f"HOME={home}\nCHECK_ONLY=0\nlog() {{ :; }}\nok() {{ :; }}\n" + _fn(script, "retire_kit_command")
            + "retire_kit_command vibe-retain-private\n")
    subprocess.run(["/bin/bash", "-c", body], check=True)
    assert mine.exists()


def test_vibe_is_told_to_save_only_what_matters() -> None:
    pointer = _fn(_script(), "_vibe_agents_pointer")
    assert "automatically" not in pointer
    assert "long-term" in pointer and "remember" in pointer
    assert "Never retain" in pointer and "memory-api" in pointer


def test_the_gate_is_installed_and_vibe_launches_through_it() -> None:
    script = _script()
    assert "install_kit_command agent-kit-private" in script
    private = next(e for e in _registry()["workstation_secrets"] if e["env"] == "MEMORY_PRIVATE_TOKEN")
    assert private["gate"] == "passphrase" and private["export"] is False
