---
name: deploy-status
description: Fetch and summarise the gate status for a homelab-deploy pull request. Reports each of the four required checks and whether the PR is merge-ready.
---

# deploy-status skill

## Purpose

Fetch and summarise the gate status for a homelab-deploy pull request.
Reports each of the four required checks and whether the PR is merge-ready.

## Usage

```
deploy-status --pr <number> [--repo homelab-deploy]
```

## What it does

1. Resolves the PR head SHA.
2. Lists check runs for that SHA.
3. Downloads gate-summary artifacts (SC-4) from the associated Pipeline Complete run.
4. Returns a structured summary:
   - `gates[]` — one entry per required check: Compose Gate, Leak Scan,
     Stack Integration Gate, Pipeline Complete.
   - `merge_ready` — true only when Pipeline Complete is "pass" AND all gates pass.
   - `blocker` — name of the first non-pass gate, or null.

## Authoritative source

`Pipeline Complete` is the authoritative merge-readiness signal.
Do not infer merge-readiness from individual gate conclusions alone.

## Limitations

- Read-only. Never merges, approves, or enqueues.
- Requires `GITHUB_TOKEN` env var with `contents:read` + `actions:read`.
- Stale artifacts (force-pushed head): always uses latest-head-SHA artifacts.

## Edge cases

- Missing artifact: gate status shown as "pending" or "missing"; not an error.
- Merge-queue vs PR-head drift: always uses PR head SHA, not merge_group SHA.
- Tag re-pushed: latest artifact by created_at wins.
