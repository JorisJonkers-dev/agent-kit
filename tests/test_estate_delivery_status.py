"""The estate-delivery-status skill: its worked examples, its sources and its install.

The verdict script is the `estate verdict` subcommand of the `estate` crate
(scripts/crates/estate), whose tests run the three worked examples. What stays here is
the skill text.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

import pytest
import yaml
from setup_harness import Run, setup_run

KIT_ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(KIT_ROOT / "scripts"))

import render_registry  # noqa: E402

SKILL = KIT_ROOT / "skills" / "workstation" / "estate-delivery-status"
SKILL_MD = SKILL / "SKILL.md"
SOURCES = SKILL / "sources.yaml"

EXAMPLES = ("landed", "mixed", "not landed")


# ---------------------------------------------------------------------------
# SKILL.md: the verdict rule and the three worked examples
# ---------------------------------------------------------------------------


def missing_examples(text: str) -> list[str]:
    """The worked-example headings that `text` lacks."""
    return [name for name in EXAMPLES if f"### Worked example: {name}\n" not in text]


def example_section(text: str, name: str) -> str:
    heading = f"### Worked example: {name}\n"
    start = text.index(heading)
    rest = text[start + len(heading) :]
    following = re.search(r"^#{1,3} ", rest, re.MULTILINE)
    return rest[: following.start()] if following else rest


def test_skill_md_has_all_three_worked_examples() -> None:
    assert missing_examples(SKILL_MD.read_text()) == []


@pytest.mark.parametrize("name", EXAMPLES)
def test_the_checker_fails_when_a_worked_example_is_removed(name: str) -> None:
    text = SKILL_MD.read_text().replace(f"### Worked example: {name}\n", "")
    assert missing_examples(text) == [name]


def test_skill_md_states_the_verdict_rule_and_that_the_deployment_does_not_gate_landed() -> None:
    text = SKILL_MD.read_text()
    assert "**landed** only when the compose row is clean and `estate/production` is success" in text
    assert "The Deployment and the publish run do not gate the verdict" in text
    assert "**not landed** when neither of the two is good, whatever the Deployment says" in text
    assert "**mixed** when exactly one of the two is good" in text
    assert "headSha" in text
    assert "differs from `sha`" in text


# Phrases from each example's section; the estate crate feeds the same values to `estate verdict`.
EXAMPLE_FACTS = {
    "landed": ["compose `clean`", "status `success`", "deployment `success`", "no open issues"],
    "mixed": ["compose `clean`", "status `pending`", "deployment `in_progress`", "`home-portal/isolated`"],
    "not landed": ["compose `errors`", "`E_UNKNOWN_SURFACE`", "status `failure`", "deployment `missing`"],
}


@pytest.mark.parametrize("name", EXAMPLES)
def test_each_worked_example_gives_the_verdict_it_states(name: str) -> None:
    section = example_section(SKILL_MD.read_text(), name)
    for fact in EXAMPLE_FACTS[name]:
        assert fact in section, fact
    stated = re.search(r"Result: `VERDICT: ([a-z ]+)`", section)
    assert stated is not None
    assert stated.group(1) == name


# ---------------------------------------------------------------------------
# Sources and the stop rule
# ---------------------------------------------------------------------------


def test_skill_md_names_the_docs_from_sources_yaml() -> None:
    text = SKILL_MD.read_text()
    sources = yaml.safe_load(SOURCES.read_text())["sources"]
    assert sources
    for source in sources:
        assert source["kind"] == "file"
        assert Path(source["path"]).name in text, source["path"]
        assert source["repo"].split("/")[-1] in text, source["repo"]


def test_skill_md_says_to_stop_and_name_the_doc_that_cannot_be_read() -> None:
    text = SKILL_MD.read_text()
    section = text[text.index("## When a doc cannot be read") :]
    assert "STOP" in section
    assert "Name the doc" in section
    assert "repository and path" in section
    assert "Do not answer from memory" in section


# ---------------------------------------------------------------------------
# Registry and install
# ---------------------------------------------------------------------------

AGENTS = ["claude", "codex", "vibe", "antigravity", "opencode"]


def test_registry_lists_the_skill_as_a_local_skill_for_every_agent() -> None:
    skills = render_registry.load_registry()["local_skills"]
    entry = next(s for s in skills if s["name"] == "estate-delivery-status")
    assert entry["agents"] == AGENTS
    assert entry["surfaces"] == ["workstation"]
    assert entry["path"] == "skills/workstation/estate-delivery-status"
    assert (KIT_ROOT / entry["path"] / "SKILL.md").is_file()


@pytest.fixture(scope="module")
def fresh(tmp_path_factory: pytest.TempPathFactory) -> Run:
    return setup_run(tmp_path_factory.mktemp("delivery-fresh"), "Darwin")


def test_the_rendered_setup_installs_the_skill_without_a_script(fresh: Run) -> None:
    for agent in (".claude", ".codex"):
        installed = fresh.home / agent / "skills" / "estate-delivery-status"
        assert (installed / "SKILL.md").read_bytes() == SKILL_MD.read_bytes()
        assert not (installed / "scripts").exists()
        assert (installed / "sources.yaml").read_bytes() == SOURCES.read_bytes()
