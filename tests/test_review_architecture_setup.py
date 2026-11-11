"""The `/review-architecture` skill (#483): declared for every agent, installed
beside the shared `critique` skill, running the `architecture-review` stage,
ruling findings by a table, capping the owner's questions at ten across both
kinds, filing no issue, and offering `/improve-codebase-architecture`.
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

import pytest
import yaml

KIT_ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(KIT_ROOT / "scripts"))

from setup_harness import Run, setup_run  # noqa: E402

CRITIQUE = KIT_ROOT / "skills" / "workstation" / "critique"
SKILL = KIT_ROOT / "skills" / "workstation" / "review-architecture" / "SKILL.md"
HOMES = (".claude", ".codex", ".vibe", ".vibe-private", ".gemini/config", ".gemini/antigravity-cli")
LENSES = ("dependency-direction", "coupling", "domain-language", "aggregates", "test-seams", "quality-scenarios")


def _text() -> str:
    return " ".join(SKILL.read_text().split())


@pytest.fixture(scope="module")
def macos_run(tmp_path_factory: pytest.TempPathFactory) -> Run:
    return setup_run(tmp_path_factory.mktemp("review-architecture"), "Darwin")


def test_review_architecture_is_a_built_local_skill_for_every_agent() -> None:
    data = yaml.safe_load((KIT_ROOT / "registry" / "estate-tooling.yaml").read_text())
    entry = next(s for s in data["local_skills"] if s["name"] == "review-architecture")
    assert entry["path"] == "skills/workstation/review-architecture"
    assert set(entry["agents"]) == {"claude", "codex", "vibe", "antigravity", "opencode"}
    assert entry["surfaces"] == ["workstation"]
    assert "pending" not in entry
    assert SKILL.is_file()


def test_its_description_says_when_to_load_it() -> None:
    front = yaml.safe_load(SKILL.read_text().split("---")[1])
    assert front["name"] == "review-architecture"
    assert front["description"].startswith("Use when the owner asks to review a repository's architecture")
    assert front["disable-model-invocation"] is True


def test_setup_installs_it_beside_the_shared_skill_in_every_agent_home(macos_run: Run) -> None:
    for home in HOMES:
        skills = macos_run.home / home / "skills"
        assert (skills / "review-architecture" / "SKILL.md").read_bytes() == SKILL.read_bytes(), home
        assert (skills / "review-architecture" / ".." / "critique" / "critique.mjs").is_file(), home
        assert (skills / "critique" / "roles" / "architecture-critic.md").is_file(), home


def test_it_runs_the_review_stage_and_never_passes_a_flow() -> None:
    text = _text()
    assert 'stage: "architecture-review"' in text
    assert "Never pass `flow`: a review run is its own flow." in text


def test_it_rules_by_a_table_and_never_accepts_a_recovered_decision() -> None:
    text = SKILL.read_text()
    assert "| Finding | Verdict |" in text
    for code in ("`taste`", "`verdict-word`", "`needs-human`", "`accepted`"):
        assert code in text, code
    assert "never `accepted`" in _text()


def test_it_caps_the_owners_questions_at_ten_across_both_kinds() -> None:
    text = _text()
    assert "at most ten questions, counted across both kinds" in text
    assert "ask the first ten one at a time" in text
    assert "The rest are **deferred**" in text


def test_it_files_no_issue_and_offers_the_improvement_skill() -> None:
    text = _text()
    assert "No issue is filed automatically" in text
    assert "never run `gh issue create`" in text
    assert "Offer `/improve-codebase-architecture` on the candidate the owner picks" in text


def test_it_needs_the_code_permission_and_reads_no_history() -> None:
    text = _text()
    assert '`"code": true`' in text
    assert "It reads no history" in text


def test_the_pipeline_readme_has_the_stage_row_for_architecture_review() -> None:
    readme = (CRITIQUE / "README.md").read_text()
    row = next(line for line in readme.splitlines() if line.startswith("| `architecture-review` |"))
    assert row.startswith("| `architecture-review` | `/review-architecture` |")
    assert "`architecture-critic`" in row
    assert "`decision-recoverer`" in row


def test_shipped_routing_follows_the_architecture_review_roster() -> None:
    stage = json.loads((CRITIQUE / "routing.json").read_text())["stages"]["architecture-review"]
    base = [(slot["client"], slot["role"], slot["lenses"]) for slot in stage["base"]]
    assert base == [
        ("agy-gemini-flash-high", "architecture-critic", ["dependency-direction", "coupling"]),
        ("vibe-mistral-large", "architecture-critic", ["domain-language", "aggregates"]),
    ]
    deep = [(slot["client"], slot["role"], slot["lenses"]) for slot in stage["deep"]]
    assert deep == [
        ("vibe-glm", "architecture-critic", ["test-seams", "quality-scenarios"]),
        ("vibe-deepseek-flash", "decision-recoverer", []),
    ]
    assert stage["distinct_families"] is True
    assert stage["depth_signals"]["thresholds"] == {"source_lines": 20000, "context_maps": 0}


def test_every_lens_the_routing_names_exists_and_so_do_the_roles() -> None:
    stage = json.loads((CRITIQUE / "routing.json").read_text())["stages"]["architecture-review"]
    named = {lens for tier in ("work", "base", "deep") for slot in stage[tier] for lens in slot["lenses"]}
    assert named == set(LENSES)
    for lens in named:
        assert (CRITIQUE / "lenses" / f"{lens}.md").is_file(), lens
    assert (CRITIQUE / "roles" / "architecture-critic.md").is_file()
    assert (CRITIQUE / "roles" / "decision-recoverer.md").is_file()
