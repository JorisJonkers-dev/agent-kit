"""The Wave 3 templates over the runners (issue #627): critique, critique-stats, flow, build-tracker.

Each template at ``installer/gold-band/workflows/<name>.json`` runs the same
commands the direct lane runs, through the gate shim, and holds no logic of its
own: a run-start gate, then one gate node (the pinned ``gold-band-gate``
wrapper around a command) followed by the read-only copier that verifies its
report. The ``critique`` template is the runner's four entry points with the
orchestrator's verdict node between fan-out and refute.

These tests read the templates as files. Their headless proof belongs to the
``gold-band-proof`` crate (``scripts/crates/gold-band-proof/tests/headless.rs``).
"""

from __future__ import annotations

import json
from pathlib import Path
from typing import Any

import pytest

KIT_ROOT = Path(__file__).resolve().parent.parent
WORKFLOWS = KIT_ROOT / "installer" / "gold-band" / "workflows"
SKILLS = KIT_ROOT / "skills" / "workstation"

#: Each single-command template, with the command its gate wraps and the skill that runs it directly.
SINGLE = {
    "critique-stats": ("`critique stats --strict`", "critique-stats", "critique stats"),
    "flow": ("`critique state --strict`", "flow", "critique state"),
    "build-tracker": ("`build-tracker refresh`", "flow", "build-tracker.mjs refresh"),  # refreshed by /flow, step 3,
}
FORBIDDEN_FAMILIES = ["codex", "opus"]


def _load(name: str) -> dict[str, Any]:
    return json.loads((WORKFLOWS / f"{name}.json").read_text(encoding="utf-8"))


def _gate_chain(name: str) -> list[str]:
    return [f"gate-{name}-start", f"gate-{name}", f"copy-{name}"]


@pytest.mark.parametrize("name", list(SINGLE))
def test_a_single_command_template_is_the_run_start_gate_one_gate_and_its_copier(name: str) -> None:
    workflow = _load(name)["workflow"]
    ids = _gate_chain(name)
    assert workflow["id"] == name
    assert workflow["entry"] == ids[0]
    assert [node["id"] for node in workflow["nodes"]] == ids
    assert [(e["from"], e["to"], e["on"]) for e in workflow["edges"]] == [
        (ids[0], ids[1], "success"),
        (ids[1], ids[2], "success"),
        (ids[2], "$end", "success"),
    ]


@pytest.mark.parametrize("name", list(SINGLE))
def test_the_gate_wraps_the_command_the_direct_lane_runs(name: str) -> None:
    """The gate names the command and the direct lane's skill runs the same runner command."""
    command, skill, direct = SINGLE[name]
    gate = next(node for node in _load(name)["workflow"]["nodes"] if node["id"] == f"gate-{name}")
    assert command in gate["goal"]
    assert "gold-band-gate" in gate["goal"]
    # The direct lane's own invocation, runner and subcommand together: `sh <dir>/critique stats`.
    assert f"/{direct} " in (SKILLS / skill / "SKILL.md").read_text(encoding="utf-8")


@pytest.mark.parametrize("name", list(SINGLE))
def test_the_start_gate_names_the_run_start_wrapper(name: str) -> None:
    start = _load(name)["workflow"]["nodes"][0]
    assert "gold-band-run-start" in start["goal"]


def test_the_critique_template_runs_the_runner_commands_in_order() -> None:
    workflow = _load("critique")["workflow"]
    ids = [node["id"] for node in workflow["nodes"]]
    assert workflow["id"] == "critique"
    assert workflow["entry"] == "gate-critique-start"
    assert ids == [
        "gate-critique-start",
        "gate-critique-prepare",
        "copy-critique-prepare",
        "gate-critique-fan-out",
        "copy-critique-fan-out",
        "critique-verdicts",
        "critique-needs-human",
        "gate-critique-refute",
        "copy-critique-refute",
        "gate-critique-record",
        "copy-critique-record",
    ]
    by_id = {node["id"]: node for node in workflow["nodes"]}
    for command in ["prepare", "fan-out", "refute", "record"]:
        assert f"`critique {command} --strict`" in by_id[f"gate-critique-{command}"]["goal"]


def test_the_critique_template_forks_only_at_the_verdicts() -> None:
    edges = [(e["from"], e["to"], e["on"]) for e in _load("critique")["workflow"]["edges"]]
    assert ("critique-verdicts", "gate-critique-refute", "success") in edges
    assert ("critique-verdicts", "critique-needs-human", "failure") in edges
    assert ("critique-needs-human", "gate-critique-refute", "success") in edges
    assert edges[-1] == ("copy-critique-record", "$end", "success")
    assert [e for e in edges if e[2] == "failure"] == [("critique-verdicts", "critique-needs-human", "failure")]


@pytest.mark.parametrize("name", ["critique", *SINGLE])
def test_every_node_is_bound_to_the_agent_it_names_and_slots_are_unique(name: str) -> None:
    template = _load(name)
    nodes = template["workflow"]["nodes"]
    bindings = template["modelBindings"]["bindings"]
    slots = [b["executionSlotId"] for b in bindings]
    assert len(set(slots)) == len(slots) == len(nodes)
    by_slot = {b["executionSlotId"]: b for b in bindings}
    for node in nodes:
        binding = by_slot[node["executionSlotId"]]
        assert (binding["agentId"], binding["modelId"]) == (node["provider"], node["model"])


@pytest.mark.parametrize("name", ["critique", *SINGLE])
def test_no_forbidden_family_and_no_json_suffix_and_no_failure_edge_from_a_shim(name: str) -> None:
    text = (WORKFLOWS / f"{name}.json").read_text(encoding="utf-8")
    for family in FORBIDDEN_FAMILIES:
        assert family not in text.lower()
    workflow = json.loads(text)["workflow"]
    for node in workflow["nodes"]:
        if isinstance(node.get("output"), dict):
            assert not node["output"]["artifact"].endswith(".json")
    shims = {n["id"] for n in workflow["nodes"] if n["id"].startswith(("gate-", "copy-"))}
    assert [e for e in workflow["edges"] if e["on"] == "failure" and e["from"] in shims] == []


@pytest.mark.parametrize("name", ["critique", *SINGLE])
def test_the_description_keeps_the_runner_the_executor_and_holds_no_logic(name: str) -> None:
    description = _load(name)["workflow"]["description"]
    assert "no logic of its own" in description
    assert "issue #627" in description
