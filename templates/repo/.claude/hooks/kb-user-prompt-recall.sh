#!/usr/bin/env bash
# Claude UserPromptSubmit hook: inject a tiny KB recall block before the
# agent sees the prompt. Silent on failure.

set -u

[ "${KB_AUTO_MCP_DISABLED:-0}" = 1 ] && exit 0
[ -z "${KB_BEARER_TOKEN:-}" ] && exit 0

KB_URL="${KB_URL:-https://kb.jorisjonkers.dev}"
case "${KB_URL}" in
  */mcp) KB_MCP_URL="${KB_URL}" ;;
  *) KB_MCP_URL="${KB_URL%/}/mcp" ;;
esac

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

[ "${#prompt}" -lt "${KB_RECALL_MIN_PROMPT_CHARS:-40}" ] && exit 0

project="$(git remote get-url origin 2>/dev/null | sed -e 's#\.git$##' -e 's#.*[/:]##')"
[ -n "${project}" ] || project="$(basename "$(git rev-parse --show-toplevel 2>/dev/null || pwd)")"
scope="${KB_RECALL_SCOPE:-project:${project}}"

# Adaptive mode: short prompts rarely need semantic search.
prompt_len="${#prompt}"
if [ -n "${KB_RECALL_HOOK_MODE:-}" ]; then
  mode="${KB_RECALL_HOOK_MODE}"
elif [ "${prompt_len}" -lt 80 ]; then
  mode="fast"
else
  mode="hybrid"
fi
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
  payload="$(recall_payload "$1" "$2" "$3" "$4")" || return 1
  curl -sS --connect-timeout 3 --max-time 5 \
    -H "Authorization: Bearer ${KB_BEARER_TOKEN}" \
    -H "Content-Type: application/json" \
    -d "${payload}" \
    "${KB_MCP_URL}" 2>/dev/null
}

response="$(call_recall "${prompt}" "${limit}" "${mode}" "${scope}")" || response=""
if [ -z "${response}" ] && [ "${mode}" != "fast" ]; then
  response="$(call_recall "${prompt}" "${limit}" fast "${scope}")" || exit 0
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
print("## Knowledge base context")
print()
for h in hits[:3]:
    title = h.get("title", "")
    scope = h.get("scope", "")
    note_id = h.get("id", "")
    score = h.get("score", 0)
    print(f"- **{title}** (`{scope}`, score {score:.2f}) - id `{note_id}`")
    snip = (h.get("snippet") or "").replace("\n", " ").strip()
    if snip:
        print(f"  > {snip[:160]}")
' 2>/dev/null || true
