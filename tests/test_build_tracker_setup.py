"""The build tracker's install: its skill in every agent home, its runner
pinned in manifest.yaml the way the critique runner is, and runnable with
`node` as it ships.

The setup assertions run the checked-in setup script against a temp HOME with
every external command stubbed (tests/setup_harness.py).
"""

from __future__ import annotations

import copy
import hashlib
import importlib.util
import json
import re
import shutil
import subprocess
import sys
from pathlib import Path

import pytest
import yaml

KIT_ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(KIT_ROOT / "scripts"))

from setup_harness import Run, setup_run  # noqa: E402

SKILL = KIT_ROOT / "skills" / "build-tracker"
RUNNER = SKILL / "build-tracker.mjs"
HOMES = (".claude", ".codex", ".vibe", ".vibe-private", ".gemini/config", ".gemini/antigravity-cli")


def _registry() -> dict:
    return yaml.safe_load((KIT_ROOT / "registry" / "estate-tooling.yaml").read_text())


def _manifest() -> dict:
    return yaml.safe_load((KIT_ROOT / "manifest.yaml").read_text())


def _validator():
    spec = importlib.util.spec_from_file_location("validate_manifest", KIT_ROOT / "scripts" / "validate_manifest.py")
    assert spec and spec.loader
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


@pytest.fixture(scope="module")
def macos_run(tmp_path_factory: pytest.TempPathFactory) -> Run:
    return setup_run(tmp_path_factory.mktemp("build-tracker-macos"), "Darwin")


def test_the_build_tracker_skill_is_declared_for_claude_codex_and_the_other_agents() -> None:
    skill = next(s for s in _registry()["local_skills"] if s["name"] == "build-tracker")
    assert skill["path"] == "skills/build-tracker"
    assert {"claude", "codex"} <= set(skill["agents"])
    assert set(skill["agents"]) == {"claude", "codex", "vibe", "antigravity"}
    assert skill["surfaces"] == ["workstation"]
    assert "pending" not in skill


def test_setup_installs_the_whole_skill_into_every_agent_home(macos_run: Run) -> None:
    want = sorted(p.relative_to(SKILL).as_posix() for p in SKILL.rglob("*") if p.is_file())
    assert want == ["SKILL.md", "build-tracker.mjs"]
    for home in HOMES:
        dest = macos_run.home / home / "skills" / "build-tracker"
        got = sorted(p.relative_to(dest).as_posix() for p in dest.rglob("*") if p.is_file())
        assert got == want, home
        assert (dest / "build-tracker.mjs").read_bytes() == RUNNER.read_bytes(), home


def test_the_runner_is_pinned_in_the_manifest_like_the_critique_runner() -> None:
    files = {f["path"]: f for f in _manifest()["build_tracker"]["files"]}
    assert files["skills/build-tracker/build-tracker.mjs"]["mode"] == "0755"
    for path, entry in files.items():
        digest = hashlib.sha256((KIT_ROOT / path).read_bytes()).hexdigest()
        assert entry["sha256"] == digest, f"{path}: rebuild council/ts and update manifest.yaml build_tracker.files"


def test_the_manifest_pins_every_file_the_skill_ships_but_its_skill_md() -> None:
    _validator().validate_build_tracker(_manifest())


def test_a_runner_missing_from_the_pin_fails_validation() -> None:
    manifest = copy.deepcopy(_manifest())
    manifest["build_tracker"]["files"] = []
    with pytest.raises(AssertionError, match=re.escape("build_tracker.files mismatch")):
        _validator().validate_build_tracker(manifest)


@pytest.mark.skipif(shutil.which("node") is None, reason="node is not installed")
def test_the_shipped_runner_renders_a_page_with_node(tmp_path: Path) -> None:
    state = tmp_path / "state.json"
    state.write_text(
        json.dumps(
            {
                "version": 1,
                "spec": "acme/widgets#10",
                "title": "Widgets build",
                "tickets": {"1": {"title": "widget store", "blocked_by": [], "status": "todo"}},
            }
        )
    )
    page = tmp_path / "tracker.html"

    rendered = subprocess.run(
        ["node", str(RUNNER), "render", "--state", str(state), "--out", str(page)],
        capture_output=True, text=True, check=False,
    )
    assert rendered.returncode == 0, rendered.stderr
    assert json.loads(rendered.stdout)["status"] == "rendered"
    assert '<a href="https://github.com/acme/widgets/issues/1">#1</a>' in page.read_text()

    changed = subprocess.run(
        ["node", str(RUNNER), "set", "--state", str(state), "1=merged:abc1234"],
        capture_output=True, text=True, check=False,
    )
    assert changed.returncode == 0, changed.stderr
    assert json.loads(state.read_text())["tickets"]["1"] == {
        "title": "widget store", "blocked_by": [], "status": "merged", "note": "abc1234",
    }


@pytest.mark.skipif(shutil.which("node") is None, reason="node is not installed")
def test_the_shipped_runner_names_init_for_a_missing_state(tmp_path: Path) -> None:
    result = subprocess.run(
        ["node", str(RUNNER), "set", "--state", str(tmp_path / "state.json"), "1=merged"],
        capture_output=True, text=True, check=False,
    )
    assert result.returncode == 1
    assert json.loads(result.stdout)["code"] == "missing-state"
    assert "init" in result.stderr
