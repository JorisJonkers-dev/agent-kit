#!/usr/bin/env bash
# Codex PreToolUse hook for Edit/Write/apply_patch. Recall prior captures
# for the target file/module before the edit lands. Silent on KB failure.

set -u

[ "${KB_AUTO_MCP_DISABLED:-0}" = 1 ] && exit 0
[ -z "${KB_BEARER_TOKEN:-}" ] && exit 0

KB_URL="${KB_URL:-http://knowledge-api.knowledge-system.svc.cluster.local:8080}"
case "${KB_URL}" in
  */mcp) KB_MCP_URL="${KB_URL}" ;;
  *) KB_MCP_URL="${KB_URL%/}/mcp" ;;
esac

CODEX_ROOT="${CODEX_HOME:-$HOME/.codex}"
STATE_DIR="${KB_AUTO_MCP_STATE_DIR:-${CODEX_ROOT}/state}"
ALLOWLIST="${KB_AUTO_MCP_ALLOWLIST:-${CODEX_ROOT}/.knowledge-system-allowlist}"

input="$(cat 2>/dev/null || true)"
file_path="$(printf '%s' "${input}" | python3 -c '
import json
import re
import sys

try:
    data = json.load(sys.stdin)
except Exception:
    sys.exit(0)

inputs = []
for key in ("tool_input", "input", "arguments", "params"):
    value = data.get(key)
    if isinstance(value, dict):
        inputs.append(value)

tool = data.get("tool")
if isinstance(tool, dict):
    for key in ("input", "arguments", "params"):
        value = tool.get(key)
        if isinstance(value, dict):
            inputs.append(value)

for source in inputs + [data]:
    for key in ("file_path", "filePath", "path", "target_file", "targetFile"):
        value = source.get(key) if isinstance(source, dict) else None
        if isinstance(value, str) and value.strip():
            print(value.strip(), end="")
            sys.exit(0)

patch_parts = []
for source in inputs + [data]:
    if not isinstance(source, dict):
        continue
    for key in ("patch", "content", "body", "diff"):
        value = source.get(key)
        if isinstance(value, str):
            patch_parts.append(value)
    value = source.get("input")
    if isinstance(value, str):
        patch_parts.append(value)

patch_text = "\n".join(patch_parts)
for line in patch_text.splitlines():
    match = re.match(r"^\*\*\* (?:Update|Add|Delete) File: (.+)$", line)
    if match:
        print(match.group(1).strip(), end="")
        sys.exit(0)
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

session="${CODEX_THREAD_ID:-${CODEX_SESSION_ID:-unknown}}"
mkdir -p "${STATE_DIR}/sessions/${session}"
path_hash="$(python3 -c 'import hashlib,sys; print(hashlib.sha1(sys.argv[1].encode()).hexdigest()[:12])' "${file_path}" 2>/dev/null || echo unknown)"
marker="${STATE_DIR}/sessions/${session}/edit-${path_hash}"
[ -e "${marker}" ] && exit 0
: > "${marker}"

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
