#!/bin/sh
# critique-commit-gate (docs/adr/0002-exactly-three-critique-hooks.md):
# PreToolUse on Bash. Hands the agent's payload to the critique runner's commit
# gate, which prints the deny the agent reads when a risky staged change has no
# passing /review-implementation run, prints nothing otherwise, and exits 0. A missing
# node or runner is a failed hook, which Claude Code and Codex both let through.
# Never in the work Claude profile (~/.claude): critique there may use only
# Codex or Claude, and the review stages route to other families (owner rule,
# 2026-11-09). A Claude Code payload's transcript_path names the profile, so a
# work session is let through silently; Codex and the personal profile
# (~/.claude-personal) are gated as before.
payload=$(cat)
case "$payload" in
  *"\"transcript_path\":\"$HOME/.claude/"* | *"\"transcript_path\": \"$HOME/.claude/"*) exit 0 ;;
esac
printf '%s' "$payload" | exec node "${0%/*}/../critique.mjs" hook critique-commit-gate
