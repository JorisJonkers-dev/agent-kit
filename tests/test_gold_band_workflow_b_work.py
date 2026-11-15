"""Workflow B v2 in the work instance (issues #462 and #797).

The template at ``installer/gold-band/workflows/workflow-b-work.json`` is
Workflow B v2 bound to the work instance's two agents only: the work Claude agent
(``claude-work``, Anthropic) and Codex (``codex``, OpenAI). It has the shape of the
personal ``workflow-b.json`` (unrolled TDD slices, a test critic, gate repair edges,
the runner's ticket-review stage). Claude plans and writes each slice's test and
reviews the code, Codex writes the code, the repairs, the writer's self-review and,
because two families cannot make three, the test critic. Every Codex node is
preceded by a grant-check node (#460), and that node's failure edge leads to a
manual check.

The static tests read the template as a file. The headless proof runs each
Codex node's stretch of the real template (its grant-check node, the Codex node
and the manual check, taken from the file) against the headless ``gold-band``
named by ``GOLD_BAND_BIN``, in a throw-away ``GOLD_BAND_HOME``, with the real
wrapper and the real gateway bundle over a fake grants file in a throw-away
configuration outside any repository. No Codex and no Claude is called: the
model nodes are stub agents that write marker files. The whole template's graph
is proved by the ``gold-band-proof`` crate (docs/gold-band-proof.md).
"""

from __future__ import annotations

import json
import subprocess
from pathlib import Path
from typing import Any

import pytest

# Reused from the grant-check proof: the throw-away machine, its stub agents and its skip rules.
from test_gold_band_grant_check import (  # noqa: F401
    GOLD_BAND_BIN,
    NO_TOOLS,
    RUN_ID,
    SETTINGS_SCHEMA_VERSION,
    TIMEOUT_SECONDS,
    Machine,
    _agent,
    _report,
    wrapper,
)

KIT_ROOT = Path(__file__).resolve().parent.parent
WORKFLOWS = KIT_ROOT / "installer" / "gold-band" / "workflows"
TEMPLATE = WORKFLOWS / "workflow-b-work.json"
PERSONAL = WORKFLOWS / "workflow-b.json"

WORKFLOW_NAME = "workflow-b-work"
RUN_NAME = f"{WORKFLOW_NAME}@github.com/nedap/healthcare@{RUN_ID}"

#: The work instance's two agents and their families (the families file's mapping).
FAMILY = {"claude-work": "anthropic", "codex": "openai"}
#: The stub agent ids the gate shim and the verdicts run as, as in the personal template.
SHIM_PREFIXES = ("gate-", "copy-", "verdict-", "grant-", "accept-")
#: Every node that binds Codex, and the grant-check node that must precede it: the three slices'
#: test critics and writers, the two repairs, the self-review and the review stage's own Codex nodes.
CODEX_NODES = [
    "test-critic-1",
    "writer-s1",
    "test-critic-2",
    "writer-s2",
    "test-critic-3",
    "writer-s3",
    "repair-1",
    "repair-2",
    "self-review",
    "ticket-review-verdicts",
    "ticket-review-revise-1",
    "ticket-review-verdicts-2",
]
WRITERS = ["writer-s1", "writer-s2", "writer-s3", "repair-1", "repair-2", "self-review"]
AUTHORS = [
    "slice-plan",
    "test-author-s1",
    "test-author-s2",
    "test-author-s3",
    "test-revise-1",
    "test-revise-2",
    "test-revise-3",
]


@pytest.fixture(scope="module")
def template() -> dict[str, Any]:
    template: dict[str, Any] = json.loads(TEMPLATE.read_text())
    return template


@pytest.fixture(scope="module")
def workflow(template: dict[str, Any]) -> dict[str, Any]:
    definition: dict[str, Any] = template["workflow"]
    return definition


@pytest.fixture(scope="module")
def nodes(workflow: dict[str, Any]) -> dict[str, dict[str, Any]]:
    return {node["id"]: node for node in workflow["nodes"]}


def _into(workflow: dict[str, Any], target: str) -> list[dict[str, Any]]:
    return [edge for edge in workflow["edges"] if edge["to"] == target]


def _out(workflow: dict[str, Any], source: str) -> dict[str, str]:
    return {edge["on"]: edge["to"] for edge in workflow["edges"] if edge["from"] == source}


def _agents(node: dict[str, Any]) -> list[str]:
    """The agent ids a node binds: its provider, or an ai-dynamic node's strategy."""
    if node["type"] == "ai-dynamic":
        strategy = node["agentStrategy"]
        return [strategy["bootstrapProvider"], *(a["provider"] for a in strategy["availableAgents"])]
    return [node["provider"]]


def _is_shim(agent: str) -> bool:
    return agent.startswith(SHIM_PREFIXES)


# ---- the template as a file ---------------------------------------------


def test_the_work_variant_is_its_own_workflow_and_holds_no_work_agent_in_the_personal_one(
    workflow: dict[str, Any],
) -> None:
    assert workflow["id"] == WORKFLOW_NAME
    personal = PERSONAL.read_text()
    assert json.loads(personal)["workflow"]["id"] == "workflow-b"
    assert "codex" not in personal.lower() and "claude-work" not in personal


def test_only_the_work_agents_and_the_shim_stubs_are_bound(
    template: dict[str, Any], nodes: dict[str, dict[str, Any]]
) -> None:
    bound = {
        agent for node in nodes.values() if node["type"] != "worker" or "provider" in node for agent in _agents(node)
    }
    bound |= {binding["agentId"] for binding in template["modelBindings"]["bindings"]}
    assert {agent for agent in bound if not _is_shim(agent)} == set(FAMILY)
    for node in nodes.values():
        for agent in _agents(node):
            assert agent in FAMILY or _is_shim(agent), (node["id"], agent)


def test_author_and_reviewer_are_of_different_families_where_two_families_allow(
    nodes: dict[str, dict[str, Any]],
) -> None:
    """Codex writes code, Claude writes the tests; the test critic is Codex (the author's competitor).

    The work instance has two families, so a test critic cannot differ from both the test author and
    the writer: it differs from the author (which the lint checks) and the writer is the tests' consumer.
    """
    assert {FAMILY[nodes[n]["provider"]] for n in WRITERS} == {"openai"}
    assert {FAMILY[nodes[n]["provider"]] for n in AUTHORS} == {"anthropic"}
    for slice_ in (1, 2, 3):
        critic = nodes[f"test-critic-{slice_}"]
        assert critic["type"] == "worker"
        assert FAMILY[critic["provider"]] == "openai"
        assert critic["profile"] == "pf-diff-reviewer"
        assert f"findings/test-critic-{slice_}.json" in critic["goal"] and "quote" in critic["goal"]


def test_the_code_review_leaves_the_orchestrator_s_family_out_so_claude_reviews_both_axes() -> None:
    routing = json.loads((KIT_ROOT / "skills" / "workstation" / "critique" / "routing.json").read_text())
    work = routing["stages"]["ticket-review"]["work"]
    families = {routing["clients"][e["client"]]["family"]: e["lenses"] for e in work}
    assert set(families) == {"openai", "anthropic"}
    # The orchestrator is Codex, the writer's family, so the runner drops it: the work Claude agent is left.
    assert families["anthropic"][:2] == ["spec-conformance", "standards"]


def test_codex_is_bound_only_by_the_writers_the_test_critics_and_the_review_stage_s_codex_nodes(
    nodes: dict[str, dict[str, Any]],
) -> None:
    codex = [n["id"] for n in nodes.values() if "codex" in _agents(n)]
    assert codex == CODEX_NODES


def test_a_grant_check_node_is_the_only_way_into_each_codex_node(workflow: dict[str, Any]) -> None:
    """Every edge into a Codex node comes from the grant-check node named for it, on success."""
    for name in CODEX_NODES:
        assert _into(workflow, name) == [{"from": f"grant-check-{name}", "to": name, "on": "success"}]


def test_each_grant_check_node_runs_the_pinned_wrapper_and_holds_its_exit_to_zero(
    nodes: dict[str, dict[str, Any]],
) -> None:
    for name in CODEX_NODES:
        node = nodes[f"grant-check-{name}"]
        assert node["provider"] == f"grant-{name}"
        assert f"--node {name}" in node["goal"] and "gold-band-run-start grant-check" in node["goal"]
        assert node["success_condition"] == {"path": "$.exit_code", "equals": 0}


def test_a_refused_grant_check_goes_to_a_manual_check_and_never_to_codex(
    workflow: dict[str, Any], nodes: dict[str, dict[str, Any]]
) -> None:
    for name in CODEX_NODES:
        out = _out(workflow, f"grant-check-{name}")
        assert out == {"success": name, "failure": "grant-refused"}
    assert nodes["grant-refused"].get("manual_check") is True
    assert _out(workflow, "grant-refused") == {"success": "$end"}


def test_a_failed_gate_reaches_a_repair_through_its_grant_check_and_a_failed_review_the_writer(
    workflow: dict[str, Any],
) -> None:
    for gate in ("build", "test", "lint", "coverage"):
        assert _out(workflow, f"gate-{gate}")["failure"] == "grant-check-repair-1"
        assert _out(workflow, f"gate-{gate}-r1")["failure"] == "grant-check-repair-2"
        assert _out(workflow, f"gate-{gate}-r2")["failure"] == "gates-escalation"
    assert _out(workflow, "accept-ticket-review-1")["failure"] == "grant-check-ticket-review-revise-1"
    assert _out(workflow, "accept-ticket-review-2")["failure"] == "ticket-review-escalation"


def test_every_worker_binds_the_agent_and_model_its_slot_names(
    template: dict[str, Any], nodes: dict[str, dict[str, Any]]
) -> None:
    by_slot = {b["executionSlotId"]: b for b in template["modelBindings"]["bindings"]}
    workers = [n for n in nodes.values() if n["type"] == "worker"]
    assert len(by_slot) == len(workers)
    for node in workers:
        binding = by_slot[node["executionSlotId"]]
        assert (binding["agentId"], binding["modelId"]) == (node["provider"], node["model"])


def test_the_work_variant_keeps_workflow_bs_slices_gates_review_and_escalation(
    workflow: dict[str, Any], nodes: dict[str, dict[str, Any]]
) -> None:
    personal = json.loads(PERSONAL.read_text())["workflow"]
    shared = {n["id"] for n in personal["nodes"]}
    assert shared <= set(nodes)
    assert set(nodes) - shared == {"grant-refused", *(f"grant-check-{n}" for n in CODEX_NODES)}
    # A copier has no failure edge; a gate's goes to a repair (through its grant check), an escalation or a check.
    failing = {e["from"] for e in workflow["edges"] if e["from"].startswith("copy-") and e["on"] != "success"}
    assert failing == set()
    assert nodes["ticket-review-escalation"]["model"] == "opus"
    opus = [n for n in nodes if "opus" in json.dumps(nodes[n]).lower()]
    assert opus == ["gates-escalation", "ticket-review-escalation"]


# ---- the headless proof: a missing grant refuses, a live grant lets Codex run ----


class WorkMachine(Machine):
    """The grant-check machine, naming the work variant's run."""

    def grant(self, starts: int) -> None:
        self.gateway_call("workflow", "start", "--json", json.dumps({"name": RUN_NAME}))
        self.gateway_call(
            "grant",
            "write",
            "--json",
            json.dumps(
                {
                    "workflow": RUN_NAME,
                    "client": "codex",
                    "cap_usd": 5,
                    "max_starts": starts,
                    "question": "may the proof run spend Codex node starts?",
                    "answer": f"yes, {starts} of them",
                }
            ),
        )

    def check_command(self, node: str, reports: Path) -> list[str]:
        command = super().check_command(node, reports)
        command[command.index("--workflow-name") + 1] = WORKFLOW_NAME
        return command


needs_headless = pytest.mark.skipif(
    NO_TOOLS or GOLD_BAND_BIN is None,
    reason="GOLD_BAND_BIN, node, git or cargo (or GOLD_BAND_RUN_START_BIN) is missing: the headless proof cannot run",
)


@pytest.fixture
def work(tmp_path: Path, wrapper: str) -> WorkMachine:  # noqa: F811
    return WorkMachine(tmp_path, wrapper)


def _stretch(template: dict[str, Any], codex_node: str) -> dict[str, Any]:
    """The real template cut to one Codex node: its grant-check node, the node, the manual check."""
    workflow = template["workflow"]
    keep = {f"grant-check-{codex_node}", codex_node, "grant-refused"}
    # The throw-away machine holds the builtin profile only, so a test critic's review profile is cut to it:
    # this proof is about the grant check before the node, not about the profile the node reads with.
    nodes = [{**n, "profile": "pf-builtin-dev"} for n in workflow["nodes"] if n["id"] in keep]
    for node in nodes:
        if node["id"] == codex_node:
            # The review orchestrator answers with a findings list in the real template; the stub answers ok.
            node["output"] = {"kind": "json", "artifact": f"{codex_node}-stretch", "schema": {"ok": True}}
            node["success_condition"] = {"path": "$.ok", "equals": True}
    edges = [e for e in workflow["edges"] if e["from"] in keep and e["to"] in keep]
    edges.append({"from": codex_node, "to": "$end", "on": "success"})
    slots = {n["executionSlotId"] for n in nodes}
    bindings = [b for b in template["modelBindings"]["bindings"] if b["executionSlotId"] in slots]
    cut = {**workflow, "entry": f"grant-check-{codex_node}", "nodes": nodes, "edges": edges}
    return {"workflow": cut, "modelBindings": {"bindings": bindings}}


def _run(work: WorkMachine, template: dict[str, Any], codex_node: str) -> tuple[int, dict[str, Path], str]:
    cut = _stretch(template, codex_node)
    path = work.tmp / f"{codex_node}.workflow.json"
    path.write_text(json.dumps(cut))
    markers = {name: work.tmp / f"{codex_node}-{name}.marker" for name in ("codex", "manual")}
    check = f"grant-check-{codex_node}"
    reports = work.reports / check
    settings: dict[str, Any] = {"settingsSchemaVersion": SETTINGS_SCHEMA_VERSION, "agents": {}}
    settings["agents"][f"grant-{codex_node}"] = _agent(
        check,
        {
            "STUB_ACP_RUN": json.dumps(work.check_command(codex_node, reports)),
            "STUB_ACP_ANSWER": str(reports / "grant-check.json"),
        },
    )
    settings["agents"]["codex"] = _agent(
        "codex", {"STUB_ACP_MARKER": str(markers["codex"]), "STUB_ACP_MODELS": "gpt-5.5"}
    )
    settings["agents"]["claude-work"] = _agent(
        "claude-work", {"STUB_ACP_MARKER": str(markers["manual"]), "STUB_ACP_MODELS": "sonnet"}
    )
    band_home = work.home / ".gold-band"
    band_home.mkdir(exist_ok=True)
    (band_home / "settings.json").write_text(json.dumps(settings))
    requirement = work.repo / "requirement.md"
    requirement.write_text("# Workflow B, work instance\n\nThe stubs do not read this requirement.\n")
    assert GOLD_BAND_BIN is not None
    done = subprocess.run(
        [GOLD_BAND_BIN, "run", "--requirement-file", str(requirement), "--workflow", str(path)],
        capture_output=True,
        text=True,
        timeout=TIMEOUT_SECONDS,
        env=work.env,
        cwd=str(work.repo),
        check=False,
    )
    said = f"exit {done.returncode}\nstdout:\n{done.stdout[-3000:]}\nstderr:\n{done.stderr[-3000:]}"
    return done.returncode, markers, said


@needs_headless
@pytest.mark.parametrize("codex_node", CODEX_NODES)
def test_a_missing_grant_refuses_before_the_codex_node_and_pauses_the_run(
    work: WorkMachine, template: dict[str, Any], codex_node: str
) -> None:
    code, markers, said = _run(work, template, codex_node)
    assert code == 2, said
    assert not markers["codex"].exists(), "Codex ran without a grant"
    refused = _report(work.reports / f"grant-check-{codex_node}")
    assert refused["exit_code"] == 1 and refused["failed_check"] == "grant-lapsed"
    assert work.reservations() == []


@needs_headless
@pytest.mark.parametrize("codex_node", CODEX_NODES)
def test_a_live_grant_lets_the_codex_node_run_after_reserving_one_start(
    work: WorkMachine, template: dict[str, Any], codex_node: str
) -> None:
    work.grant(3)
    code, markers, said = _run(work, template, codex_node)
    assert code == 0, said
    assert markers["codex"].exists(), "Codex did not run on a live grant"
    assert _report(work.reports / f"grant-check-{codex_node}")["exit_code"] == 0
    reserved = work.reservations()
    assert len(reserved) == 1 and reserved[0]["workflow"] == RUN_NAME


@needs_headless
def test_a_grant_with_no_start_left_refuses_the_next_codex_node(work: WorkMachine, template: dict[str, Any]) -> None:
    work.grant(1)
    assert _run(work, template, "writer-s1")[0] == 0
    code, markers, said = _run(work, template, "repair-1")
    assert code == 2, said
    assert not markers["codex"].exists(), "Codex ran past its last start"
    assert _report(work.reports / "grant-check-repair-1")["failed_check"] == "grant-no-start-left"
