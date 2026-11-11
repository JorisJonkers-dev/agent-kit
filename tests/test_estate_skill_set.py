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


def test_every_declared_personal_agent_names_the_estate_set() -> None:
    agents = _registry()["gold_band"]["agents"]
    assert [agent["id"] for agent in agents] == ["opencode", "vibe", "vibe-study"]
    assert {agent["skill_set"] for agent in agents} == {"estate"}


def test_an_id_that_names_the_set_gets_exactly_the_four_skills(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    settings = _converge(tmp_path, monkeypatch)
    for agent_id in ("opencode", "vibe", "vibe-study"):
        directory = settings.parent / "skill-sets" / agent_id
        assert sorted(child.name for child in directory.iterdir()) == ESTATE
        for skill in ESTATE:
            assert (directory / skill / "SKILL.md").is_file()
        entry = json.loads(settings.read_text())["agents"][agent_id]
        assert entry["primaryAgentDir"] == str(directory)
        assert entry["compatibleAgentDirs"] == []


def test_an_id_that_does_not_name_the_set_gets_none(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    registry = _registry()
    registry["gold_band"]["agents"][1].pop("skill_set")
    edited = tmp_path / "registry.yaml"
    edited.write_text(yaml.safe_dump(registry))
    settings = _converge(tmp_path, monkeypatch, edited)
    assert not (settings.parent / "skill-sets" / "vibe").exists()
    assert (settings.parent / "skill-sets" / "opencode").is_dir()
