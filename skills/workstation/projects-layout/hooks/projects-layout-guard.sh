#!/bin/sh
# projects-layout-guard (docs/adr/0009-projects-layout-guard.md): PreToolUse
# on Bash and EnterWorktree. Hands the agent's payload to the Rust guard setup
# installs (the `projects-layout-guard` workspace tool, ~/.local/bin by default),
# which prints the deny the agent reads when a clone or worktree would land
# outside ~/Projects/<owner>/..., prints nothing otherwise, and exits 0. A guard
# that is not installed allows the call: the hook fails open (ADR-0009).
bin=${WORKSPACE_TOOLS_BIN_DIR:-$HOME/.local/bin}/projects-layout-guard
if [ -f "$bin" ] && [ -x "$bin" ]; then
  exec "$bin"
fi
exit 0
