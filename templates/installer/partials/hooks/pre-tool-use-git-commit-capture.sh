#!/usr/bin/env bash
# PreToolUse hook for Bash. Fires only when the command looks like a
# `git commit -m "..."` (or heredoc variant). Captures the commit
# message as a `decision` note scoped to the current repo, with a
# distinct source so the operator can bulk-revoke if needed.
#
# Skips merge / fixup / WIP commits — too noisy to capture.

set -u

[ "${KB_AUTO_MCP_DISABLED:-0}" = 1 ] && exit 0
[ -z "${KB_BEARER_TOKEN:-}" ] && exit 0

KB_URL="${KB_URL:-@KB_URL@}"
case "${KB_URL}" in
  */mcp) KB_MCP_URL="${KB_URL}" ;;
  *) KB_MCP_URL="${KB_URL%/}/mcp" ;;
esac

input=$(cat 2>/dev/null || true)
parsed=$(printf '%s' "${input}" | python3 -c '
import json, sys
try:
    data = json.load(sys.stdin)
except Exception:
    sys.exit(0)

tool = data.get("tool_name") or data.get("name") or ""
raw_tool = data.get("tool")
if isinstance(raw_tool, str):
    tool = tool or raw_tool
elif isinstance(raw_tool, dict):
    tool = tool or raw_tool.get("name") or ""

inputs = []
for key in ("tool_input", "input", "arguments", "params"):
    value = data.get(key)
    if isinstance(value, dict):
        inputs.append(value)
if isinstance(raw_tool, dict):
    for key in ("input", "arguments", "params"):
        value = raw_tool.get(key)
        if isinstance(value, dict):
            inputs.append(value)

command = ""
for source in inputs + [data]:
    if not isinstance(source, dict):
        continue
    for key in ("command", "cmd", "script", "shell_command", "shellCommand"):
        value = source.get(key)
        if isinstance(value, str) and value.strip():
            command = value.strip()
            break
    if command:
        break

print(f"{tool}\x1f{command}", end="")' 2>/dev/null || true)
tool="${parsed%%$'\x1f'*}"
command="${parsed#*$'\x1f'}"
[ "${tool}" = "${parsed}" ] && command=""
if [ -n "${tool}" ] && [ "${tool}" != "Bash" ] && [ "${tool}" != "bash" ]; then
  exit 0
fi

# Match `git commit -m "..."` shape; both single and double quotes.
case "${command}" in
  *"git commit"*"-m"*) : ;;
  *) exit 0 ;;
esac
# Skip noise: merge, fixup, WIP.
case "${command}" in
  *"fixup!"*|*"WIP"*|*"wip"*|*"Merge "*) exit 0 ;;
esac

# Extract the message: everything between the first matching quote
# pair after `-m`. Defer the regex to python for sanity.
title=$(printf '%s' "${command}" | python3 -c '
import re, sys
cmd = sys.stdin.read()
m = re.search(r"-m\s+(\x27([^\x27]+)\x27|\"([^\"]+)\")", cmd)
if not m: sys.exit(0)
print(m.group(2) or m.group(3) or "", end="")' 2>/dev/null)
[ -z "${title}" ] && exit 0

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

scope="$(canonical_project_scope_from_origin)"

body=$(cat <<BODY
Commit message: ${title}

Captured automatically by ${KB_AUTO_MCP_CLIENT_NAME:-the Claude PreToolUse} \`git commit\` hook. The
diff and surrounding context live in git history.
BODY
)

source="${KB_AUTO_MCP_SOURCE:-claude-code:auto-capture:git-commit}"
payload=$(python3 -c 'import json,sys
args = {
  "title": sys.argv[1],
  "body": sys.argv[2],
  "source": sys.argv[4],
  "tags": ["auto-capture","git-commit"]
}
if sys.argv[3]:
    args["scope"] = sys.argv[3]
print(json.dumps({
  "jsonrpc":"2.0","id":1,"method":"tools/call","params":{
    "name":"knowledge.capture_decision","arguments":args}}))' "${title}" "${body}" "${scope}" "${source}")

curl -sS --connect-timeout 3 --max-time 5 \
  -H "Authorization: Bearer ${KB_BEARER_TOKEN}" \
  -H "Content-Type: application/json" \
  -d "${payload}" \
  "${KB_MCP_URL}" >/dev/null 2>&1 || true
