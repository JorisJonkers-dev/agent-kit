"""The estate-project-intent skill: its named sources and its install.

scaffold and validate are the `estate intent` subcommands of the `estate` crate
(scripts/crates/estate); its tests are there. What stays here is the skill text.
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

SKILL = KIT_ROOT / "skills" / "workstation" / "estate-project-intent"
SKILL_MD = SKILL / "SKILL.md"
SOURCES = SKILL / "sources.yaml"


# ---------------------------------------------------------------------------
# The skill names its sources and its two disagreements
# ---------------------------------------------------------------------------


def test_skill_md_names_the_deploy_kit_spec_paths_from_sources_yaml() -> None:
    text = SKILL_MD.read_text()
    sources = yaml.safe_load(SOURCES.read_text())["sources"]
    spec_paths = [s["path"] for s in sources if s["kind"] == "file" and s["repo"] == "JorisJonkers-dev/deploy-kit"]
    assert "spec/v1/10-project-intent.md" in spec_paths
    for path in spec_paths:
        assert path.split("/")[-1] in text, path
    assert "spec/v1/10-project-intent.md" in text
    assert "JorisJonkers-dev/github-workflows" in text


def _disagreement_section() -> str:
    text = SKILL_MD.read_text()
    start = text.index("## Where the docs and reality disagree")
    end = text.index("## When a doc cannot be read")
    return text[start:end]


def test_skill_md_states_the_location_disagreement() -> None:
    section = _disagreement_section()
    assert "(a) Location" in section
    assert "`platform/`" in section
    assert "`deploy/`" in section


def test_skill_md_states_the_toolkit_pin_disagreement() -> None:
    section = _disagreement_section()
    assert "(b) Toolkit pin" in section
    assert "1.0.0" in section
    assert "1.10.0" in section
    assert "toolkit.yml" in section


def test_skill_md_names_every_file_source_it_reads_in_sources_yaml() -> None:
    text = SKILL_MD.read_text()
    recorded = {(s["repo"], s["path"]) for s in yaml.safe_load(SOURCES.read_text())["sources"] if s["kind"] == "file"}
    for needed in (
        ("JorisJonkers-dev/deploy-kit", "spec/v1/examples/minimal/README.md"),
        ("JorisJonkers-dev/deploy-kit", "spec/v1/examples/minimal/env/notes-api/base.env"),
        ("JorisJonkers-dev/n8n", ".github/workflows/publish.yml"),
        ("JorisJonkers-dev/n8n", "tools/deploy-kit/package.json"),
        ("JorisJonkers-dev/github-workflows", ".github/workflows/publish-fragment.yml"),
        ("JorisJonkers-dev/estate", "toolkit.yml"),
    ):
        assert needed in recorded, needed
        assert needed[1].split("/")[-1] in text, needed


def test_skill_md_validation_step_says_which_deploy_kit_gates_publishing() -> None:
    text = SKILL_MD.read_text()
    step = text[text.index("3. Validate.") : text.index("4. Wire publishing.")]
    assert "gates publishing" in step
    assert "toolkit.yml" in step
    assert "tools/deploy-kit" in step


def test_skill_md_step_4_commands_print_what_they_claim() -> None:
    text = SKILL_MD.read_text()
    step = text[text.index("4. Wire publishing.") : text.index("5. Propose")]
    assert "'.object.type + \" \" + .object.sha'" in step
    assert "Accept: application/vnd.github.raw" in step


def test_skill_md_says_to_stop_and_name_the_doc_that_cannot_be_read() -> None:
    text = SKILL_MD.read_text()
    section = text[text.index("## When a doc cannot be read") :]
    assert "STOP" in section
    assert "Name the doc you could not read" in section


# ---------------------------------------------------------------------------
# Registry and install
# ---------------------------------------------------------------------------

AGENTS = ["claude", "codex", "vibe", "antigravity", "opencode"]


def test_registry_lists_the_skill_as_a_local_skill_for_every_agent() -> None:
    skills = render_registry.load_registry()["local_skills"]
    entry = next(s for s in skills if s["name"] == "estate-project-intent")
    assert entry["agents"] == AGENTS
    assert entry["surfaces"] == ["workstation"]
    assert entry["path"] == "skills/workstation/estate-project-intent"
    assert (KIT_ROOT / entry["path"] / "SKILL.md").is_file()


@pytest.fixture(scope="module")
def fresh(tmp_path_factory: pytest.TempPathFactory) -> Run:
    return setup_run(tmp_path_factory.mktemp("intent-fresh"), "Darwin")


def test_the_rendered_setup_installs_the_skill_without_a_script(fresh: Run) -> None:
    for agent in (".claude", ".codex"):
        installed = fresh.home / agent / "skills" / "estate-project-intent"
        assert (installed / "SKILL.md").read_bytes() == SKILL_MD.read_bytes()
        assert not (installed / "scripts").exists()
        assert (installed / "sources.yaml").read_bytes() == SOURCES.read_bytes()
