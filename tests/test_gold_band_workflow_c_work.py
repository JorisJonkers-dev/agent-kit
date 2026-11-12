"""Workflow C in the work instance (issue #463).

``installer/gold-band/workflows/workflow-c-work.json`` is the personal Workflow C
(``workflow-c.json``, left unchanged) bound to the work agents: ``claude-work`` and
``codex``, nothing else. Each Codex node (the review orchestrators) is preceded by a grant-check node whose
failure edge pauses the run on a manual check, and the critique steps are the
critique stage templates' nodes, whose runner routes a ``nedap`` repository to the
work-only critic roster.

The tests read the files and use a fixture setup file with a placeholder
repository and a fake ``gh``; no real repository is named and no issue is created.
The headless proof belongs to the ``gold-band-proof`` crate (docs/gold-band-proof.md).
"""

from __future__ import annotations

import json
import os
import shutil
import stat
import subprocess
from pathlib import Path
from typing import Any

import pytest

KIT_ROOT = Path(__file__).resolve().parent.parent
WORKFLOWS = KIT_ROOT / "installer" / "gold-band" / "workflows"
WORK = WORKFLOWS / "workflow-c-work.json"
PERSONAL = WORKFLOWS / "workflow-c.json"
FIXTURE_REPO = KIT_ROOT / "tests" / "fixtures" / "gold-band" / "work-repo"
MANIFEST = KIT_ROOT / "scripts" / "Cargo.toml"

WORK_AGENTS = {"claude-work", "codex"}
CODEX_NODES = ["spec-verdicts", "tickets-verdicts"]
GRANT_PAUSES = ["grant-refused"]
MANUAL_CHECKS = [
    "grill-check-1",
    "grill-check-2",
    "grill-check-3",
    "spec-needs-human",
    "tickets-needs-human",
    "publication-check",
    *GRANT_PAUSES,
]


@pytest.fixture(scope="module")
def template() -> dict[str, Any]:
    template: dict[str, Any] = json.loads(WORK.read_text())
    return template


@pytest.fixture(scope="module")
def nodes(template: dict[str, Any]) -> dict[str, dict[str, Any]]:
    return {node["id"]: node for node in template["workflow"]["nodes"]}


@pytest.fixture(scope="module")
def edges(template: dict[str, Any]) -> list[tuple[str, str, str]]:
    return [(e["from"], e["to"], e["on"]) for e in template["workflow"]["edges"]]


@pytest.fixture(scope="module")
def personal() -> dict[str, Any]:
    personal: dict[str, Any] = json.loads(PERSONAL.read_text())
    return personal


def _is_shim(node_id: str) -> bool:
    return node_id.startswith(("gate-", "copy-", "grant-check-"))


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


def test_the_personal_workflow_c_is_untouched_by_the_work_variant(personal: dict[str, Any]) -> None:
    assert personal["workflow"]["id"] == "workflow-c"
    text = PERSONAL.read_text().lower()
    assert "claude-work" not in text
    assert "grant-check" not in text


def test_the_variant_is_workflow_c_for_the_work_instance(template: dict[str, Any]) -> None:
    workflow = template["workflow"]
    assert workflow["id"] == "workflow-c-work"
    assert workflow["entry"] == "gate-workflow-c-start"
    assert "work instance" in workflow["description"]


def test_every_node_the_personal_workflow_has_is_kept_with_the_same_shape(
    nodes: dict[str, dict[str, Any]], personal: dict[str, Any]
) -> None:
    """The graph is the personal one; only bindings and the work-instance goal text differ."""
    for node in personal["workflow"]["nodes"]:
        mine = nodes[node["id"]]
        for key in ("type", "executionSlotId", "profile", "output", "success_condition", "manual_check"):
            assert mine.get(key) == node.get(key), (node["id"], key)


def test_the_edges_are_the_personal_ones_with_a_grant_check_before_each_codex_node(
    edges: list[tuple[str, str, str]], personal: dict[str, Any]
) -> None:
    theirs = {(e["from"], e["to"], e["on"]) for e in personal["workflow"]["edges"]}
    redirected = {(f, f"grant-check-{t}" if t in CODEX_NODES else t, on) for f, t, on in theirs}
    extra = set(edges) - redirected
    assert redirected <= set(edges)
    assert extra == {(f"grant-check-{name}", name, "success") for name in CODEX_NODES} | {
        (f"grant-check-{name}", "grant-refused", "failure") for name in CODEX_NODES
    }


def test_only_the_work_agents_and_shims_are_bound(nodes: dict[str, dict[str, Any]], template: dict[str, Any]) -> None:
    for node_id, node in nodes.items():
        if _is_shim(node_id):
            assert node["model"] == "stub-model", node_id
        else:
            assert node["provider"] in WORK_AGENTS, node_id
    bound = {b["agentId"] for b in template["modelBindings"]["bindings"] if not _is_shim(b["executionSlotId"][5:])}
    assert bound == WORK_AGENTS


def test_no_personal_agent_is_named_anywhere() -> None:
    text = WORK.read_text()
    for personal_agent in ('"vibe"', '"vibe-study"', '"claude"', "opencode", "antigravity", "hermes", "mistral", "glm"):
        assert personal_agent not in text


def test_every_node_has_a_model_binding(template: dict[str, Any], nodes: dict[str, dict[str, Any]]) -> None:
    bindings = template["modelBindings"]["bindings"]
    slots = [b["executionSlotId"] for b in bindings]
    assert len(set(slots)) == len(slots) == len(nodes)
    by_slot = {b["executionSlotId"]: b for b in bindings}
    for node in nodes.values():
        binding = by_slot[node["executionSlotId"]]
        assert (binding["agentId"], binding["modelId"]) == (node["provider"], node["model"])


def test_the_codex_nodes_are_exactly_the_orchestrators(nodes: dict[str, dict[str, Any]]) -> None:
    assert sorted(n for n, node in nodes.items() if "codex" in node["provider"]) == sorted(CODEX_NODES)


@pytest.mark.parametrize("draft", CODEX_NODES)
def test_each_codex_node_is_reachable_only_through_its_grant_check(
    nodes: dict[str, dict[str, Any]], edges: list[tuple[str, str, str]], draft: str
) -> None:
    grant = f"grant-check-{draft}"
    assert [frm for frm, to, _ in edges if to == draft] == [grant]
    assert nodes[grant]["success_condition"] == {"path": "$.exit_code", "equals": 0}
    assert "grant-check wrapper" in nodes[grant]["goal"]
    assert draft not in _reachable(edges, "gate-workflow-c-start", without={grant})


@pytest.mark.parametrize("draft", CODEX_NODES)
def test_a_refused_grant_pauses_the_run_on_a_manual_check_that_goes_nowhere(
    nodes: dict[str, dict[str, Any]], edges: list[tuple[str, str, str]], draft: str
) -> None:
    pause = "grant-refused"
    assert (f"grant-check-{draft}", pause, "failure") in edges
    assert nodes[pause]["manual_check"] is True
    assert [e for e in edges if e[0] == pause] == []


def test_the_nodes_that_pause_for_the_owner_are_exactly_the_manual_checks(
    nodes: dict[str, dict[str, Any]],
) -> None:
    assert sorted(n for n, node in nodes.items() if node.get("manual_check")) == sorted(MANUAL_CHECKS)


def test_the_publish_gate_is_reachable_only_through_the_publication_check(
    edges: list[tuple[str, str, str]],
) -> None:
    assert [frm for frm, to, _ in edges if to == "gate-workflow-c-publish"] == ["publication-check"]
    reached = _reachable(edges, "gate-workflow-c-start", without={"publication-check"})
    assert not reached & {"gate-workflow-c-publish", "copy-workflow-c-publish"}
    assert "gate-workflow-c-publish" in _reachable(edges, "gate-workflow-c-start", without=set())


def test_no_gate_or_copier_node_has_a_failure_edge(edges: list[tuple[str, str, str]]) -> None:
    failing = [e for e in edges if e[2] == "failure" and e[0].startswith(("gate-", "copy-"))]
    assert failing == []


def test_every_node_is_reachable_and_every_edge_resolves(
    nodes: dict[str, dict[str, Any]], edges: list[tuple[str, str, str]]
) -> None:
    assert _reachable(edges, "gate-workflow-c-start", set()) - {"$end"} == set(nodes)
    for frm, to, _ in edges:
        assert frm in nodes
        assert to == "$end" or to in nodes


@pytest.mark.parametrize("stage", ["spec", "tickets"])
@pytest.mark.parametrize("step", ["prepare", "fan-out"])
def test_the_critique_steps_name_the_work_only_roster(nodes: dict[str, dict[str, Any]], stage: str, step: str) -> None:
    goal = nodes[f"gate-critique-{stage}-{step}"]["goal"]
    assert f"`critique {step} --strict`" in goal
    assert "work-only critic roster" in goal
    assert "nedap" in goal


@pytest.mark.parametrize("stage", ["spec", "tickets"])
def test_the_other_critique_nodes_are_the_stage_template_s_own(nodes: dict[str, dict[str, Any]], stage: str) -> None:
    template = json.loads((WORKFLOWS / f"critique-{stage}.json").read_text())
    for node in template["workflow"]["nodes"]:
        if node["id"].endswith("-start") or (node["id"].endswith(("-prepare", "-fan-out")) and _is_shim(node["id"])):
            continue
        if node["provider"] in ("vibe", "vibe-study"):
            for key in ("goal", "output", "success_condition", "manual_check"):
                assert nodes[node["id"]].get(key) == node.get(key)
        else:
            assert nodes[node["id"]] == node


def test_the_spec_author_and_the_review_orchestrator_are_in_different_families(
    nodes: dict[str, dict[str, Any]],
) -> None:
    for draft, verdicts in (("spec-draft", "spec-verdicts"), ("tickets-draft", "tickets-verdicts")):
        assert nodes[draft]["provider"] != nodes[verdicts]["provider"]


def test_the_publish_gate_names_the_work_repository_s_setup_file(nodes: dict[str, dict[str, Any]]) -> None:
    goal = nodes["gate-workflow-c-publish"]["goal"]
    assert "`ticket-publish publish`" in goal
    assert "setup file" in goal
    assert "tickets.json" in goal
    assert "docs/agents/setup.yaml" in nodes["gate-workflow-c-setup"]["goal"]


def test_no_node_writes_memory(nodes: dict[str, dict[str, Any]]) -> None:
    assert "memory-write" not in WORK.read_text().lower()


# --- publication goes to the repository named by the setup file (fixture, fake gh) ---


def _ticket_publish() -> str:
    named = os.environ.get("TICKET_PUBLISH_BIN")
    if named:
        return named
    if shutil.which("cargo") is None:
        pytest.skip("cargo is missing and TICKET_PUBLISH_BIN is not set")
    built = subprocess.run(
        ["cargo", "build", "--offline", "-q", "--manifest-path", str(MANIFEST), "-p", "ticket-publish"],
        capture_output=True,
        text=True,
        check=False,
    )
    if built.returncode != 0:
        pytest.skip(f"ticket-publish cannot be built offline (set TICKET_PUBLISH_BIN): {built.stderr[-300:]}")
    return str(KIT_ROOT / "scripts" / "target" / "debug" / "ticket-publish")


def _fake_gh(directory: Path) -> Path:
    """A `gh` that records its arguments and creates nothing."""
    log = directory / "gh.log"
    script = directory / "bin" / "gh"
    script.parent.mkdir()
    script.write_text(f'#!/bin/sh\necho "$@" >> "{log}"\necho https://example.invalid/issues/1\n')
    script.chmod(script.stat().st_mode | stat.S_IEXEC)
    return log


def _env(directory: Path) -> dict[str, str]:
    return {**os.environ, "PATH": f"{directory / 'bin'}{os.pathsep}{os.environ['PATH']}"}


def test_ticket_publish_creates_issues_only_in_the_repository_the_setup_file_names(tmp_path: Path) -> None:
    binary = _ticket_publish()
    log = _fake_gh(tmp_path)
    tickets = tmp_path / "tickets.json"
    tickets.write_text(json.dumps({"tickets": [{"title": "One", "body": "b", "labels": ["type: feature"]}]}))
    env = _env(tmp_path)
    checked = subprocess.run([binary, "check", str(FIXTURE_REPO)], env=env, capture_output=True, text=True, check=False)
    assert checked.returncode == 0, checked.stderr
    assert not log.exists(), "check must not call the tracker"
    published = subprocess.run(
        [binary, "publish", str(FIXTURE_REPO), str(tickets)], env=env, capture_output=True, text=True, check=False
    )
    assert published.returncode == 0, published.stderr
    calls = log.read_text().splitlines()
    assert len(calls) == 1
    assert "--repo example-org/work-tickets" in calls[0]
    assert "--label type: feature" in calls[0]


def test_ticket_publish_refuses_a_label_the_setup_file_does_not_list_and_creates_nothing(tmp_path: Path) -> None:
    binary = _ticket_publish()
    log = _fake_gh(tmp_path)
    tickets = tmp_path / "tickets.json"
    tickets.write_text(json.dumps({"tickets": [{"title": "One", "body": "b", "labels": ["bug"]}]}))
    refused = subprocess.run(
        [binary, "publish", str(FIXTURE_REPO), str(tickets)],
        env=_env(tmp_path),
        capture_output=True,
        text=True,
        check=False,
    )
    assert refused.returncode != 0
    assert not log.exists()
