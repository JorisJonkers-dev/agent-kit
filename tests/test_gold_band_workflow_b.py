"""Workflow B's main path as a Gold Band workflow template (issue #448).

The template at ``installer/gold-band/workflows/workflow-b.json`` is the
Gold Band shape of Workflow B's main path for the personal instance: a test
author from one model family writes the failing tests, one writer from another
family implements until they pass, and build, tests and lint each run through
the gate shim -- a gate node that runs the pinned ``gold-band-gate`` wrapper,
then a read-only copier that verifies the report it printed.

No gate node has a failure edge, so a red, hung or forged gate ends the run in
failure rather than routing somewhere else. The repair rounds, critics and CI
wait are added by later tickets.

These tests read the template as a file and need no Gold Band binary or Rust
toolchain (the workflow lint runs over it in the gold-band-proof crate's
tests). Its headless proof, the run against stub ACP agents, belongs to the
``gold-band-proof`` crate (docs/gold-band-proof.md).
"""

from __future__ import annotations

import json
from pathlib import Path
from typing import Any

import pytest

KIT_ROOT = Path(__file__).resolve().parent.parent
TEMPLATE = KIT_ROOT / "installer" / "gold-band" / "workflows" / "workflow-b.json"

#: The main path, in order: the test author, the one writer, then each gate
#: shim as its gate node followed by the copier that verifies its report.
MAIN_PATH = [
    "test-author",
    "writer",
    "gate-build",
    "copy-build",
    "gate-test",
    "copy-test",
    "gate-lint",
    "copy-lint",
]

#: The gate shim's two halves, by suffix: the wrapper that runs the command and
#: the read-only copier that verifies the report it printed.
GATE_SUFFIXES = ["build", "test", "lint"]

#: The families the personal instance may not bind: the ticket's main path is
#: proven with the two families below and nothing else.
FORBIDDEN_FAMILIES = ["codex", "opus"]

# ---------------------------------------------------------------------------
# The template as a file: what no Gold Band binary is needed to see
# ---------------------------------------------------------------------------


@pytest.fixture(scope="module")
def template() -> dict[str, Any]:
    return json.loads(TEMPLATE.read_text())


@pytest.fixture(scope="module")
def workflow(template: dict[str, Any]) -> dict[str, Any]:
    """The definition inside the authoring wrapper."""
    return template["workflow"]


@pytest.fixture(scope="module")
def nodes(workflow: dict[str, Any]) -> list[dict[str, Any]]:
    return workflow["nodes"]


@pytest.fixture(scope="module")
def edges(workflow: dict[str, Any]) -> list[dict[str, Any]]:
    return workflow["edges"]


@pytest.fixture(scope="module")
def bindings(template: dict[str, Any]) -> list[dict[str, Any]]:
    return template["modelBindings"]["bindings"]


def _node(nodes: list[dict[str, Any]], node_id: str) -> dict[str, Any]:
    return next(node for node in nodes if node["id"] == node_id)


def test_the_workflow_is_workflow_b(workflow: dict[str, Any]) -> None:
    """The template is Workflow B's, not another workflow's."""
    assert workflow["id"] == "workflow-b"


def test_the_nodes_are_the_main_path_in_order(
    workflow: dict[str, Any],
    nodes: list[dict[str, Any]],
) -> None:
    """The nodes are the main path, in order, entered at the test author."""
    assert [node["id"] for node in nodes] == MAIN_PATH
    assert workflow["entry"] == "test-author"


def test_the_edges_chain_the_main_path_on_success(edges: list[dict[str, Any]]) -> None:
    """Each node hands over to the next on success, and the last one ends the run."""
    assert [(edge["from"], edge["to"]) for edge in edges] == [
        (MAIN_PATH[index], MAIN_PATH[index + 1]) for index in range(len(MAIN_PATH) - 1)
    ] + [(MAIN_PATH[-1], "$end")]
    assert {edge["on"] for edge in edges} == {"success"}


def test_no_gate_has_a_failure_edge(edges: list[dict[str, Any]]) -> None:
    """A failed gate ends the run: nothing routes a failure anywhere."""
    assert [edge for edge in edges if edge.get("on") == "failure"] == []


def test_the_test_author_and_the_writer_are_different_families(
    nodes: list[dict[str, Any]],
) -> None:
    """The tests come from one family and the implementation from another."""
    author = _node(nodes, "test-author")
    writer = _node(nodes, "writer")
    assert author["provider"] == "vibe"
    assert writer["provider"] == "vibe-study"
    assert author["provider"] != writer["provider"]
    assert author["model"] != writer["model"]


def test_no_node_binds_codex_or_opus(template: dict[str, Any]) -> None:
    """Neither forbidden family appears anywhere in the template."""
    text = TEMPLATE.read_text().lower()
    for family in FORBIDDEN_FAMILIES:
        assert family not in text


def test_every_gate_and_copier_holds_its_exit_status_to_zero(
    nodes: list[dict[str, Any]],
) -> None:
    """A gate or copier node succeeds only on exit status zero."""
    shim = [node for node in nodes if node["id"].startswith(("gate-", "copy-"))]
    assert len(shim) == 2 * len(GATE_SUFFIXES)
    for node in shim:
        assert node["success_condition"] == {"path": "$.exit_code", "equals": 0}


def test_each_gate_is_followed_by_its_own_copier(nodes: list[dict[str, Any]]) -> None:
    """Every gate-X node pairs with the copy-X node that directly follows it."""
    ids = [node["id"] for node in nodes]
    for suffix in GATE_SUFFIXES:
        gate = f"gate-{suffix}"
        copier = f"copy-{suffix}"
        assert gate in ids
        assert copier in ids
        assert ids[ids.index(gate) + 1] == copier


def test_each_gate_and_copier_pins_its_own_agent(nodes: list[dict[str, Any]]) -> None:
    """The gate and copier nodes run as the agent named after them."""
    for suffix in GATE_SUFFIXES:
        for node_id in (f"gate-{suffix}", f"copy-{suffix}"):
            assert _node(nodes, node_id)["provider"] == node_id


def test_every_node_has_a_model_binding(
    nodes: list[dict[str, Any]],
    bindings: list[dict[str, Any]],
) -> None:
    """Each node's execution slot is bound to the agent and model it names."""
    by_slot = {binding["executionSlotId"]: binding for binding in bindings}
    assert len(by_slot) == len(bindings) == len(nodes)
    for node in nodes:
        binding = by_slot[node["executionSlotId"]]
        assert binding["agentId"] == node["provider"]
        assert binding["modelId"] == node["model"]


def test_every_output_is_a_json_artifact_without_the_extension(
    nodes: list[dict[str, Any]],
) -> None:
    """Each node answers with a json artifact, and Gold Band adds the extension."""
    for node in nodes:
        output = node["output"]
        assert output["kind"] == "json"
        # Gold Band appends ".json" to a json artifact's name itself.
        assert not output["artifact"].endswith(".json")
