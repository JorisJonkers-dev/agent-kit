#!/bin/sh
# critique-session-start (docs/adr/0002-exactly-three-critique-hooks.md):
# SessionStart. Prints one line suggesting /flow setup in an opted-in
# repository whose machine setup is missing or failing, and nothing otherwise.
# The critique runner decides; this script only hands it the payload.
# Never in the work Claude profile (~/.claude): critique there may use only
# Codex or Claude, and the review stages route to other families (owner rule,
# 2026-11-09). A Claude Code payload's transcript_path names the profile, so a
# work session is let through silently; Codex and the personal profile
# (~/.claude-personal) are gated as before.
payload=$(cat)
case "$payload" in
  *"\"transcript_path\":\"$HOME/.claude/"* | *"\"transcript_path\": \"$HOME/.claude/"*) exit 0 ;;
esac
printf '%s' "$payload" | exec node "${0%/*}/../critique.mjs" hook critique-session-start
