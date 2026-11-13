#!/usr/bin/env bash
# GENERATED from registry/estate-tooling.yaml (litellm.routes); do not edit.
# The owner runs this once the proxy is deployed, with the proxy's master key in
# LITELLM_MASTER_KEY. The key goes in a curl config on stdin, so it reaches no argv. Each
# response is a new virtual key: paste it into setup when it asks, and nowhere else.
# `google-free-public` is on the public key only (Google may train on free-tier prompts).
set -euo pipefail
: "${LITELLM_MASTER_KEY:?set LITELLM_MASTER_KEY to the proxy master key}"

make_key() {
  curl -sS --config - --data-binary "$1" https://litellm.jorisjonkers.dev/key/generate <<CFG
header = "Authorization: Bearer ${LITELLM_MASTER_KEY}"
header = "Content-Type: application/json"
CFG
  echo
}

make_key '{"key_alias":"vibe","models":["canonical"],"max_budget":255,"budget_duration":"30d"}'
make_key '{"key_alias":"vibe-study","models":["canonical"],"max_budget":127.5,"budget_duration":"30d"}'
make_key '{"key_alias":"opencode-go","models":["canonical"],"max_budget":60,"budget_duration":"30d"}'
make_key '{"key_alias":"openrouter","models":["canonical"],"max_budget":5,"budget_duration":"30d"}'
make_key '{"key_alias":"local","models":["canonical"]}'
make_key '{"key_alias":"public","models":["google-free-public","canonical"],"max_budget":5,"budget_duration":"30d"}'
