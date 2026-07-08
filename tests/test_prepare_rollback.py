"""Tests for prepare_rollback tool."""

from unittest.mock import MagicMock, patch

import pytest

from tools.deploy.prepare_rollback import (
    AppKeyTooOldError,
    PrivilegedEnvPresentError,
    prepare_rollback,
)


@pytest.fixture
def mock_github_token(monkeypatch):
    """Set GITHUB_TOKEN environment variable."""
    monkeypatch.setenv("GITHUB_TOKEN", "test-token")


class TestPrepareRollbackEnvCheck:
    """T-I3: prepare_rollback refuses when privileged key present."""

    def test_prepare_rollback_fails_when_release_key_in_env(
        self, monkeypatch, mock_github_token
    ):
        """Reject when RELEASE_APP_PRIVATE_KEY is set."""
        monkeypatch.setenv("RELEASE_APP_PRIVATE_KEY", "-----BEGIN RSA PRIVATE KEY-----...")

        with pytest.raises(PrivilegedEnvPresentError) as exc:
            prepare_rollback(
                service="agents-api",
                prior_tag="v1.2.3",
                reason="test",
                alert_url="https://github.com/issue/1",
            )
        assert "RELEASE_APP_PRIVATE_KEY" in str(exc.value.vars)

    def test_prepare_rollback_fails_when_kubeconfig_present(
        self, monkeypatch, mock_github_token
    ):
        """Reject when KUBECONFIG is set."""
        monkeypatch.setenv("KUBECONFIG", "/home/runner/.kube/config")

        with pytest.raises(PrivilegedEnvPresentError):
            prepare_rollback(
                service="agents-api",
                prior_tag="v1.2.3",
                reason="test",
                alert_url="https://github.com/issue/1",
            )

    def test_prepare_rollback_fails_when_vault_token_present(
        self, monkeypatch, mock_github_token
    ):
        """Reject when VAULT_TOKEN is set."""
        monkeypatch.setenv("VAULT_TOKEN", "s.abc123")

        with pytest.raises(PrivilegedEnvPresentError):
            prepare_rollback(
                service="agents-api",
                prior_tag="v1.2.3",
                reason="test",
                alert_url="https://github.com/issue/1",
            )


class TestPrepareRollbackAppKeyAge:
    """T-I4: prepare_rollback fails when App key > 90 days old."""

    @patch("tools.deploy.prepare_rollback.fetch_file_content")
    @patch("tools.deploy.prepare_rollback.get_branch_sha")
    def test_prepare_rollback_refuses_stale_app_key(
        self, mock_get_sha, mock_fetch, mock_github_token
    ):
        """Reject when App key is older than 90 days."""
        # Simulate a key created 91 days ago (as of 2026-07-08)
        old_date = "2026-04-08"  # 91 days before 2026-07-08
        mock_fetch.return_value = f"App key created: {old_date}"
        mock_get_sha.return_value = "abc123"

        with pytest.raises(AppKeyTooOldError) as exc:
            prepare_rollback(
                service="agents-api",
                prior_tag="v1.2.3",
                reason="test",
                alert_url="https://github.com/issue/1",
            )
        assert "rotate" in str(exc.value.reason).lower()
        assert exc.value.age_days > 90

    @patch("tools.deploy.prepare_rollback.fetch_file_content")
    @patch("tools.deploy.prepare_rollback.get_branch_sha")
    def test_prepare_rollback_accepts_fresh_app_key(
        self, mock_get_sha, mock_fetch, mock_github_token
    ):
        """Accept when App key is fresh (< 90 days old)."""
        # Simulate a key created 30 days ago
        fresh_date = "2026-06-08"
        mock_fetch.return_value = f"App key created: {fresh_date}"
        mock_get_sha.return_value = "abc123"

        # This should not raise during app key check
        # (it will fail later on missing registry file, which is fine for this test)
        try:
            prepare_rollback(
                service="agents-api",
                prior_tag="v1.2.3",
                reason="test",
                alert_url="https://github.com/issue/1",
            )
        except AppKeyTooOldError:
            pytest.fail("Should not raise AppKeyTooOldError for fresh key")
        except Exception:
            # Other exceptions are ok (like missing files in real github)
            pass

    @patch("tools.deploy.prepare_rollback.fetch_file_content")
    @patch("tools.deploy.prepare_rollback.get_branch_sha")
    def test_prepare_rollback_graceful_on_unparseable_key_date(
        self, mock_get_sha, mock_fetch, mock_github_token
    ):
        """Gracefully skip key check if date is unparseable."""
        # Unparseable date format
        mock_fetch.return_value = "App key created: unknown"
        mock_get_sha.return_value = "abc123"

        # Should not raise AppKeyTooOldError; warning logged but continues
        try:
            prepare_rollback(
                service="agents-api",
                prior_tag="v1.2.3",
                reason="test",
                alert_url="https://github.com/issue/1",
            )
        except AppKeyTooOldError:
            pytest.fail("Should be graceful when key date is unparseable")
        except Exception:
            # Other exceptions ok for this test
            pass


class TestGithubApiSecurity:
    """T-I5: github_api blocks forbidden operations."""

    def test_github_api_blocks_merge(self):
        """Merge operation is blocked."""
        from tools.deploy.github_api import ForbiddenOperationError, github_api_put

        with pytest.raises(ForbiddenOperationError) as exc:
            github_api_put(
                "/repos/JorisJonkers-dev/homelab-deploy/pulls/42/merge",
                token="fake-token",
            )
        assert "merge" in str(exc.value).lower()

    def test_github_api_blocks_approve_review(self):
        """Approve review operation is blocked."""
        from tools.deploy.github_api import ForbiddenOperationError, github_api_post

        with pytest.raises(ForbiddenOperationError):
            github_api_post(
                "/repos/JorisJonkers-dev/homelab-deploy/pulls/42/reviews",
                token="fake-token",
                body={"event": "APPROVE"},
            )

    @patch("requests.get")
    def test_github_api_allows_get(self, mock_get):
        """GET requests are never blocked."""
        from tools.deploy.github_api import github_api_get

        mock_response = MagicMock()
        mock_response.json.return_value = {"test": "data"}
        mock_get.return_value = mock_response

        result = github_api_get("/repos/Org/repo/pulls/1", token="fake-token")
        assert result is not None


class TestRollbackBranchCreation:
    """Test rollback branch creation."""

    @patch("tools.deploy.prepare_rollback.commit_files")
    @patch("tools.deploy.prepare_rollback.get_file_contents")
    @patch("tools.deploy.prepare_rollback.create_branch")
    @patch("tools.deploy.prepare_rollback.get_branch_sha")
    @patch("tools.deploy.prepare_rollback.fetch_file_content")
    def test_prepare_rollback_returns_compare_url(
        self,
        mock_fetch,
        mock_get_sha,
        mock_create_branch,
        mock_get_contents,
        mock_commit,
        mock_github_token,
    ):
        """prepare_rollback returns a COMPARE URL and branch name."""
        mock_fetch.return_value = "App key created: 2026-07-01"
        mock_get_sha.return_value = "abc123"
        mock_get_contents.side_effect = Exception("File not found")

        result = prepare_rollback(
            service="agents-api",
            prior_tag="v1.2.3",
            reason="test failure",
            alert_url="https://github.com/issue/1",
        )

        assert result["merged"] is False
        assert result["rollback_branch"] == "rollback/agents-api/v1.2.3"
        assert "https://github.com/homelab-deploy/compare/main..." in result["rollback_pr"]
