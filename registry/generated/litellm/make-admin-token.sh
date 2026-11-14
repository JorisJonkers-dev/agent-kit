#!/usr/bin/env bash
# GENERATED from registry/estate-tooling.yaml (litellm); do not edit.
# Makes a LiteLLM management key that expires in one hour, for setup to mint this host's
# agent keys and their teams with. It can call the management routes only (no model call).
# LiteLLM lets such a key manage teams and keys only when its owner is a proxy admin, so the
# key belongs to the user agent-kit-setup (role proxy_admin, no key of its own), made here
# the first time. Run it with the proxy's master key in LITELLM_MASTER_KEY; the key goes in a
# curl config on stdin, so it reaches no argv. The new key is printed for setup
# (LITELLM_ADMIN_TOKEN, or paste it when setup asks); setup never stores it.
set -euo pipefail
: "${LITELLM_MASTER_KEY:?set LITELLM_MASTER_KEY to the proxy master key}"

proxy() {
  curl -sS --config - "$@" <<CFG
header = "Authorization: Bearer ${LITELLM_MASTER_KEY}"
header = "Content-Type: application/json"
CFG
}

if ! proxy -o /dev/null --fail 'https://litellm.jorisjonkers.dev/user/info?user_id=agent-kit-setup' 2>/dev/null; then
  made=$(proxy --fail-with-body --data-binary '{"user_id":"agent-kit-setup","user_role":"proxy_admin","auto_create_key":false}' https://litellm.jorisjonkers.dev/user/new) || { printf '%s\n' "$made" >&2; exit 1; }
fi
proxy --fail-with-body --data-binary '{"user_id":"agent-kit-setup","key_type":"management","duration":"1h","metadata":{"purpose":"agent-kit setup mints agent keys"}}' https://litellm.jorisjonkers.dev/key/generate | python3 -c 'import json, sys; print(json.load(sys.stdin)["key"])'
