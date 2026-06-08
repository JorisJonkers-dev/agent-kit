#!/usr/bin/env bash
# Claude PreToolUse hook for Edit/Write/MultiEdit. Recall prior captures
# for the target file/module before the edit lands. Silent on KB failure.

set -u

[ "${KB_AUTO_MCP_DISABLED:-0}" = 1 ] && exit 0
[ -z "${KB_BEARER_TOKEN:-}" ] && exit 0

KB_URL="${KB_URL:-https://kb.jorisjonkers.dev}"
case "${KB_URL}" in
  */mcp) KB_MCP_URL="${KB_URL}" ;;
  *) KB_MCP_URL="${KB_URL%/}/mcp" ;;
esac

STATE_DIR="${HOME}/.claude/state"
ALLOWLIST="${HOME}/.claude/.knowledge-system-allowlist"

input="$(cat 2>/dev/null || true)"
file_path="$(printf '%s' "${input}" | python3 -c '
import json, sys
try:
    data = json.load(sys.stdin)
    inp = data.get("tool_input") or {}
    print(inp.get("file_path") or inp.get("filePath") or inp.get("path") or "", end="")
except Exception:
    pass
' 2>/dev/null || true)"
[ -z "${file_path}" ] && exit 0

if [ -r "${ALLOWLIST}" ]; then
  if python3 - "${ALLOWLIST}" "${file_path}" <<'PY' 2>/dev/null
import fnmatch
import os
import sys

allowlist, path = sys.argv[1], sys.argv[2]
with open(allowlist) as f:
    for line in f:
        pattern = line.strip()
        if not pattern or pattern.startswith("#"):
            continue
        if fnmatch.fnmatch(path, pattern) or fnmatch.fnmatch(os.path.basename(path), pattern):
            sys.exit(0)
sys.exit(1)
PY
  then
    exit 0
  fi
fi

session="${CLAUDE_SESSION_ID:-unknown}"
mkdir -p "${STATE_DIR}/sessions/${session}"
path_hash="$(python3 -c 'import hashlib,sys; print(hashlib.sha1(sys.argv[1].encode()).hexdigest()[:12])' "${file_path}" 2>/dev/null || echo unknown)"
marker="${STATE_DIR}/sessions/${session}/edit-${path_hash}"
[ -e "${marker}" ] && exit 0
: > "${marker}"

project="$(git remote get-url origin 2>/dev/null | sed -e 's#\.git$##' -e 's#.*[/:]##')"
[ -n "${project}" ] || project="$(basename "$(git rev-parse --show-toplevel 2>/dev/null || pwd)")"
scope="${KB_RECALL_SCOPE:-project:${project}}"

basename="$(basename "${file_path}")"
parent="$(basename "$(dirname "${file_path}")")"
query="${basename} ${parent} ${file_path}"
mode="${KB_RECALL_HOOK_MODE:-hybrid}"
limit="${KB_RECALL_EDIT_LIMIT:-2}"

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

response="$(call_recall "${query}" "${limit}" "${mode}" "${scope}")" || response=""
if [ -z "${response}" ] && [ "${mode}" != "fast" ]; then
  response="$(call_recall "${query}" "${limit}" fast "${scope}")" || exit 0
fi
[ -n "${response}" ] || exit 0

printf '%s' "${response}" | python3 -c '
import json
import sys

try:
    data = json.load(sys.stdin)
    hits = data["result"]["structuredContent"]["hits"]
except Exception:
    sys.exit(0)

if not hits:
    sys.exit(0)

print()
print("## Related captures for this file")
for hit in hits[:2]:
    print(f"- **{hit.get(\"title\", \"\")}** (`{hit.get(\"scope\", \"\")}`) - id `{hit.get(\"id\", \"\")}`")
    snippet = (hit.get("snippet") or "").replace("\n", " ").strip()
    if snippet:
        print(f"  > {snippet[:160]}")
' 2>/dev/null || true
