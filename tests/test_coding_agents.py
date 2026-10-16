"""Hindsight Coding Agents replaces the deprecated hindsight-memory plugin, locked down.

One pinned install wires Claude Code and Codex to the shared `default` bank.
It recalls once, at a session's first prompt, and saves nothing on its own:
session write-back, git seeding, the codebase survey, knowledge-page refreshes
and self-update are all off, and agents save deliberately under the memory
policy. The token comes from the environment, never from a file.
"""

from __future__ import annotations

import json
import os
import re
import subprocess
import sys
from pathlib import Path

KIT_ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(KIT_ROOT / "scripts"))

import render_registry  # noqa: E402
import yaml  # noqa: E402

PACKAGE = "@vectorize-io/hindsight-coding-agents"


def _registry() -> dict:
    return yaml.safe_load((KIT_ROOT / "registry" / "estate-tooling.yaml").read_text())


def _script() -> str:
    return render_registry.render_setup_script(_registry())


def _fn(script: str, name: str) -> str:
    return f"{name}() {{" + script.split(f"\n{name}() {{")[1].split("\n}\n")[0] + "\n}\n"


# --- the registry entry ---


def test_the_package_is_pinned_and_wires_claude_and_codex() -> None:
    ca = _registry()["coding_agents"]
    assert ca["package"] == PACKAGE
    assert re.fullmatch(r"\d+\.\d+\.\d+", ca["version"])
    assert ca["harnesses"] == ["claude-code", "codex"]


def test_it_is_locked_down_to_recall_only_on_the_shared_bank() -> None:
    config = _registry()["coding_agents"]["config"]
    assert config["apiUrl"] == "https://hindsight-api.jorisjonkers.dev"
    assert (config["bankId"], config["dynamicBankId"]) == ("default", False)
    for off in ("retainSessions", "autoSeed", "codebaseSurvey", "autoUpdate", "manageBankConfig"):
        assert config[off] is False, off
    assert config["gitIngest"] == "none"
    assert config["autoInject"] == "recall"
    assert config["pageTriggerType"] == "manual"
    assert config["injectTimeoutMs"] >= 10000
    assert config["recallOptions"]["max_tokens"] <= 1200, "a modest memory block per session"
    assert "apiToken" not in config, "the token comes from HINDSIGHT_API_TOKEN, never a file"


def test_agents_are_told_what_is_worth_saving() -> None:
    guide = _registry()["coding_agents"]["config"]["toolGuideExtra"]
    for phrase in ("long-term", "remember", "secrets", "near-duplicate"):
        assert phrase in guide


def test_validation_refuses_a_floating_version_or_a_token_in_the_config() -> None:
    data = _registry()
    data["coding_agents"]["version"] = "latest"
    try:
        render_registry.validate(data)
    except render_registry.RegistryError:
        pass
    else:
        raise AssertionError("a floating version passed validation")
    data = _registry()
    data["coding_agents"]["config"]["apiToken"] = "x"
    try:
        render_registry.validate(data)
    except render_registry.RegistryError:
        pass
    else:
        raise AssertionError("a token in the config passed validation")


# --- the old plugin goes ---


def test_the_deprecated_plugin_and_its_marketplace_are_retired() -> None:
    data = _registry()
    assert not any(p["name"] == "hindsight-memory" for p in data["plugins"])
    assert not any(m["name"] == "hindsight" for m in data["marketplaces"])
    script = _script()
    assert "claude plugin install hindsight-memory@hindsight" not in script
    assert "--retired-plugins hindsight-memory@hindsight " in script
    assert "--retired-markets hindsight " in script


# --- the install step, run against stubs ---


def _run(
    tmp_path: Path, *, cloud: bool = False, config: dict | None = None,
) -> tuple[subprocess.CompletedProcess, Path]:
    script = _script()
    home = tmp_path / "home"
    (home / ".hindsight").mkdir(parents=True)
    if config is not None:
        (home / ".hindsight" / "coding-agent.json").write_text(json.dumps(config))
    bin_dir = tmp_path / "bin"
    bin_dir.mkdir()
    log = tmp_path / "calls"
    for tool in ("npx", "claude"):
        (bin_dir / tool).write_text(f'#!/bin/sh\nprintf "{tool} %s\\n" "$*" >> {log}\n')
        (bin_dir / tool).chmod(0o755)
    profiles = f"{home}/.claude {home}/.claude-personal"
    body = (
        f"set -u\nHOME={home}\nCLOUD={int(cloud)}\nCHECK_ONLY=0\nfailures=0\nwarnings=0\n"
        "todo_failures=()\ntodo_warnings=()\n"
        f"CLAUDE_PROFILE_DIRS=({profiles})\n"
        "log() { :; }\nok() { :; }\nwarn() { echo \"warn $*\" >&2; }\nfail() { echo \"fail $*\" >&2; }\n"
        'run() { "$@"; }\n'
        'claude_each_profile() { local d; for d in "${CLAUDE_PROFILE_DIRS[@]}"; do\n'
        '  CLAUDE_CONFIG_DIR="$d" "$@"; done; }\n'
        + _fn(script, "_merge_user_config")
        + _fn(script, "hindsight_coding_agents")
        + "hindsight_coding_agents </dev/null\n"
    )
    env = {**os.environ, "PATH": f"{bin_dir}:{os.environ['PATH']}"}
    result = subprocess.run(["/bin/bash", "-c", body], capture_output=True, text=True, check=False, env=env)
    return result, log


def test_setup_installs_the_pinned_package_for_both_agents(tmp_path: Path) -> None:
    result, log = _run(tmp_path)
    assert result.returncode == 0, result.stderr
    calls = log.read_text().splitlines()
    version = _registry()["coding_agents"]["version"]
    assert (f"npx -y {PACKAGE}@{version} install claude-code codex --server self-hosted "
            "--api-url https://hindsight-api.jorisjonkers.dev") in calls


def test_the_locked_config_wins_and_no_token_stays_on_disk(tmp_path: Path) -> None:
    mine = {"logLevel": "warn", "retainSessions": True, "apiToken": "smt_leaked"}
    result, _ = _run(tmp_path, config=mine)
    assert result.returncode == 0, result.stderr
    written = json.loads((tmp_path / "home" / ".hindsight" / "coding-agent.json").read_text())
    assert written["logLevel"] == "warn", "a key the kit does not manage is kept"
    assert written["retainSessions"] is False
    assert written["bankId"] == "default"
    assert "apiToken" not in written


def test_every_claude_profile_gets_the_memory_tools(tmp_path: Path) -> None:
    _, log = _run(tmp_path)
    adds = [c for c in log.read_text().splitlines()
            if c.startswith("claude mcp add --scope user hindsight --env HINDSIGHT_MCP_HARNESS=claude-code")]
    assert len(adds) == 2, "primary and personal profile"
    assert all(c.endswith("/.hindsight/coding-agents/dist/mcp-server.js") for c in adds)


def test_a_cloud_session_gets_it_too(tmp_path: Path) -> None:
    # The old plugin ran in cloud sessions; recall must not quietly vanish there.
    result, log = _run(tmp_path, cloud=True)
    assert result.returncode == 0, result.stderr
    assert any(c.startswith(f"npx -y {PACKAGE}@") for c in log.read_text().splitlines())


def test_setup_runs_it_after_the_plugins() -> None:
    script = _script()
    main = script.split(_fn(script, "hindsight_coding_agents"))[1]
    assert main.index('log "plugins"') < main.index("\n  hindsight_coding_agents\n")


# --- a private session leaves the shared bank alone ---


def test_a_private_session_switches_the_plugin_off_and_blocks_its_tools() -> None:
    text = (KIT_ROOT / "installer" / "memory-private").read_text()
    assert "export HINDSIGHT_DISABLED=true" in text
    assert "mcp__hindsight" in text
    assert "mcp_servers.hindsight.enabled=false" in text


def test_the_tool_guide_is_shown_once_a_session_not_every_turn() -> None:
    assert _registry()["coding_agents"]["config"]["pageRefreshEveryTurns"] >= 1000


def test_agents_are_told_nothing_is_captured_for_them() -> None:
    guide = _registry()["coding_agents"]["config"]["toolGuideExtra"]
    assert "not captured automatically" in guide
