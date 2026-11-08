#!/bin/sh
# projects-layout-guard (docs/adr/0009-projects-layout-guard.md): PreToolUse
# on Bash and EnterWorktree. Prints the deny the agent reads when a clone or
# worktree would land outside ~/Projects/<owner>/..., prints nothing
# otherwise, and exits 0. A missing python3 is a failed hook, which Claude
# Code and Codex both let through.
exec python3 "${0%/*}/projects-layout-guard.py"
