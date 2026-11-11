"""The `/recover-decisions` skill (#480): declared for every agent, installed
beside the shared `critique` skill, ruling recovered decisions by a table
that includes `not-significant`, `duplicates-adr` and `invented-rationale`,
asking the owner at most ten questions, writing an ADR only from the owner's
answer, and ending by suggesting `/critique-adrs`.
"""

from __future__ import annotations

import json
import re
import sys
from pathlib import Path

import pytest
import yaml

KIT_ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(KIT_ROOT / "scripts"))

from setup_harness import Run, setup_run  # noqa: E402

CRITIQUE = KIT_ROOT / "skills" / "workstation" / "critique"
SKILL = KIT_ROOT / "skills" / "workstation" / "recover-decisions" / "SKILL.md"
HOMES = (".claude", ".codex", ".vibe", ".vibe-private", ".gemini/config", ".gemini/antigravity-cli")


def _text() -> str:
    return " ".join(SKILL.read_text().split())


def _ruling_rows() -> dict[str, str]:
    text = SKILL.read_text()
    table = text[text.index("| Candidate | Verdict |") :].split("\n\n", 1)[0]
    rows = [line.strip("|").split(" | ") for line in table.splitlines()[2:]]
    return {cells[0].strip(): cells[1].strip() for cells in rows}


def _record_template() -> str:
    text = SKILL.read_text()
    block = re.search(r"```markdown\n(.*?)```", text, flags=re.DOTALL)
    assert block is not None, "the skill shows the record's house format"
    return "\n".join(line.removeprefix("  ") for line in block.group(1).splitlines())


@pytest.fixture(scope="module")
def macos_run(tmp_path_factory: pytest.TempPathFactory) -> Run:
    return setup_run(tmp_path_factory.mktemp("recover-decisions"), "Darwin")


def test_recover_decisions_is_a_built_local_skill_for_every_agent() -> None:
    data = yaml.safe_load((KIT_ROOT / "registry" / "estate-tooling.yaml").read_text())
    entry = next(s for s in data["local_skills"] if s["name"] == "recover-decisions")
    assert entry["path"] == "skills/workstation/recover-decisions"
    assert set(entry["agents"]) == {"claude", "codex", "vibe", "antigravity", "opencode"}
    assert entry["surfaces"] == ["workstation"]
    assert "pending" not in entry
    assert SKILL.is_file()


def test_its_description_says_when_to_load_it() -> None:
    front = yaml.safe_load(SKILL.read_text().split("---")[1])
    assert front["name"] == "recover-decisions"
    assert front["description"].startswith("Use when the owner asks to recover the decisions")
    assert front["disable-model-invocation"] is True


def test_setup_installs_it_beside_the_shared_skill_in_every_agent_home(macos_run: Run) -> None:
    for home in HOMES:
        skills = macos_run.home / home / "skills"
        assert (skills / "recover-decisions" / "SKILL.md").read_bytes() == SKILL.read_bytes(), home
        assert (skills / "recover-decisions" / ".." / "critique" / "critique.mjs").is_file(), home


def test_it_runs_the_review_stage_and_never_passes_a_flow() -> None:
    text = _text()
    assert 'stage: "decision-recovery"' in text
    assert "Never pass `flow`: a review run is its own flow." in text


def test_its_ruling_table_has_the_three_rejections_and_never_accepts() -> None:
    rows = _ruling_rows()
    verdicts = " ".join(rows.values())
    for code in ("`not-significant`", "`duplicates-adr`", "`invented-rationale`"):
        assert code in verdicts, code
    assert next(v for v in rows.values() if v.startswith("`needs-human`"))
    assert "A recovered decision is never `accepted`" in _text()
    assert "`not-significant`" in next(v for r, v in rows.items() if "three tests" in r)


def test_it_names_the_three_tests_the_domain_modeling_skill_sets() -> None:
    row = next(r for r in _ruling_rows() if "three tests" in r)
    assert "easy to reverse" in row
    assert "not surprising without its context" in row
    assert "no real trade-off" in row


def test_it_asks_at_most_ten_questions_in_order_of_confidence_and_defers_the_rest() -> None:
    text = _text()
    assert "Ask the first ten, one at a time" in text
    assert "Never ask an eleventh question." in text
    assert "by severity, critical first" in text
    assert "the report lists them" in text
    assert "is this your decision; why; what did you reject; and what does it cost" in text


def test_the_record_it_writes_has_a_rejected_alternative_a_cost_and_no_status_line() -> None:
    template = _record_template()
    assert not re.search(r"^status\b", template, flags=re.IGNORECASE | re.MULTILINE)
    assert "we decided for" in template
    assert re.search(r"^We rejected .+: ", template, flags=re.MULTILINE)
    assert "## Consequences" in template
    assert "the cost the owner named" in template
    assert "with **no status line**" in _text()


def test_it_writes_nothing_before_the_owner_accepts_and_notes_an_accident_for_a_refactoring_plan() -> None:
    text = _text()
    assert "write it only once they approve it" in text
    assert "nothing of it is written under the decision log" in text
    assert "write nothing to the decision log" in text
    assert "refactoring-notes.md" in text
    assert "Never write a reason the owner did not give" in text


def test_it_ends_by_suggesting_critique_adrs_and_spends_nothing_more() -> None:
    text = _text()
    assert "suggest `/critique-adrs` on the ADRs you wrote" in text
    assert "run nothing more, and spend nothing more" in text


def test_it_offers_the_code_permission_and_says_what_the_history_permission_adds() -> None:
    text = _text()
    assert '`"code": true`' in text
    assert "history permission" in text
    assert "off by default" in text
    assert '"history": true' in text


def test_the_pipeline_readme_has_the_stage_row_for_decision_recovery() -> None:
    readme = (CRITIQUE / "README.md").read_text()
    row = next(line for line in readme.splitlines() if line.startswith("| `decision-recovery` |"))
    assert row.startswith("| `decision-recovery` | `/recover-decisions` |")
    assert "DeepSeek Flash, Mistral Large (`decision-recoverer`)" in row


def test_shipped_routing_gives_decision_recovery_deepseek_and_mistral_large_and_its_depth_signals() -> None:
    stage = json.loads((CRITIQUE / "routing.json").read_text())["stages"]["decision-recovery"]
    assert [(slot["client"], slot["role"]) for slot in stage["base"]] == [
        ("vibe-deepseek-flash", "decision-recoverer"),
        ("vibe-mistral-large", "decision-recoverer"),
    ]
    assert stage["distinct_families"] is True
    assert stage["depth_signals"]["thresholds"] == {"source_lines": 20000, "context_maps": 0}
    assert "deep" not in stage
    assert (CRITIQUE / "roles" / "decision-recoverer.md").is_file()
