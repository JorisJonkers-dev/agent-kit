---
name: prepare-rollback
description: Prepare a rollback branch that pins one service to a prior artifact version. Returns a COMPARE URL for human review. NEVER merges or approves.
---

# prepare-rollback skill

## Purpose

Prepare a rollback branch that pins one service to a prior artifact version.
Returns a COMPARE URL for human review. NEVER merges or approves.

## Usage

```
prepare-rollback \
  --service     <unit-name> \
  --prior-tag   <semver-tag> \
  --reason      "<short description>" \
  --alert-url   "<github issue or alert URL>"
```

## What it does

1. Asserts no privileged credentials in environment (RELEASE_APP_PRIVATE_KEY,
   KUBECONFIG, VAULT_TOKEN, ACTIONS_ID_TOKEN_REQUEST_TOKEN).
2. Checks App key age ≤ 90 days from SECURITY.md (fails loudly if stale).
3. Resolves the artifact digest for `prior-tag` from GHCR.
4. Creates branch `rollback/<service>/<prior-tag>` from main HEAD.
5. Updates `registry/<service>.yaml` — sets `spec.artifact.digest` to prior digest.
6. Appends an INCIDENTS.md entry.
7. Returns: `{ rollback_branch, rollback_pr (COMPARE URL), merged: false }`.

## Boundary

This skill NEVER:
- Opens a PR (returns COMPARE URL only — human must open the PR).
- Merges, approves, or queues a PR.
- Holds RELEASE_APP_PRIVATE_KEY, kubeconfig, Vault token, or OIDC token.

## Required permissions

`GITHUB_TOKEN` with `contents:write` for homelab-deploy (scoped App token via
owner-controlled path — not a personal access token).

## Edge cases

- Key age > 90d: fails with rotation instructions before any mutation.
- `rollbackTargetRetention.acknowledged` not set: fails before branch creation.
- Tag re-pushed: uses digest resolution (not tag timestamp) for the prior version.
