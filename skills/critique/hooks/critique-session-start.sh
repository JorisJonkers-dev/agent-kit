#!/bin/sh
# critique-session-start (docs/adr/0002-exactly-three-critique-hooks.md):
# SessionStart. Prints one line suggesting /flow setup in an opted-in
# repository whose machine setup is missing or failing, and nothing otherwise.
# The critique runner decides; this script only hands it the payload.
exec node "${0%/*}/../critique.mjs" hook critique-session-start
