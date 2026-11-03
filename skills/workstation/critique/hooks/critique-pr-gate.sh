#!/bin/sh
# critique-pr-gate (docs/adr/0002-exactly-three-critique-hooks.md):
# PreToolUse on Bash. Hands the agent's payload to the critique runner's PR
# gate, which prints the deny the agent reads when `gh pr create`, or a push
# outside the feature branch patterns, sends a head /review-feature has not
# passed, prints nothing otherwise, and exits 0. A missing node or runner is a
# failed hook, which Claude Code and Codex both let through.
exec node "${0%/*}/../critique.mjs" hook critique-pr-gate
