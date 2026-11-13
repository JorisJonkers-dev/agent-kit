#!/bin/sh
# critique-session-start (docs/adr/0002-exactly-three-critique-hooks.md):
# SessionStart. Prints one line suggesting /flow setup in an opted-in
# repository whose machine setup is missing or failing, and nothing otherwise.
# The critique runner decides; this script only hands it the payload.
exec /bin/sh "${0%/*}/../critique" hook critique-session-start
