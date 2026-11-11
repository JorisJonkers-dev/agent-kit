"""The research skill: its front matter, the sections that teach an agent to
start a Gold Band Workflow A run in the personal instance, the two fallbacks it
must say word for word, and its install in every agent home.

The skill is a document, so these assert on its text. The setup assertion runs
the checked-in setup script against a temp HOME with every external command
stubbed (tests/setup_harness.py).
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

import pytest
import yaml

KIT_ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(KIT_ROOT / "scripts"))

from setup_harness import Run, setup_run  # noqa: E402

SKILL = KIT_ROOT / "skills" / "workstation" / "research"
SKILL_MD = SKILL / "SKILL.md"
HOMES = (".claude", ".codex", ".vibe", ".vibe-private", ".gemini/config", ".gemini/antigravity-cli")
SECTIONS = [
    "Check the repository rule",
    "Check the instance",
    "Start the run",
    "Fall back to direct delegation",
    "What to report",
]
NO_INSTANCE = "No Gold Band instance is available on this machine; falling back to direct delegation."
RULE_REFUSES = (
    "The personal instance's repository rule (remotes: not-nedap) refuses this repository; "
    "falling back to direct delegation."
)
WORKFLOW = "agent-kit/gold-band-personal/workflows/research.json"


def _text() -> str:
    return SKILL_MD.read_text()


def _flat() -> str:
    return " ".join(_text().split())


@pytest.fixture(scope="module")
def macos_run(tmp_path_factory: pytest.TempPathFactory) -> Run:
    return setup_run(tmp_path_factory.mktemp("research-macos"), "Darwin")


# ---------------------------------------------------------------------------
# Front matter and declaration
# ---------------------------------------------------------------------------


def test_the_front_matter_names_the_skill_and_says_when_to_use_it() -> None:
    front = yaml.safe_load(_text().split("---")[1])
    assert set(front) == {"name", "description"}
    assert front["name"] == "research"
    assert front["description"].startswith(
        "Use when the owner asks to research a topic, survey literature or prior art, "
        "or find the latest work on a question."
    )


def test_the_skill_carries_every_section_the_run_needs_in_order() -> None:
    headings = [line[3:] for line in _text().splitlines() if line.startswith("## ")]
    assert headings == SECTIONS


def test_the_skill_is_declared_for_every_agent_the_registry_allows() -> None:
    registry = yaml.safe_load((KIT_ROOT / "registry" / "estate-tooling.yaml").read_text())
    skill = next(s for s in registry["local_skills"] if s["name"] == "research")
    assert skill["path"] == "skills/workstation/research"
    assert set(skill["agents"]) == {"claude", "codex", "vibe", "antigravity", "opencode"}
    assert skill["surfaces"] == ["workstation"]


def test_setup_installs_the_skill_into_every_agent_home(macos_run: Run) -> None:
    for home in HOMES:
        dest = macos_run.home / home / "skills" / "research" / "SKILL.md"
        assert dest.read_bytes() == SKILL_MD.read_bytes(), home


# ---------------------------------------------------------------------------
# Start the run
# ---------------------------------------------------------------------------


def test_the_run_command_is_the_headless_runner_with_a_requirement_file() -> None:
    text = _flat()
    assert re.search(r"gold-band run( \\)? --workflow", text)
    assert "--requirement-file" in text


def test_the_workflow_file_is_the_personal_instances_research_workflow() -> None:
    text = _flat()
    assert WORKFLOW in text
    assert "XDG_DATA_HOME" in text


# ---------------------------------------------------------------------------
# The two fallbacks, word for word
# ---------------------------------------------------------------------------


def test_the_missing_instance_fallback_is_said_word_for_word() -> None:
    assert NO_INSTANCE in _flat()


def test_the_refused_repository_fallback_is_said_word_for_word() -> None:
    assert RULE_REFUSES in _flat()


# ---------------------------------------------------------------------------
# Shape
# ---------------------------------------------------------------------------


def test_the_skill_is_short_and_carries_no_assistant_attribution() -> None:
    text = _text()
    assert len(text.splitlines()) < 120
    assert "Co-Authored-By" not in text
    assert "Generated with" not in text


# ---------------------------------------------------------------------------
# Order and refusal paths
# ---------------------------------------------------------------------------


def test_the_rule_is_checked_before_the_instance_and_the_instance_before_the_run() -> None:
    text = _text()
    assert text.index("## Check the repository rule") < text.index("## Check the instance")
    assert text.index("command -v gold-band") < text.index("gold-band run")


def test_a_nedap_remote_in_any_form_is_refused_and_no_remote_is_allowed() -> None:
    text = _flat()
    assert "git remote -v" in text
    assert "git@github.com:nedap/<repo>.git" in text
    assert "https://github.com/nedap/<repo>.git" in text
    assert "A repository with no remote is allowed." in text


def test_a_run_start_refusal_is_quoted_as_it_stands_then_falls_back() -> None:
    text = _flat()
    assert "report its refusal text as it stands (not the rule message below)" in text
