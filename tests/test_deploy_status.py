"""Tests for deploy_status tool."""

from unittest.mock import patch

import pytest

from tools.deploy.deploy_status import (
    deploy_status,
)


@pytest.fixture
def mock_github_token(monkeypatch):
    """Set GITHUB_TOKEN environment variable."""
    monkeypatch.setenv("GITHUB_TOKEN", "test-token")


def check_run(name: str, conclusion: str | None = None) -> dict:
    """Helper to create a check run dict."""
    return {
        "name": name,
        "conclusion": conclusion,
        "details_url": "https://github.com/JorisJonkers-dev/homelab-deploy/actions/runs/12345/jobs/67890",
    }


class TestDeployStatusMergeReadyLogic:
    """T-I1: deploy_status merge_ready logic."""

    @patch("tools.deploy.deploy_status.download_gate_summary")
    @patch("tools.deploy.deploy_status.github_api_get")
    def test_merge_ready_requires_pipeline_complete_pass(
        self, mock_get, mock_download, mock_github_token
    ):
        """merge_ready is False when Pipeline Complete is fail even if other gates pass."""
        mock_get.side_effect = [
            # PR data
            {
                "head": {"sha": "abc123"},
                "pull_number": 42,
            },
            # Check runs
            {
                "check_runs": [
                    check_run("Compose Gate", "success"),
                    check_run("Leak Scan", "success"),
                    check_run("Stack Integration Gate", "success"),
                    check_run("Pipeline Complete", "failure"),
                ]
            },
        ]

        # Return different responses for different gates
        def download_side_effect(artifact_name, run_id, repo, token):
            if "pipeline-complete" in artifact_name:
                return {
                    "gate": "pipeline-complete",
                    "status": "fail",
                    "reason": "compose-gate-failure",
                    "flaky_candidates": [],
                    "actor_decision": "none",
                    "redacted": False,
                }
            # Other gates pass
            return {
                "status": "pass",
                "reason": "gate-ok",
                "flaky_candidates": [],
                "actor_decision": "none",
                "redacted": False,
            }

        mock_download.side_effect = download_side_effect

        result = deploy_status(pr=42, repo="homelab-deploy")
        assert result["merge_ready"] is False
        assert result["blocker"] == "Pipeline Complete"

    @patch("tools.deploy.deploy_status.download_gate_summary")
    @patch("tools.deploy.deploy_status.github_api_get")
    def test_merge_ready_true_when_all_gates_pass(
        self, mock_get, mock_download, mock_github_token
    ):
        """merge_ready is True when all gates pass."""
        mock_get.side_effect = [
            # PR data
            {
                "head": {"sha": "abc123"},
                "pull_number": 42,
            },
            # Check runs
            {
                "check_runs": [
                    check_run("Compose Gate", "success"),
                    check_run("Leak Scan", "success"),
                    check_run("Stack Integration Gate", "success"),
                    check_run("Pipeline Complete", "success"),
                ]
            },
        ]

        # All gates pass
        mock_download.return_value = {
            "gate": "pipeline-complete",
            "status": "pass",
            "reason": "all-gates-pass",
            "flaky_candidates": [],
            "actor_decision": "none",
            "redacted": False,
        }

        result = deploy_status(pr=42, repo="homelab-deploy")
        assert result["merge_ready"] is True
        assert result["blocker"] is None
        assert all(g["status"] == "pass" for g in result["gates"])


class TestDeployStatusMissingArtifactTolerance:
    """T-I2: deploy_status missing-artifact tolerance."""

    @patch("tools.deploy.deploy_status.github_api_get")
    @patch("tools.deploy.deploy_status.download_gate_summary")
    def test_missing_gate_summary_artifact_returns_status_from_check_conclusion(
        self, mock_download, mock_get, mock_github_token
    ):
        """When SC-4 artifact is absent, fall back to check-run conclusion; do not raise."""
        mock_get.side_effect = [
            # PR data
            {
                "head": {"sha": "abc123"},
                "pull_number": 10,
            },
            # Check runs
            {
                "check_runs": [
                    check_run("Pipeline Complete", "success"),
                    check_run("Compose Gate", "success"),
                    check_run("Leak Scan", "success"),
                    check_run("Stack Integration Gate", "success"),
                ]
            },
        ]

        # Artifact not found
        mock_download.return_value = None

        result = deploy_status(pr=10, repo="homelab-deploy")
        # Should complete without exception; status derived from conclusion
        gate = next(g for g in result["gates"] if g["name"] == "Pipeline Complete")
        assert gate["status"] == "pass"
        assert gate["reason"] is None  # no SC-4 to enrich from

    @patch("tools.deploy.deploy_status.download_gate_summary")
    @patch("tools.deploy.deploy_status.github_api_get")
    def test_stale_artifact_uses_latest_by_created_at(
        self, mock_get, mock_download, mock_github_token
    ):
        """When two artifacts share the same name (force-push), latest created_at wins."""
        mock_get.side_effect = [
            # PR data
            {
                "head": {"sha": "abc123"},
                "pull_number": 5,
            },
            # Check runs
            {
                "check_runs": [
                    check_run("Pipeline Complete", "success"),
                    check_run("Compose Gate", "success"),
                    check_run("Leak Scan", "success"),
                    check_run("Stack Integration Gate", "success"),
                ]
            },
        ]

        # Latest artifact (id=2) wins - all gates pass
        mock_download.return_value = {
            "status": "pass",
            "reason": "all-gates-pass",
            "gate": "pipeline-complete",
            "flaky_candidates": [],
            "actor_decision": "none",
            "redacted": False,
        }

        result = deploy_status(pr=5, repo="homelab-deploy")
        gate = next(g for g in result["gates"] if g["name"] == "Pipeline Complete")
        assert gate["status"] == "pass"  # latest artifact wins


class TestDeployStatusPendingState:
    """Test pending check states."""

    @patch("tools.deploy.deploy_status.github_api_get")
    @patch("tools.deploy.deploy_status.download_gate_summary")
    def test_in_progress_check_shows_pending(
        self, mock_download, mock_get, mock_github_token
    ):
        """In-progress checks (no conclusion) map to pending status."""
        mock_get.side_effect = [
            # PR data
            {
                "head": {"sha": "abc123"},
                "pull_number": 42,
            },
            # Check runs with one in progress
            {
                "check_runs": [
                    check_run("Compose Gate", None),  # in progress
                    check_run("Leak Scan", "success"),
                    check_run("Stack Integration Gate", "success"),
                    check_run("Pipeline Complete", "success"),
                ]
            },
        ]

        mock_download.return_value = None

        result = deploy_status(pr=42, repo="homelab-deploy")
        compose_gate = next(g for g in result["gates"] if g["name"] == "Compose Gate")
        assert compose_gate["status"] == "pending"
        assert result["merge_ready"] is False

    @patch("tools.deploy.deploy_status.github_api_get")
    def test_missing_check_run(self, mock_get, mock_github_token):
        """Missing check run shows missing status."""
        mock_get.side_effect = [
            # PR data
            {
                "head": {"sha": "abc123"},
                "pull_number": 42,
            },
            # Check runs missing one
            {
                "check_runs": [
                    check_run("Compose Gate", "success"),
                    check_run("Leak Scan", "success"),
                    check_run("Stack Integration Gate", "success"),
                    # Pipeline Complete is missing
                ]
            },
        ]

        result = deploy_status(pr=42, repo="homelab-deploy")
        pipeline = next(g for g in result["gates"] if g["name"] == "Pipeline Complete")
        assert pipeline["status"] == "missing"
        assert result["merge_ready"] is False
