"""Workflow B as a Gold Band workflow template (issues #448, #451).

The template at ``installer/gold-band/workflows/workflow-b.json`` is the Gold
Band shape of Workflow B for the personal instance: a test author from one
model family writes the failing tests, one writer from another family
implements until they pass, and build, tests, lint and coverage each run
through the gate shim -- a gate node that runs the pinned ``gold-band-gate``
wrapper, then a read-only copier that verifies the report it printed. Two
critics from other families then review the change in parallel in one
ai-dynamic node and the finding check keeps the findings with a checkable
quote. A round with an actionable finding sends the ticket back to the writer,
twice at most, through fixed nodes (every repair is followed by the gates and
a new critics round, so the file holds no loop), and after the second failed
repair an Opus escalation node and a manual check end the run for the owner.

No gate node has a failure edge, so a failing gate ends the run in failure
rather than routing somewhere else. The CI wait is added by a later ticket.

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
    "gate-coverage",
    "copy-coverage",
]

#: The gate shim's two halves, by suffix: the wrapper that runs the command and
#: the read-only copier that verifies the report it printed.
GATE_SUFFIXES = ["build", "test", "lint", "coverage"]

#: The family the personal instance may not bind at all.
FORBIDDEN_FAMILY = "codex"

#: The critic rounds the file unrolls, which is also the cap: the first review
#: and one more after each of the two repairs.
ROUNDS = [1, 2, 3]

#: The repairs the file unrolls: the writer is sent back at most twice.
REPAIRS = [1, 2]

#: The Opus judgment node and the manual check that pauses the run after it.
ESCALATION = "workflow-b-escalation"
ESCALATION_CHECK = "escalation-check"

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


def _gate_names(round_: int | None = None) -> list[str]:
    """The gate names of the main path (``round_`` None) or of one repair round."""
    if round_ is None:
        return list(GATE_SUFFIXES)
    return [f"{suffix}-r{round_}" for suffix in GATE_SUFFIXES]


def _edges_from(edges: list[dict[str, Any]], node_id: str) -> dict[str, str]:
    """The targets of a node's edges, by the outcome they follow."""
    return {edge["on"]: edge["to"] for edge in edges if edge["from"] == node_id}


def _is_shim(node_id: str) -> bool:
    return node_id.startswith(("gate-", "copy-"))


def test_the_workflow_is_workflow_b(workflow: dict[str, Any]) -> None:
    """The template is Workflow B's, not another workflow's."""
    assert workflow["id"] == "workflow-b"


def test_the_main_path_comes_first_and_is_entered_at_the_test_author(
    workflow: dict[str, Any],
    nodes: list[dict[str, Any]],
) -> None:
    """The test author, the writer and the four gates lead the node list."""
    assert [node["id"] for node in nodes][: len(MAIN_PATH)] == MAIN_PATH
    assert workflow["entry"] == "test-author"


def test_the_edges_chain_the_main_path_on_success(edges: list[dict[str, Any]]) -> None:
    """Each main path node hands over to the next on success."""
    for index in range(len(MAIN_PATH) - 1):
        assert _edges_from(edges, MAIN_PATH[index]) == {"success": MAIN_PATH[index + 1]}
    assert _edges_from(edges, MAIN_PATH[-1]) == {"success": "critics-1"}


def test_no_gate_has_a_failure_edge(edges: list[dict[str, Any]]) -> None:
    """A failed gate or copier ends the run: nothing routes a failure anywhere."""
    assert [edge for edge in edges if _is_shim(edge["from"]) and edge["on"] != "success"] == []


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


def test_no_node_binds_codex(template: dict[str, Any]) -> None:
    """The personal instance never binds Codex."""
    assert FORBIDDEN_FAMILY not in TEMPLATE.read_text().lower()


def test_opus_is_bound_only_by_the_escalation_node(nodes: list[dict[str, Any]]) -> None:
    """The one place Opus is spent is the judgment node, named for it."""
    opus = [node["id"] for node in nodes if "opus" in json.dumps(node).lower()]
    assert opus == [ESCALATION]
    assert ESCALATION.endswith("-escalation")


def test_every_gate_and_copier_holds_its_exit_status_to_zero(
    nodes: list[dict[str, Any]],
) -> None:
    """A gate or copier node succeeds only on exit status zero."""
    shim = [node for node in nodes if _is_shim(node["id"])]
    assert shim
    for node in shim:
        assert node["success_condition"] == {"path": "$.exit_code", "equals": 0}


def test_each_gate_is_followed_by_its_own_copier(nodes: list[dict[str, Any]]) -> None:
    """Every gate-X node pairs with the copy-X node that directly follows it."""
    ids = [node["id"] for node in nodes]
    names = _gate_names() + [f"findings-{n}" for n in ROUNDS]
    for repair in REPAIRS:
        names += _gate_names(repair)
    for name in names:
        assert ids[ids.index(f"gate-{name}") + 1] == f"copy-{name}"
    assert len([i for i in ids if i.startswith("gate-")]) == len(names)


def test_each_gate_and_copier_pins_its_own_agent(nodes: list[dict[str, Any]]) -> None:
    """The gate and copier nodes run as the agent named after them."""
    for node in nodes:
        if _is_shim(node["id"]):
            assert node["provider"] == node["id"]


def test_every_worker_has_a_model_binding(
    nodes: list[dict[str, Any]],
    bindings: list[dict[str, Any]],
) -> None:
    """Each worker's execution slot is bound to the agent and model it names."""
    by_slot = {binding["executionSlotId"]: binding for binding in bindings}
    workers = [node for node in nodes if node["type"] == "worker"]
    assert len(by_slot) == len(bindings) == len(workers)
    for node in workers:
        binding = by_slot[node["executionSlotId"]]
        assert binding["agentId"] == node["provider"]
        assert binding["modelId"] == node["model"]


def test_every_worker_output_is_a_json_artifact_without_the_extension(
    nodes: list[dict[str, Any]],
) -> None:
    """Each answering worker names a json artifact, and Gold Band adds the extension."""
    for node in nodes:
        if node["type"] != "worker" or node.get("manual_check"):
            continue
        output = node["output"]
        assert output["kind"] == "json"
        # Gold Band appends ".json" to a json artifact's name itself.
        assert not output["artifact"].endswith(".json")


# ---------------------------------------------------------------------------
# The critics, the repairs and the escalation (issue #451)
# ---------------------------------------------------------------------------


def test_each_critics_round_is_one_ai_dynamic_node_fanning_out_to_two_critics(
    nodes: list[dict[str, Any]],
) -> None:
    """Two critics run in parallel inside one dynamic node, with explicit limits."""
    for round_ in ROUNDS:
        node = _node(nodes, f"critics-{round_}")
        assert node["type"] == "ai-dynamic"
        control = node["control"]
        assert control["maxFanout"] == 2
        assert control["maxParallel"] == 2
        assert control["maxGroupDepth"] == 1
        assert control["allowNestedDynamic"] is False
        # The bootstrap, two critics, the merge and the acceptance.
        assert control["maxDynamicNodes"] == 5
        assert node["agentStrategy"]["mode"] == "dynamic"


def test_the_critics_are_two_families_that_differ_from_the_writers(
    nodes: list[dict[str, Any]],
) -> None:
    """The critics run as Vibe (GLM) and Claude, and neither is the writer's family."""
    writer = _node(nodes, "writer")
    for round_ in ROUNDS:
        strategy = _node(nodes, f"critics-{round_}")["agentStrategy"]
        providers = [agent["provider"] for agent in strategy["availableAgents"]]
        assert providers == ["vibe", "claude"]
        assert writer["provider"] not in providers
        models = {agent["provider"]: agent["model"] for agent in strategy["availableAgents"]}
        assert writer["model"] not in models.values()
        assert models["vibe"] != _node(nodes, "test-author")["model"]
        assert strategy["bootstrapProvider"] in providers


def test_the_routing_prompt_asks_for_findings_in_the_finding_schema(
    nodes: list[dict[str, Any]],
) -> None:
    """The critics are told to write the critic finding schema, with quotes."""
    for round_ in ROUNDS:
        prompt = _node(nodes, f"critics-{round_}")["agentStrategy"]["routingPrompt"]
        assert "findings/<its node id>.json" in prompt
        assert "quote" in prompt
        assert "pf-diff-reviewer" in prompt


def test_each_round_runs_the_finding_check_and_a_verdict_on_its_actionable_count(
    nodes: list[dict[str, Any]],
    edges: list[dict[str, Any]],
) -> None:
    """Critics, then the findings gate and its copier, then the verdict node."""
    for round_ in ROUNDS:
        assert _edges_from(edges, f"critics-{round_}") == {"success": f"gate-findings-{round_}"}
        assert _edges_from(edges, f"copy-findings-{round_}") == {"success": f"verdict-{round_}"}
        verdict = _node(nodes, f"verdict-{round_}")
        assert verdict["output"]["schema"] == {"actionable": 0}
        assert verdict["success_condition"] == {"path": "$.actionable", "equals": 0}


def test_a_failed_verdict_goes_to_the_next_repair_and_a_clean_one_ends_the_run(
    edges: list[dict[str, Any]],
) -> None:
    """Rounds one and two go back to the writer on failure; every success ends."""
    for round_ in ROUNDS:
        targets = _edges_from(edges, f"verdict-{round_}")
        assert targets["success"] == "$end"
        assert targets["failure"] == (
            f"repair-{round_}" if round_ in REPAIRS else ESCALATION
        )


def test_there_are_exactly_two_repairs_both_by_the_writer(
    nodes: list[dict[str, Any]],
) -> None:
    """The writer's family repairs, twice at most, and no third repair exists."""
    ids = [node["id"] for node in nodes]
    assert [i for i in ids if i.startswith("repair-")] == [f"repair-{r}" for r in REPAIRS]
    writer = _node(nodes, "writer")
    for repair in REPAIRS:
        node = _node(nodes, f"repair-{repair}")
        assert (node["provider"], node["model"]) == (writer["provider"], writer["model"])
        assert "never act on a hint" in node["goal"]


def test_every_repair_is_followed_by_all_the_gates_and_a_new_critics_round(
    edges: list[dict[str, Any]],
) -> None:
    """After a repair the build, tests, lint and coverage run again, then the critics."""
    for repair in REPAIRS:
        chain = [f"repair-{repair}"]
        for name in _gate_names(repair):
            chain += [f"gate-{name}", f"copy-{name}"]
        chain.append(f"critics-{repair + 1}")
        for index in range(len(chain) - 1):
            assert _edges_from(edges, chain[index]) == {"success": chain[index + 1]}


def test_the_second_failed_repair_reaches_the_escalation_and_a_manual_check(
    nodes: list[dict[str, Any]],
    edges: list[dict[str, Any]],
) -> None:
    """The Opus judgment node always hands over to the manual check, then the run ends."""
    escalation = _node(nodes, ESCALATION)
    assert (escalation["provider"], escalation["model"]) == ("claude", "opus")
    assert _edges_from(edges, ESCALATION) == {
        "success": ESCALATION_CHECK,
        "failure": ESCALATION_CHECK,
    }
    assert _node(nodes, ESCALATION_CHECK)["manual_check"] is True
    assert _edges_from(edges, ESCALATION_CHECK) == {"success": "$end"}


def test_only_the_verdict_nodes_and_the_escalation_route_a_failure(
    edges: list[dict[str, Any]],
) -> None:
    """The only failure edges are the three verdicts' and the escalation's."""
    sources = sorted({edge["from"] for edge in edges if edge["on"] == "failure"})
    assert sources == sorted([f"verdict-{r}" for r in ROUNDS] + [ESCALATION])


def test_the_graph_is_a_finite_dag_so_the_attempt_cap_is_never_reached(
    workflow: dict[str, Any],
    nodes: list[dict[str, Any]],
    edges: list[dict[str, Any]],
) -> None:
    """No cycle, so no edge runs twice and one repair attempt per edge is enough.

    Gold Band counts a failure jump per edge (from node to node), so on a graph
    where every edge runs at most once a cap of one is never exceeded and the run
    cannot end on the control limit before it reaches the escalation.
    """
    ids = [node["id"] for node in nodes]
    out: dict[str, list[str]] = {i: [] for i in ids}
    for edge in edges:
        if edge["to"] != "$end":
            out[edge["from"]].append(edge["to"])

    def visit(node_id: str, path: tuple[str, ...]) -> None:
        assert node_id not in path, f"cycle through {node_id}"
        for target in out[node_id]:
            visit(target, (*path, node_id))

    visit(workflow["entry"], ())
    assert workflow["control"]["max_attempts"] >= 1
    assert all(edge["to"] != "$new-round" for edge in edges)
    pairs = [(edge["from"], edge["to"], edge["on"]) for edge in edges]
    assert len(pairs) == len(set(pairs))
