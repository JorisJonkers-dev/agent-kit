"""Wave 4 human-gate workflows as Gold Band templates (issue #628).

grill-me, grill-with-docs, brainstorming, to-spec, to-tickets and pr-composer
are checked-in templates under ``installer/gold-band/workflows/``. Each unrolls
its question rounds as one manual-check node per round, up to the round cap its
file states, and pr-composer's push-and-create gate is reachable only through a
confirmation node. The tests read the files and need no Gold Band binary; the
headless proof belongs to the ``gold-band-proof`` crate (docs/gold-band-proof.md).
"""

from __future__ import annotations

import json
from pathlib import Path
from typing import Any

import pytest

KIT_ROOT = Path(__file__).resolve().parent.parent
WORKFLOWS = KIT_ROOT / "installer" / "gold-band" / "workflows"

#: workflow id -> (round node prefix, check node prefix, round cap, node after the last round)
ROUND_WORKFLOWS = {
    "grill-me": ("grill-round-", "grill-check-", 5, "grill-summary"),
    "grill-with-docs": ("grill-round-", "grill-check-", 5, "docs-update"),
    "brainstorming": ("explore-round-", "explore-check-", 4, "design-draft"),
    "to-spec": ("spec-round-", "spec-check-", 3, "spec-write"),
    "to-tickets": ("tickets-round-", "tickets-check-", 3, "tickets-write"),
}
ALL = [*ROUND_WORKFLOWS, "pr-composer"]
PR_CAP = 3
CAP_WORDS = {3: "three", 4: "four", 5: "five"}


def load(name: str) -> dict[str, Any]:
    return json.loads((WORKFLOWS / f"{name}.json").read_text(encoding="utf-8"))


def nodes_of(name: str) -> dict[str, dict[str, Any]]:
    return {n["id"]: n for n in load(name)["workflow"]["nodes"]}


def edges_of(name: str) -> list[tuple[str, str, str]]:
    return [(e["from"], e["to"], e["on"]) for e in load(name)["workflow"]["edges"]]


def manual_checks(name: str) -> list[str]:
    return [i for i, n in nodes_of(name).items() if n.get("manual_check")]


def reachable(edges: list[tuple[str, str, str]], entry: str, without: set[str]) -> set[str]:
    seen: set[str] = set()
    todo = [entry]
    while todo:
        node = todo.pop()
        if node in seen or node in without:
            continue
        seen.add(node)
        todo.extend(to for frm, to, _ in edges if frm == node)
    return seen


@pytest.mark.parametrize("name", ALL)
def test_the_workflow_is_entered_at_its_run_start_gate(name: str) -> None:
    workflow = load(name)["workflow"]
    assert workflow["id"] == name
    assert workflow["entry"] == f"gate-{name}-start"
    assert edges_of(name)[0] == (f"gate-{name}-start", edges_of(name)[0][1], "success")
    assert "gold-band-run-start" in nodes_of(name)[f"gate-{name}-start"]["goal"]


@pytest.mark.parametrize("name", ROUND_WORKFLOWS)
def test_each_round_has_one_manual_check_up_to_the_stated_cap(name: str) -> None:
    round_prefix, check_prefix, cap, tail = ROUND_WORKFLOWS[name]
    nodes, edges = nodes_of(name), edges_of(name)
    assert [i for i in range(1, 20) if f"{round_prefix}{i}" in nodes] == list(range(1, cap + 1))
    checks = [f"{check_prefix}{i}" for i in range(1, cap + 1)]
    assert [c for c in manual_checks(name) if c.startswith(check_prefix)] == checks
    for i in range(1, cap + 1):
        round_id, check_id = f"{round_prefix}{i}", f"{check_prefix}{i}"
        following = f"{round_prefix}{i + 1}" if i < cap else tail
        assert nodes[round_id]["success_condition"] == {"path": "$.open_questions", "equals": []}
        assert (round_id, tail, "success") in edges
        assert (round_id, check_id, "failure") in edges
        assert (check_id, following, "success") in edges
    description = load(name)["workflow"]["description"]
    assert f"capped at {CAP_WORDS[cap]}" in description


@pytest.mark.parametrize("name", ROUND_WORKFLOWS)
def test_the_manual_checks_write_nothing_and_have_no_output(name: str) -> None:
    for node_id in manual_checks(name):
        node = nodes_of(name)[node_id]
        assert "output" not in node
        assert "Do not" in node["goal"]


def test_brainstorming_records_the_design_only_after_the_approval_check() -> None:
    edges = edges_of("brainstorming")
    assert [f for f, t, _ in edges if t == "design-record"] == ["design-approval"]
    assert "design-approval" in manual_checks("brainstorming")
    reached = reachable(edges, "gate-brainstorming-start", without={"design-approval"})
    assert "design-record" not in reached


def test_to_tickets_creates_nothing_in_a_tracker() -> None:
    goal = nodes_of("to-tickets")["tickets-write"]["goal"]
    assert "Create nothing in any tracker" in goal
    assert not any(n.startswith(("gate-ticket-publish", "gate-workflow-c-publish")) for n in nodes_of("to-tickets"))


def test_pr_composer_unrolls_its_confirmation_rounds_as_manual_checks() -> None:
    nodes, edges = nodes_of("pr-composer"), edges_of("pr-composer")
    confirms = [f"pr-confirm-{i}" for i in range(1, PR_CAP + 1)]
    assert [c for c in manual_checks("pr-composer") if c.startswith("pr-confirm-")] == confirms
    assert [i for i in range(1, 20) if f"pr-confirm-{i}" in nodes] == list(range(1, PR_CAP + 1))
    assert "pr-links-check" in manual_checks("pr-composer")
    assert sorted(manual_checks("pr-composer")) == sorted([*confirms, "pr-links-check"])
    for i in range(1, PR_CAP + 1):
        assert (f"pr-confirm-{i}", f"pr-apply-{i}", "success") in edges
        assert (f"pr-apply-{i}", "gate-pr-composer-publish", "success") in edges
        if i < PR_CAP:
            assert (f"pr-apply-{i}", f"pr-confirm-{i + 1}", "failure") in edges
    assert not [e for e in edges if e[0] == f"pr-apply-{PR_CAP}" and e[2] == "failure"]
    assert "capped at three" in load("pr-composer")["workflow"]["description"]


def test_the_push_and_create_step_is_reachable_only_after_a_confirmation_node() -> None:
    edges = edges_of("pr-composer")
    confirms = {f"pr-confirm-{i}" for i in range(1, PR_CAP + 1)}
    applies = {f"pr-apply-{i}" for i in range(1, PR_CAP + 1)}
    assert {f for f, t, _ in edges if t == "gate-pr-composer-publish"} == applies
    for apply_id in applies:
        assert [f for f, t, _ in edges if t == apply_id] == [f"pr-confirm-{apply_id[-1]}"]
    # With every confirmation node out of the graph, neither half of the shim is reachable.
    reached = reachable(edges, "gate-pr-composer-start", without=confirms)
    assert "gate-pr-composer-publish" not in reached
    assert "copy-pr-composer-publish" not in reached
    assert "gate-pr-composer-publish" in reachable(edges, "gate-pr-composer-start", without=set())
    assert [f for f, t, _ in edges if t == "$end"] == ["copy-pr-composer-publish"]


def test_the_publish_gate_rechecks_the_body_and_updates_an_existing_pull_request() -> None:
    goal = nodes_of("pr-composer")["gate-pr-composer-publish"]["goal"]
    assert "re-checks the body file against the template" in goal
    assert "`gh pr edit`" in goal


def test_no_node_before_the_first_confirmation_pushes_or_creates() -> None:
    nodes, edges = nodes_of("pr-composer"), edges_of("pr-composer")
    before = reachable(edges, "gate-pr-composer-start", without={"pr-confirm-1", "pr-links-check"})
    before |= {"pr-links-check"}
    for node_id in before - {"gate-pr-composer-publish", "copy-pr-composer-publish", "$end"}:
        goal = nodes[node_id]["goal"]
        assert "gh pr create" not in goal and "git push" not in goal
    assert "never runs without the owner's confirmation" in nodes["gate-pr-composer-publish"]["goal"]


@pytest.mark.parametrize("name", ALL)
def test_no_shim_node_has_a_failure_edge(name: str) -> None:
    assert [e for e in edges_of(name) if e[2] == "failure" and e[0].startswith(("gate-", "copy-"))] == []


@pytest.mark.parametrize("name", ALL)
def test_every_edge_target_exists_and_every_node_is_reachable(name: str) -> None:
    nodes, edges = nodes_of(name), edges_of(name)
    for frm, to, _ in edges:
        assert frm in nodes
        assert to == "$end" or to in nodes
    assert reachable(edges, load(name)["workflow"]["entry"], set()) - {"$end"} == set(nodes)


@pytest.mark.parametrize("name", ALL)
def test_every_node_has_one_model_binding_and_no_built_in_agent_id(name: str) -> None:
    template, nodes = load(name), nodes_of(name)
    bindings = template["modelBindings"]["bindings"]
    slots = [b["executionSlotId"] for b in bindings]
    assert len(set(slots)) == len(slots) == len(nodes)
    by_slot = {b["executionSlotId"]: b for b in bindings}
    for node in nodes.values():
        binding = by_slot[node["executionSlotId"]]
        assert (binding["agentId"], binding["modelId"]) == (node["provider"], node["model"])
        assert node["provider"] not in {"claude", "codex", "opencode", "gemini"}


@pytest.mark.parametrize("name", ALL)
def test_no_forbidden_family_is_bound_and_no_node_writes_memory(name: str) -> None:
    lowered = (WORKFLOWS / f"{name}.json").read_text(encoding="utf-8").lower()
    assert "codex" not in lowered and "opus" not in lowered
    assert "memory-write" not in lowered


@pytest.mark.parametrize("name", ALL)
def test_every_output_artifact_has_no_json_suffix(name: str) -> None:
    for node in nodes_of(name).values():
        if isinstance(node.get("output"), dict):
            assert not node["output"]["artifact"].endswith(".json")
