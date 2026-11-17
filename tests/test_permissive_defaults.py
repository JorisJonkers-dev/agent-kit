"""Permissive auto mode defaults written into each Claude profile.

Setup joins the allow rules and the tagged ``autoMode`` entries to the user's
own, replaces its own earlier entries instead of duplicating them, and keeps
the built-in ``$defaults`` first.
"""

from __future__ import annotations

import json
import os
import subprocess
import sys
from pathlib import Path

KIT_ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(KIT_ROOT / "scripts"))

import render_registry  # noqa: E402
import yaml  # noqa: E402

TAG = "[agent-kit] "


def _run(tmp_path: Path, settings: dict | None, check: bool = False) -> Path:
    script = render_registry.render_setup_script(
        yaml.safe_load((KIT_ROOT / "registry" / "estate-tooling.yaml").read_text()))
    fn = "claude_permissive_defaults() {" + script.split("\nclaude_permissive_defaults() {")[1].split("\n}\n")[0] + "\n}\n"
    profile = tmp_path / "claude"
    profile.mkdir(exist_ok=True)
    path = profile / "settings.json"
    if settings is not None:
        path.write_text(json.dumps(settings))
    body = (f"set -u\nCHECK_ONLY={int(check)}\nCLAUDE_PROFILE_DIRS=({profile})\n"
            "log() { :; }\nok() { :; }\nfail() { echo \"fail $*\" >&2; }\n" + fn + "claude_permissive_defaults\n")
    result = subprocess.run(["/bin/bash", "-c", body], capture_output=True, text=True, check=False,
                            env={**os.environ, "HOME": str(tmp_path / "home")})
    assert result.returncode == 0 and not result.stderr, result.stderr
    return path


def test_rules_join_the_users_own_and_change_nothing_else(tmp_path: Path) -> None:
    mine = {"model": "opus", "permissions": {"allow": ["Bash(ls)"], "deny": ["Bash(rm -rf:*)"]}}
    data = json.loads(_run(tmp_path, mine).read_text())
    assert data["model"] == "opus"
    assert data["permissions"]["deny"] == ["Bash(rm -rf:*)"]
    assert data["permissions"]["allow"][0] == "Bash(ls)"
    for rule in ("Bash(gh pr view *)", "Bash(gh issue list *)", f"Read(/{tmp_path}/home/Projects/**)"):
        assert rule in data["permissions"]["allow"]
    assert f"{tmp_path}/home/Projects" in data["permissions"]["additionalDirectories"]


def test_gh_writes_are_never_pre_allowed() -> None:
    for rule in render_registry.PERMISSIVE_ALLOW_RULES:
        assert not rule.startswith(("Bash(gh pr merge", "Bash(gh pr create", "Bash(gh api", "Bash(git push"))


def test_auto_mode_entries_keep_defaults_first_and_the_users_own(tmp_path: Path) -> None:
    mine = {"autoMode": {"environment": ["my own line"]}}
    am = json.loads(_run(tmp_path, mine).read_text())["autoMode"]
    assert am["environment"][0] == "$defaults" and "my own line" in am["environment"]
    assert am["soft_deny"][0] == "$defaults" and am["allow"][0] == "$defaults"
    assert all(e.startswith(TAG) for e in am["soft_deny"][1:])


def test_legacy_entries_are_replaced_not_duplicated(tmp_path: Path) -> None:
    old = {"autoMode": {"environment": ["$defaults", "Organization: the owner's personal software development and home-lab estate."]}}
    env = json.loads(_run(tmp_path, old).read_text())["autoMode"]["environment"]
    assert not any(e.startswith("Organization: the owner") for e in env)


def test_a_second_run_changes_nothing(tmp_path: Path) -> None:
    path = _run(tmp_path, {})
    first = path.read_text()
    _run(tmp_path, None)
    assert path.read_text() == first


def test_check_only_writes_nothing(tmp_path: Path) -> None:
    path = _run(tmp_path, {"model": "x"}, check=True)
    assert json.loads(path.read_text()) == {"model": "x"}
