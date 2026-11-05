"""The `/review-grill` skill's install: declared for every agent, said to
apply after grilling and before /to-spec, and copied beside the shared
`critique` skill in every agent home, since it finds the runner at
../critique/critique.mjs.
"""

from __future__ import annotations

import sys
from pathlib import Path

import pytest
import yaml

KIT_ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(KIT_ROOT / "scripts"))

from setup_harness import Run, setup_run  # noqa: E402

SKILL = KIT_ROOT / "skills" / "workstation" / "review-grill" / "SKILL.md"
HOMES = (".claude", ".codex", ".vibe", ".vibe-private", ".gemini/config", ".gemini/antigravity-cli")


def _entry() -> dict:
    data = yaml.safe_load((KIT_ROOT / "registry" / "estate-tooling.yaml").read_text())
    return next(s for s in data["local_skills"] if s["name"] == "review-grill")


def _description() -> str:
    front = SKILL.read_text().split("---")[1]
    return yaml.safe_load(front)["description"]


@pytest.fixture(scope="module")
def macos_run(tmp_path_factory: pytest.TempPathFactory) -> Run:
    return setup_run(tmp_path_factory.mktemp("review-grill"), "Darwin")


def test_critique_decisions_is_a_built_local_skill_for_every_agent() -> None:
    entry = _entry()
    assert entry["path"] == "skills/workstation/review-grill"
    assert set(entry["agents"]) == {"claude", "codex", "vibe", "antigravity"}
    assert entry["surfaces"] == ["workstation"]
    assert "pending" not in entry
    assert SKILL.is_file()


def test_its_description_says_it_applies_after_grilling_and_before_to_spec() -> None:
    description = _description()
    assert description.startswith("Use after /grill-with-docs or /grill-me")
    assert "before /to-spec" in description


def test_setup_installs_it_beside_the_shared_skill_in_every_agent_home(macos_run: Run) -> None:
    for home in HOMES:
        skills = macos_run.home / home / "skills"
        assert (skills / "review-grill" / "SKILL.md").read_bytes() == SKILL.read_bytes(), home
        assert (skills / "review-grill" / ".." / "critique" / "critique.mjs").is_file(), home
