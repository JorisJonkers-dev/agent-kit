"""GitHub API wrapper enforcing operation security denylist."""

from typing import Any

import re

import requests

FORBIDDEN_OPERATIONS: frozenset[str] = frozenset({
    # Merge / queue
    "PUT /repos/{repo}/pulls/{pull_number}/merge",
    "POST /repos/{repo}/merges",
    "POST /repos/{repo}/pulls/{pull_number}/update-branch",  # merge-queue enqueue
    # PR reviews / approvals
    "POST /repos/{repo}/pulls/{pull_number}/reviews",
    "PUT /repos/{repo}/pulls/{pull_number}/reviews/{review_id}/events",
    "POST /repos/{repo}/pulls/{pull_number}/requested_reviewers",
    # Deployment environment approvals
    "POST /repos/{repo}/actions/runs/{run_id}/pending_deployments",
    # Branch/tag force-operations
    "DELETE /repos/{repo}/git/refs/{ref}",
    "PATCH /repos/{repo}/git/refs/{ref}",     # force-update (blocked; only safe create allowed)
    # Secret management
    "PUT /repos/{repo}/actions/secrets/{secret_name}",
    "DELETE /repos/{repo}/actions/secrets/{secret_name}",
    # Self-hosted runner tokens
    "POST /repos/{repo}/actions/runners/registration-token",
    "POST /repos/{repo}/actions/runners/remove-token",
})

ALLOWED_METHODS = frozenset({"GET", "HEAD"})  # GET/HEAD always allowed without check


class ForbiddenOperationError(Exception):
    """Raised when an operation is blocked by the denylist."""

    def __init__(self, operation: str, reason: str) -> None:
        self.operation = operation
        self.reason = reason
        super().__init__(f"Operation '{operation}' is blocked: {reason}")


def assert_allowed(method: str, path_template: str) -> None:
    """
    Raise ForbiddenOperationError if the (method, path_template) pair is in the denylist.
    path_template uses {repo}, {pull_number} etc. placeholders as written in FORBIDDEN_OPERATIONS.
    """
    if method.upper() in ALLOWED_METHODS:
        return
    normalized = f"{method.upper()} {path_template}"
    if normalized in FORBIDDEN_OPERATIONS:
        raise ForbiddenOperationError(
            operation=normalized,
            reason=(
                f"Operation '{normalized}' is blocked by FORBIDDEN_OPERATIONS denylist. "
                "Agent tools must not merge, approve, enqueue, or mutate deployment protection."
            ),
        )


def _to_template(path: str) -> str:
    """
    Normalize a concrete path to a template for denylist lookup.
    e.g. "/repos/Org/my-repo/pulls/42/merge" → "/repos/{repo}/pulls/{pull_number}/merge"
    Covers common patterns; unrecognized paths pass through unchanged.
    """
    path = re.sub(r"/repos/[^/]+/[^/]+/", "/repos/{repo}/", path)
    path = re.sub(r"/pulls/\d+/", "/pulls/{pull_number}/", path)
    path = re.sub(r"/runs/\d+/", "/runs/{run_id}/", path)
    path = re.sub(r"/reviews/\d+/", "/reviews/{review_id}/", path)
    path = re.sub(r"/secrets/[^/]+", "/secrets/{secret_name}", path)
    return path


def github_api_get(
    path: str, token: str, params: dict[str, str | int] | None = None
) -> Any:
    """Authenticated GET; never blocked by assert_allowed (GET is always allowed)."""
    url = f"https://api.github.com{path}"
    resp = requests.get(
        url,
        headers={"Authorization": f"Bearer {token}", "Accept": "application/vnd.github+json"},
        params=params or {},
    )
    resp.raise_for_status()
    return resp.json()


def github_api_post(
    path: str, token: str, body: dict[str, Any] | None = None
) -> Any:
    """Authenticated POST; checks FORBIDDEN_OPERATIONS before sending."""
    assert_allowed("POST", _to_template(path))
    url = f"https://api.github.com{path}"
    resp = requests.post(
        url,
        headers={"Authorization": f"Bearer {token}", "Accept": "application/vnd.github+json"},
        json=body or {},
    )
    resp.raise_for_status()
    return resp.json()


def github_api_put(
    path: str, token: str, body: dict[str, Any] | None = None
) -> Any:
    """Authenticated PUT; checks FORBIDDEN_OPERATIONS before sending."""
    assert_allowed("PUT", _to_template(path))
    url = f"https://api.github.com{path}"
    resp = requests.put(
        url,
        headers={"Authorization": f"Bearer {token}", "Accept": "application/vnd.github+json"},
        json=body or {},
    )
    resp.raise_for_status()
    return resp.json()
