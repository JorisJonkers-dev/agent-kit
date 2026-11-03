"""Main deployment status tool; maps required checks to SC-4 gate summaries."""

import logging
import os
import re
from typing import Any

from .gate_summary import download_gate_summary
from .github_api import github_api_get

logger = logging.getLogger(__name__)

REQUIRED_CHECK_NAMES = [
    "Compose Gate",
    "Leak Scan",
    "Stack Integration Gate",
    "Pipeline Complete",
]

# Map check_name → gate-summary artifact name (SC-4 gate field)
CHECK_TO_ARTIFACT: dict[str, str] = {
    "Compose Gate": "gate-summary-compose-gate",
    "Leak Scan": "gate-summary-leak-scan",
    "Stack Integration Gate": "gate-summary-stack-integration-gate",
    "Pipeline Complete": "gate-summary-pipeline-complete",
}


def require_env(var_name: str) -> str:
    """Get an environment variable or raise ValueError if not set."""
    value = os.environ.get(var_name)
    if value is None:
        raise ValueError(f"Required environment variable {var_name} not set")
    return value


def deploy_status(pr: int, repo: str = "homelab-deploy") -> dict[str, Any]:
    """
    Returns:
        {
          "gates": [
            {
              "name":             "Compose Gate",
              "status":           "pass" | "fail" | "pending" | "missing",
              "reason":           str | None,
              "summary_artifact": "gate-summary-compose-gate" | None
            },
            ...
          ],
          "merge_ready": bool,
          "blocker":     str | None    # name of first non-pass gate, or None
        }

    Pipeline Complete is authoritative for merge_ready (lens C7-agents).
    """
    token = require_env("GITHUB_TOKEN")

    # (1) Resolve latest head SHA for this PR (merge-queue vs PR-head check drift edge case)
    pr_data = github_api_get(f"/repos/{repo}/pulls/{pr}", token)
    head_sha = pr_data["head"]["sha"]

    # (2) Fetch check runs for head SHA
    check_runs = _list_check_runs(repo, head_sha, token)

    # (3) Find the most recent run_id associated with Pipeline Complete
    #     (latest-head-SHA policy: if force-pushed, multiple runs may exist)
    pipeline_run_id = _resolve_pipeline_run_id(check_runs, head_sha, repo, token)

    # (4) Build gate statuses
    gates: list[dict[str, Any]] = []
    for check_name in REQUIRED_CHECK_NAMES:
        gate_result = _resolve_gate(
            check_name=check_name,
            check_runs=check_runs,
            pipeline_run_id=pipeline_run_id,
            repo=repo,
            token=token,
        )
        gates.append(gate_result)

    # (5) merge_ready = Pipeline Complete is "pass" AND all required gates are "pass"
    pipeline_gate = next((g for g in gates if g["name"] == "Pipeline Complete"), None)
    all_pass = all(g["status"] == "pass" for g in gates)
    merge_ready = (
        pipeline_gate is not None and pipeline_gate["status"] == "pass" and all_pass
    )

    # (6) blocker = first non-pass gate (in required order)
    blocker = next((g["name"] for g in gates if g["status"] != "pass"), None)

    return {"gates": gates, "merge_ready": merge_ready, "blocker": blocker}


def _resolve_gate(
    check_name: str,
    check_runs: list[dict[str, Any]],
    pipeline_run_id: int | None,
    repo: str,
    token: str,
) -> dict[str, Any]:
    """
    Determine a single gate's status.
    Priority: SC-4 gate summary reason > raw check-run conclusion.
    Gracefully tolerates missing artifact (log + continue).
    """
    artifact_name = CHECK_TO_ARTIFACT.get(check_name)

    # Find corresponding check run
    cr = next((r for r in check_runs if r["name"] == check_name), None)
    if cr is None:
        return {
            "name": check_name,
            "status": "missing",
            "reason": "check-run-not-found",
            "summary_artifact": artifact_name,
        }

    # Map GitHub check conclusion to gate status
    conclusion = cr.get("conclusion")  # success|failure|cancelled|skipped|None(in_progress)
    if conclusion is None:
        raw_status = "pending"
    elif conclusion == "success":
        raw_status = "pass"
    else:
        raw_status = "fail"

    # Try to enrich with SC-4 reason from artifact
    reason = None
    if pipeline_run_id is not None and artifact_name is not None:
        summary = download_gate_summary(artifact_name, pipeline_run_id, repo, token)
        if summary:
            reason = summary.get("reason")
            # Prefer SC-4 status if artifact present (more authoritative than conclusion)
            sc4_status = summary.get("status")
            if sc4_status in ("pass", "fail"):
                raw_status = sc4_status

    return {
        "name": check_name,
        "status": raw_status,
        "reason": reason,
        "summary_artifact": artifact_name,
    }


def _list_check_runs(repo: str, sha: str, token: str) -> list[dict[str, Any]]:
    """
    List all check runs for a commit SHA.
    Edge case: merge-queue vs PR-head check drift — use PR head SHA not merge_group SHA.
    """
    result = github_api_get(
        f"/repos/{repo}/commits/{sha}/check-runs", token, params={"per_page": 100}
    )
    check_runs: list[dict[str, Any]] = result.get("check_runs", [])
    return check_runs


def _resolve_pipeline_run_id(
    check_runs: list[dict[str, Any]], head_sha: str, repo: str, token: str
) -> int | None:
    """
    Find the workflow run_id associated with the Pipeline Complete check run on this SHA.
    Returns None if not found (artifacts won't be downloadable; graceful fallback).
    Edge case: tag re-pushed with new digest — check run's details_url contains run_id.
    """
    cr = next((r for r in check_runs if r["name"] == "Pipeline Complete"), None)
    if cr is None:
        return None

    # GitHub check run details_url: https://github.com/<org>/<repo>/actions/runs/<run_id>/jobs/<job_id>
    details_url = cr.get("details_url", "")
    match = re.search(r"/actions/runs/(\d+)/", details_url)
    if match:
        return int(match.group(1))

    return None
