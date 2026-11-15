"""The estate-rollback skill: its named sources and its install.

The plan is the `estate plan` subcommand of the `estate` crate (scripts/crates/estate),
whose tests also check the skill's refused worked examples. What stays here is the
skill text.
"""

from __future__ import annotations

import sys
from pathlib import Path

import pytest
import yaml
from setup_harness import Run, setup_run

KIT_ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(KIT_ROOT / "scripts"))

import render_registry  # noqa: E402

SKILL = KIT_ROOT / "skills" / "workstation" / "estate-rollback"
SKILL_MD = SKILL / "SKILL.md"
SOURCES = SKILL / "sources.yaml"


# ---------------------------------------------------------------------------
# The skill text
# ---------------------------------------------------------------------------

REFUSED_HEADINGS = (
    "### Worked example: target later than the pin (refused)",
    "### Worked example: target with no Migration Proof (refused)",
)


@pytest.mark.parametrize("heading", REFUSED_HEADINGS)
def test_skill_md_has_each_refused_worked_example(heading: str) -> None:
    assert heading in SKILL_MD.read_text()


def test_skill_md_names_the_readme_and_workflow_disagreement() -> None:
    text = SKILL_MD.read_text()
    section = text[text.index("## Where the docs and reality disagree") :]
    assert "README" in section
    assert "two commits" in section
    assert "rollback.yml" in section
    assert "The skill follows the estate workflows" in section


def test_skill_md_says_to_stop_and_name_the_doc_that_cannot_be_read() -> None:
    text = SKILL_MD.read_text()
    section = text[text.index("## When a doc cannot be read") :]
    assert "stop" in section.lower()
    assert "Name the doc" in section


def test_skill_md_never_dispatches_and_names_its_sources() -> None:
    text = SKILL_MD.read_text()
    assert "never dispatches a workflow" in text
    sources = yaml.safe_load(SOURCES.read_text())["sources"]
    for source in sources:
        if source["path"].startswith(".github/workflows/"):
            assert source["path"].split("/")[-1] in text


# ---------------------------------------------------------------------------
# Registry and install
# ---------------------------------------------------------------------------

AGENTS = ["claude", "codex", "vibe", "antigravity", "opencode"]


def test_registry_lists_the_skill_as_a_local_skill_for_every_agent() -> None:
    skills = render_registry.load_registry()["local_skills"]
    entry = next(s for s in skills if s["name"] == "estate-rollback")
    assert entry["agents"] == AGENTS
    assert entry["surfaces"] == ["workstation"]
    assert entry["path"] == "skills/workstation/estate-rollback"
    assert (KIT_ROOT / entry["path"] / "SKILL.md").is_file()


@pytest.fixture(scope="module")
def fresh(tmp_path_factory: pytest.TempPathFactory) -> Run:
    return setup_run(tmp_path_factory.mktemp("rollback-fresh"), "Darwin")


def test_the_rendered_setup_installs_the_skill_without_a_script(fresh: Run) -> None:
    for agent in (".claude", ".codex"):
        installed = fresh.home / agent / "skills" / "estate-rollback"
        assert (installed / "SKILL.md").read_bytes() == SKILL_MD.read_bytes()
        assert not (installed / "scripts").exists()
        assert (installed / "sources.yaml").read_bytes() == SOURCES.read_bytes()
