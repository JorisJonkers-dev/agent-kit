#!/usr/bin/env bash
# GENERATED from registry/estate-tooling.yaml (litellm); do not edit.
# Makes a LiteLLM management key that expires in one hour, for setup to mint this host's
# agent keys and their teams with. It can call the management routes only (no model call).
# Run it with the proxy's master key in LITELLM_MASTER_KEY; the key goes in a curl config on
# stdin, so it reaches no argv. Paste the printed key into setup when it asks; setup never
# stores it.
set -euo pipefail
: "${LITELLM_MASTER_KEY:?set LITELLM_MASTER_KEY to the proxy master key}"

curl -sS --fail-with-body --config - --data-binary '{"key_type":"management","duration":"1h","metadata":{"purpose":"agent-kit setup mints agent keys"}}' https://litellm.jorisjonkers.dev/key/generate <<CFG | python3 -c 'import json, sys; print(json.load(sys.stdin)["key"])'
header = "Authorization: Bearer ${LITELLM_MASTER_KEY}"
header = "Content-Type: application/json"
CFG
