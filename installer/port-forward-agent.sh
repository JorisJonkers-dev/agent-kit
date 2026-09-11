# shellcheck shell=bash
#
# Keeps a `kubectl port-forward` to a ClusterIP service alive on this
# workstation, so an MCP server registered at a loopback URL keeps answering.
#
# Sourced by the generated setup-workstation.sh, which provides log/ok/warn/
# fail and CHECK_ONLY. Not generated itself: it is plain shell with no
# registry input, and a separate file is what lets the tests drive it.
#
# macOS: a launchd user agent with KeepAlive. A bare `kubectl port-forward &`
# dies when the laptop sleeps, the network changes or the target pod restarts,
# and nothing brings it back, so the MCP server silently disappears from every
# session. launchd restarts the forward whenever kubectl exits.
#
# Elsewhere: the one-shot background forward, as before.
#
# The forward is pinned to the kube context that is current when setup runs.
# Without the pin, switching contexts later would point a "read-only cluster
# diagnostics" server at whichever cluster happened to be current.

AK_AGENT_PREFIX="dev.jorisjonkers.agent-kit.port-forward"
AK_LAUNCH_AGENTS_DIR="${AK_LAUNCH_AGENTS_DIR:-$HOME/Library/LaunchAgents}"
AK_LOG_DIR="${AK_LOG_DIR:-$HOME/Library/Logs/agent-kit}"

_pf_xml() {
  local s="$1"
  s="${s//&/&amp;}"
  s="${s//</&lt;}"
  s="${s//>/&gt;}"
  printf '%s' "$s"
}

_pf_answers() {
  curl -s -o /dev/null --max-time 1 "http://127.0.0.1:$1/" >/dev/null 2>&1
}

# _pf_plist <label> <log> <kubeconfig> <path> <argv...>
_pf_plist() {
  local label="$1" logfile="$2" kubeconfig="$3" path="$4"
  shift 4
  printf '%s\n' \
    '<?xml version="1.0" encoding="UTF-8"?>' \
    '<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">' \
    '<plist version="1.0">' \
    '<dict>' \
    '  <key>Label</key>' \
    "  <string>$(_pf_xml "$label")</string>" \
    '  <key>ProgramArguments</key>' \
    '  <array>'
  local arg
  for arg in "$@"; do
    printf '    <string>%s</string>\n' "$(_pf_xml "$arg")"
  done
  printf '%s\n' \
    '  </array>' \
    '  <key>EnvironmentVariables</key>' \
    '  <dict>' \
    '    <key>KUBECONFIG</key>' \
    "    <string>$(_pf_xml "$kubeconfig")</string>" \
    '    <key>PATH</key>' \
    "    <string>$(_pf_xml "$path")</string>" \
    '  </dict>' \
    '  <key>RunAtLoad</key>' \
    '  <true/>' \
    '  <key>KeepAlive</key>' \
    '  <true/>' \
    '  <key>ThrottleInterval</key>' \
    '  <integer>10</integer>' \
    '  <key>StandardOutPath</key>' \
    "  <string>$(_pf_xml "$logfile")</string>" \
    '  <key>StandardErrorPath</key>' \
    "  <string>$(_pf_xml "$logfile")</string>" \
    '</dict>' \
    '</plist>'
}

# ensure_port_forward <name> <namespace> <service> <local_port> <remote_port>
ensure_port_forward() {
  local name="$1" ns="$2" svc="$3" lport="$4" rport="$5"

  if ! command -v kubectl >/dev/null 2>&1; then
    warn "${name}: kubectl is not on PATH; no port-forward, registration will point at a dead port"
    return 0
  fi

  if [ "$(uname -s)" != "Darwin" ]; then
    if _pf_answers "${lport}"; then
      ok "${name}: port-forward already answering on 127.0.0.1:${lport}"
      return 0
    fi
    if [ "${CHECK_ONLY}" = 1 ]; then
      log "would start kubectl port-forward svc/${svc} ${lport}:${rport}"
      return 0
    fi
    warn "${name}: starting a one-shot kubectl port-forward svc/${svc} ${lport}:${rport} (not kept alive on this OS)"
    kubectl port-forward -n "${ns}" "svc/${svc}" --address 127.0.0.1 "${lport}:${rport}" >/dev/null 2>&1 &
    sleep 2
    _pf_answers "${lport}" || warn "${name}: port-forward did not come up; registration may fail"
    return 0
  fi

  local label="${AK_AGENT_PREFIX}.${name}"
  local plist="${AK_LAUNCH_AGENTS_DIR}/${label}.plist"
  local logfile="${AK_LOG_DIR}/port-forward-${name}.log"
  local domain kubectl_bin context
  domain="gui/$(id -u)"
  kubectl_bin="$(command -v kubectl)"
  context="$(kubectl config current-context 2>/dev/null || true)"
  if [ -z "${context}" ]; then
    warn "${name}: no current kube context; not installing the port-forward agent"
    return 0
  fi

  local desired
  desired="$(_pf_plist "${label}" "${logfile}" "${KUBECONFIG:-$HOME/.kube/config}" "${PATH}" \
    "${kubectl_bin}" port-forward --context "${context}" -n "${ns}" "svc/${svc}" \
    --address 127.0.0.1 "${lport}:${rport}")"

  local loaded=0
  launchctl print "${domain}/${label}" >/dev/null 2>&1 && loaded=1

  if [ "${loaded}" = 1 ] && [ -f "${plist}" ] && [ "$(cat "${plist}")" = "${desired}" ]; then
    ok "${name}: launchd agent ${label} already loaded (context ${context})"
    return 0
  fi

  if [ "${CHECK_ONLY}" = 1 ]; then
    log "would install launchd agent ${label}: kubectl port-forward svc/${svc} ${lport}:${rport} (context ${context})"
    return 0
  fi

  # Something other than our agent already holds the port, e.g. a forward
  # started by hand. The agent cannot bind until it exits; launchd keeps
  # retrying every ThrottleInterval, so it takes over on its own afterwards.
  if [ "${loaded}" = 0 ] && _pf_answers "${lport}"; then
    warn "${name}: 127.0.0.1:${lport} is held by another process; the agent takes over once that exits"
  fi

  mkdir -p "${AK_LAUNCH_AGENTS_DIR}" "${AK_LOG_DIR}"
  if [ "${loaded}" = 1 ]; then
    launchctl bootout "${domain}/${label}" >/dev/null 2>&1 || true
  fi
  printf '%s\n' "${desired}" > "${plist}"
  if ! launchctl bootstrap "${domain}" "${plist}"; then
    fail "${name}: launchctl bootstrap ${plist} failed"
    return 0
  fi

  local _
  for _ in 1 2 3 4 5 6 7 8 9 10; do
    if _pf_answers "${lport}"; then
      ok "${name}: launchd agent ${label} forwarding 127.0.0.1:${lport} (context ${context})"
      return 0
    fi
    sleep 1
  done
  warn "${name}: agent ${label} loaded but 127.0.0.1:${lport} is not answering yet; see ${logfile}"
}
