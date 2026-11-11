"""The `/critique-plan` skill and the `plan` stage's shipped files (#486): the skill is
declared for every agent and installed beside the shared `critique` skill; the
`plan-critic` role and its two lenses fix a severity per rule; routing names the
stage's roster and depth signals; and the README has the stage row.
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
SKILL = KIT_ROOT / "skills" / "workstation" / "critique-plan" / "SKILL.md"
HOMES = (".claude", ".codex", ".vibe", ".vibe-private", ".gemini/config", ".gemini/antigravity-cli")
CHECK = re.compile(r"^- `([a-z0-9]+(?:-[a-z0-9]+)*)`(?: \((critical|major|minor)\))?:", re.MULTILINE)
SAFETY = ("mixed-change", "no-safety-net", "big-bang-change", "broken-main-order", "no-rollback", "scaffold-left")
ANCHORING = (
    "unanchored-step",
    "stale-hotspot",
    "adr-contradiction",
    "closes-import-cycle",
    "premature-service-split",
    "single-adapter-seam",
    "deletion-without-decision",
)


def _text() -> str:
    return " ".join(SKILL.read_text().split())


def _checks(path: Path) -> list[tuple[str, str | None]]:
    return CHECK.findall(path.read_text())


@pytest.fixture(scope="module")
def macos_run(tmp_path_factory: pytest.TempPathFactory) -> Run:
    return setup_run(tmp_path_factory.mktemp("critique-plan"), "Darwin")


def test_critique_plan_is_a_built_local_skill_for_every_agent() -> None:
    data = yaml.safe_load((KIT_ROOT / "registry" / "estate-tooling.yaml").read_text())
    entry = next(s for s in data["local_skills"] if s["name"] == "critique-plan")
    assert entry["path"] == "skills/workstation/critique-plan"
    assert set(entry["agents"]) == {"claude", "codex", "vibe", "antigravity", "opencode"}
    assert entry["surfaces"] == ["workstation"]
    assert "pending" not in entry
    assert SKILL.is_file()


def test_its_description_says_when_to_load_it() -> None:
    front = yaml.safe_load(SKILL.read_text().split("---")[1])
    assert front["name"] == "critique-plan"
    assert front["description"].startswith("Use after /improve-codebase-architecture has grilled a refactor candidate")
    assert front["disable-model-invocation"] is True


def test_setup_installs_it_beside_the_shared_skill_in_every_agent_home(macos_run: Run) -> None:
    for home in HOMES:
        skills = macos_run.home / home / "skills"
        assert (skills / "critique-plan" / "SKILL.md").read_bytes() == SKILL.read_bytes(), home
        assert (skills / "critique" / "critique.mjs").is_file(), home
        assert (skills / "critique" / "roles" / "plan-critic.md").is_file(), home
        for lens in ("incremental-safety", "evidence-anchoring"):
            assert (skills / "critique" / "lenses" / f"{lens}.md").is_file(), (home, lens)


def test_it_runs_the_plan_stage_in_the_drafts_flow() -> None:
    text = _text()
    assert 'stage: "plan"' in text
    assert "`flow` (the draft flow)" in text
    assert "`plan_path`" in text


def test_it_drafts_the_plan_when_none_exists_and_never_commits_it() -> None:
    text = _text()
    assert "there is no plan yet. Draft it from the grilling" in text
    assert "Never commit it" in text
    assert "`.scratch/<draft>/refactoring-plan.md`" in text
    assert "`check-plan`" in text


def test_it_rules_by_a_table_and_asks_one_question_for_an_adr_clash() -> None:
    text = SKILL.read_text()
    assert "| Verdict | When |" in text
    for code in ("`taste`", "`fact-misread`", "`needs-human`", "`accepted`"):
        assert code in text, code
    assert '"Keep ADR-NNNN and drop S0n, or supersede ADR-NNNN?"' in _text()


def test_it_gates_to_spec_on_the_open_decisions_and_runs_a_second_round() -> None:
    text = _text()
    assert "`/to-spec` runs only when the plan's `## Open decisions` section is empty" in text
    assert '"previous_run_dir": <this run_dir>' in text


def test_it_needs_the_code_permission() -> None:
    text = _text()
    assert '`"code": true`' in text
    assert "This stage sends code" in text


def test_the_pipeline_readme_has_the_stage_row_for_plan() -> None:
    readme = (CRITIQUE / "README.md").read_text()
    row = next(line for line in readme.splitlines() if line.startswith("| `plan` |"))
    assert row.startswith("| `plan` | `/critique-plan` |")
    assert "`plan-critic`" in row
    assert "`incremental-safety`" in row
    assert "`evidence-anchoring`" in row


def test_the_flow_skill_names_the_plan_stage_skill() -> None:
    flow = (KIT_ROOT / "skills" / "workstation" / "flow" / "SKILL.md").read_text()
    assert "| `plan` | `critique-plan` |" in flow


def test_shipped_routing_follows_the_plan_roster() -> None:
    stage = json.loads((CRITIQUE / "routing.json").read_text())["stages"]["plan"]
    base = [(slot["client"], slot["role"], slot["lenses"]) for slot in stage["base"]]
    assert base == [
        ("vibe-deepseek-flash", "plan-critic", ["incremental-safety"]),
        ("vibe-mistral-large", "plan-critic", ["evidence-anchoring"]),
    ]
    deep = [(slot["client"], slot["role"], slot["lenses"]) for slot in stage["deep"]]
    assert deep == [("vibe-glm", "plan-critic", ["coupling", "test-seams"])]
    assert stage["distinct_families"] is True
    assert stage["depth_signals"]["thresholds"] == {"steps": 8, "stored_data_steps": 0}
    spec = json.loads((CRITIQUE / "routing.json").read_text())["stages"]["spec"]
    assert stage["depth_signals"]["risk_terms"] == spec["depth_signals"]["risk_terms"]


def test_every_lens_the_routing_names_exists_and_so_does_the_role() -> None:
    stage = json.loads((CRITIQUE / "routing.json").read_text())["stages"]["plan"]
    named = {lens for tier in ("work", "base", "deep") for slot in stage[tier] for lens in slot["lenses"]}
    assert named == {"incremental-safety", "evidence-anchoring", "coupling", "test-seams"}
    for lens in named:
        assert (CRITIQUE / "lenses" / f"{lens}.md").is_file(), lens
    assert (CRITIQUE / "roles" / "plan-critic.md").is_file()


def test_each_lens_declares_its_rules_each_with_a_fixed_severity() -> None:
    safety = _checks(CRITIQUE / "lenses" / "incremental-safety.md")
    anchoring = _checks(CRITIQUE / "lenses" / "evidence-anchoring.md")
    assert tuple(id_ for id_, _ in safety) == SAFETY
    assert tuple(id_ for id_, _ in anchoring) == ANCHORING
    assert all(severity for _, severity in safety + anchoring), (safety, anchoring)
    # A rule with no way back, a broken main, a contradicted ADR and untested structure are the critical ones.
    critical = {id_ for id_, severity in safety + anchoring if severity == "critical"}
    assert critical == {"no-safety-net", "broken-main-order", "no-rollback", "adr-contradiction"}


def test_the_role_names_one_rule_of_its_own_and_fixes_no_severity() -> None:
    role = _checks(CRITIQUE / "roles" / "plan-critic.md")
    assert role == [("plan-consequence", "")]


def test_the_runner_knows_exactly_the_rules_the_lenses_declare() -> None:
    source = (
        KIT_ROOT / "council" / "ts" / "src" / "contexts" / "critique" / "domain" / "refactoring-plan.ts"
    ).read_text()
    block = source.split("export const PLAN_RULES = [", 1)[1].split("] as const", 1)[0]
    known = re.findall(r"'([a-z0-9-]+)'", block)
    assert known[: len(SAFETY) + len(ANCHORING) + 1] == [*SAFETY, *ANCHORING, "plan-consequence"]
    for lens in ("coupling", "test-seams"):
        assert {i for i, _ in _checks(CRITIQUE / "lenses" / f"{lens}.md")} <= set(known), lens
