#!/bin/bash
# Paste into a Claude Code cloud environment's "Setup script" field (agent-kit docs/CLOUD.md).
# The result is cached for about a week; bump this date to pull a newer release sooner.
# refreshed: 2026-09-26

url="https://assets.jorisjonkers.dev/setup-workstation.sh"
log="${AGENT_KIT_SETUP_LOG:-/var/log/agent-kit-setup.log}"
script="$(mktemp)"

if curl -fsSL --retry 3 -o "${script}" "${url}"; then
  echo "agent-kit: $(sed -n 2p "${script}") from ${url}"
  bash "${script}" --cloud 2>&1 | tee "${log}"
else
  echo "agent-kit: cannot fetch ${url}; is *.jorisjonkers.dev allowed?" | tee "${log}" >&2
fi

# A failed step must not stop the session starting; the log says what broke.
exit 0
