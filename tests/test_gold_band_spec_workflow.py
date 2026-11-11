"""The critique spec stage as a Gold Band workflow template, proved headless (issue #348).

The template at ``installer/gold-band/workflows/critique-spec-stage.json`` is
the Gold Band shape of the critique pipeline's spec stage: one worker per
critic of the stage's base roster (``skills/workstation/critique/routing.json``,
``stages.spec.base`` — agent ``vibe`` on the Mistral family, agent ``agy`` on
the Google family, both as the spec red team with their roster lenses), a merge
node whose artifact is the findings document against the finding schema, and a
manual-check node a needs-human verdict routes to.

The binary tests run the template headless through ``gold-band run`` against
the stub ACP agent ``tests/fixtures/gold_band_stub_acp.py``: a tmp home whose
``settings.json`` declares agents ``vibe`` and ``agy`` with the stub as their
command, so no model and no network is spent. A clean stub run ends with exit
0; ``STUB_ACP_VERDICT=needs-human`` makes the stub's findings carry a
needs-human case, the merge node's success condition fails, and the run
pauses at the manual-check node with exit 2. Every findings artifact the run
wrote must validate against the finding schema. Without ``GOLD_BAND_BIN``
the binary tests skip with the reason naming the build doc; the template
tests need no binary at all.
"""

from __future__ import annotations

import json
import os
import shutil
import subprocess
import sys
from pathlib import Path
from typing import Any

import pytest

KIT_ROOT = Path(__file__).resolve().parent.parent
TEMPLATE = KIT_ROOT / "installer" / "gold-band" / "workflows" / "critique-spec-stage.json"
STUB = KIT_ROOT / "tests" / "fixtures" / "gold_band_stub_acp.py"
FINDING_SCHEMA = KIT_ROOT / "skills" / "workstation" / "critique" / "schemas" / "finding.schema.json"
ROUTING = KIT_ROOT / "skills" / "workstation" / "critique" / "routing.json"
LINT_MANIFEST = KIT_ROOT / "scripts" / "Cargo.toml"

#: The exact skip reason the binary tests carry (docs/gold-band-headless-build.md).
SKIP_NO_GOLD_BAND = (
    "GOLD_BAND_BIN not set: point it at a headless gold-band binary (docs/gold-band-headless-build.md)"
)
GOLD_BAND_BIN = os.environ.get("GOLD_BAND_BIN")
# The proof renders the six role profiles with the gold-band-roles crate, so
# it needs cargo too; without it the binary tests skip instead of erroring.
SKIP_NO_CARGO = "cargo not on PATH: the proof renders the role profiles with the gold-band-roles crate"
NO_HEADLESS_PROOF = GOLD_BAND_BIN is None or shutil.which("cargo") is None
HEADLESS_SKIP_REASON = SKIP_NO_GOLD_BAND if GOLD_BAND_BIN is None else SKIP_NO_CARGO
GOLD_BAND_TIMEOUT_SECONDS = 600

#: The critic client of the stage's roster (``stages.spec.base``) names the
#: Gold Band agent that runs it (registry/estate-tooling.yaml, ``gold_band.agents``):
#: vibe-mistral-large runs as ``vibe``, agy-gemini-flash-high as ``agy``.
ROSTER_AGENT = {"vibe-mistral-large": "vibe", "agy-gemini-flash-high": "agy"}

REQUIREMENT = """# Spec-stage headless proof

This requirement document is the input the stub critics read. The proof checks
the workflow's structure and routing, not the critique itself, so the
requirement needs no content of its own.

## Acceptance

The workflow runs; the critics answer; the merged findings decide.
"""


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


def test_the_template_passes_workflow_lint() -> None:
    if shutil.which("cargo") is None:
        pytest.skip("Rust toolchain (cargo) is absent: workflow-lint cannot be built and run")
    lint = subprocess.run(
        [
            "cargo", "run", "-q", "--manifest-path", str(LINT_MANIFEST), "-p", "workflow-lint", "--",
            # The template binds Vibe and Antigravity, which are personal-instance agents.
            "--instance", "gold-band-personal", str(TEMPLATE),
        ],
        capture_output=True,
        text=True,
        timeout=GOLD_BAND_TIMEOUT_SECONDS,
    )
    assert lint.returncode == 0, lint.stdout + lint.stderr
    assert "workflow-lint: ok" in lint.stdout


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


# ---------------------------------------------------------------------------
# The headless proof: the real binary, the stub agent
# ---------------------------------------------------------------------------


# Gold Band v0.19.1 (987fc3e): CURRENT_SETTINGS_SCHEMA_VERSION in src/config/mod.rs.
GOLD_BAND_SETTINGS_SCHEMA_VERSION = 12


def _bound_models(agent_id: str) -> str:
    """The models the template binds to an agent, which the stub advertises."""
    template = json.loads(TEMPLATE.read_text())
    return ",".join(sorted({b["modelId"] for b in template["modelBindings"]["bindings"]
                            if b["agentId"] == agent_id and b.get("modelId")}))


def _agent(display_name: str, verdict: str, models: str) -> dict[str, Any]:
    """One settings.json agent: the stub ACP agent as its adapter command."""
    return {
        "adapter": {
            "command": sys.executable,
            "args": [str(STUB)],
            "displayName": display_name,
            "env": {"STUB_ACP_VERDICT": verdict, "STUB_ACP_MODELS": models},
        }
    }


def _gold_band_run(home: Path, workspace: Path, verdict: str) -> subprocess.CompletedProcess[str]:
    """One headless run of the template against a fresh home with the stub agents."""
    band_home = home / ".gold-band"
    band_home.mkdir(parents=True)
    # Without the current schema version Gold Band runs its version-0
    # migration, which rejects custom agent ids, and then silently loads no
    # agents at all (gold-band-src src/config/mod.rs, migrate_managed_agent_directories).
    settings = {
        "settingsSchemaVersion": GOLD_BAND_SETTINGS_SCHEMA_VERSION,
        "agents": {
            "vibe": _agent("Stub ACP (Vibe)", verdict, _bound_models("vibe")),
            "agy": _agent("Stub ACP (Agy)", verdict, _bound_models("agy")),
        },
    }
    (band_home / "settings.json").write_text(json.dumps(settings))
    # The template's nodes name the spec red-team profile; Gold Band refuses a
    # node whose profile is missing, so render the six roles (#345) first.
    # Gold Band reads user profiles from context/profiles (gold-band-src
    # src/app/profiles.rs, profile_dir -> user_context_profiles_dir).
    profiles = band_home / "context" / "profiles"
    profiles.mkdir(parents=True)
    subprocess.run(
        ["cargo", "run", "-q", "--release", "--manifest-path", str(KIT_ROOT / "scripts" / "Cargo.toml"),
         "-p", "gold-band-roles", "--", "render",
         str(KIT_ROOT / "skills" / "workstation" / "critique"), str(profiles)],
        check=True, capture_output=True, text=True, timeout=GOLD_BAND_TIMEOUT_SECONDS,
    )
    workspace.mkdir()
    requirement = workspace / "requirement.md"
    requirement.write_text(REQUIREMENT)
    assert GOLD_BAND_BIN is not None  # narrowed by the tests' skip marker
    env = dict(os.environ, GOLD_BAND_HOME=str(home), HOME=str(home), STUB_ACP_VERDICT=verdict)
    return subprocess.run(
        [GOLD_BAND_BIN, "run", "--requirement-file", str(requirement), "--workflow", str(TEMPLATE)],
        capture_output=True,
        text=True,
        timeout=GOLD_BAND_TIMEOUT_SECONDS,
        env=env,
        cwd=str(workspace),
        check=False,
    )


def _prompt_sent_to(home: Path, node_id: str) -> str:
    """The session/prompt frame Gold Band sent to one node's agent, as text."""
    for raw in (home / ".gold-band").rglob("acp.raw.jsonl"):
        if f"/nodes/{node_id}/" not in raw.as_posix():
            continue
        for line in raw.read_text().splitlines():
            if '"session/prompt"' in line and '"prompt"' in line:
                return line
    raise AssertionError(f"no session/prompt recorded for node {node_id}")


def _report(run: subprocess.CompletedProcess[str]) -> str:
    """The run's tail, so a failed proof names what Gold Band said."""
    return f"exit {run.returncode}\nstdout:\n{run.stdout[-4000:]}\nstderr:\n{run.stderr[-4000:]}"


def _findings_artifacts(home: Path, names: set[str]) -> list[Path]:
    """Every findings artifact the run wrote, by the template's artifact names."""
    # Run artifacts land in the workspace's own .gold-band (the repo root),
    # beside the home's; search both.
    roots = [home / ".gold-band", home.parent / "workspace" / ".gold-band"]
    return sorted(path for root in roots if root.exists()
                  for path in root.rglob("*") if path.is_file() and path.name in names)


def _artifact_names(workflow: dict[str, Any]) -> set[str]:
    """The file names Gold Band writes: the artifact name plus ".json"."""
    return {node["output"]["artifact"] + ".json" for node in workflow["nodes"] if isinstance(node.get("output"), dict)}


def _assert_valid_findings(document: Any, schema: dict[str, Any]) -> None:
    """The findings document validates against the schema; without the
    jsonschema package, the schema's own required keys and enums stand in."""
    try:
        import jsonschema
    except ImportError:
        _assert_required_keys(document, schema)
        return
    jsonschema.validate(document, schema)


def _assert_required_keys(document: Any, schema: dict[str, Any]) -> None:
    """The fallback: the schema's required keys, read from the schema itself."""
    assert isinstance(document, dict)
    for key in schema["required"]:
        assert key in document
    findings = document.get("findings")
    assert isinstance(findings, list)
    finding = schema["$defs"]["finding"]
    for item in findings:
        assert isinstance(item, dict)
        for key in finding["required"]:
            assert key in item
        assert item["severity"] in finding["properties"]["severity"]["enum"]
        assert item["category"] in finding["properties"]["category"]["enum"]
        for entry in item["evidence"]:
            assert isinstance(entry, dict)
            for key in schema["$defs"]["evidence"]["required"]:
                assert key in entry


def _assert_the_runs_findings_validate(home: Path, workflow: dict[str, Any], at_least: set[str]) -> list[Path]:
    schema = json.loads(FINDING_SCHEMA.read_text())
    artifacts = _findings_artifacts(home, _artifact_names(workflow))
    assert {path.name for path in artifacts} >= at_least
    for path in artifacts:
        _assert_valid_findings(json.loads(path.read_text()), schema)
    return artifacts


@pytest.mark.skipif(NO_HEADLESS_PROOF, reason=HEADLESS_SKIP_REASON)
def test_a_clean_stub_run_ends_with_exit_zero(tmp_path: Path, workflow: dict[str, Any]) -> None:
    home = tmp_path / "home"
    run = _gold_band_run(home, tmp_path / "workspace", "")
    assert run.returncode == 0, _report(run)
    _assert_the_runs_findings_validate(
        home,
        workflow,
        {"spec-findings-vibe.json", "spec-findings-agy.json", "spec-findings.json"},
    )
    # The merge node runs after both critics, so Gold Band hands it both
    # critics' artifacts as predecessor context, not only the last one's.
    merge_prompt = _prompt_sent_to(home, "spec-merge-findings")
    assert "spec-findings-vibe" in merge_prompt
    assert "spec-findings-agy" in merge_prompt


@pytest.mark.skipif(NO_HEADLESS_PROOF, reason=HEADLESS_SKIP_REASON)
def test_a_needs_human_stub_run_pauses_with_exit_two(tmp_path: Path, workflow: dict[str, Any]) -> None:
    home = tmp_path / "home"
    run = _gold_band_run(home, tmp_path / "workspace", "needs-human")
    assert run.returncode == 2, _report(run)
    found = _assert_the_runs_findings_validate(
        home, workflow, {"spec-findings-vibe.json", "spec-findings-agy.json"}
    )
    # The stub raised a needs-human case, so the merged findings are not empty.
    if "spec-findings.json" in {path.name for path in found}:
        merged = json.loads((next(p for p in found if p.name == "spec-findings.json")).read_text())
        assert merged["findings"], "the needs-human verdict reached $end with no finding in the merge"


def test_without_cargo_the_headless_proof_skips_rather_than_errors() -> None:
    """A machine with the binary but no Rust toolchain skips the proof with a
    reason (review of #348): the skip condition covers cargo, and its reason
    names it."""
    if GOLD_BAND_BIN is not None and shutil.which("cargo") is None:
        assert NO_HEADLESS_PROOF and HEADLESS_SKIP_REASON == SKIP_NO_CARGO
    assert "cargo" in SKIP_NO_CARGO
    assert (GOLD_BAND_BIN is None or shutil.which("cargo") is None) == NO_HEADLESS_PROOF
