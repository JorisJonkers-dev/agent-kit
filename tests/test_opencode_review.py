"""Regression tests from the /review-ticket run on #312."""

from __future__ import annotations

import copy
import json
import os
import subprocess
import sys
from pathlib import Path

import yaml

KIT_ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(KIT_ROOT / "scripts"))

import render_registry  # noqa: E402
from setup_harness import setup_run  # noqa: E402

MERGE = KIT_ROOT / "scripts" / "opencode-merge-mcp.py"
PLUGIN_SKILLS = KIT_ROOT / "scripts" / "opencode-plugin-skills.py"
FRAGMENT = KIT_ROOT / "registry" / "generated" / "opencode" / "opencode.json"


def _registry() -> dict:
    return yaml.safe_load((KIT_ROOT / "registry" / "estate-tooling.yaml").read_text())


def _fake_claude(tmp_path: Path, listing: list[dict]) -> dict[str, str]:
    fake = tmp_path / "bin"
    fake.mkdir()
    (fake / "claude").write_text(f"#!/bin/sh\ncat <<'EOF'\n{json.dumps(listing)}\nEOF\n")
    (fake / "claude").chmod(0o755)
    return {**os.environ, "PATH": f"{fake}:{os.environ['PATH']}"}


def test_a_second_merge_on_a_fresh_machine_writes_nothing(tmp_path: Path) -> None:
    """F1: with no config before agent-kit, there is no user config to back up, then or later."""
    config = tmp_path / "opencode.json"

    def once() -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [sys.executable, str(MERGE), str(FRAGMENT), "--config", str(config)],
            capture_output=True,
            text=True,
            check=False,
        )

    assert once().returncode == 0
    before = sorted(p.name for p in tmp_path.iterdir())
    assert once().returncode == 0
    assert sorted(p.name for p in tmp_path.iterdir()) == before


def test_a_copy_whose_ledger_write_was_lost_is_replaced_not_left_alone(tmp_path: Path) -> None:
    """F2: no interrupted run may orphan a copy. The ledger is gone: each copy is marked
    before it is renamed into place, so a run killed mid-install leaves only a staging
    folder, which the next run clears before it replaces the copy."""
    plugin = tmp_path / "plugin"
    (plugin / "skills" / "alpha").mkdir(parents=True)
    (plugin / "skills" / "alpha" / "SKILL.md").write_text("---\nname: alpha\n---\none\n")
    env = _fake_claude(tmp_path, [{"id": "alpha@market", "scope": "user", "enabled": True, "installPath": str(plugin)}])
    skills = tmp_path / "skills"

    def once() -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [sys.executable, str(PLUGIN_SKILLS), "--skills-dir", str(skills), "--plugins", "alpha@market"],
            capture_output=True,
            text=True,
            check=False,
            env=env,
        )

    assert once().returncode == 0
    # A run killed while it built the next copy leaves its staging folder behind.
    (skills / ".agent-kit-staging-alpha").mkdir()
    (skills / ".agent-kit-staging-alpha" / "SKILL.md").write_text("half\n")
    (plugin / "skills" / "alpha" / "SKILL.md").write_text("---\nname: alpha\n---\ntwo\n")
    assert once().returncode == 0
    assert "two" in (skills / "alpha" / "SKILL.md").read_text()
    assert not (skills / ".agent-kit-staging-alpha").exists()


def test_an_opencode_1_config_is_not_fed_the_v2_shape(tmp_path: Path) -> None:
    """F3: a V1 binary is named, and its config left as it is."""

    def seed(home: Path) -> None:
        config = home / ".config" / "opencode" / "opencode.json"
        config.parent.mkdir(parents=True)
        config.write_text(json.dumps({"mcp": {"mine": {"type": "local", "command": ["x"]}}}))

    run = setup_run(tmp_path, "Darwin", prepare_home=seed, stubs={"opencode": '[ "$1" = --version ] && echo "1.19.5"'})
    doc = json.loads((run.home / ".config" / "opencode" / "opencode.json").read_text())
    assert doc == {"mcp": {"mine": {"type": "local", "command": ["x"]}}}


def test_a_malformed_plugin_json_is_a_note_not_a_traceback(tmp_path: Path) -> None:
    """F4: one broken plugin is skipped with a note; the others still land."""
    good = tmp_path / "good"
    (good / "skills" / "one").mkdir(parents=True)
    (good / "skills" / "one" / "SKILL.md").write_text("---\nname: one\n---\n")
    bad = tmp_path / "bad"
    (bad / ".claude-plugin").mkdir(parents=True)
    (bad / ".claude-plugin" / "plugin.json").write_text("{not json")
    env = _fake_claude(
        tmp_path,
        [
            {"id": "bad@m", "scope": "user", "installPath": str(bad)},
            {"id": "good@m", "scope": "user", "installPath": str(good)},
        ],
    )
    result = subprocess.run(
        [sys.executable, str(PLUGIN_SKILLS), "--skills-dir", str(tmp_path / "skills"), "--plugins", "bad@m,good@m"],
        capture_output=True,
        text=True,
        check=False,
        env=env,
    )
    assert "Traceback" not in result.stderr
    assert (tmp_path / "skills" / "one" / "SKILL.md").is_file()
    assert "bad@m:" in result.stdout


def test_no_enabled_plugin_renders_a_valid_plugin_skills_call() -> None:
    """F5: an empty plugin list must not leave a valueless --plugins."""
    data = copy.deepcopy(_registry())
    for plugin in data["plugins"]:
        plugin["enabled"] = False
    script = render_registry.render_setup_script(data)
    line = next(ln for ln in script.splitlines() if "opencode_skill_args=(" in ln)
    assert not line.rstrip().endswith("--plugins )"), line
