"""Setup removes the runner's /workspace trust that leaked into a workstation's agent configs.

Run outside its container, the runner entrypoint once trusted its workspace,
/workspace, in the invoking user's ~/.claude.json (as a Claude project) and in
Codex's config.toml (as a ``[projects."/workspace"]`` table). On a machine
without /workspace that trust is the leak and nothing else, so setup converges
it away: every Claude profile it manages and the Codex config lose it, the
owner's own projects stay, and one ``fixed:`` line says so. Under --check it
only warns. The full runs are the checked-in setup script against a temp HOME
(tests/setup_harness.py); the rest run only the rendered purge function, so a
workspace root that exists can be staged without touching /workspace.
"""

from __future__ import annotations

import json
import os
import subprocess
import sys
import tomllib
from pathlib import Path

import pytest
import yaml

KIT_ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(KIT_ROOT / "scripts"))
sys.path.insert(0, str(Path(__file__).resolve().parent))

import render_registry  # noqa: E402
from setup_harness import Run, setup_run  # noqa: E402

OWN = "/Users/owner/code"


def _claude_json(workspace: str) -> dict:
    return {
        "theme": "light",
        "projects": {
            workspace: {"hasTrustDialogAccepted": True, "hasCompletedProjectOnboarding": True},
            f"{workspace}/repo": {"hasTrustDialogAccepted": True, "hasCompletedProjectOnboarding": True},
            OWN: {"hasTrustDialogAccepted": True, "allowedTools": []},
        },
    }


def _codex_toml(workspace: str) -> str:
    return f"""\
model = "gpt-5"

[projects."{OWN}"]
trust_level = "trusted"

[projects."{workspace}"]
trust_level = "trusted"

[projects."{workspace}/repo"]
trust_level = "trusted"

# the owner's own server
[mcp_servers.mine]
command = "mine"
"""


def _claude_configs(home: Path) -> list[Path]:
    return [home / ".claude.json", home / ".claude-personal" / ".claude.json"]


def _seed(home: Path, workspace: str = "/workspace") -> None:
    for config in _claude_configs(home):
        config.parent.mkdir(parents=True, exist_ok=True)
        config.write_text(json.dumps(_claude_json(workspace), indent=2))
        config.chmod(0o600)
    codex = home / ".codex" / "config.toml"
    codex.parent.mkdir(parents=True, exist_ok=True)
    codex.write_text(_codex_toml(workspace))


def _fixed_lines(run: Run) -> list[str]:
    return [line for line in run.stdout.splitlines() if "fixed:" in line]


no_workspace = pytest.mark.skipif(Path("/workspace").exists(), reason="this machine has a real /workspace")


@no_workspace
def test_setup_removes_the_leaked_workspace_trust_and_says_so_once(tmp_path: Path) -> None:
    run = setup_run(tmp_path, "Darwin", prepare_home=_seed)

    for config in _claude_configs(run.home):
        data = json.loads(config.read_text())
        assert list(data["projects"]) == [OWN], config
        assert data["theme"] == "light"
        assert config.stat().st_mode & 0o777 == 0o600
    codex_text = (run.home / ".codex" / "config.toml").read_text()
    codex = tomllib.loads(codex_text)
    assert list(codex["projects"]) == [OWN]
    assert codex["mcp_servers"]["mine"] == {"command": "mine"}
    assert "# the owner's own server" in codex_text
    fixed = _fixed_lines(run)
    assert len(fixed) == 1, run.stdout
    assert fixed[0].startswith("setup:   ok    fixed: ")
    assert "/workspace" in fixed[0]


@no_workspace
def test_check_warns_about_the_leaked_workspace_trust_and_changes_nothing(tmp_path: Path) -> None:
    run = setup_run(tmp_path, "Darwin", args=("--check",), prepare_home=_seed)

    for config in _claude_configs(run.home):
        assert json.loads(config.read_text()) == _claude_json("/workspace")
    assert (run.home / ".codex" / "config.toml").read_text() == _codex_toml("/workspace")
    assert _fixed_lines(run) == []
    assert "/workspace" in run.stderr


def _purge(home: Path, root: Path) -> subprocess.CompletedProcess[str]:
    """Run only the rendered purge function, against ``home`` and the workspace root ``root``."""
    script = render_registry.render_setup_script(
        yaml.safe_load((KIT_ROOT / "registry" / "estate-tooling.yaml").read_text()),
    )
    start = script.index("\npurge_leaked_runner_workspace_trust() {") + 1
    function = script[start : script.index("\n}\n", start) + 3]
    body = (
        # The primary profile's .claude.json is $HOME's only while CLAUDE_CONFIG_DIR is unset.
        f'unset CLAUDE_CONFIG_DIR\nCHECK_ONLY=0\nHOME="{home}"\n'
        f'CLAUDE_HOME="{home}/.claude"\nCODEX_HOME="{home}/.codex"\n'
        f'CLAUDE_PROFILE_DIRS=("{home}/.claude" "{home}/.claude-personal")\n'
        'ok() { echo "ok $*"; }\nwarn() { echo "warn $*"; }\nfail() { echo "fail $*"; }\n'
        + function
        + f'purge_leaked_runner_workspace_trust "{root}"\n'
    )
    return subprocess.run(["/bin/bash", "-c", body], capture_output=True, text=True, check=False, env={**os.environ})


def _files(home: Path) -> dict[str, bytes]:
    return {str(path): path.read_bytes() for path in home.rglob("*") if path.is_file()}


def test_a_workspace_that_exists_on_this_machine_is_the_owners_and_stays(tmp_path: Path) -> None:
    root = tmp_path / "workspace"
    root.mkdir()
    home = tmp_path / "home"
    _seed(home, workspace=str(root))
    before = _files(home)

    result = _purge(home, root)

    assert result.returncode == 0, result.stderr
    assert result.stdout == ""
    assert _files(home) == before


def test_the_same_trust_goes_once_its_workspace_is_missing(tmp_path: Path) -> None:
    root = tmp_path / "gone"
    home = tmp_path / "home"
    _seed(home, workspace=str(root))

    result = _purge(home, root)

    assert result.returncode == 0, result.stderr
    lines = result.stdout.splitlines()
    assert len(lines) == 1 and lines[0].startswith("ok fixed: "), result.stdout
    for config in _claude_configs(home):
        assert list(json.loads(config.read_text())["projects"]) == [OWN]
    assert list(tomllib.loads((home / ".codex" / "config.toml").read_text())["projects"]) == [OWN]


def test_nothing_to_remove_prints_nothing(tmp_path: Path) -> None:
    home = tmp_path / "home"
    _seed(home, workspace="/elsewhere-not-the-runner")
    before = _files(home)

    result = _purge(home, tmp_path / "gone")

    assert result.returncode == 0, result.stderr
    assert result.stdout == ""
    assert _files(home) == before
