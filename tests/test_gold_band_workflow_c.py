"""Workflow C (grill to tickets) as a Gold Band workflow template (issue #459).

The template at ``installer/gold-band/workflows/workflow-c.json`` is Workflow C
for the personal instance: a run-start gate and a setup gate, up to three
unrolled grill rounds that each pause on a manual check when a question is
open, the spec review and the ticket review (the critique stage templates'
nodes, each pausing on a needs-human manual check), the publication manual
check, and the publish gate, which is reachable only through that check.

These tests read the template as a file and need no Gold Band binary or Rust
toolchain. Its headless proof belongs to the ``gold-band-proof`` crate
(docs/gold-band-proof.md); the workflow lint runs over it in that crate's tests.
"""

from __future__ import annotations

import json
from pathlib import Path
from typing import Any

import pytest

KIT_ROOT = Path(__file__).resolve().parent.parent
WORKFLOWS = KIT_ROOT / "installer" / "gold-band" / "workflows"
TEMPLATE = WORKFLOWS / "workflow-c.json"

#: Every node that pauses the run for the owner.
MANUAL_CHECKS = [
    "grill-check-1",
    "grill-check-2",
    "grill-check-3",
    "spec-needs-human",
    "tickets-needs-human",
    "publication-check",
]

#: The families the personal instance may not bind.
FORBIDDEN_FAMILIES = ["codex", "opus"]

#: The grill rounds the file unrolls, which is also its cap.
ROUNDS = [1, 2, 3]


@pytest.fixture(scope="module")
def text() -> str:
    return TEMPLATE.read_text()


@pytest.fixture(scope="module")
def template(text: str) -> dict[str, Any]:
    return json.loads(text)


@pytest.fixture(scope="module")
def workflow(template: dict[str, Any]) -> dict[str, Any]:
    return template["workflow"]


@pytest.fixture(scope="module")
def nodes(workflow: dict[str, Any]) -> dict[str, dict[str, Any]]:
    return {node["id"]: node for node in workflow["nodes"]}


@pytest.fixture(scope="module")
def edges(workflow: dict[str, Any]) -> list[tuple[str, str, str]]:
    return [(edge["from"], edge["to"], edge["on"]) for edge in workflow["edges"]]


def _stage_nodes(stage: str) -> dict[str, dict[str, Any]]:
    """A critique stage template's nodes other than its run-start gate."""
    template = json.loads((WORKFLOWS / f"critique-{stage}.json").read_text())
    return {n["id"]: n for n in template["workflow"]["nodes"] if not n["id"].endswith("-start")}


def _reachable(edges: list[tuple[str, str, str]], entry: str, without: set[str]) -> set[str]:
    seen: set[str] = set()
    todo = [entry]
    while todo:
        node = todo.pop()
        if node in seen or node in without:
            continue
        seen.add(node)
        todo.extend(to for frm, to, _ in edges if frm == node)
    return seen


def test_the_workflow_is_workflow_c_entered_at_the_run_start_gate(workflow: dict[str, Any]) -> None:
    assert workflow["id"] == "workflow-c"
    assert workflow["entry"] == "gate-workflow-c-start"


def test_the_run_starts_with_the_rule_check_then_the_setup_check_then_the_first_round(
    edges: list[tuple[str, str, str]],
) -> None:
    """No model node runs before the repository rule and the setup file are checked."""
    assert edges[:3] == [
        ("gate-workflow-c-start", "gate-workflow-c-setup", "success"),
        ("gate-workflow-c-setup", "copy-workflow-c-setup", "success"),
        ("copy-workflow-c-setup", "grill-round-1", "success"),
    ]


def test_the_setup_gate_runs_the_publisher_check(nodes: dict[str, dict[str, Any]]) -> None:
    goal = nodes["gate-workflow-c-setup"]["goal"]
    assert "gold-band-gate" in goal
    assert "`ticket-publish check`" in goal
    assert "docs/agents/setup.yaml" in goal


def test_the_nodes_that_pause_for_the_owner_are_exactly_the_manual_checks(
    nodes: dict[str, dict[str, Any]],
) -> None:
    paused = [node_id for node_id, node in nodes.items() if node.get("manual_check")]
    assert sorted(paused) == sorted(MANUAL_CHECKS)


@pytest.mark.parametrize("round_", ROUNDS)
def test_a_round_with_an_open_question_pauses_on_its_manual_check_then_the_next_round(
    nodes: dict[str, dict[str, Any]],
    edges: list[tuple[str, str, str]],
    round_: int,
) -> None:
    """Each round's failure edge goes to that round's check, which hands over to the next round."""
    grill = nodes[f"grill-round-{round_}"]
    assert grill["success_condition"] == {"path": "$.open_questions", "equals": []}
    following = f"grill-round-{round_ + 1}" if round_ < len(ROUNDS) else "spec-draft"
    assert (f"grill-round-{round_}", f"grill-check-{round_}", "failure") in edges
    assert (f"grill-round-{round_}", "spec-draft", "success") in edges
    assert (f"grill-check-{round_}", following, "success") in edges


def test_the_grill_rounds_are_capped_at_three(nodes: dict[str, dict[str, Any]]) -> None:
    assert [i for i in range(1, 10) if f"grill-round-{i}" in nodes] == ROUNDS


@pytest.mark.parametrize("stage", ["spec", "tickets"])
def test_each_review_stage_is_the_critique_template_s_nodes(
    nodes: dict[str, dict[str, Any]],
    edges: list[tuple[str, str, str]],
    stage: str,
) -> None:
    """The review nodes are the critique stage template's own, unchanged, and keep its needs-human fork."""
    for node_id, node in _stage_nodes(stage).items():
        assert nodes[node_id] == node
    verdicts, needs_human = f"{stage}-verdicts", f"{stage}-needs-human"
    refute = f"gate-critique-{stage}-refute"
    assert (verdicts, refute, "success") in edges
    assert (verdicts, needs_human, "failure") in edges
    assert (needs_human, refute, "success") in edges


def test_the_spec_review_feeds_the_ticket_draft_and_the_ticket_review_the_publication_check(
    edges: list[tuple[str, str, str]],
) -> None:
    assert ("spec-draft", "gate-critique-spec-prepare", "success") in edges
    assert ("copy-critique-spec-record", "tickets-draft", "success") in edges
    assert ("tickets-draft", "gate-critique-tickets-prepare", "success") in edges
    assert ("copy-critique-tickets-record", "publication-check", "success") in edges


def test_the_publish_gate_is_reachable_only_through_the_publication_check(
    edges: list[tuple[str, str, str]],
) -> None:
    """Without the publication check, neither half of the publish shim is reachable."""
    incoming = [frm for frm, to, _ in edges if to == "gate-workflow-c-publish"]
    assert incoming == ["publication-check"]
    reached = _reachable(edges, "gate-workflow-c-start", without={"publication-check"})
    assert "gate-workflow-c-publish" not in reached
    assert "copy-workflow-c-publish" not in reached
    assert "gate-workflow-c-publish" in _reachable(edges, "gate-workflow-c-start", without=set())


def test_the_publish_gate_names_the_tracker_the_setup_file_gives(nodes: dict[str, dict[str, Any]]) -> None:
    goal = nodes["gate-workflow-c-publish"]["goal"]
    assert "`ticket-publish publish`" in goal
    assert "setup file" in goal
    assert "tickets.json" in goal


def test_the_last_copier_ends_the_run_and_only_it(edges: list[tuple[str, str, str]]) -> None:
    assert [frm for frm, to, _ in edges if to == "$end"] == ["copy-workflow-c-publish"]


def test_no_shim_node_has_a_failure_edge(edges: list[tuple[str, str, str]]) -> None:
    shim = ("gate-", "copy-")
    assert [e for e in edges if e[2] == "failure" and e[0].startswith(shim)] == []


def test_every_edge_target_exists_or_ends_the_run(
    nodes: dict[str, dict[str, Any]],
    edges: list[tuple[str, str, str]],
) -> None:
    for frm, to, _ in edges:
        assert frm in nodes
        assert to == "$end" or to in nodes


def test_every_node_is_reachable_from_the_entry(
    nodes: dict[str, dict[str, Any]],
    edges: list[tuple[str, str, str]],
) -> None:
    assert _reachable(edges, "gate-workflow-c-start", set()) - {"$end"} == set(nodes)


def test_every_node_has_a_model_binding(template: dict[str, Any], nodes: dict[str, dict[str, Any]]) -> None:
    bindings = template["modelBindings"]["bindings"]
    slots = [binding["executionSlotId"] for binding in bindings]
    assert len(set(slots)) == len(slots) == len(nodes)
    by_slot = {binding["executionSlotId"]: binding for binding in bindings}
    for node in nodes.values():
        binding = by_slot[node["executionSlotId"]]
        assert (binding["agentId"], binding["modelId"]) == (node["provider"], node["model"])


def test_the_spec_author_and_its_orchestrator_are_in_different_families(
    nodes: dict[str, dict[str, Any]],
) -> None:
    """The model that drafts the spec and the tickets is not the one that rules on their review."""
    for draft, verdicts in (("spec-draft", "spec-verdicts"), ("tickets-draft", "tickets-verdicts")):
        assert nodes[draft]["provider"] != nodes[verdicts]["provider"]
        assert nodes[draft]["model"] != nodes[verdicts]["model"]


def test_no_node_writes_memory_and_no_forbidden_family_is_bound(text: str) -> None:
    """The personal instance binds neither Codex nor Opus, and no node holds a memory-write binding."""
    lowered = text.lower()
    for family in FORBIDDEN_FAMILIES:
        assert family not in lowered
    assert "memory-write" not in lowered


def test_every_output_artifact_has_no_json_suffix(nodes: dict[str, dict[str, Any]]) -> None:
    for node in nodes.values():
        if isinstance(node.get("output"), dict):
            assert not node["output"]["artifact"].endswith(".json")


def test_the_manual_check_nodes_write_nothing_and_have_no_output(nodes: dict[str, dict[str, Any]]) -> None:
    for node_id in MANUAL_CHECKS:
        assert "output" not in nodes[node_id]
        assert "Do not" in nodes[node_id]["goal"]
