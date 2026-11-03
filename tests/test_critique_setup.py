"""The critique pipeline's install: its skills in every agent home, and the
read-only `critic-<model>` Vibe agents in the shared Vibe home only.

The setup assertions run the checked-in setup script against a temp HOME with
every external command stubbed (tests/setup_harness.py), so they check the tree
a run leaves behind. The words are CONTEXT.md's: a critic client is one CLI
pinned to one model, and the Vibe ones are pinned through these agent files.
"""

from __future__ import annotations

import copy
import hashlib
import importlib.util
import json
import re
import subprocess
import sys
import tomllib
from pathlib import Path

import pytest
import yaml

KIT_ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(KIT_ROOT / "scripts"))

import render_registry  # noqa: E402
from setup_harness import Run, setup_run  # noqa: E402

ROUTING = KIT_ROOT / "skills" / "critique" / "routing.json"
AGENTS_DIR = render_registry.VIBE_CRITIC_AGENTS_DIR
EVERY_AGENT = {"claude", "codex", "vibe", "antigravity"}
CRITIC_AGENT_NAMES = ["critic-mistral-large", "critic-mistral-small", "critic-glm", "critic-deepseek-flash"]

# Every key a rendered agent file may hold. Each is a Vibe setting: the
# top-level ones are fields of Vibe's config schema (vibe/core/config/
# vibe_schema.py) or the agent profile's own keys, and #168 loaded agent files
# holding them under VIBE_STRICT_CONFIG=1, which refuses any setting Vibe does
# not use. A key outside these sets is a typo the strict loader would refuse.
PROFILE_KEYS = {"display_name", "description", "safety"}
SETTING_KEYS = {
    "active_model",
    "enabled_tools",
    "enabled_skills",
    "outside_workdir",
    "enable_mcp",
    "enable_subagents",
    "include_project_context",
    "session_logging",
    "tools",
    "models",
}
MODEL_KEYS = {"name", "provider", "alias", "input_price", "output_price", "cached_input_price", "thinking"}


def _registry() -> dict:
    return yaml.safe_load((KIT_ROOT / "registry" / "estate-tooling.yaml").read_text())


def _skill(data: dict, name: str) -> dict:
    return next(s for s in data["local_skills"] if s["name"] == name)


def _critics(data: dict) -> list[dict]:
    return [a for a in data["vibe_critic_agents"] if a["surfaces"]]


def _agent_file(name: str) -> dict:
    return tomllib.loads((KIT_ROOT / AGENTS_DIR / f"{name}.toml").read_text())


@pytest.fixture(scope="module")
def macos_run(tmp_path_factory: pytest.TempPathFactory) -> Run:
    return setup_run(tmp_path_factory.mktemp("critique-macos"), "Darwin")


@pytest.fixture(scope="module")
def check_run(tmp_path_factory: pytest.TempPathFactory) -> Run:
    return setup_run(tmp_path_factory.mktemp("critique-check"), "Darwin", args=("--check",))


# ---------------------------------------------------------------------------
# The skills
# ---------------------------------------------------------------------------


def test_the_shared_critique_skill_is_declared_for_every_agent() -> None:
    skill = _skill(_registry(), "critique")
    assert skill["path"] == "skills/critique"
    assert set(skill["agents"]) == EVERY_AGENT
    assert skill["surfaces"] == ["workstation"]
    assert "pending" not in skill


def test_critique_spec_is_declared_for_every_agent() -> None:
    skill = _skill(_registry(), "critique-spec")
    assert skill["path"] == "skills/critique-spec"
    assert set(skill["agents"]) == EVERY_AGENT
    assert skill["surfaces"] == ["workstation"]


def test_critique_stats_is_declared_for_every_agent_and_built() -> None:
    skill = _skill(_registry(), "critique-stats")
    assert skill["path"] == "skills/critique-stats"
    assert set(skill["agents"]) == EVERY_AGENT
    assert skill["surfaces"] == ["workstation"]
    assert "pending" not in skill
    assert (KIT_ROOT / skill["path"] / "SKILL.md").is_file()


def test_setup_installs_critique_stats_beside_the_shared_skill_in_every_agent_home(macos_run: Run) -> None:
    """The skill finds the runner at ../critique/critique.mjs, so both land side by side."""
    source = KIT_ROOT / "skills" / "critique-stats" / "SKILL.md"
    homes = (".claude", ".codex", ".vibe", ".vibe-private", ".gemini/config", ".gemini/antigravity-cli")
    for home in homes:
        skills = macos_run.home / home / "skills"
        assert (skills / "critique-stats" / "SKILL.md").read_bytes() == source.read_bytes(), home
        assert (skills / "critique-stats" / ".." / "critique" / "critique.mjs").is_file(), home


def test_routing_carries_the_stats_thresholds() -> None:
    """/critique-stats reads its thresholds from routing and never writes them."""
    assert json.loads(ROUTING.read_text())["stats"] == {"min_adjudicated_findings": 20, "removal_features": 3}


def test_the_shared_skill_carries_the_runner_prompts_lenses_schema_and_routing() -> None:
    root = KIT_ROOT / "skills" / "critique"
    for path in ("SKILL.md", "critique.mjs", "routing.json", "prompts/preamble.md", "prompts/triage.md",
                 "roles/spec-redteam.md", "lenses/fidelity.md", "schemas/finding.schema.json"):
        assert (root / path).is_file(), path


def test_setup_copies_the_whole_critique_skill_into_every_agent_home(macos_run: Run) -> None:
    source = KIT_ROOT / "skills" / "critique"
    want = sorted(p.relative_to(source).as_posix() for p in source.rglob("*") if p.is_file())
    homes = (".claude", ".codex", ".vibe", ".vibe-private", ".gemini/config", ".gemini/antigravity-cli")
    for home in homes:
        dest = macos_run.home / home / "skills" / "critique"
        got = sorted(p.relative_to(dest).as_posix() for p in dest.rglob("*") if p.is_file())
        assert got == want, home
        assert (dest / "critique.mjs").read_bytes() == (source / "critique.mjs").read_bytes(), home


def test_critique_spec_is_built_so_it_is_no_longer_pending() -> None:
    skill = _skill(_registry(), "critique-spec")
    assert "pending" not in skill
    assert (KIT_ROOT / skill["path"] / "SKILL.md").is_file()


def test_setup_installs_critique_spec_beside_the_shared_skill_in_every_agent_home(macos_run: Run) -> None:
    """The skill finds the runner at ../critique/critique.mjs, so both land side by side."""
    source = KIT_ROOT / "skills" / "critique-spec" / "SKILL.md"
    homes = (".claude", ".codex", ".vibe", ".vibe-private", ".gemini/config", ".gemini/antigravity-cli")
    for home in homes:
        skills = macos_run.home / home / "skills"
        assert (skills / "critique-spec" / "SKILL.md").read_bytes() == source.read_bytes(), home
        assert (skills / "critique-spec" / ".." / "critique" / "critique.mjs").is_file(), home
    assert "critique-spec: not built yet" not in macos_run.stdout


def test_check_tickets_is_declared_for_every_agent_and_built() -> None:
    skill = _skill(_registry(), "check-tickets")
    assert skill["path"] == "skills/check-tickets"
    assert set(skill["agents"]) == EVERY_AGENT
    assert skill["surfaces"] == ["workstation"]
    assert "pending" not in skill
    assert (KIT_ROOT / skill["path"] / "SKILL.md").is_file()


def test_check_tickets_says_it_applies_after_to_tickets_and_before_the_first_implement() -> None:
    text = (KIT_ROOT / "skills" / "check-tickets" / "SKILL.md").read_text()
    front = yaml.safe_load(text.split("---")[1])
    assert front["name"] == "check-tickets"
    assert front["description"].startswith("Use after /to-tickets and before the first /implement.")


def test_setup_installs_check_tickets_beside_the_shared_skill_in_every_agent_home(macos_run: Run) -> None:
    """The skill finds the runner at ../critique/critique.mjs, so both land side by side."""
    source = KIT_ROOT / "skills" / "check-tickets" / "SKILL.md"
    homes = (".claude", ".codex", ".vibe", ".vibe-private", ".gemini/config", ".gemini/antigravity-cli")
    for home in homes:
        skills = macos_run.home / home / "skills"
        assert (skills / "check-tickets" / "SKILL.md").read_bytes() == source.read_bytes(), home
        assert (skills / "check-tickets" / ".." / "critique" / "critique.mjs").is_file(), home


def test_shipped_routing_defines_the_tickets_stage() -> None:
    tickets = json.loads(ROUTING.read_text())["stages"]["tickets"]
    assert [(s["client"], s["role"], s["lenses"], s["max_price_usd"], s["time_limit_s"]) for s in tickets["base"]] == [
        ("vibe-deepseek-flash", "ticket-linter", ["testability"], 1, 600)
    ]
    assert [(s["client"], s["role"], s["lenses"], s["max_price_usd"], s["time_limit_s"]) for s in tickets["deep"]] == [
        ("vibe-mistral-large", "ticket-linter", ["simplicity"], 1, 600)
    ]
    assert tickets["ticket_size"] == {"max_modules": 5, "max_criteria": 8}
    assert (KIT_ROOT / "skills" / "critique" / "roles" / "ticket-linter.md").is_file()


def test_review_feature_is_declared_for_every_agent() -> None:
    skill = _skill(_registry(), "review-feature")
    assert skill["path"] == "skills/review-feature"
    assert set(skill["agents"]) == EVERY_AGENT
    assert skill["surfaces"] == ["workstation"]
    assert "pending" not in skill


def test_review_feature_says_it_applies_after_all_tickets_landed_and_before_the_pr() -> None:
    text = (KIT_ROOT / "skills" / "review-feature" / "SKILL.md").read_text()
    front = yaml.safe_load(text.split("---")[1])
    assert front["name"] == "review-feature"
    assert front["description"].startswith(
        "Use after every ticket of a spec has landed on its feature branch and before the PR."
    )


def test_setup_installs_review_feature_beside_the_shared_skill_in_every_agent_home(macos_run: Run) -> None:
    """The skill finds the runner at ../critique/critique.mjs, so both land side by side."""
    source = KIT_ROOT / "skills" / "review-feature" / "SKILL.md"
    homes = (".claude", ".codex", ".vibe", ".vibe-private", ".gemini/config", ".gemini/antigravity-cli")
    for home in homes:
        skills = macos_run.home / home / "skills"
        assert (skills / "review-feature" / "SKILL.md").read_bytes() == source.read_bytes(), home
        assert (skills / "review-feature" / ".." / "critique" / "critique.mjs").is_file(), home


def test_shipped_routing_defines_the_adr_review_stage_outside_the_antigravity_reserve() -> None:
    routing = json.loads(ROUTING.read_text())
    review = routing["stages"]["adr-review"]
    assert [(s["client"], s["max_price_usd"]) for s in review["base"]] == [
        ("vibe-mistral-large", 0.5), ("agy-gemini-flash-high", 0.5)
    ]
    assert [s["client"] for s in review["deep"]] == ["vibe-glm"]
    assert [(s["role"], s["lenses"]) for s in review["base"]] == [
        ("adr-reviewer", ["adr-record"]), ("adr-reviewer", ["decision-log"])
    ]
    assert {s["role"] for s in review["deep"]} == {"adr-reviewer"}
    assert review["depth_signals"]["thresholds"] == {"adrs": 10}
    assert review["depth_signals"]["risk_terms"] == routing["stages"]["spec"]["depth_signals"]["risk_terms"]
    assert "adr-review" not in routing["antigravity_quota"]["reserved_stages"]


def test_shipped_routing_caps_the_repository_packet_at_adr_0004s_starting_limits() -> None:
    """#221: the phase-0 live check retunes these in routing, so the runner reads them from there."""
    packet = json.loads(ROUTING.read_text())["packet"]
    assert (packet["cap_bytes"], packet["facts_cap_bytes"], packet["window_lines"]) == (150 * 1024, 30 * 1024, 400)
    assert set(packet["exclude"]) == {"vendored", "generated", "lock"}
    assert "**/node_modules/**" in packet["exclude"]["vendored"]
    assert "**/package-lock.json" in packet["exclude"]["lock"]


def test_shipped_routing_caps_a_code_packets_repo_map_inside_the_packet_cap() -> None:
    """#254: tree/REPO-MAP.md has its own cap, counted inside the packet's."""
    packet = json.loads(ROUTING.read_text())["packet"]
    assert packet["repo_map_cap_bytes"] == 20 * 1024
    assert packet["repo_map_cap_bytes"] < packet["cap_bytes"]


def test_shipped_routing_caps_a_repository_packets_manifest_inside_the_packet_cap() -> None:
    """#256: past its own cap, tree/MANIFEST.tsv counts its least-ranked paths per directory."""
    packet = json.loads(ROUTING.read_text())["packet"]
    assert packet["manifest_cap_bytes"] == 30 * 1024
    assert packet["manifest_cap_bytes"] < packet["cap_bytes"]


def test_flow_reaches_the_review_stages_through_a_separate_entry_that_never_calls_one_due() -> None:
    flow = (KIT_ROOT / "skills" / "flow" / "SKILL.md").read_text()
    assert "| `/flow reviews` |" in flow
    entry = flow[flow.index("## /flow reviews"):]
    end = entry.find("\n## ", 1)
    entry = " ".join((entry if end < 0 else entry[:end]).split())
    assert "critique.mjs reviews" in entry
    assert "never due" in entry
    for field in ("stage", "run_id", "verdicts", "spent_usd"):
        assert f"`{field}`" in entry, field


def test_shipped_routing_defines_the_ticket_review_stage() -> None:
    review = json.loads(ROUTING.read_text())["stages"]["ticket-review"]
    assert [(s["client"], s["role"], s["lenses"], s["max_price_usd"], s["time_limit_s"]) for s in review["base"]] == [
        ("agy-gemini-flash-high", "diff-reviewer", ["failure-modes", "security"], 0.5, 480)
    ]
    assert [(s["client"], s["role"]) for s in review["deep"]] == [("vibe-mistral-large", "diff-reviewer")]
    assert review["depth_signals"]["thresholds"] == {"changed_lines": 600, "risk_path_hits": 3}
    assert (KIT_ROOT / "skills" / "critique" / "roles" / "diff-reviewer.md").is_file()


def test_review_ticket_is_declared_for_every_agent() -> None:
    skill = _skill(_registry(), "review-ticket")
    assert skill["path"] == "skills/review-ticket"
    assert set(skill["agents"]) == EVERY_AGENT
    assert skill["surfaces"] == ["workstation"]
    assert "pending" not in skill


def test_review_ticket_says_it_applies_after_code_review_and_before_the_commit() -> None:
    text = (KIT_ROOT / "skills" / "review-ticket" / "SKILL.md").read_text()
    front = yaml.safe_load(text.split("---")[1])
    assert front["name"] == "review-ticket"
    assert front["description"].startswith("Use after a ticket's /code-review and before its commit.")


def test_review_ticket_says_a_degraded_run_leaves_the_commit_blocked() -> None:
    """#259: a run in which no critic reported vouches for nothing, so its summary must not read as a pass."""
    text = (KIT_ROOT / "skills" / "review-ticket" / "SKILL.md").read_text()
    step = text.split("### 9. Return")[1]
    assert "**No critic reported, so the review is degraded**" in step
    assert "the commit gate stays shut" in step
    assert "another `/review-ticket` run in which a critic reports" in step


def test_setup_installs_review_ticket_beside_the_shared_skill_in_every_agent_home(macos_run: Run) -> None:
    """The skill finds the runner at ../critique/critique.mjs, so both land side by side."""
    source = KIT_ROOT / "skills" / "review-ticket" / "SKILL.md"
    homes = (".claude", ".codex", ".vibe", ".vibe-private", ".gemini/config", ".gemini/antigravity-cli")
    for home in homes:
        skills = macos_run.home / home / "skills"
        assert (skills / "review-ticket" / "SKILL.md").read_bytes() == source.read_bytes(), home
        assert (skills / "review-ticket" / ".." / "critique" / "critique.mjs").is_file(), home


def test_flow_is_declared_for_every_agent() -> None:
    skill = _skill(_registry(), "flow")
    assert skill["path"] == "skills/flow"
    assert set(skill["agents"]) == EVERY_AGENT
    assert skill["surfaces"] == ["workstation"]
    assert "pending" not in skill


def test_flow_loads_only_when_the_owner_asks_about_the_critique_pipeline() -> None:
    text = (KIT_ROOT / "skills" / "flow" / "SKILL.md").read_text()
    front = yaml.safe_load(text.split("---")[1])
    assert front["name"] == "flow"
    assert front["description"].startswith(
        "Use when the owner asks how the critique pipeline works, what to do next in it,"
        " or to set it up or check its budget."
    )


def test_flow_explain_reads_every_section_it_names_from_the_pipeline_readme() -> None:
    """/flow explain answers from skills/critique/README.md, so each section it names must be there."""
    flow = (KIT_ROOT / "skills" / "flow" / "SKILL.md").read_text()
    explain = flow[flow.index("## /flow explain"):]
    end = explain.find("\n## ", 1)
    explain = explain if end < 0 else explain[:end]
    assert "../critique/README.md" in explain
    named = set(re.findall(r"`(## [^`]+)`", explain))
    assert len(named) == 6, named
    headings = {line for line in (KIT_ROOT / "skills" / "critique" / "README.md").read_text().splitlines()
                if line.startswith("## ")}
    assert named <= headings, named - headings


@pytest.mark.parametrize("skill", ["flow", "critique-spec", "critique-decisions"])
def test_the_opt_in_names_the_history_permission_and_says_it_is_off_by_default(skill: str) -> None:
    """The allowlist entry's history permission (commit messages, issue text) is off until the owner turns it on."""
    text = " ".join((KIT_ROOT / "skills" / skill / "SKILL.md").read_text().split())
    assert "history permission" in text, skill
    assert "off by default" in text, skill
    assert '"history": true' in text, skill


REFUTING_SKILLS = ["critique-decisions", "critique-spec", "check-tickets", "review-ticket", "review-feature"]


@pytest.mark.parametrize("skill", REFUTING_SKILLS)
def test_the_stage_skill_runs_the_refute_round_right_after_its_verdicts(skill: str) -> None:
    """#224: refute comes after the ruling and before any fix, follow-up or owner question."""
    text = (KIT_ROOT / "skills" / skill / "SKILL.md").read_text()
    headings = [line for line in text.splitlines() if line.startswith("### ")]
    numbers = [int(re.match(r"### (\d+)\. ", h).group(1)) for h in headings]
    assert numbers == list(range(1, len(headings) + 1)), headings
    refute = next(i for i, h in enumerate(headings) if h.endswith(". Refute"))
    assert "Rule on every finding" in headings[refute - 1], headings
    assert "|refute|" in text, "the command line names refute"
    assert "run refute with `--input <run_dir>/verdicts.json`" in " ".join(text.split())


@pytest.mark.parametrize("skill", REFUTING_SKILLS)
def test_the_stage_skill_says_how_the_refute_round_changes_a_verdict(skill: str) -> None:
    text = " ".join((KIT_ROOT / "skills" / skill / "SKILL.md").read_text().split())
    assert '`rejected` with `reason_code: "refuted"`' in text, skill
    assert "- `downgrade`:" in text, skill
    assert "A skipped refute is reported, not retried" in text, skill
    assert "`Refute skipped: <finding_id> (<reason_code>)`" in text, skill


def test_the_pipeline_readme_names_every_stage_and_lens_it_explains() -> None:
    readme = (KIT_ROOT / "skills" / "critique" / "README.md").read_text()
    for stage in json.loads(ROUTING.read_text())["stages"]:
        assert f"| `{stage}` |" in readme, stage
    for lens in (KIT_ROOT / "skills" / "critique" / "lenses").glob("*.md"):
        assert f"- `{lens.stem}`:" in readme, lens.stem


def test_setup_installs_flow_beside_the_shared_skill_in_every_agent_home(macos_run: Run) -> None:
    """The skill finds the runner at ../critique/critique.mjs, so both land side by side."""
    source = KIT_ROOT / "skills" / "flow" / "SKILL.md"
    homes = (".claude", ".codex", ".vibe", ".vibe-private", ".gemini/config", ".gemini/antigravity-cli")
    for home in homes:
        skills = macos_run.home / home / "skills"
        assert (skills / "flow" / "SKILL.md").read_bytes() == source.read_bytes(), home
        assert (skills / "flow" / ".." / "critique" / "critique.mjs").is_file(), home
        assert (skills / "flow" / ".." / "build-tracker" / "build-tracker.mjs").is_file(), home


# ---------------------------------------------------------------------------
# /flow and the build tracker (#273)
# ---------------------------------------------------------------------------


def _flow_text() -> str:
    return (KIT_ROOT / "skills" / "flow" / "SKILL.md").read_text()


def _flow_part(heading: str) -> str:
    """From `heading` to the next heading of the same or a higher level, on one line."""
    text = _flow_text()
    start = text.index(heading)
    level = len(heading) - len(heading.lstrip("#"))
    rest = text[start + len(heading):]
    end = re.search(rf"\n#{{1,{level}}} ", rest)
    part = text[start:] if end is None else text[start : start + len(heading) + end.start()]
    return " ".join(part.split())


def test_flow_refreshes_the_build_tracker_before_it_reports() -> None:
    text = _flow_text()
    flow = text[text.index("## /flow\n"):text.index("## /flow finish")]
    headings = re.findall(r"^### (\d+)\. (.+)$", flow, re.M)
    assert headings == [
        ("1", "Find the feature"), ("2", "Read the state"), ("3", "Refresh the build tracker"),
        ("4", "Report"), ("5", "Act on the answer"),
    ], headings


def test_flow_refreshes_the_build_tracker_for_a_feature_in_implementation_or_finishing() -> None:
    step = _flow_part("### 3. Refresh the build tracker")
    assert "its `phase` is `implementation` or `finishing`" in step
    assert "`../build-tracker/build-tracker.mjs`" in step
    assert "build-tracker.mjs refresh --spec '<state.spec>' --landed '<id>' ..." in step
    assert "one `--landed` per ticket of `state.tickets` whose `landed` is true" in step
    assert "`refresh` runs `init` when the spec has no build tracker state yet, and `sync` when it has one" in step
    assert "it marks each landed ticket `merged`, renders the page" in step
    assert "the Artifact tool with `page` as its file path" in step
    assert "In planning, or for a draft, skip this step" in step
    assert re.search(r"Done when\b", step)


def test_flow_never_reads_the_build_trackers_state_to_rebuild_flow_state() -> None:
    step = _flow_part("### 3. Refresh the build tracker")
    assert "It never reads the build tracker's state file to rebuild flow state" in step
    assert "the flow state and the issue tracker stay its only sources" in step


def test_flow_reports_the_build_tracker_on_a_build_tracker_line() -> None:
    report = _flow_text().split("### 4. Report", 1)[1].split("### 5.", 1)[0]
    template = re.search(r"```\n(.*?)```", report, re.S)
    assert template, "the report is one fenced block"
    lines = template.group(1).splitlines()
    assert lines[lines.index(next(line for line in lines if line.startswith("Tickets:"))) + 1] == (
        "Build tracker: <page address or path>"
    ), lines
    assert len(lines) <= 8, "the report stays within about eight lines"
    assert "- **Build tracker**: during implementation and finishing only" in " ".join(report.split())
    step = _flow_part("### 3. Refresh the build tracker")
    assert "When `refresh` fails, the line gives its `code` and `message`" in step
    assert "when the republish fails, the page path and what failed" in step


def test_flow_never_calls_the_build_tracker_tracker_alone() -> None:
    """"Tracker" alone means the issue tracker (CONTEXT.md)."""
    assert re.findall(r"(?<!Build )(?<!build )\bTracker\b", _flow_text()) == []


def test_flow_finish_renders_the_build_tracker_once_the_gate_passes() -> None:
    finish = _flow_part("## /flow finish")
    assert "refresh the build tracker as step 3 of `/flow` says" in finish
    assert "Once the gate passes, that is the build's last render" in finish
    assert "the Build tracker line" in finish


def test_a_no_critique_build_gets_the_build_tracker_too() -> None:
    plain = _flow_part("## /flow no-critique")
    assert "refresh the build tracker and report as `/flow` does (its steps 3 and 4)" in plain
    assert "a feature built without critique gets its build tracker too" in plain


def test_flow_refreshes_again_whenever_it_reports_again() -> None:
    act = _flow_part("### 5. Act on the answer")
    assert act.count("refresh and report again (steps 3 and 4)") == 2
    assert "refresh and report as `/flow` does (steps 3 and 4)" in _flow_part("## /flow finish")


def test_a_pending_skill_is_skipped_with_a_notice_not_a_failure(macos_run: Run) -> None:
    """Until a pending skill's directory exists, setup says so and carries on."""
    for skill in (s for s in _registry()["local_skills"] if s.get("pending")):
        name = skill["name"]
        assert f"{name}: not built yet" in macos_run.stdout, name
        assert name not in macos_run.stderr, name
        assert not (macos_run.home / ".claude" / "skills" / name).exists(), name


def test_a_pending_skill_installs_once_its_directory_exists() -> None:
    """The copy is rendered in full; only a missing SKILL.md turns it into a notice."""
    data = _registry()
    skill = _skill(data, "critique-spec")
    skill["path"] = "skills/no-such-skill"
    skill["pending"] = "https://github.com/JorisJonkers-dev/agent-kit/issues/177"
    script = render_registry.render_setup_script(data)
    block = script[script.index('src="${KIT_ROOT}/skills/no-such-skill"'):]
    block = block[: block.index("\nfi\n")]
    assert (
        'log "critique-spec: not built yet '
        '(https://github.com/JorisJonkers-dev/agent-kit/issues/177); skipped"'
    ) in block
    assert "fail " not in block.split("else", 1)[0]
    assert 'dest="${CLAUDE_HOME}/skills/critique-spec"' in block


def test_a_pending_skill_that_now_exists_must_drop_pending() -> None:
    data = _registry()
    skill = _skill(data, "critique-spec")
    skill["path"] = "skills/critique"
    skill["pending"] = "https://github.com/JorisJonkers-dev/agent-kit/issues/177"
    with pytest.raises(render_registry.RegistryError, match="drop pending"):
        render_registry.validate(data)


def test_a_missing_skill_that_is_not_pending_is_refused() -> None:
    data = _registry()
    skill = _skill(data, "critique-spec")
    skill.pop("pending", None)
    skill["path"] = "skills/no-such-skill"
    with pytest.raises(render_registry.RegistryError, match=re.escape("has no SKILL.md")):
        render_registry.validate(data)


def test_pending_must_name_the_issue_that_builds_the_skill() -> None:
    data = _registry()
    skill = _skill(data, "critique-spec")
    skill["path"] = "skills/no-such-skill"
    skill["pending"] = "soon"
    with pytest.raises(render_registry.RegistryError, match="pending must be"):
        render_registry.validate(data)


# ---------------------------------------------------------------------------
# The runner bundle's pin
# ---------------------------------------------------------------------------


def _validator():
    spec = importlib.util.spec_from_file_location("validate_manifest", KIT_ROOT / "scripts" / "validate_manifest.py")
    assert spec and spec.loader
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def _manifest() -> dict:
    return yaml.safe_load((KIT_ROOT / "manifest.yaml").read_text())


def test_the_critique_runner_is_pinned_in_the_manifest_like_councils() -> None:
    files = {f["path"]: f for f in _manifest()["critique"]["files"]}
    runner = files["skills/critique/critique.mjs"]
    assert runner["mode"] == "0755"
    for path, entry in files.items():
        digest = hashlib.sha256((KIT_ROOT / path).read_bytes()).hexdigest()
        assert entry["sha256"] == digest, f"{path}: rebuild council/ts and update manifest.yaml critique.files"


def test_the_manifest_pins_every_file_the_skill_ships_but_its_skill_md() -> None:
    _validator().validate_critique(_manifest())


def test_a_runner_file_missing_from_the_pin_fails_validation() -> None:
    manifest = copy.deepcopy(_manifest())
    manifest["critique"]["files"] = [
        f for f in manifest["critique"]["files"] if f["path"] != "skills/critique/routing.json"
    ]
    with pytest.raises(AssertionError, match=re.escape("critique.files mismatch")):
        _validator().validate_critique(manifest)


# ---------------------------------------------------------------------------
# The critic agents in the registry
# ---------------------------------------------------------------------------


def test_one_critic_agent_is_declared_per_vibe_critic_client() -> None:
    """#167's Vibe clients: Mistral Large, Mistral Small, GLM and DeepSeek Flash."""
    names = {a["name"] for a in _critics(_registry())}
    assert names == {"critic-mistral-large", "critic-mistral-small", "critic-glm", "critic-deepseek-flash"}


def test_every_vibe_client_in_routing_names_a_declared_agent() -> None:
    routing = json.loads(ROUTING.read_text())
    declared = {a["name"] for a in _critics(_registry())}
    agents = {c["agent"] for c in routing["clients"].values() if c["kind"] == "vibe"}
    assert agents, "routing has no Vibe client, so this test proves nothing"
    assert agents <= declared


def test_every_vibe_client_in_routing_names_the_model_its_agent_pins() -> None:
    """#178: routing's `model` is informational, so it must say what the agent really calls."""
    routing = json.loads(ROUTING.read_text())
    clients = {name: c for name, c in routing["clients"].items() if c["kind"] == "vibe"}
    assert clients, "routing has no Vibe client, so this test proves nothing"
    for name, client in clients.items():
        (model,) = _agent_file(client["agent"])["models"]
        assert client["model"] == model["name"], name


def test_the_triage_model_runs_through_the_priced_mistral_small_agent() -> None:
    """#187: triage asks Mistral Small through its own agent, whose priced model entry makes its $0.03 cap real."""
    routing = json.loads(ROUTING.read_text())
    client = routing["clients"][routing["triage"]["model"]["client"]]
    assert (client["kind"], client["agent"], client["family"]) == ("vibe", "critic-mistral-small", "mistral")
    (model,) = _agent_file(client["agent"])["models"]
    assert model["name"] == client["model"] == "mistral-small-latest"
    assert model["input_price"] > 0 and model["output_price"] > 0
    assert routing["triage"]["model"]["max_price_usd"] == 0.03
    assert routing["triage"]["model"]["time_limit_s"] == 60


def test_every_critic_agent_is_a_generated_artifact() -> None:
    data = _registry()
    rendered = render_registry.artifacts(data)
    for agent in _critics(data):
        assert AGENTS_DIR / f"{agent['name']}.toml" in rendered, agent["name"]


@pytest.mark.parametrize("field", ["input_price", "output_price"])
def test_a_critic_agent_without_a_real_price_is_refused(field: str) -> None:
    """Vibe computes cost from these prices, so 0 would make every price cap a no-op."""
    data = _registry()
    data["vibe_critic_agents"][0][field] = 0
    with pytest.raises(render_registry.RegistryError, match=field):
        render_registry.validate(data)


def test_a_critic_agent_must_be_named_critic_something() -> None:
    data = _registry()
    data["vibe_critic_agents"][0]["name"] = "mistral-large"
    with pytest.raises(render_registry.RegistryError, match="critic-"):
        render_registry.validate(data)


def test_a_critic_agent_with_an_unknown_key_is_refused() -> None:
    """The isolation settings are the renderer's, not each entry's to change."""
    data = _registry()
    data["vibe_critic_agents"][0]["enable_mcp"] = True
    with pytest.raises(render_registry.RegistryError, match="enable_mcp"):
        render_registry.validate(data)


def test_a_critic_agent_is_only_for_the_workstation() -> None:
    data = _registry()
    data["vibe_critic_agents"][0]["surfaces"] = ["workstation", "hermes"]
    with pytest.raises(render_registry.RegistryError, match="surfaces"):
        render_registry.validate(data)


# ---------------------------------------------------------------------------
# The agent files
# ---------------------------------------------------------------------------


@pytest.mark.parametrize("name", CRITIC_AGENT_NAMES)
def test_each_agent_file_can_only_read_and_search_inside_its_working_directory(name: str) -> None:
    """``read_file`` and the runner's search server's tools (ADR-0006), nothing else.

    MCP is the runner's to switch per launch, so the file leaves ``enable_mcp``
    unset: off for a critic that reads alone, on with the runner's server as the
    one live server for a critic that searches. Headless Vibe refuses a tool
    call that asks for approval, so the search tools are approved outright.
    """
    doc = _agent_file(name)
    assert doc["enabled_tools"] == ["read_file", "packet_grep", "packet_list", "packet_symbol"]
    assert doc["tools"] == {tool: {"permission": "always"} for tool in ("packet_grep", "packet_list", "packet_symbol")}
    assert doc["outside_workdir"] == "deny"
    assert "enable_mcp" not in doc
    assert "mcp_servers" not in doc
    assert doc["enable_subagents"] is False
    assert doc["include_project_context"] is False
    assert doc["session_logging"] == {"enabled": False}
    assert doc["safety"] == "safe"


@pytest.mark.parametrize("name", CRITIC_AGENT_NAMES)
def test_each_agent_file_pins_its_own_priced_model(name: str) -> None:
    entry = next(a for a in _registry()["vibe_critic_agents"] if a["name"] == name)
    doc = _agent_file(name)
    (model,) = doc["models"]
    assert doc["active_model"] == model["alias"] == name
    assert model["name"] == entry["model"]
    assert model["provider"] == "mistral"
    assert model["input_price"] == entry["input_price"] > 0
    assert model["output_price"] == entry["output_price"] > 0


def test_the_prices_are_mistrals_published_ones() -> None:
    """docs.mistral.ai/inference/pricing, read 2026-11-01, in USD per million tokens.

    The model is the id the API takes, never the personal home's alias: the
    live check of #178 saw `mistral-large-4-0-3` (an alias) rejected with
    `Status 400 ... Invalid model`, and the alias resolve to `mistral-large-4-0`.
    """
    prices = {a["name"]: (a["model"], a["input_price"], a["output_price"]) for a in _critics(_registry())}
    assert prices == {
        "critic-mistral-large": ("mistral-large-4-0", 0.7, 2.1),
        "critic-mistral-small": ("mistral-small-latest", 0.15, 0.6),
        "critic-glm": ("zai-glm-5-3", 1.4, 4.4),
        "critic-deepseek-flash": ("deepseek-v4-1-flash-1017", 0.3, 1.2),
    }


@pytest.mark.parametrize("name", CRITIC_AGENT_NAMES)
def test_every_key_in_an_agent_file_is_a_vibe_setting(name: str) -> None:
    doc = _agent_file(name)
    assert set(doc) <= PROFILE_KEYS | SETTING_KEYS, set(doc) - PROFILE_KEYS - SETTING_KEYS
    for model in doc["models"]:
        assert set(model) <= MODEL_KEYS, set(model) - MODEL_KEYS
    assert set(doc["session_logging"]) == {"enabled"}


def test_no_agent_file_makes_itself_vibes_default_agent() -> None:
    for agent in _critics(_registry()):
        assert "default_agent" not in _agent_file(agent["name"])
    assert "default_agent" not in render_registry.render_setup_script(_registry())


def test_a_file_left_over_from_a_retired_agent_is_a_stale_artifact(tmp_path: Path) -> None:
    data = _registry()
    for relative, (content, _mode) in render_registry.artifacts(data).items():
        (tmp_path / relative).parent.mkdir(parents=True, exist_ok=True)
        (tmp_path / relative).write_text(content)
    assert render_registry.stale_artifacts(data, tmp_path) == []
    (tmp_path / AGENTS_DIR / "critic-gone.toml").write_text("active_model = 'x'\n")
    assert render_registry.stale_artifacts(data, tmp_path) == [str(AGENTS_DIR / "critic-gone.toml")]


# ---------------------------------------------------------------------------
# What a setup run installs
# ---------------------------------------------------------------------------


def test_setup_installs_every_critic_agent_into_the_shared_vibe_home(macos_run: Run) -> None:
    for agent in _critics(_registry()):
        name = agent["name"]
        installed = macos_run.home / ".vibe" / "agents" / f"{name}.toml"
        assert installed.read_text() == (KIT_ROOT / AGENTS_DIR / f"{name}.toml").read_text(), name
        assert f"Vibe critic agent {name} -> " in macos_run.stdout


# Mistral bills cached prompt tokens at 10% of the input price
# (docs.mistral.ai/studio/conversations/advanced/prompt-caching, read 2026-11-02).
CACHED_INPUT_PRICES = {
    "critic-mistral-large": 0.07,
    "critic-mistral-small": 0.015,
    "critic-glm": 0.14,
    "critic-deepseek-flash": 0.03,
}


@pytest.mark.parametrize("name", CRITIC_AGENT_NAMES)
def test_setup_installs_each_critic_model_with_its_cached_price(macos_run: Run, name: str) -> None:
    """#261: without a cached price Vibe bills every cached token at the full rate, so its estimate and its
    --max-price stop ran about three times what Mistral charged."""
    installed = tomllib.loads((macos_run.home / ".vibe" / "agents" / f"{name}.toml").read_text())
    (model,) = installed["models"]
    assert model["cached_input_price"] == CACHED_INPUT_PRICES[name]
    assert model["cached_input_price"] == pytest.approx(model["input_price"] / 10)


def test_every_vibe_client_in_routing_has_its_agents_prices() -> None:
    """The runner prices a launch from routing, Vibe from the agent file; the two must agree."""
    routing = json.loads(ROUTING.read_text())
    clients = {name: c for name, c in routing["clients"].items() if c["kind"] == "vibe"}
    assert clients, "routing has no Vibe client, so this test proves nothing"
    for name, client in clients.items():
        (model,) = _agent_file(client["agent"])["models"]
        assert client["prices"] == {
            "input": model["input_price"],
            "cached_input": model["cached_input_price"],
            "output": model["output_price"],
        }, name


def test_setup_never_puts_a_critic_agent_into_vibe_privates_home(macos_run: Run) -> None:
    private = macos_run.home / ".vibe-private"
    assert private.is_dir(), "the run never built vibe-private's home, so this test proves nothing"
    assert not list(private.rglob("critic-*"))


def test_setup_adds_nothing_for_critics_to_antigravity(macos_run: Run) -> None:
    gemini = macos_run.home / ".gemini"
    assert (gemini / "config" / "mcp_config.json").is_file(), "the run never built Antigravity's surface"
    assert not list(gemini.rglob("critic-*"))
    for path in gemini.rglob("*"):
        if path.is_file() and "skills" not in path.relative_to(gemini).parts:
            assert "critic-" not in path.read_text(errors="ignore"), path


def test_a_check_run_installs_no_critic_agent(check_run: Run) -> None:
    assert not (check_run.home / ".vibe" / "agents").exists()
    assert "would write" in check_run.stdout and "critic-glm.toml" in check_run.stdout


def _critic_step(data: dict) -> str:
    script = render_registry.render_setup_script(data)
    start = script.index("_vibe_critic_agents() {")
    return script[start : script.index("\n}\n", start) + 3]


def _run_critic_step(tmp_path: Path, data: dict, *, check: bool = False) -> subprocess.CompletedProcess[str]:
    kit = tmp_path / "kit"
    for relative, (content, _mode) in render_registry.artifacts(data).items():
        if relative.parent == AGENTS_DIR:
            (kit / relative).parent.mkdir(parents=True, exist_ok=True)
            (kit / relative).write_text(content)
    stubs = 'ok() { echo "ok $*"; }\nlog() { echo "log $*"; }\nfail() { echo "fail $*"; }\n'
    body = (
        f'CHECK_ONLY={1 if check else 0}\nKIT_ROOT="{kit}"\n{stubs}{_critic_step(data)}\n'
        f'_vibe_critic_agents "{tmp_path}/vibe"\n'
    )
    return subprocess.run(["bash", "-c", body], capture_output=True, text=True, check=False)


def test_a_retired_critic_agent_is_removed_from_the_shared_home(tmp_path: Path) -> None:
    data = _registry()
    data["vibe_critic_agents"][0]["surfaces"] = []
    retired = data["vibe_critic_agents"][0]["name"]
    agents = tmp_path / "vibe" / "agents"
    agents.mkdir(parents=True)
    (agents / f"{retired}.toml").write_text("stale\n")
    result = _run_critic_step(tmp_path, data)
    assert result.returncode == 0, result.stderr
    assert not (agents / f"{retired}.toml").exists()
    assert f"removed retired Vibe critic agent {retired}" in result.stdout


def test_the_owners_own_vibe_agents_are_left_alone(tmp_path: Path) -> None:
    agents = tmp_path / "vibe" / "agents"
    agents.mkdir(parents=True)
    (agents / "mine.toml").write_text('display_name = "Mine"\n')
    result = _run_critic_step(tmp_path, _registry())
    assert result.returncode == 0, result.stderr
    assert (agents / "mine.toml").read_text() == 'display_name = "Mine"\n'


def test_a_rerun_replaces_an_edited_critic_agent(tmp_path: Path) -> None:
    """The agent file is declared: a hand edit that, say, adds bash is undone."""
    agents = tmp_path / "vibe" / "agents"
    agents.mkdir(parents=True)
    (agents / "critic-glm.toml").write_text('enabled_tools = ["read_file", "bash"]\n')
    assert _run_critic_step(tmp_path, _registry()).returncode == 0
    assert (agents / "critic-glm.toml").read_text() == (KIT_ROOT / AGENTS_DIR / "critic-glm.toml").read_text()


def test_setup_installs_the_critic_agents_only_into_the_shared_home() -> None:
    script = render_registry.render_setup_script(_registry())
    calls = [line.strip() for line in script.splitlines() if line.strip().startswith("_vibe_critic_agents ")]
    assert calls == ['_vibe_critic_agents "${VIBE_HOME}"']


# ---------------------------------------------------------------------------
# Model sync leaves the agents' own model entries alone
# ---------------------------------------------------------------------------


def _sync_module():
    spec = importlib.util.spec_from_file_location("vibe_sync_models", KIT_ROOT / "scripts" / "vibe-sync-models.py")
    assert spec and spec.loader
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def test_model_sync_never_rewrites_a_critic_agents_model_entry(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    """Sync writes config.toml alone, under aliases Mistral's API names, never critic-*."""
    monkeypatch.setenv("HOME", str(tmp_path))
    home = tmp_path / ".vibe"
    agents = home / "agents"
    agents.mkdir(parents=True)
    installed = agents / "critic-mistral-large.toml"
    installed.write_text((KIT_ROOT / AGENTS_DIR / "critic-mistral-large.toml").read_text())
    before = installed.read_bytes()
    sync = _sync_module()
    entries = sync.wanted([{
        "id": "mistral-large-4-0-3", "name": "mistral-large-4-0-3", "aliases": [], "deprecation": None,
        "capabilities": {"completion_chat": True, "function_calling": True},
    }])
    assert sync.sync(home / "config.toml", entries) == 0
    assert installed.read_bytes() == before
    aliases = {m["alias"] for m in tomllib.loads((home / "config.toml").read_text())["models"]}
    assert not {a for a in aliases if a.startswith("critic-")}
