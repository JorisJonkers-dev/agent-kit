"""The estate-bootstrap skill: its named sources and its install.

The offline audit itself is the `estate audit` subcommand of the `estate` crate
(scripts/crates/estate); its tests are there. What stays here is the skill's text.
"""

from __future__ import annotations

import sys
from pathlib import Path

import pytest
import yaml
from setup_harness import Run, setup_run, skill_text

KIT_ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(KIT_ROOT / "scripts"))

import render_registry  # noqa: E402

SKILL = KIT_ROOT / "skills" / "workstation" / "estate-bootstrap"
SKILL_MD = SKILL / "SKILL.md"
SOURCES = SKILL / "sources.yaml"


# ---------------------------------------------------------------------------
# The skill names its disagreements, its rules and its sources
# ---------------------------------------------------------------------------


def _section(heading: str) -> str:
    text = skill_text(SKILL_MD)
    start = text.index(heading)
    nxt = text.find("\n## ", start + 1)
    return text[start:] if nxt == -1 else text[start:nxt]


def _bullets() -> list[str]:
    section = _section("## 5. Where the docs and reality disagree")
    return [b for b in section.split("\n- ")[1:]]


def _bullet(key: str) -> str:
    found = [b for b in _bullets() if key in b]
    assert len(found) == 1, key
    return found[0]


@pytest.mark.parametrize(
    ("key", "followed", "audit"),
    [
        ("is not a GitHub template", "the live repository setting", "no,"),
        ("Workflow pins across the org", "the latest github-workflows release", "yes,"),
        ("migrates with goose", "`ADR-0021`", "no;"),
        ("run several CI jobs", "`ADR-0018`", "only the count"),
        ("recovery scripts", "what exists in `fleet-infra`", "no;"),
        ("deploy-config-schema trap", "deploy-kit", "yes,"),
        ("Coverage:", "`testing-standards` section 6 and `ADR-0022`", "no;"),
        ("under `inbox/`", "REPO_SETUP", "no."),
        ("`fleet.yaml` is flagged", "no document", "yes, with the source `none (unverified)`"),
    ],
)
def test_each_disagreement_bullet_names_the_side_followed_and_the_audit(key: str, followed: str, audit: str) -> None:
    bullet = _bullet(key)
    after_followed = bullet.split("Followed:", 1)[1]
    assert after_followed.split("The audit reports it:", 1)[0].count(followed) == 1
    assert "The audit reports it:" in bullet
    assert bullet.split("The audit reports it:", 1)[1].strip().startswith(audit)


def test_every_disagreement_bullet_is_covered_by_the_table() -> None:
    assert len(_bullets()) == 9
    assert all("Followed:" in b and "The audit reports it:" in b for b in _bullets())


def test_skill_md_points_at_docs_instead_of_restating_their_numbers() -> None:
    text = skill_text(SKILL_MD)
    assert "14 days" not in text
    assert "80%" not in text
    assert "100%" not in text
    assert "v0.45" not in text
    assert "inside the window the standards give" in text
    assert "live" in text.split("## 1. Read first")[0]
    assert "the live read wins" in text


def test_skill_md_documents_the_audit_scope_and_the_platform_caveat() -> None:
    audit = _section("## 3. Audit")
    assert "`templates/**/*.tmpl`" in audit
    assert "`.github/actions/`" in audit
    assert "top-level `jobs:` key" in audit
    assert "could not confirm the preset" in audit
    assert "not meaningful for the `workspace` repository" in audit


def test_skill_md_labels_the_fleet_yaml_check_as_unverified() -> None:
    assert "treat it as unverified" in _bullet("`fleet.yaml` is flagged")


def test_skill_md_files_issues_only_after_yes_and_never_opens_template_prs() -> None:
    text = skill_text(SKILL_MD)
    rules = _section("## Rules")
    assert "File an issue only after the owner says yes" in rules
    assert "Never open a pull request against a template or library repository" in rules
    improve = _section("## 4. Improve")
    assert "Once the owner says yes, file it with `gh issue create`" in improve
    assert "Never open a pull request against a template or library repository" in improve
    assert "gh label list" in text


def test_skill_md_says_to_stop_and_name_the_doc_that_cannot_be_read() -> None:
    section = _section("## When a doc cannot be read")
    assert "stop" in section.lower()
    assert "Name the document" in section
    assert "Do not fall back to remembered content" in section


def test_sources_yaml_has_a_release_entry_for_github_workflows() -> None:
    sources = yaml.safe_load(SOURCES.read_text())["sources"]
    releases = [s for s in sources if s["kind"] == "release" and s["repo"] == "JorisJonkers-dev/github-workflows"]
    assert len(releases) == 1
    assert releases[0]["tag"].startswith("v")


# ---------------------------------------------------------------------------
# Registry and install
# ---------------------------------------------------------------------------

AGENTS = ["claude", "codex", "vibe", "antigravity", "opencode"]


def test_registry_lists_the_skill_as_a_local_skill_for_every_agent() -> None:
    skills = render_registry.load_registry()["local_skills"]
    entry = next(s for s in skills if s["name"] == "estate-bootstrap")
    assert entry["agents"] == AGENTS
    assert entry["surfaces"] == ["workstation"]
    assert entry["path"] == "skills/workstation/estate-bootstrap"
    assert (KIT_ROOT / entry["path"] / "SKILL.md").is_file()


@pytest.fixture(scope="module")
def fresh(tmp_path_factory: pytest.TempPathFactory) -> Run:
    return setup_run(tmp_path_factory.mktemp("bootstrap-fresh"), "Darwin")


def test_the_rendered_setup_installs_the_skill_without_a_script(fresh: Run) -> None:
    for agent in (".claude", ".codex"):
        installed = fresh.home / agent / "skills" / "estate-bootstrap"
        assert (installed / "SKILL.md").read_bytes() == SKILL_MD.read_bytes()
        assert not (installed / "scripts").exists()
        assert (installed / "sources.yaml").read_bytes() == SOURCES.read_bytes()
