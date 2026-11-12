"""The shipped registry's estate skill set, resolved per agent id.

Drives ``converge.main(["gold-band", ...])`` against the real registry and the
real kit, with a temporary settings file and fake binaries on PATH, so nothing
outside the temporary directory is read or written.
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

import pytest
import yaml

KIT_ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(KIT_ROOT / "scripts"))

import converge  # noqa: E402

REGISTRY = KIT_ROOT / "registry" / "estate-tooling.yaml"
GENERAL = ["agent-session-bootstrap", "kb-first", "token-economy"]
ESTATE = ["estate-bootstrap", "estate-delivery-status", "estate-project-intent", "estate-rollback"]


def _registry() -> dict:
    return yaml.safe_load(REGISTRY.read_text())


def _converge(tmp_path: Path, monkeypatch: pytest.MonkeyPatch, registry: Path = REGISTRY) -> Path:
    bin_dir = tmp_path / "bin"
    bin_dir.mkdir()
    for agent in _registry()["gold_band"]["agents"]:
        fake = bin_dir / agent["binary"]
        fake.write_text("#!/bin/sh\nexit 0\n")
        fake.chmod(0o755)
    monkeypatch.setenv("PATH", str(bin_dir))
    monkeypatch.setenv("AGENT_KIT_GOLD_BAND_RUNNING", "0")
    monkeypatch.delenv("GOLD_BAND_HOME", raising=False)
    settings = tmp_path / "gold-band" / "settings.json"
    settings.parent.mkdir()
    settings.write_text(json.dumps({"agents": {}}))
    keep = tmp_path / "keep.yaml"
    keep.write_text(yaml.safe_dump({"keep": []}))
    argv = ["gold-band", "--keep", str(keep), "--kit", str(KIT_ROOT), "--settings", str(settings)]
    assert converge.main([*argv, "--registry", str(registry)]) == 0
    return settings


def test_the_estate_set_holds_the_four_estate_skills() -> None:
    assert sorted(_registry()["gold_band"]["skill_sets"]["estate"]) == ESTATE


def test_each_general_set_holds_its_one_skill_and_both_groups_agree() -> None:
    sets = _registry()["gold_band"]["skill_sets"]
    for skill in GENERAL:
        assert sets[skill] == [skill]
        source = (KIT_ROOT / "skills" / "workstation" / skill / "SKILL.md").read_bytes()
        for target in ("claude", "codex"):
            assert (KIT_ROOT / "skills" / "repo" / skill / target / "SKILL.md").read_bytes() == source


def _tree(root: Path) -> dict[str, bytes]:
    return {str(path.relative_to(root)): path.read_bytes() for path in sorted(root.rglob("*")) if path.is_file()}


def test_every_declared_personal_agent_names_the_estate_set() -> None:
    agents = _registry()["gold_band"]["agents"]
    # Other agents (claude, codex, pi and the rest) keep their own directories.
    named = [agent for agent in agents if "skill_set" in agent]
    assert [agent["id"] for agent in named] == ["opencode", "vibe", "vibe-study"]
    assert {tuple(agent["skill_set"]) for agent in named} == {("estate", *GENERAL)}


def test_an_id_that_names_the_sets_gets_exactly_their_skills(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    settings = _converge(tmp_path, monkeypatch)
    for agent_id in ("opencode", "vibe", "vibe-study"):
        directory = settings.parent / "skill-sets" / agent_id
        assert sorted(child.name for child in directory.iterdir()) == sorted(ESTATE + GENERAL)
        for skill in ESTATE + GENERAL:
            assert (directory / skill / "SKILL.md").is_file()
        entry = json.loads(settings.read_text())["agents"][agent_id]
        assert entry["primaryAgentDir"] == str(directory)
        assert entry["compatibleAgentDirs"] == []


def test_an_id_that_does_not_name_the_set_gets_none(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    registry = _registry()
    vibe = next(agent for agent in registry["gold_band"]["agents"] if agent["id"] == "vibe")
    vibe.pop("skill_set")
    vibe.update(primaryAgentDir=".vibe", compatibleAgentDirs=[".agents"])
    edited = tmp_path / "registry.yaml"
    edited.write_text(yaml.safe_dump(registry))
    settings = _converge(tmp_path, monkeypatch, edited)
    assert not (settings.parent / "skill-sets" / "vibe").exists()
    entry = json.loads(settings.read_text())["agents"]["vibe"]
    assert entry["primaryAgentDir"] == ".vibe"
    assert entry["compatibleAgentDirs"] == [".agents"]
    assert (settings.parent / "skill-sets" / "opencode").is_dir()


def test_ids_that_name_no_set_get_none_and_the_direct_lane_copies_are_unchanged(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    direct = [KIT_ROOT / "skills" / group / skill for group in ("workstation", "repo") for skill in GENERAL + ESTATE]
    assert [path for path in direct if not path.is_dir()] == [
        path for path in direct if path.parts[-2] == "repo" and path.name.startswith("estate-")
    ]
    direct = [path for path in direct if path.is_dir()]
    before = [_tree(path) for path in direct]
    settings = _converge(tmp_path, monkeypatch)
    assert [_tree(path) for path in direct] == before
    agents = json.loads(settings.read_text())["agents"]
    assert "claude" in agents
    assert not (settings.parent / "skill-sets" / "claude").exists()
    assert agents["claude"]["primaryAgentDir"] == ".claude-personal"
    assert sorted(p.name for p in (settings.parent / "skill-sets").iterdir()) == ["opencode", "vibe", "vibe-study"]
    for agent_id, entry in agents.items():
        assert ("skill_set" in entry) is False
        assert (agent_id in {"opencode", "vibe", "vibe-study"}) == (
            entry["primaryAgentDir"] == str(settings.parent / "skill-sets" / agent_id)
        )
