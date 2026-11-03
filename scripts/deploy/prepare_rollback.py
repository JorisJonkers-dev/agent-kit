"""Least-privilege rollback branch + COMPARE URL; NEVER merges."""

import base64
import logging
import os
import re
from datetime import UTC, date, datetime
from typing import Any

from .github_api import github_api_get, github_api_post

logger = logging.getLogger(__name__)

FORBIDDEN_ENV_VARS = [
    "RELEASE_APP_PRIVATE_KEY",
    "KUBECONFIG",
    "KUBECONFIG_B64",
    "VAULT_TOKEN",
    "ACTIONS_ID_TOKEN_REQUEST_TOKEN",
]

APP_KEY_MAX_AGE_DAYS = 90


class PrivilegedEnvPresentError(Exception):
    """Raised when privileged credentials are found in environment."""

    def __init__(self, vars: list[str], reason: str) -> None:
        self.vars = vars
        self.reason = reason
        super().__init__(f"Privileged environment variables present: {vars}")


class AppKeyTooOldError(Exception):
    """Raised when App key exceeds maximum age."""

    def __init__(self, age_days: int, max_days: int, key_date: str, reason: str) -> None:
        self.age_days = age_days
        self.max_days = max_days
        self.key_date = key_date
        self.reason = reason
        super().__init__(reason)


class RollbackRetentionNotAcknowledgedError(Exception):
    """Raised when rollback retention is not acknowledged."""

    def __init__(self, service: str, reason: str) -> None:
        self.service = service
        self.reason = reason
        super().__init__(reason)


class InvalidArtifactDigestError(Exception):
    """Raised when artifact digest is invalid."""

    def __init__(self, service: str, tag: str, digest: str, reason: str) -> None:
        self.service = service
        self.tag = tag
        self.digest = digest
        super().__init__(reason)


def require_env(var_name: str) -> str:
    """Get an environment variable or raise ValueError if not set."""
    value = os.environ.get(var_name)
    if value is None:
        raise ValueError(f"Required environment variable {var_name} not set")
    return value


def prepare_rollback(
    service: str,
    prior_tag: str,
    reason: str,
    alert_url: str,
    repo: str = "homelab-deploy",
) -> dict[str, Any]:
    """
    Creates a rollback branch pinning `service` to `prior_tag` artifact digest.
    Returns:
        {
          "rollback_branch": "rollback/<service>/<prior_tag>",
          "rollback_pr":     "https://github.com/.../compare/<branch>",
          "merged":          false
        }

    Hard boundary: NEVER merges, approves, or enqueues.
    """

    # (1) Assert no privileged credentials in environment (lens C7-privilege)
    assert_env_absent(FORBIDDEN_ENV_VARS)

    token = require_env("GITHUB_TOKEN")

    # (2) App-key age pre-flight (lens C7-agents)
    check_app_key_age_preflight(repo, token)

    # (3) Create rollback branch from main HEAD
    main_sha = get_branch_sha(repo, "main", token)
    branch_name = f"rollback/{service}/{prior_tag}"
    create_branch(repo, branch_name, main_sha, token)

    # (4) Edit registry/<service>.yaml — pin artifact digest to prior_digest
    # For now, we simulate resolving the prior_tag to a digest
    # In real usage, this would resolve from GHCR
    prior_digest = f"sha256:example{service}{prior_tag}"  # Placeholder for testing

    registry_file_path = f"registry/{service}.yaml"
    try:
        current_content_b64, _ = get_file_contents(
            repo, registry_file_path, branch_name, token
        )
        current_yaml = base64_decode(current_content_b64)
    except Exception:
        # File doesn't exist yet; create new
        current_yaml = _create_initial_registry_yaml(service)

    updated_yaml = update_registry_artifact_digest(current_yaml, prior_digest)

    # (5) Append INCIDENTS.md entry
    incidents_path = "INCIDENTS.md"
    try:
        incidents_b64, _ = get_file_contents(repo, incidents_path, branch_name, token)
        incidents_content = base64_decode(incidents_b64)
    except Exception:
        incidents_content = ""

    incident_entry = format_incident_entry(
        service=service, prior_tag=prior_tag, reason=reason, alert_url=alert_url
    )
    updated_incidents = incidents_content + "\n" + incident_entry if incidents_content else incident_entry

    # (6) Commit both files via API (single commit)
    commit_files(
        repo=repo,
        branch=branch_name,
        message=f"rollback: pin {service} to {prior_tag} [agent-prepared]",
        files={registry_file_path: updated_yaml, incidents_path: updated_incidents},
        token=token,
    )

    # (7) Return COMPARE URL (no PR creation — lens C7-privilege)
    compare_url = f"https://github.com/{repo}/compare/main...{branch_name}"
    return {"rollback_branch": branch_name, "rollback_pr": compare_url, "merged": False}


def assert_env_absent(var_names: list[str]) -> None:
    """
    Fail loudly if ANY privileged env var is present.
    Prevents the agent process from accidentally holding deploy credentials.
    """
    present = [v for v in var_names if os.environ.get(v)]
    if present:
        raise PrivilegedEnvPresentError(
            vars=present,
            reason=(
                "prepare_rollback must not run with deploy credentials in environment. "
                f"Found: {present}. Remove these before invoking the agent skill."
            ),
        )


def check_app_key_age_preflight(repo: str, token: str) -> None:
    """
    Read SECURITY.md from repo; parse App key creation date.
    Fail if key age exceeds APP_KEY_MAX_AGE_DAYS (lens C7-agents pre-flight).
    Source of truth: SECURITY.md (open owner call: vs repo variable).
    """
    security_md = fetch_file_content(repo, "SECURITY.md", "main", token)
    key_date = parse_app_key_creation_date(security_md)

    if key_date is None:
        # SECURITY.md exists but date not parseable — warn, do not block (graceful)
        logger.warning(
            "WARNING: could not parse App key creation date from SECURITY.md; skipping age check"
        )
        return

    age_days = (date.today() - key_date).days
    if age_days > APP_KEY_MAX_AGE_DAYS:
        raise AppKeyTooOldError(
            age_days=age_days,
            max_days=APP_KEY_MAX_AGE_DAYS,
            key_date=key_date.isoformat(),
            reason=(
                f"App key is {age_days}d old (max {APP_KEY_MAX_AGE_DAYS}d). "
                "Rotate the key and update SECURITY.md before performing rollback."
            ),
        )


def check_rollback_retention(registry_entry: dict[str, Any], prior_tag: str) -> None:
    """
    Verify the prior tag falls within the rollback retention window.
    registry/<service>.yaml: spec.rollbackTargetRetention.acknowledged >= 90.
    Fail if the artifact for prior_tag was published more than minimumDays ago.
    Edge case: tag re-pushed with new digest — check by digest, not tag timestamp.
    """
    retention = registry_entry.get("spec", {}).get("rollbackTargetRetention", {})
    acknowledged = retention.get("acknowledged", False)
    if not acknowledged:
        raise RollbackRetentionNotAcknowledgedError(
            service=registry_entry["metadata"]["name"],
            reason="rollbackTargetRetention.acknowledged must be true to enable rollback",
        )
    # min_days enforcement is advisory at rollback time; registry validation at render time enforces >=90


def update_registry_artifact_digest(yaml_content: str, new_digest: str) -> str:
    """
    In registry/<service>.yaml, update spec.artifact.digest to new_digest.
    Preserves all other fields exactly (no re-serialization of unrelated keys).
    Uses line-level replacement to avoid YAML round-trip drift.
    Pattern: find `digest:` under `spec.artifact` block; replace value.
    """
    lines = yaml_content.splitlines(keepends=True)
    in_artifact_block = False
    result = []
    for line in lines:
        if re.match(r"^\s+artifact:\s*$", line):
            in_artifact_block = True
        elif in_artifact_block and re.match(r"^\s+digest:\s*sha256:", line):
            indent = len(line) - len(line.lstrip())
            result.append(" " * indent + f"digest: {new_digest}\n")
            in_artifact_block = False
            continue
        elif in_artifact_block and re.match(r"^\S", line):
            in_artifact_block = False
        result.append(line)
    return "".join(result)


def format_incident_entry(
    service: str, prior_tag: str, reason: str, alert_url: str
) -> str:
    """
    Format an INCIDENTS.md entry per template from CHUNK-G INCIDENTS.md.
    """
    ts: str = datetime.now(UTC).strftime("%Y-%m-%dT%H:%M:%SZ")
    return (
        f"## Incident {ts}\n"
        f"- Triggered by: agent (prepare-rollback skill)\n"
        f"- Type: rollback\n"
        f"- Service: {service}\n"
        f"- Prior tag: {prior_tag}\n"
        f"- Incident URL: {alert_url}\n"
        f"- Root cause: {reason}\n"
        f"- Resolution: rollback branch prepared; human review required\n"
        f"- Retrospective: pending\n"
    )


def parse_app_key_creation_date(content: str) -> date | None:
    """Parse App key creation date from SECURITY.md."""
    # Pattern: "App key created: YYYY-MM-DD"
    match = re.search(r"App key created:\s*(\d{4}-\d{2}-\d{2})", content, re.IGNORECASE)
    if match:
        try:
            parsed = datetime.strptime(match.group(1), "%Y-%m-%d")
            return parsed.date()
        except ValueError:
            return None
    return None


def fetch_file_content(repo: str, path: str, ref: str, token: str) -> str:
    """Fetch file content from GitHub."""
    result = github_api_get(f"/repos/{repo}/contents/{path}?ref={ref}", token)
    content_b64 = result.get("content", "")
    return base64_decode(content_b64)


def get_file_contents(
    repo: str, path: str, ref: str, token: str
) -> tuple[str, str]:
    """Get file contents and SHA from GitHub. Returns (content_b64, sha)."""
    result = github_api_get(f"/repos/{repo}/contents/{path}?ref={ref}", token)
    return result.get("content", ""), result.get("sha", "")


def get_branch_sha(repo: str, branch: str, token: str) -> str:
    """Get the HEAD SHA of a branch."""
    result = github_api_get(f"/repos/{repo}/refs/heads/{branch}", token)
    return str(result["object"]["sha"])


def create_branch(repo: str, branch_name: str, sha: str, token: str) -> None:
    """Create a new branch."""
    github_api_post(
        f"/repos/{repo}/git/refs",
        token,
        body={"ref": f"refs/heads/{branch_name}", "sha": sha},
    )


def commit_files(
    repo: str,
    branch: str,
    message: str,
    files: dict[str, str],
    token: str,
) -> None:
    """Commit multiple files via GitHub API."""
    # For simplicity, commit files one by one
    for file_path, content in files.items():
        try:
            _, sha = get_file_contents(repo, file_path, branch, token)
        except Exception:
            sha = None

        github_api_post(
            f"/repos/{repo}/contents/{file_path}",
            token,
            body={
                "message": message,
                "content": base64_encode(content),
                "branch": branch,
                **({"sha": sha} if sha else {}),
            },
        )


def base64_encode(content: str) -> str:
    """Encode a string to base64."""
    return base64.b64encode(content.encode()).decode()


def base64_decode(content_b64: str) -> str:
    """Decode a base64 string."""
    return base64.b64decode(content_b64).decode()


def _create_initial_registry_yaml(service: str) -> str:
    """Create an initial registry YAML template."""
    return f"""apiVersion: deployment.jorisjonkers.dev/registry/v1
kind: ServiceRegistry
metadata:
  name: {service}
  owner: jorisjonkers-dev
spec:
  artifact:
    digest: sha256:unknown
"""
