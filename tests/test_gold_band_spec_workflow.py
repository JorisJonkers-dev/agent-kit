"""The critique spec stage as a Gold Band workflow template (issues #348, #447).

The template at ``installer/gold-band/workflows/critique-spec-stage.json`` is
the Gold Band shape of the critique pipeline's spec stage: one worker per
critic of the stage's base roster (``skills/workstation/critique/routing.json``,
``stages.spec.base`` -- agent ``vibe`` on the Mistral family, agent ``agy`` on
the Google family, both as the spec red team with their roster lenses), a merge
node whose artifact is the findings document against the finding schema, and a
manual-check node a needs-human verdict routes to.

These tests read the template as a file and need no Gold Band binary or
Rust toolchain (the workflow lint runs over it in the gold-band-proof crate's
tests). Its
headless proof, the run against a stub ACP agent, belongs to the
``gold-band-proof`` crate (docs/gold-band-proof.md).
"""

from __future__ import annotations

import json
from pathlib import Path
from typing import Any

import pytest

KIT_ROOT = Path(__file__).resolve().parent.parent
TEMPLATE = KIT_ROOT / "installer" / "gold-band" / "workflows" / "critique-spec-stage.json"
FINDING_SCHEMA = KIT_ROOT / "skills" / "workstation" / "critique" / "schemas" / "finding.schema.json"
ROUTING = KIT_ROOT / "skills" / "workstation" / "critique" / "routing.json"

#: The critic client of the stage's roster (``stages.spec.base``) names the
#: Gold Band agent that runs it (registry/estate-tooling.yaml, ``gold_band.agents``):
#: vibe-mistral-large runs as ``vibe``, agy-gemini-flash-high as ``agy``.
ROSTER_AGENT = {"vibe-mistral-large": "vibe", "agy-gemini-flash-high": "agy"}
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


def _critics(nodes: list[dict[str, Any]]) -> list[dict[str, Any]]:
    """The worker nodes that run a critic: not the merge node, not the manual check."""
    return [
        node
        for node in nodes
        if node.get("type") == "worker"
        and not node.get("manual_check")
        and node.get("id") != "spec-merge-findings"
    ]


def test_the_template_is_for_personal_repositories_only(workflow: dict[str, Any]) -> None:
    """The template's own description states its restriction."""
    assert "personal repositories only" in workflow.get("description", "").lower()


def test_the_template_names_only_vibe_and_agy_and_never_codex(template: dict[str, Any]) -> None:
    """Every agent binding is one of the two roster families' agents, and codex appears nowhere."""
    assert "codex" not in TEMPLATE.read_text().lower()
    providers = {node.get("provider") for node in template["workflow"]["nodes"]}
    agent_ids = {binding.get("agentId") for binding in template["modelBindings"]["bindings"]}
    assert providers == {"vibe", "agy"}
    assert agent_ids == {"vibe", "agy"}


def test_one_critic_worker_per_roster_family_with_its_lenses(nodes: list[dict[str, Any]]) -> None:
    """Two critic workers, one per family, each the spec red team with its roster lenses."""
    roster = json.loads(ROUTING.read_text())["stages"]["spec"]["base"]
    critics = _critics(nodes)
    assert len(critics) == len(roster) == 2
    by_provider = {node["provider"]: node for node in critics}
    assert set(by_provider) == set(ROSTER_AGENT.values())
    for critic in roster:
        node = by_provider[ROSTER_AGENT[critic["client"]]]
        assert node["profile"] == "pf-spec-redteam"
        for lens in critic["lenses"]:
            assert lens in node["goal"]


def test_every_output_contract_is_the_finding_schema_shape(nodes: list[dict[str, Any]]) -> None:
    """Each critic (and the merge node) is held to the finding schema."""
    schema = json.loads(FINDING_SCHEMA.read_text())
    contracted = [node for node in nodes if isinstance(node.get("output"), dict)]
    assert len(contracted) == 3
    for node in contracted:
        output = node["output"]
        assert output["kind"] == "json"
        # Gold Band appends ".json" to a json artifact's name itself.
        assert not output["artifact"].endswith(".json")
        # Gold Band refuses JSON Schema in a worker output and takes its own
        # simplified shape instead (gold-band-src src/dsl/mod.rs,
        # looks_like_json_schema), so the contract is the finding schema's
        # required fields as that shape; the full schema is checked on the
        # artifacts the run writes.
        shape = output["schema"]
        finding = schema["$defs"]["finding"]
        assert list(shape) == schema["required"]
        assert set(shape["findings"][0]) == set(finding["required"])
        assert set(shape["findings"][0]["evidence"][0]) == set(schema["$defs"]["evidence"]["required"])


def test_a_needs_human_verdict_routes_to_a_manual_check(
    workflow: dict[str, Any],
    nodes: list[dict[str, Any]],
) -> None:
    """The merge node decides, and its failure edge is a manual-check node that pauses."""
    merge = next(node for node in nodes if node["id"] == "spec-merge-findings")
    assert merge["success_condition"] == {"path": "$.findings", "equals": []}
    incoming = [
        edge for edge in workflow["edges"] if edge["to"] == "spec-needs-human" and edge["on"] == "failure"
    ]
    assert [edge["from"] for edge in incoming] == ["spec-merge-findings"]
    manual = next(node for node in nodes if node["id"] == "spec-needs-human")
    assert manual.get("manual_check") is True
    assert not isinstance(manual.get("output"), dict)
