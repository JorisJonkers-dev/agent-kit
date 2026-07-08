"""Tests for github_api module."""

from unittest.mock import MagicMock, patch

import pytest

from tools.deploy.github_api import (
    ForbiddenOperationError,
    _to_template,
    assert_allowed,
    github_api_get,
    github_api_post,
    github_api_put,
)


class TestDenylistNormalization:
    """Test path-to-template normalization."""

    def test_normalize_repo_and_pr(self):
        """Normalize repo and PR number in path."""
        path = "/repos/Org/my-repo/pulls/42/merge"
        expected = "/repos/{repo}/pulls/{pull_number}/merge"
        assert _to_template(path) == expected

    def test_normalize_run_id(self):
        """Normalize run ID in path."""
        path = "/repos/Org/my-repo/actions/runs/12345/jobs/67890"
        result = _to_template(path)
        assert "/runs/{run_id}/" in result
        assert result.startswith("/repos/{repo}/actions/")

    def test_normalize_secret_name(self):
        """Normalize secret name in path."""
        path = "/repos/Org/my-repo/actions/secrets/MY_SECRET"
        expected = "/repos/{repo}/actions/secrets/{secret_name}"
        assert _to_template(path) == expected

    def test_pass_through_unknown(self):
        """Unknown paths pass through unchanged."""
        path = "/some/unknown/path"
        assert _to_template(path) == path


class TestAssertAllowed:
    """Test assert_allowed blocking logic."""

    def test_get_always_allowed(self):
        """GET is always allowed."""
        assert_allowed("GET", "/repos/{repo}/pulls/{pull_number}/merge")
        # Should not raise

    def test_head_always_allowed(self):
        """HEAD is always allowed."""
        assert_allowed("HEAD", "/repos/{repo}/actions/secrets/{secret_name}")
        # Should not raise

    def test_post_merge_blocked(self):
        """POST merge is blocked."""
        with pytest.raises(ForbiddenOperationError):
            assert_allowed("POST", "/repos/{repo}/merges")

    def test_put_merge_blocked(self):
        """PUT merge is blocked."""
        with pytest.raises(ForbiddenOperationError):
            assert_allowed("PUT", "/repos/{repo}/pulls/{pull_number}/merge")

    def test_post_review_blocked(self):
        """POST review is blocked."""
        with pytest.raises(ForbiddenOperationError):
            assert_allowed("POST", "/repos/{repo}/pulls/{pull_number}/reviews")

    def test_put_review_events_blocked(self):
        """PUT review events is blocked."""
        with pytest.raises(ForbiddenOperationError):
            assert_allowed("PUT", "/repos/{repo}/pulls/{pull_number}/reviews/{review_id}/events")

    def test_post_update_branch_blocked(self):
        """POST update-branch (merge-queue enqueue) is blocked."""
        with pytest.raises(ForbiddenOperationError):
            assert_allowed("POST", "/repos/{repo}/pulls/{pull_number}/update-branch")

    def test_post_pending_deployments_blocked(self):
        """POST pending deployments is blocked."""
        with pytest.raises(ForbiddenOperationError):
            assert_allowed("POST", "/repos/{repo}/actions/runs/{run_id}/pending_deployments")

    def test_delete_git_ref_blocked(self):
        """DELETE git refs is blocked."""
        with pytest.raises(ForbiddenOperationError):
            assert_allowed("DELETE", "/repos/{repo}/git/refs/{ref}")

    def test_patch_git_ref_blocked(self):
        """PATCH git refs is blocked."""
        with pytest.raises(ForbiddenOperationError):
            assert_allowed("PATCH", "/repos/{repo}/git/refs/{ref}")

    def test_put_secrets_blocked(self):
        """PUT secrets is blocked."""
        with pytest.raises(ForbiddenOperationError):
            assert_allowed("PUT", "/repos/{repo}/actions/secrets/{secret_name}")

    def test_delete_secrets_blocked(self):
        """DELETE secrets is blocked."""
        with pytest.raises(ForbiddenOperationError):
            assert_allowed("DELETE", "/repos/{repo}/actions/secrets/{secret_name}")

    def test_post_registration_token_blocked(self):
        """POST registration token is blocked."""
        with pytest.raises(ForbiddenOperationError):
            assert_allowed("POST", "/repos/{repo}/actions/runners/registration-token")

    def test_post_remove_token_blocked(self):
        """POST remove token is blocked."""
        with pytest.raises(ForbiddenOperationError):
            assert_allowed("POST", "/repos/{repo}/actions/runners/remove-token")


class TestGithubAPIGet:
    """Test github_api_get function."""

    @patch("requests.get")
    def test_get_with_params(self, mock_get):
        """GET request with params."""
        mock_response = MagicMock()
        mock_response.json.return_value = {"test": "data"}
        mock_get.return_value = mock_response

        result = github_api_get(
            "/repos/test/repo/issues", token="test-token", params={"state": "open"}
        )

        assert result == {"test": "data"}
        mock_get.assert_called_once()
        kwargs = mock_get.call_args.kwargs
        assert "test-token" in kwargs["headers"]["Authorization"]
        assert kwargs["params"] == {"state": "open"}

    @patch("requests.get")
    def test_get_no_params(self, mock_get):
        """GET request without params."""
        mock_response = MagicMock()
        mock_response.json.return_value = {"test": "data"}
        mock_get.return_value = mock_response

        result = github_api_get("/repos/test/repo/issues", token="test-token")

        assert result == {"test": "data"}


class TestGithubAPIPost:
    """Test github_api_post function."""

    @patch("requests.post")
    def test_post_allowed_operation(self, mock_post):
        """POST for allowed operation."""
        mock_response = MagicMock()
        mock_response.json.return_value = {"status": "ok"}
        mock_post.return_value = mock_response

        # Create a branch is allowed
        result = github_api_post(
            "/repos/{repo}/git/refs",
            token="test-token",
            body={"ref": "refs/heads/main", "sha": "abc123"},
        )

        assert result == {"status": "ok"}

    def test_post_forbidden_operation(self):
        """POST for forbidden operation raises."""
        with pytest.raises(ForbiddenOperationError):
            # This should raise before any HTTP request
            github_api_post(
                "/repos/test-org/test-repo/merges",
                token="test-token",
            )


class TestGithubAPIPut:
    """Test github_api_put function."""

    def test_put_forbidden_operation(self):
        """PUT for forbidden operation raises."""
        with pytest.raises(ForbiddenOperationError):
            # This should raise before any HTTP request
            github_api_put(
                "/repos/test-org/test-repo/pulls/42/merge",
                token="test-token",
            )


class TestForbiddenOperationError:
    """Test ForbiddenOperationError exception."""

    def test_error_message(self):
        """Error message includes operation and reason."""
        err = ForbiddenOperationError("PUT /merge", "merge not allowed")
        assert "PUT /merge" in str(err)
        assert "merge not allowed" in str(err)

    def test_error_attributes(self):
        """Error has operation and reason attributes."""
        err = ForbiddenOperationError("PUT /merge", "merge not allowed")
        assert err.operation == "PUT /merge"
        assert err.reason == "merge not allowed"
