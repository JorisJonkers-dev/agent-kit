#!/usr/bin/env bash
# Codex PreToolUse hook for Bash. Capture deliberate git commit
# messages as decision notes. Silent on KB failure.

set -u

[ "${KB_AUTO_MCP_DISABLED:-0}" = 1 ] && exit 0
[ -z "${KB_BEARER_TOKEN:-}" ] && exit 0

KB_URL="${KB_URL:-http://knowledge-api.knowledge-system.svc.cluster.local:8080}"
case "${KB_URL}" in
  */mcp) KB_MCP_URL="${KB_URL}" ;;
  *) KB_MCP_URL="${KB_URL%/}/mcp" ;;
esac

input="$(cat 2>/dev/null || true)"
parsed="$(printf '%s' "${input}" | python3 -c '
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

print(f"{tool}\x1f{command}", end="")' 2>/dev/null || true)"
tool="${parsed%%$'\x1f'*}"
command="${parsed#*$'\x1f'}"
[ "${tool}" = "${parsed}" ] && command=""
if [ -n "${tool}" ] && [ "${tool}" != "Bash" ] && [ "${tool}" != "bash" ]; then
  exit 0
fi

case "${command}" in
  *"git commit"*"-m"*) : ;;
  *) exit 0 ;;
esac

case "${command}" in
  *"fixup!"*|*"WIP"*|*"wip"*|*"Merge "*) exit 0 ;;
esac

title="$(printf '%s' "${command}" | python3 -c '
import re, sys
cmd = sys.stdin.read()
match = re.search(r"-m\s+(\x27([^\x27]+)\x27|\"([^\"]+)\")", cmd)
if not match:
    sys.exit(0)
print(match.group(2) or match.group(3) or "", end="")
' 2>/dev/null)"
[ -z "${title}" ] && exit 0

project="$(git remote get-url origin 2>/dev/null | sed -e 's#\.git$##' -e 's#.*[/:]##')"
[ -n "${project}" ] || project="$(basename "$(git rev-parse --show-toplevel 2>/dev/null || pwd)")"
scope="project:${project}"

body="$(cat <<BODY
Commit message: ${title}

Captured automatically by ${KB_AUTO_MCP_CLIENT_NAME:-the Codex PreToolUse} \`git commit\` hook. The
diff and surrounding context live in git history.
BODY
)"

source="${KB_AUTO_MCP_SOURCE:-codex:auto-capture:git-commit}"
payload="$(python3 -c 'import json,sys; print(json.dumps({
  "jsonrpc":"2.0","id":1,"method":"tools/call","params":{
    "name":"knowledge.capture_decision","arguments":{
      "title": sys.argv[1],
      "body": sys.argv[2],
      "scope": sys.argv[3],
      "source": sys.argv[4],
      "tags": ["auto-capture","git-commit"]
    }}}))' "${title}" "${body}" "${scope}" "${source}")"

curl -sS --connect-timeout 3 --max-time 5 \
  -H "Authorization: Bearer ${KB_BEARER_TOKEN}" \
  -H "Content-Type: application/json" \
  -d "${payload}" \
  "${KB_MCP_URL}" >/dev/null 2>&1 || true
