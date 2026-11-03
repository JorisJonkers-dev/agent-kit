#!/bin/sh
# critique-commit-gate (docs/adr/0002-exactly-three-critique-hooks.md):
# PreToolUse on Bash. Hands the agent's payload to the critique runner's commit
# gate, which prints the deny the agent reads when a risky staged change has no
# passing /review-ticket run, prints nothing otherwise, and exits 0. A missing
# node or runner is a failed hook, which Claude Code and Codex both let through.
exec node "${0%/*}/../critique.mjs" hook critique-commit-gate
