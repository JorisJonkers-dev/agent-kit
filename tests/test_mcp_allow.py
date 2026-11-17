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


# --- auto mode defaults and the personal profile's Nedap lockout ---

TAG = "[agent-kit] "


def _run(tmp_path: Path, settings: dict | None, check: bool = False) -> Path:
    script = render_registry.render_setup_script(
        yaml.safe_load((KIT_ROOT / "registry" / "estate-tooling.yaml").read_text()))
    fn = _fn(script, "claude_permissive_defaults")
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
    stale = "Organization: the owner's personal software development and home-lab estate."
    old = {"autoMode": {"environment": ["$defaults", stale]}}
    env = json.loads(_run(tmp_path, old).read_text())["autoMode"]["environment"]
    assert not any(e.startswith("Organization: the owner") for e in env)


def test_auto_mode_second_run_changes_nothing(tmp_path: Path) -> None:
    path = _run(tmp_path, {})
    first = path.read_text()
    _run(tmp_path, None)
    assert path.read_text() == first


def test_check_only_writes_nothing(tmp_path: Path) -> None:
    path = _run(tmp_path, {"model": "x"}, check=True)
    assert json.loads(path.read_text()) == {"model": "x"}


def test_only_moves_dev_cli_merges_without_a_request() -> None:
    allow = [e for e in render_registry.AUTO_MODE_ALLOW if "Merging a pull request" in e]
    assert len(allow) == 1 and "moves-dev-cli" in allow[0] and "release" in allow[0] and "--admin" in allow[0]
    deny = [e for e in render_registry.AUTO_MODE_SOFT_DENY if "Merging a pull request" in e]
    assert len(deny) == 1 and "nedap/moves-dev-cli" in deny[0]


def test_branch_push_allows_exclude_the_default_branch_and_tags_need_a_request() -> None:
    pushes = [e for e in render_registry.AUTO_MODE_ALLOW if e.startswith("Pushing")]
    assert pushes and all("non-default" in e for e in pushes)
    assert any("default branch" in e and "presumed" in e for e in render_registry.AUTO_MODE_SOFT_DENY)
    assert any("Pushing a tag" in e for e in render_registry.AUTO_MODE_SOFT_DENY)


def test_nedap_code_is_confidential_not_a_sensitive_location() -> None:
    sensitive = next(e for e in render_registry.AUTO_MODE_ENVIRONMENT if e.startswith("Sensitive data locations"))
    assert "customer data" in sensitive and "source code" not in sensitive and ".env" not in sensitive
    assert any("separate trust boundaries" in e for e in render_registry.AUTO_MODE_ENVIRONMENT)


def test_every_soft_deny_names_what_it_needs() -> None:
    assert all(e.startswith("[named+specifics — must name:") for e in render_registry.AUTO_MODE_SOFT_DENY)


def test_both_profiles_get_the_entries(tmp_path: Path) -> None:
    script = render_registry.render_setup_script(
        yaml.safe_load((KIT_ROOT / "registry" / "estate-tooling.yaml").read_text()))
    fn = _fn(script, "claude_permissive_defaults")
    work, personal = tmp_path / "work", tmp_path / "personal"
    work.mkdir()
    personal.mkdir()
    body = (f"set -u\nCHECK_ONLY=0\nCLAUDE_PROFILE_DIRS=({work} {personal})\nlog() {{ :; }}\nok() {{ :; }}\n"
            "fail() { echo fail >&2; }\n" + fn + "claude_permissive_defaults\n")
    result = subprocess.run(["/bin/bash", "-c", body], capture_output=True, text=True, check=False,
                            env={**os.environ, "HOME": str(tmp_path / "home")})
    assert result.returncode == 0 and not result.stderr, result.stderr
    for d in (work, personal):
        allow = json.loads((d / "settings.json").read_text())["autoMode"]["allow"]
        assert any("moves-dev-cli" in e for e in allow)


def test_the_model_gateway_is_hard_denied_on_nedap() -> None:
    assert any("model gateway" in e and "Nedap" in e for e in render_registry.AUTO_MODE_HARD_DENY)


def test_the_personal_profile_overlay_blocks_the_nedap_orgs() -> None:
    overlay = render_registry.profile_overlay(["nedap", "NedapISEP"])
    deny = overlay["permissions"]["deny"]
    assert "Read(~/Projects/nedap/**)" in deny and "Edit(~/Projects/NedapISEP/**)" in deny
    assert "Bash(gh * nedap/*)" in deny and "Bash(gh * --repo NedapISEP/*)" in deny
    hard = overlay["autoMode"]["hard_deny"]
    assert hard[0] == "$defaults" and "nedap or NedapISEP" in hard[1]


def test_only_the_personal_launcher_loads_the_overlay(tmp_path: Path) -> None:
    script = render_registry.render_setup_script(
        yaml.safe_load((KIT_ROOT / "registry" / "estate-tooling.yaml").read_text()))
    fn = "install_profile_launcher() {" + script.split("\ninstall_profile_launcher() {")[1].split("\n}\n")[0] + "\n}\n"
    bin_dir = tmp_path / "bin"
    body = (f"set -u\nCHECK_ONLY=0\nCLAUDE_LAUNCHER_DIR={bin_dir}\nunshared_profile_paths=()\nPATH=$PATH:{bin_dir}\n"
            "log() { :; }\nok() { :; }\nwarn() { :; }\nfail() { echo fail >&2; }\n" + fn
            + f"install_profile_launcher personal {tmp_path}/p {tmp_path}/p/overlay.json\n"
            + f"install_profile_launcher plain {tmp_path}/q\n")
    result = subprocess.run(["/bin/bash", "-c", body], capture_output=True, text=True, check=False,
                            env={**os.environ})
    assert result.returncode == 0 and not result.stderr, result.stderr
    assert f"exec claude --settings {tmp_path}/p/overlay.json \"$@\"" in (bin_dir / "claude-personal").read_text()
    assert (bin_dir / "claude-plain").read_text().splitlines()[-1] == 'exec claude "$@"'


def test_the_registry_blocks_nedap_for_the_personal_profile_only() -> None:
    profiles = yaml.safe_load((KIT_ROOT / "registry" / "estate-tooling.yaml").read_text())["claude_profiles"]
    blocked = {p["name"]: p.get("blocked_orgs") for p in profiles}
    assert blocked["personal"] == ["nedap", "NedapISEP"] and not blocked["work"]
