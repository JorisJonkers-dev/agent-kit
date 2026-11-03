"""SC-4 gate-summary artifact download and parsing helper."""

import io
import json
import logging
import zipfile
from typing import Any

from .github_api import github_api_get

GATE_ARTIFACT_PREFIX = "gate-summary-"
logger = logging.getLogger(__name__)


def download_gate_summary(
    artifact_name: str, run_id: int, repo: str, github_token: str
) -> dict[str, Any] | None:
    """
    Download gate-summary.json from a named artifact on a specific run.
    Returns parsed SC-4 dict or None on any failure (graceful — lens C7-agents).
    Never raises; logs all failures.
    """
    try:
        artifact_id = _find_artifact_id(artifact_name, run_id, repo, github_token)
        if artifact_id is None:
            logger.warning(f"gate summary artifact '{artifact_name}' not found on run {run_id}")
            return None

        zip_bytes = _download_artifact_zip(artifact_id, repo, github_token)
        json_bytes = _extract_file_from_zip(zip_bytes, "gate-summary.json")
        summary: dict[str, Any] = json.loads(json_bytes)

        # Basic SC-4 shape validation
        required_fields = {
            "gate",
            "check_name",
            "status",
            "reason",
            "flaky_candidates",
            "actor_decision",
            "redacted",
        }
        missing = required_fields - set(summary.keys())
        if missing:
            logger.warning(f"gate summary missing fields {missing}; treating as partial")
            # Fill defaults so callers can reason on status
            for f in missing:
                summary.setdefault(f, None)

        return summary

    except Exception as e:
        logger.warning(
            f"could not retrieve gate summary '{artifact_name}' on run {run_id}: {e}"
        )
        return None


def _find_artifact_id(name: str, run_id: int, repo: str, token: str) -> int | None:
    """
    List artifacts for run_id; return the id of the LATEST-head artifact named `name`.
    Edge case: force-pushed stale artifacts — multiple artifacts with same name on same run.
    Pick the one with the most recent created_at (latest-head-SHA policy — lens C7-agents).
    """
    result = github_api_get(
        f"/repos/{repo}/actions/runs/{run_id}/artifacts", token, params={"per_page": 100}
    )
    artifacts = result.get("artifacts", [])
    if not isinstance(artifacts, list):
        return None

    matching = [a for a in artifacts if a["name"] == name]
    if not matching:
        return None

    # Sort by created_at descending; latest wins
    matching.sort(key=lambda a: a["created_at"], reverse=True)
    return int(matching[0]["id"])


def _download_artifact_zip(artifact_id: int, repo: str, token: str) -> bytes:
    """Download the artifact ZIP from GitHub."""
    import requests

    url = f"https://api.github.com/repos/{repo}/actions/artifacts/{artifact_id}/zip"
    resp = requests.get(
        url, headers={"Authorization": f"Bearer {token}"}, allow_redirects=True
    )
    resp.raise_for_status()
    return resp.content


def _extract_file_from_zip(zip_bytes: bytes, filename: str) -> bytes:
    """Extract a single file from a ZIP archive."""
    with zipfile.ZipFile(io.BytesIO(zip_bytes)) as zf:
        return zf.read(filename)
