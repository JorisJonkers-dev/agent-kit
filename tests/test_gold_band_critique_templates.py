"""The critique pipeline's eight stage templates as Gold Band workflows (issue #458).

Each template at ``installer/gold-band/workflows/critique-<stage>.json`` is one
stage of the critique pipeline as a Gold Band workflow: a run-start node that
refuses the run when the repository rule or the family allowlist forbids a
bound agent, then the runner's prepare, fan-out, refute and record each as a
gate shim -- a gate node that runs the pinned ``gold-band-gate`` wrapper around
one ``critique <command> --strict``, then the read-only copier that verifies the
report it printed. The orchestrator's verdict node sits between fan-out and
refute, and a finding it cannot settle routes to a manual check.

The runner stays the executor: Gold Band only sequences the stages, so the
critics keep the runner's read-only tripwire, packet confinement, time limits
and family allowlist. No shim node has a failure edge, so a failed gate ends
the run in failure rather than routing somewhere else.

These tests read the templates as files and need no Gold Band binary or Rust
toolchain (the workflow lint runs over them in the gold-band-proof crate's
tests). Their headless proof belongs to the ``gold-band-proof`` crate
(docs/gold-band-proof.md).
"""

from __future__ import annotations

import json
from pathlib import Path
from typing import Any

import pytest
import yaml

KIT_ROOT = Path(__file__).resolve().parent.parent
WORKFLOWS = KIT_ROOT / "installer" / "gold-band" / "workflows"
REGISTRY = KIT_ROOT / "registry" / "estate-tooling.yaml"

#: The eight stages, each with the slash command its description must name.
STAGES = {
    "decisions": "/review-grill",
    "spec": "/review-spec",
    "tickets": "/review-tickets",
    "ticket-review": "/review-implementation",
    "feature-review": "/flow finish",
    "adr-review": "/critique-adrs",
    "decision-recovery": "/recover-decisions",
    "architecture-review": "/review-architecture",
}

#: The runner commands the gate shims wrap, in the order the shims run them.
RUNNER_COMMANDS = ["prepare", "fan-out", "refute", "record"]

#: The families the personal instance may not bind.
FORBIDDEN_FAMILIES = ["codex", "opus"]

#: Gold Band's own profile, which any node may use without the registry declaring it.
BUILTIN_PROFILE = "pf-builtin-dev"

#: The one template in the workflows directory that is not one of the eight: the
#: spec stage's own critic-per-family template, tested by
#: ``tests/test_gold_band_spec_workflow.py``.
OTHER_TEMPLATE = "critique-spec-stage.json"

#: The Wave 3 template named for the stats report (tests/test_gold_band_wave3_templates.py).
WAVE_3_TEMPLATE = "critique-stats.json"


def _node_ids(stage: str) -> list[str]:
    """The stage's nodes in order, from the naming rule every template follows."""
    return [
        f"gate-critique-{stage}-start",
        f"gate-critique-{stage}-prepare",
        f"copy-critique-{stage}-prepare",
        f"gate-critique-{stage}-fan-out",
        f"copy-critique-{stage}-fan-out",
        f"{stage}-verdicts",
        f"{stage}-needs-human",
        f"gate-critique-{stage}-refute",
        f"copy-critique-{stage}-refute",
        f"gate-critique-{stage}-record",
        f"copy-critique-{stage}-record",
    ]


def _shim_ids(stage: str) -> list[str]:
    """The gate and copier nodes: the pinned wrappers around the runner's commands."""
    return [node_id for node_id in _node_ids(stage) if node_id.startswith(("gate-", "copy-"))]


def _chain(ids: list[str]) -> list[tuple[str, str, str]]:
    """Each node handing over to the next on success."""
    return [(ids[index], ids[index + 1], "success") for index in range(len(ids) - 1)]


def _edges(stage: str) -> list[tuple[str, str, str]]:
    """The shim chain, the fork at the verdicts, and the last copier ending the run."""
    ids = _node_ids(stage)
    verdicts = f"{stage}-verdicts"
    needs_human = f"{stage}-needs-human"
    refute = f"gate-critique-{stage}-refute"
    forks = [(verdicts, refute, "success"), (verdicts, needs_human, "failure"), (needs_human, refute, "success")]
    return [
        *_chain(ids[: ids.index(verdicts) + 1]),
        *forks,
        *_chain(ids[ids.index(needs_human) + 1 :]),
        (ids[-1], "$end", "success"),
    ]


def _node(nodes: list[dict[str, Any]], node_id: str) -> dict[str, Any]:
    return next(node for node in nodes if node["id"] == node_id)


# ---------------------------------------------------------------------------
# The templates as files: what no Gold Band binary is needed to see
# ---------------------------------------------------------------------------


@pytest.fixture
def text(stage: str) -> str:
    """The template as written, for what a parsed view would hide."""
    return (WORKFLOWS / f"critique-{stage}.json").read_text()


@pytest.fixture
def template(text: str) -> dict[str, Any]:
    return json.loads(text)


@pytest.fixture
def workflow(template: dict[str, Any]) -> dict[str, Any]:
    """The definition inside the authoring wrapper."""
    return template["workflow"]


@pytest.fixture
def nodes(workflow: dict[str, Any]) -> list[dict[str, Any]]:
    return workflow["nodes"]


@pytest.fixture
def edges(workflow: dict[str, Any]) -> list[dict[str, Any]]:
    return workflow["edges"]


@pytest.fixture
def bindings(template: dict[str, Any]) -> list[dict[str, Any]]:
    return template["modelBindings"]["bindings"]


@pytest.fixture(scope="module")
def registry() -> dict[str, Any]:
    return yaml.safe_load(REGISTRY.read_text())


@pytest.fixture(scope="module")
def declared_profiles(registry: dict[str, Any]) -> set[str]:
    """Every profile id the registry declares: the roles' ids and the instances' agents."""
    roles = {role["id"] for role in registry["gold_band"]["roles"]}
    agents = {agent for instance in registry["gold_band_instances"].values() for agent in instance["agents"]}
    return roles | agents


# ---------------------------------------------------------------------------
# The shape of one stage
# ---------------------------------------------------------------------------


@pytest.mark.parametrize("stage", list(STAGES))
def test_the_workflow_is_the_stage_template(workflow: dict[str, Any], stage: str) -> None:
    """The template is the stage's own, entered at its run-start gate."""
    assert workflow["id"] == f"critique-{stage}"
    assert workflow["entry"] == f"gate-critique-{stage}-start"


@pytest.mark.parametrize("stage", list(STAGES))
def test_the_nodes_are_the_stage_pipeline_in_order(nodes: list[dict[str, Any]], stage: str) -> None:
    """The run-start gate, the prepare and fan-out shims, the verdicts, then refute and record."""
    assert [node["id"] for node in nodes] == _node_ids(stage)


@pytest.mark.parametrize("stage", list(STAGES))
def test_the_edges_chain_the_shims_and_fork_at_the_verdicts(edges: list[dict[str, Any]], stage: str) -> None:
    """The shims chain on success, the verdicts fork on failure, and the last copier ends the run."""
    assert [(edge["from"], edge["to"], edge["on"]) for edge in edges] == _edges(stage)


@pytest.mark.parametrize("stage", list(STAGES))
def test_every_edge_target_exists_or_ends_the_run(
    edges: list[dict[str, Any]],
    nodes: list[dict[str, Any]],
) -> None:
    """No edge points at a node the template does not hold, and only ``$end`` leaves it."""
    ids = {node["id"] for node in nodes}
    for edge in edges:
        assert edge["from"] in ids
        assert edge["to"] == "$end" or edge["to"] in ids


@pytest.mark.parametrize("stage", list(STAGES))
def test_no_shim_node_has_a_failure_edge(edges: list[dict[str, Any]], stage: str) -> None:
    """A failed gate ends the run: only the verdicts fork routes a failure anywhere."""
    shim = set(_shim_ids(stage))
    assert [edge for edge in edges if edge["on"] == "failure" and edge["from"] in shim] == []


@pytest.mark.parametrize("stage", list(STAGES))
def test_every_node_has_a_model_binding(
    nodes: list[dict[str, Any]],
    bindings: list[dict[str, Any]],
) -> None:
    """Each node's execution slot is bound to the agent and model it names, and no slot repeats."""
    slots = [binding["executionSlotId"] for binding in bindings]
    assert len(set(slots)) == len(slots) == len(nodes)
    by_slot = {binding["executionSlotId"]: binding for binding in bindings}
    for node in nodes:
        binding = by_slot[node["executionSlotId"]]
        assert binding["agentId"] == node["provider"]
        assert binding["modelId"] == node["model"]


@pytest.mark.parametrize("stage", list(STAGES))
def test_no_template_mentions_codex_or_opus(text: str) -> None:
    """Neither forbidden family appears anywhere in the template."""
    for family in FORBIDDEN_FAMILIES:
        assert family not in text.lower()


@pytest.mark.parametrize("stage", list(STAGES))
@pytest.mark.parametrize("command", RUNNER_COMMANDS)
def test_each_gate_names_its_runner_command_through_strict(
    nodes: list[dict[str, Any]],
    stage: str,
    command: str,
) -> None:
    """The gate for a command runs that command, and ``--strict`` turns a refusal into a failure."""
    gate = _node(nodes, f"gate-critique-{stage}-{command}")
    assert f"`critique {command} --strict`" in gate["goal"]


@pytest.mark.parametrize("stage", list(STAGES))
def test_the_start_gate_names_the_run_start_wrapper(nodes: list[dict[str, Any]], stage: str) -> None:
    """The run-start gate runs the pinned wrapper that checks the rule and the allowlist."""
    assert "gold-band-run-start" in _node(nodes, f"gate-critique-{stage}-start")["goal"]


@pytest.mark.parametrize("stage", list(STAGES))
def test_the_description_keeps_the_runner_the_executor_and_names_the_slash_command(
    workflow: dict[str, Any],
    stage: str,
) -> None:
    """The template says the runner still executes the stage, and names the command it is."""
    description = workflow["description"]
    assert "The runner stays the executor" in description
    assert STAGES[stage] in description


@pytest.mark.parametrize("stage", list(STAGES))
def test_every_output_artifact_has_no_json_suffix(nodes: list[dict[str, Any]]) -> None:
    """Each node but the manual check answers with a json artifact, and Gold Band adds the extension."""
    artifacts = [node["output"]["artifact"] for node in nodes if isinstance(node.get("output"), dict)]
    # The manual check answers with prose, so it is the one node without an output.
    assert len(artifacts) == len(nodes) - 1
    for artifact in artifacts:
        # Gold Band appends ".json" to a json artifact's name itself.
        assert not artifact.endswith(".json")


@pytest.mark.parametrize("stage", list(STAGES))
def test_every_non_shim_node_uses_a_declared_profile(
    nodes: list[dict[str, Any]],
    stage: str,
    declared_profiles: set[str],
) -> None:
    """The verdicts and the manual check run as a profile the registry declares, or Gold Band's own."""
    shim = set(_shim_ids(stage))
    for node in nodes:
        if node["id"] in shim:
            continue
        assert node["profile"] == BUILTIN_PROFILE or node["profile"] in declared_profiles


# ---------------------------------------------------------------------------
# The family as a whole
# ---------------------------------------------------------------------------


def test_the_eight_templates_are_the_whole_critique_family() -> None:
    """The directory holds these eight, plus the two named templates the glob leaves out."""
    others = {OTHER_TEMPLATE, WAVE_3_TEMPLATE}
    found = sorted(
        path.stem.removeprefix("critique-") for path in WORKFLOWS.glob("critique-*.json") if path.name not in others
    )
    assert found == sorted(STAGES)
