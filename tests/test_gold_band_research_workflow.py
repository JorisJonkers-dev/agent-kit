"""Workflow A (deep research) as a Gold Band workflow (issue #456).

The template at ``installer/gold-band/workflows/research.json`` is Workflow A
for the personal instance: the run-start gate, a gate that gives the run its
own branch of the private research repository, the Opus planning node, the
fetch shim, one ai-dynamic node that fans out the Pi fetchers within explicit
limits, the Claude merge worker, the index gate, the read-only verifier and
the Opus acceptance node.

These tests read the template as a file and need no Gold Band binary or Rust
toolchain. Its headless proof belongs to the ``gold-band-proof`` crate
(docs/gold-band-proof.md), and the quote check to the ``research-layout``
crate's own tests.
"""

from __future__ import annotations

import json
import re
from pathlib import Path
from typing import Any

import pytest

KIT_ROOT = Path(__file__).resolve().parent.parent
TEMPLATE = KIT_ROOT / "installer" / "gold-band" / "workflows" / "research.json"

#: The nodes of the run, in the order they run.
ORDER = [
    "gate-research-start",
    "gate-research-papers",
    "copy-research-papers",
    "research-planning",
    "gate-research-fetch",
    "copy-research-fetch",
    "research-fetchers",
    "gate-research-adopt",
    "copy-research-adopt",
    "research-merge",
    "gate-research-index",
    "copy-research-index",
    "gate-research-verify",
    "copy-research-verify",
    "research-acceptance",
]

#: The private research repository's branch, as the nodes name it.
PRIVATE = "papers-worktree"


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


def _providers(node: dict[str, Any]) -> set[str]:
    """Every agent id a node binds."""
    if node["type"] != "ai-dynamic":
        return {node["provider"]}
    strategy = node["agentStrategy"]
    return {strategy["bootstrapProvider"]} | {agent["provider"] for agent in strategy["availableAgents"]}


def test_the_workflow_is_research_entered_at_the_run_start_gate(workflow: dict[str, Any]) -> None:
    assert workflow["id"] == "research"
    assert workflow["entry"] == "gate-research-start"


def test_the_run_goes_through_planning_fan_out_merge_verify_and_acceptance_in_that_order(
    edges: list[tuple[str, str, str]],
) -> None:
    """One success chain, no branch: every stage runs once, in order, and nothing else does."""
    chain = ORDER + ["$end"]
    assert edges == [(a, b, "success") for a, b in zip(chain, chain[1:])]


def test_no_model_node_runs_before_the_rule_check(edges: list[tuple[str, str, str]]) -> None:
    assert edges[0][0] == "gate-research-start"


def test_no_shim_node_has_a_failure_edge_so_a_failed_verify_ends_the_run(
    edges: list[tuple[str, str, str]],
) -> None:
    assert [e for e in edges if e[2] == "failure"] == []
    # The acceptance is reached only through the verifier's copier.
    assert [frm for frm, to, _ in edges if to == "research-acceptance"] == ["copy-research-verify"]


def test_every_gate_has_its_copier_next(edges: list[tuple[str, str, str]]) -> None:
    for frm, to, _ in edges:
        if frm.startswith("gate-") and frm != "gate-research-start":
            assert to == "copy-" + frm.removeprefix("gate-")


def test_every_shim_node_succeeds_only_on_exit_0(nodes: dict[str, dict[str, Any]]) -> None:
    for node_id, node in nodes.items():
        if node_id.startswith(("gate-", "copy-")):
            assert node["success_condition"] == {"path": "$.exit_code", "equals": 0}
            assert node["provider"] == node_id
            assert node["model"] == "stub-model"


def test_the_verifier_runs_the_quote_check_and_only_reads(nodes: dict[str, dict[str, Any]]) -> None:
    goal = nodes["gate-research-verify"]["goal"]
    assert "`research-layout verify --papers" in goal
    assert "--fetched" in goal
    assert "read-only" in goal
    assert "a quote is not on its page" in goal
    assert "change no file" in nodes["copy-research-verify"]["goal"]


def test_the_fetch_shim_holds_the_same_width_limit_as_the_fan_out(nodes: dict[str, dict[str, Any]]) -> None:
    limit = nodes["research-fetchers"]["control"]["maxFanout"]
    match = re.search(r"--max-sources (\d+)", nodes["gate-research-fetch"]["goal"])
    assert match is not None
    assert int(match.group(1)) == limit
    assert f"at most {['zero', 'one', 'two', 'three', 'four', 'five', 'six'][limit]} public web sources" in (
        nodes["research-planning"]["goal"]
    )


def test_the_fan_out_declares_every_limit_explicitly(nodes: dict[str, dict[str, Any]]) -> None:
    fetchers = nodes["research-fetchers"]
    assert fetchers["type"] == "ai-dynamic"
    assert fetchers["agentStrategy"]["mode"] == "dynamic"
    assert fetchers["control"] == {
        "maxDynamicNodes": 9,
        "maxFanout": 6,
        "maxDepth": 2,
        "maxParallel": 3,
        "maxGroupDepth": 1,
        "maxWorkflowInvocations": 1,
        "allowNestedDynamic": False,
    }
    # The bootstrap, six fetchers, the merge and the acceptance.
    assert fetchers["control"]["maxDynamicNodes"] == 1 + fetchers["control"]["maxFanout"] + 2
    assert fetchers["allowedWorkflows"] == []


def test_the_fetchers_are_pi_over_litellm_routes(nodes: dict[str, dict[str, Any]]) -> None:
    strategy = nodes["research-fetchers"]["agentStrategy"]
    assert _providers(nodes["research-fetchers"]) == {"pi"}
    models = [strategy["bootstrapModel"], strategy["acceptanceModel"]] + [
        agent["model"] for agent in strategy["availableAgents"]
    ]
    assert all(model.startswith(("litellm-vibe/", "litellm-vibe-study/")) for model in models)


def test_only_the_fetchers_bind_a_non_anthropic_agent_and_they_never_see_the_private_repository(
    nodes: dict[str, dict[str, Any]],
) -> None:
    """The research repository is private: Pi reads only the plan and the public pages."""
    for node_id, node in nodes.items():
        if node_id.startswith(("gate-", "copy-")):
            continue
        if node_id == "research-fetchers":
            assert PRIVATE not in json.dumps(node)
            assert "research repository" not in json.dumps(node)
        else:
            assert _providers(node) == {"claude"}, node_id


def test_opus_judges_the_plan_and_the_result_and_nothing_else(nodes: dict[str, dict[str, Any]]) -> None:
    opus = sorted(node_id for node_id, node in nodes.items() if node.get("model") == "opus")
    assert opus == ["research-acceptance", "research-planning"]
    assert nodes["research-merge"]["model"] == "sonnet"


def test_the_acceptance_ends_the_run_and_can_refuse_it(
    nodes: dict[str, dict[str, Any]],
    edges: list[tuple[str, str, str]],
) -> None:
    assert ORDER[-1] == "research-acceptance"
    assert [e for e in edges if e[0] == "research-acceptance"] == [("research-acceptance", "$end", "success")]
    assert nodes["research-acceptance"]["success_condition"] == {"path": "$.ok", "equals": True}


def test_the_merge_writes_the_papers_layout_on_the_run_s_own_branch(nodes: dict[str, dict[str, Any]]) -> None:
    goal = nodes["research-merge"]["goal"]
    assert f"{PRIVATE}/papers" in goal
    assert "catalog.json" in goal
    assert "copied byte for byte by the adopt gate" in goal
    assert "never edit, move or delete one" in goal
    assert "Do not write INDEX.md" in goal
    assert "`research-layout adopt --plan" in nodes["gate-research-adopt"]["goal"]
    assert "-b research/<run id> origin/main" in nodes["gate-research-papers"]["goal"]


def test_no_forbidden_family_is_bound_and_no_node_writes_memory(text: str) -> None:
    lowered = text.lower()
    for family in ("codex", "openrouter", "opencode/", "zen"):
        assert family not in lowered
    assert "memory-write" not in lowered


def test_every_node_has_a_model_binding(template: dict[str, Any], nodes: dict[str, dict[str, Any]]) -> None:
    workers = {node_id: node for node_id, node in nodes.items() if node["type"] == "worker"}
    bindings = template["modelBindings"]["bindings"]
    slots = [binding["executionSlotId"] for binding in bindings]
    assert len(set(slots)) == len(slots) == len(workers)
    by_slot = {binding["executionSlotId"]: binding for binding in bindings}
    for node in workers.values():
        binding = by_slot[node["executionSlotId"]]
        assert (binding["agentId"], binding["modelId"]) == (node["provider"], node["model"])
