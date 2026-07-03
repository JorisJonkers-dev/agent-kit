#!/usr/bin/env bash
# UserPromptSubmit hook — calls knowledge.recall with the user's
# prompt content before the agent sees it. A tiny bounded hit list is
# injected so the agent has prior captures in hand without a KB dump.
#
# Silent on failure; the KB being unreachable should not block
# typing into Claude Code.

set -u

[ "${KB_AUTO_MCP_DISABLED:-0}" = 1 ] && exit 0
if [ -z "${KB_BEARER_TOKEN:-}" ]; then exit 0; fi

KB_URL="${KB_URL:-@KB_URL@}"
case "${KB_URL}" in
  */mcp) KB_MCP_URL="${KB_URL}" ;;
  *) KB_MCP_URL="${KB_URL%/}/mcp" ;;
esac

# Stdin carries the JSON event payload. Extract the prompt from several
# possible shapes: user_prompt string, messages list, or generic prompt.
input="$(cat 2>/dev/null || true)"
prompt="$(printf '%s' "${input}" | python3 -c '
import json, sys

def text(value):
    if isinstance(value, str):
        return value
    if isinstance(value, list):
        return " ".join(text(v) for v in value)
    if isinstance(value, dict):
        if isinstance(value.get("text"), str):
            return value["text"]
        if "content" in value:
            return text(value["content"])
    return ""

try:
    data = json.load(sys.stdin)
except Exception:
    sys.exit(0)

for key in ("user_prompt", "prompt", "input"):
    value = text(data.get(key))
    if value:
        print(value, end="")
        sys.exit(0)

messages = data.get("messages")
if isinstance(messages, list) and messages:
    print(text(messages[-1]), end="")
' 2>/dev/null || true)"

# Skip trivially-short prompts — overhead > value.
[ "${#prompt}" -lt "${KB_RECALL_MIN_PROMPT_CHARS:-40}" ] && exit 0

canonical_project_scope_from_origin() {
  remote="$(git remote get-url origin 2>/dev/null || true)"
  case "${remote}" in
    git@github.com:*) path="${remote#git@github.com:}" ;;
    https://github.com/*) path="${remote#https://github.com/}" ;;
    ssh://git@github.com/*) path="${remote#ssh://git@github.com/}" ;;
    *) return 0 ;;
  esac
  path="${path%.git}"
  owner="${path%%/*}"
  repo="${path#*/}"
  [ "${repo}" != "${path}" ] || return 0
  [ -n "${owner}" ] && [ -n "${repo}" ] || return 0
  case "${repo}" in */*) return 0 ;; esac
  printf 'project:%s/%s' \
    "$(printf '%s' "${owner}" | tr '[:upper:]' '[:lower:]')" \
    "$(printf '%s' "${repo}" | tr '[:upper:]' '[:lower:]')"
}

repo_scope="$(canonical_project_scope_from_origin)"
scope="${KB_RECALL_SCOPE:-${repo_scope}}"

# Mode: default to fast (single-leg FTS, ~50ms) so the hook reliably stays
# under its settings timeout. Hybrid (pgvector+Ollama) can take seconds when
# Ollama is cold — that plus the fast fallback blew the 5s budget on every
# long prompt. Opt into hybrid explicitly via KB_RECALL_HOOK_MODE=hybrid.
mode="${KB_RECALL_HOOK_MODE:-fast}"
limit="${KB_RECALL_HOOK_LIMIT:-3}"
min_score="${KB_RECALL_MIN_SCORE:-0.004}"

recall_payload() {
  python3 -c 'import json,sys
args = {"query": sys.argv[1], "limit": int(sys.argv[2]), "mode": sys.argv[3]}
if sys.argv[4]:
    args["scope"] = sys.argv[4]
print(json.dumps({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"knowledge.recall","arguments":args}}))' \
    "$1" "$2" "$3" "$4"
}

call_recall() {
  payload=$(recall_payload "$1" "$2" "$3" "$4") || return 1
  curl -sS --connect-timeout 2 --max-time 3 \
    -H "Authorization: Bearer ${KB_BEARER_TOKEN}" \
    -H "Content-Type: application/json" \
    -d "${payload}" \
    "${KB_MCP_URL}" 2>/dev/null
}

response=$(call_recall "${prompt}" "${limit}" "${mode}" "${scope}") || response=""
if [ -z "${response}" ] && [ "${mode}" != "fast" ]; then
  response=$(call_recall "${prompt}" "${limit}" fast "${scope}") || exit 0
fi
[ -n "${response}" ] || exit 0

printf '%s' "${response}" | python3 -c '
import json, sys
try:
    data = json.load(sys.stdin)
    hits = data["result"]["structuredContent"]["hits"]
except Exception:
    sys.exit(0)
min_score = float("'"${min_score}"'")
hits = [h for h in hits if float(h.get("score", 0) or 0) >= min_score]
if not hits:
    sys.exit(0)
print("## Knowledge base — relevant prior captures")
print()
for h in hits:
    title = h.get("title", "")
    scope = h.get("scope", "")
    note_id = h.get("id", "")
    score = h.get("score", 0)
    print(f"- **{title}** (`{scope}`, score {score}) — id `{note_id}`")
    snip = (h.get("snippet") or "").replace("\n", " ").strip()
    if snip:
        print(f"  > {snip[:160]}")
' 2>/dev/null || true
