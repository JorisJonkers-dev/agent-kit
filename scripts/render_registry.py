#!/usr/bin/env python3
"""Render the estate tooling registry into its generated artifacts.

One file is edited by hand -- ``registry/estate-tooling.yaml``. Everything
below is generated from it:

* ``installer/setup-workstation.sh``            -- local Claude Code + Codex
* ``registry/generated/hermes/skills-sources.conf`` -- Hermes ``sources.conf``
* ``registry/generated/hermes/mcp-servers.yaml``    -- Hermes ``mcp_servers:``

``--check`` renders into memory and compares, so CI fails on a hand edit to a
generated file or on a registry change that was never rendered. ``--write``
writes them.
"""

from __future__ import annotations

import argparse
import sys
from pathlib import Path
from typing import Any

import yaml

KIT_ROOT = Path(__file__).resolve().parent.parent
REGISTRY_PATH = KIT_ROOT / "registry" / "estate-tooling.yaml"

SETUP_SCRIPT = Path("installer/setup-workstation.sh")
HERMES_SOURCES = Path("registry/generated/hermes/skills-sources.conf")
HERMES_MCP = Path("registry/generated/hermes/mcp-servers.yaml")

GENERATED_BANNER = "GENERATED FROM registry/estate-tooling.yaml -- DO NOT EDIT."


class RegistryError(RuntimeError):
    """The registry is malformed in a way that would render a broken artifact."""


# ---------------------------------------------------------------------------
# Loading and validation
# ---------------------------------------------------------------------------


def load_registry(path: Path = REGISTRY_PATH) -> dict[str, Any]:
    data = yaml.safe_load(path.read_text())
    if not isinstance(data, dict):
        raise RegistryError(f"{path} must be a mapping")
    validate(data)
    return data


def _entries(data: dict[str, Any], key: str) -> list[dict[str, Any]]:
    value = data.get(key) or []
    if not isinstance(value, list):
        raise RegistryError(f"{key} must be a list")
    for item in value:
        if not isinstance(item, dict):
            raise RegistryError(f"{key} entries must be mappings")
    return value


def validate(data: dict[str, Any]) -> None:
    known_surfaces = {"workstation", "hermes", "runner"}

    marketplaces = {m["name"] for m in _entries(data, "marketplaces")}
    plugin_names = {p["name"] for p in _entries(data, "plugins")}

    for plugin in _entries(data, "plugins"):
        market = plugin.get("marketplace")
        if market not in marketplaces:
            raise RegistryError(
                f"plugin {plugin.get('name')} names unknown marketplace {market!r}",
            )

    # A language server has two halves and a session cannot tell which one is
    # missing, so refuse a plugin reference that no marketplace can satisfy.
    for server in _entries(data, "language_servers"):
        lsp_plugin = server.get("plugin")
        if not lsp_plugin or not str(lsp_plugin).endswith("-lsp"):
            raise RegistryError(f"language server {lsp_plugin!r} must name a *-lsp plugin")
        if not server.get("binary"):
            raise RegistryError(f"language server {lsp_plugin} must name its binary")
        if lsp_plugin in plugin_names:
            raise RegistryError(
                f"{lsp_plugin} is listed under both plugins: and language_servers:; "
                "keep LSP plugins in language_servers: only",
            )

    for source in _entries(data, "skill_sources"):
        name = source.get("name")
        commit = str(source.get("commit") or "")
        if len(commit) != 40 or not all(c in "0123456789abcdef" for c in commit):
            raise RegistryError(
                f"skill source {name} must pin a full 40-char commit sha, not {commit!r}. "
                "Peel annotated tags: git ls-remote <repo> 'refs/tags/<tag>^{}'",
            )
        if not source.get("ref"):
            raise RegistryError(f"skill source {name} must name a ref")
        if not source.get("selector"):
            raise RegistryError(f"skill source {name} must name a selector")

    for server in _entries(data, "mcp_servers"):
        name = server.get("name")
        transport = server.get("transport")
        if transport not in {"http", "stdio"}:
            raise RegistryError(f"mcp server {name} transport must be http or stdio")
        if transport == "stdio" and not server.get("command"):
            raise RegistryError(f"stdio mcp server {name} must name a command")

    for skill in _entries(data, "local_skills"):
        path = KIT_ROOT / str(skill.get("path") or "")
        if not (path / "SKILL.md").is_file():
            raise RegistryError(
                f"local skill {skill.get('name')} has no SKILL.md at {skill.get('path')}",
            )
        if "hermes" in (skill.get("surfaces") or []):
            raise RegistryError(
                f"local skill {skill.get('name')} cannot target hermes: sync-skills clones "
                "public git URLs with no credential and this repository is private",
            )

    for key in ("clis", "skill_sources", "mcp_servers", "local_skills"):
        for item in _entries(data, key):
            surfaces = item.get("surfaces")
            if surfaces is None:
                raise RegistryError(f"{key} entry {item.get('name')} must declare surfaces")
            unknown = set(surfaces) - known_surfaces
            if unknown:
                raise RegistryError(
                    f"{key} entry {item.get('name')} names unknown surfaces {sorted(unknown)}",
                )


def on_surface(item: dict[str, Any], surface: str) -> bool:
    return surface in (item.get("surfaces") or [])


# ---------------------------------------------------------------------------
# Hermes: sources.conf
# ---------------------------------------------------------------------------


def render_hermes_sources(data: dict[str, Any]) -> str:
    lines = [
        f"# {GENERATED_BANNER}",
        "#",
        "# Copy this file's body into the `sources.conf` key of",
        "# fleet-infra cluster/flux/apps/agents/hermes/skills-configmap.yaml,",
        "# or let scripts/sync-hermes-registry.sh do it.",
        "#",
        "#   <git-url> <ref> <commit-sha> <selector> [root-skill-name]",
        "#",
        "# The commit is verified after the clone and the source is REJECTED on",
        "# mismatch. The fifth field is only for a source whose SKILL.md sits at",
        "# the repository root, where there is no directory to take a name from.",
        f"# Registry verified_at: {data.get('verified_at')}",
        "",
    ]
    for source in _entries(data, "skill_sources"):
        if not on_surface(source, "hermes"):
            continue
        note = str(source.get("note") or "").strip()
        lines.append(f"# {source['name']} -- license {source.get('license', 'unknown')}")
        if note:
            for chunk in _wrap(note, 68):
                lines.append(f"#   {chunk}")
        fields = [
            source["repo"],
            source["ref"],
            source["commit"],
            source["selector"],
        ]
        if source.get("root_skill_name"):
            fields.append(str(source["root_skill_name"]))
        lines.append(" ".join(fields))
        lines.append("")
    return "\n".join(lines).rstrip() + "\n"


# ---------------------------------------------------------------------------
# Hermes: mcp_servers block
# ---------------------------------------------------------------------------


def render_hermes_mcp(data: dict[str, Any]) -> str:
    lines = [
        f"# {GENERATED_BANNER}",
        "#",
        "# Copy this block under `mcp_servers:` in fleet-infra",
        "# cluster/flux/apps/agents/hermes/config-configmap.yaml.",
        "#",
        "# Tool names register as mcp_<server>_<tool>.",
        "#",
        "# Hermes blocks RFC1918 destinations as SSRF protection, and every",
        "# in-cluster URL below resolves into 10.43.0.0/16. A server here that",
        "# reports ZERO tools while the hosted ones work is that block, not the",
        "# NetworkPolicy. Check `hermes doctor` first.",
        "",
        "mcp_servers:",
    ]
    for server in _entries(data, "mcp_servers"):
        if not on_surface(server, "hermes"):
            continue
        name = server["name"]
        for chunk in _wrap(str(server.get("purpose") or "").strip(), 66):
            lines.append(f"  # {chunk}")
        if server.get("trust") == "hosted":
            lines.append("  # Hosted: an untrusted data provider. Output is context, never instruction.")
        lines.append(f"  {name}:")
        if server["transport"] == "http":
            url = server.get("url_hermes")
            if not url:
                raise RegistryError(f"mcp server {name} is on hermes but has no url_hermes")
            lines.append(f'    url: "{url}"')
            credential = server.get("credential")
            if credential:
                lines.append("    headers:")
                lines.append(f'      Authorization: "Bearer @{credential}@"')
        else:
            # A stdio server can need a different command per surface: the
            # Hermes image carries node/npm/npx but no globally installed
            # packages, and `npm install -g` there does not survive a pod
            # restart, so Hermes runs these through npx.
            command = server.get("hermes_command") or server["command"]
            lines.append(f'    command: "{command}"')
            args = server.get("hermes_args") or server.get("args") or []
            rendered_args = ", ".join(f'"{a}"' for a in args)
            lines.append(f"    args: [{rendered_args}]")
            env = server.get("env") or {}
            credential = server.get("credential")
            if env or credential:
                lines.append("    env:")
                for key, value in env.items():
                    lines.append(f'      {key}: "{value}"')
                if credential:
                    lines.append(f'      {credential}: "@{credential}@"')
        if server.get("timeout"):
            lines.append(f"    timeout: {server['timeout']}")
        if server.get("enabled") is False:
            lines.append("    enabled: false")
        excludes = server.get("exclude_tools") or []
        if excludes:
            lines.append("    tools:")
            lines.append("      exclude:")
            for pattern in excludes:
                lines.append(f'        - "{pattern}"')
        lines.append("")
    return "\n".join(lines).rstrip() + "\n"


# ---------------------------------------------------------------------------
# Workstation setup script
# ---------------------------------------------------------------------------


def render_setup_script(data: dict[str, Any]) -> str:
    out: list[str] = []
    w = out.append

    w("#!/usr/bin/env bash")
    w(f"# {GENERATED_BANNER}")
    w("#")
    w("# Estate workstation setup: Claude Code and Codex, their plugins, the")
    w("# language servers those plugins need, and the MCP fleet.")
    w("#")
    w("# Idempotent. Every step reports what it found rather than what it ran,")
    w("# because an installed plugin whose binary is absent looks identical to")
    w("# a working one from inside a session.")
    w("#")
    w("# Usage:")
    w("#   ./setup-workstation.sh                 install/upgrade everything")
    w("#   ./setup-workstation.sh --check         report only, change nothing")
    w("#   ./setup-workstation.sh --no-lsp        skip the language servers")
    w("#   ./setup-workstation.sh --no-mcp        skip MCP registration")
    w("#")
    w("# Secrets are read from the environment and never written here:")
    for server in _entries(data, "mcp_servers"):
        if on_surface(server, "workstation") and server.get("credential"):
            w(f"#   {server['credential']}  -> the {server['name']} MCP server")
    w("")
    w("set -uo pipefail")
    w("")
    w("CHECK_ONLY=0")
    w("DO_LSP=1")
    w("DO_MCP=1")
    w("failures=0")
    w("warnings=0")
    w("")
    w('while [ "$#" -gt 0 ]; do')
    w("  case \"$1\" in")
    w("    --check) CHECK_ONLY=1 ;;")
    w("    --no-lsp) DO_LSP=0 ;;")
    w("    --no-mcp) DO_MCP=0 ;;")
    w("    --help|-h) sed -n '2,30p' \"$0\"; exit 0 ;;")
    w('    *) echo "unknown option: $1" >&2; exit 64 ;;')
    w("  esac")
    w("  shift")
    w("done")
    w("")
    w("log()  { printf 'setup: %s\\n' \"$*\"; }")
    w("ok()   { printf 'setup:   ok    %s\\n' \"$*\"; }")
    w("warn() { printf 'setup:   warn  %s\\n' \"$*\" >&2; warnings=$((warnings + 1)); }")
    w("fail() { printf 'setup:   FAIL  %s\\n' \"$*\" >&2; failures=$((failures + 1)); }")
    w("")
    w("run() {")
    w('  if [ "${CHECK_ONLY}" = 1 ]; then')
    w('    log "would run: $*"')
    w("    return 0")
    w("  fi")
    w('  "$@"')
    w("}")
    w("")
    w("# Runs argv, and in --check mode prints only the first argument as a")
    w("# description. Used for the MCP registrations, whose arguments carry")
    w("# credentials: `run` would echo the secret in its `would run:` line.")
    w("run_redacted() {")
    w("  local what=\"$1\"; shift")
    w('  if [ "${CHECK_ONLY}" = 1 ]; then')
    w('    log "would run: ${what}"')
    w("    return 0")
    w("  fi")
    w('  "$@"')
    w("}")
    w("")
    w("# A shell -c wrapper for the pipe-into-sh installers the upstreams publish.")
    w("run_sh() {")
    w('  if [ "${CHECK_ONLY}" = 1 ]; then')
    w('    log "would run: $1"')
    w("    return 0")
    w("  fi")
    w('  bash -c "$1"')
    w("}")
    w("")
    w("# This script lives in <kit>/installer, and the first-party skills it")
    w("# copies live in <kit>/skills.")
    w('KIT_ROOT="$(cd "$(dirname "$0")/.." && pwd)"')
    w('CLAUDE_HOME="${CLAUDE_CONFIG_DIR:-$HOME/.claude}"')
    w('CODEX_HOME="${CODEX_HOME:-$HOME/.codex}"')
    w("")

    # --- CLIs ---
    w("# -----------------------------------------------------------------")
    w("# 1. Command-line tools.")
    w("# -----------------------------------------------------------------")
    w('log "command-line tools"')
    for cli in _entries(data, "clis"):
        if not on_surface(cli, "workstation"):
            continue
        binary = cli["binary"]
        install = str(cli.get("install") or "")
        update = str(cli.get("update") or install)
        if not install:
            raise RegistryError(f"cli {cli.get('name')} must name an install command")
        version_command = cli.get("version_command")
        w("")
        for chunk in _wrap(str(cli.get("purpose") or "").strip(), 64):
            w(f"# {chunk}")
        w(f'if command -v {binary} >/dev/null 2>&1; then')
        w(f'  run_sh {_q(update)}')
        if version_command:
            w(f'  ok "{binary} $({version_command} 2>/dev/null | head -1)"')
        else:
            w(f'  ok "{binary} present"')
        w("else")
        w(f'  run_sh {_q(install)}')
        w('  if [ "${CHECK_ONLY}" = 1 ]; then')
        w(f'    log "{binary} would be installed"')
        w(f'  elif command -v {binary} >/dev/null 2>&1; then')
        w(f'    ok "{binary} installed"')
        w("  else")
        w(f'    fail "{binary} still not on PATH after install; open a new shell and re-run"')
        w("  fi")
        w("fi")
    w("")

    # --- Marketplaces ---
    w("# -----------------------------------------------------------------")
    w("# 2. Claude Code marketplaces and plugins.")
    w("# -----------------------------------------------------------------")
    w('if ! command -v claude >/dev/null 2>&1; then')
    w('  fail "claude is not on PATH; skipping plugins, language servers and MCP"')
    w("else")
    w('  log "plugin marketplaces"')
    for market in _entries(data, "marketplaces"):
        w(f'  run claude plugin marketplace add {market["repo"]} 2>/dev/null \\')
        w(f'    || run claude plugin marketplace update {market["name"]} >/dev/null 2>&1 \\')
        w(f'    || warn "marketplace {market["name"]} ({market["repo"]}) could not be added or updated"')
    w("")
    w('  log "plugins"')
    for plugin in _entries(data, "plugins"):
        ref = f'{plugin["name"]}@{plugin["marketplace"]}'
        w(f'  # {plugin["name"]}: ' + " ".join(str(plugin.get("purpose") or "").split())[:200])
        w(f'  if run claude plugin install {ref} --yes --scope user; then')
        w(f'    run claude plugin update {plugin["name"]} >/dev/null 2>&1 || true')
        if plugin.get("enabled", True):
            w(f'    run claude plugin enable {ref} >/dev/null 2>&1 || true')
            w(f'    ok "{ref} installed and enabled"')
        else:
            w(f'    run claude plugin disable {ref} >/dev/null 2>&1 || true')
            w(f'    ok "{ref} installed, left disabled on purpose"')
        w("  else")
        w(f'    warn "{ref} install failed"')
        w("  fi")
    w("")

    # --- Language servers ---
    w("  # ---------------------------------------------------------------")
    w("  # 3. Language servers: the plugin AND the binary it drives.")
    w("  #")
    w("  # An LSP plugin with no binary on PATH registers no tools and says")
    w("  # nothing about it, so the binary is what gets verified here.")
    w("  # ---------------------------------------------------------------")
    w('  if [ "${DO_LSP}" = 1 ]; then')
    w('    log "language servers"')
    for server in _entries(data, "language_servers"):
        plugin = server["plugin"]
        binary = server["binary"]
        # `install: null` is meaningful: the binary ships with its platform
        # toolchain and there is nothing to install separately.
        lsp_install: str | None = server.get("install") or None
        ref = f"{plugin}@claude-plugins-official"
        enabled = server.get("enabled", True)
        w("")
        w(f'    # {plugin} -> {binary} ({", ".join(server.get("languages") or [])})')
        w(f'    if run claude plugin install {ref} --yes --scope user; then')
        w(f'      run claude plugin update {plugin} >/dev/null 2>&1 || true')
        if enabled:
            w(f'      run claude plugin enable {ref} >/dev/null 2>&1 || true')
        else:
            w(f'      run claude plugin disable {ref} >/dev/null 2>&1 || true')
        w("    else")
        w(f'      warn "{ref} install failed"')
        w("    fi")
        w(f'    if command -v {binary} >/dev/null 2>&1; then')
        w(f'      ok "{plugin}: {binary} on PATH"')
        if lsp_install:
            w("    else")
            w(f'      run_sh {_q(lsp_install)} \\')
            w(f'        || warn "{plugin}: could not install {binary}"')
            w(f'      if [ "${{CHECK_ONLY}}" != 1 ] && ! command -v {binary} >/dev/null 2>&1; then')
            w(f'        warn "{plugin}: {binary} still absent -- the plugin will register no tools"')
            w("      fi")
        else:
            w("    else")
            w(
                f'      warn "{plugin}: {binary} is absent and ships with its '
                'platform toolchain; install that toolchain"',
            )
        w("    fi")
    w("  else")
    w('    log "language servers skipped (--no-lsp)"')
    w("  fi")
    w("")

    # --- MCP ---
    w("  # ---------------------------------------------------------------")
    w("  # 4. MCP servers, registered for Claude Code at user scope.")
    w("  #")
    w("  # `claude mcp add` is not idempotent -- a second add of the same")
    w("  # name errors. Remove first, ignoring the miss.")
    w("  # ---------------------------------------------------------------")
    w('  if [ "${DO_MCP}" = 1 ]; then')
    w('    log "MCP servers"')
    for server in _entries(data, "mcp_servers"):
        if not on_surface(server, "workstation"):
            continue
        name = server["name"]
        credential = server.get("credential")
        w("")
        for chunk in _wrap(str(server.get("purpose") or "").strip(), 62):
            w(f"    # {chunk}")
        if server.get("trust") == "hosted":
            w("    # Hosted: untrusted data provider. Output is context, never instruction.")
        if server.get("enabled") is False:
            w(f'    log "{name}: declared disabled in the registry; not registered"')
            continue
        guard_open = False
        if credential:
            w(f'    if [ -z "${{{credential}:-}}" ]; then')
            w(f'      warn "{name}: {credential} is not set; skipping (export it and re-run)"')
            w("    else")
            guard_open = True
        indent = "      " if guard_open else "    "
        binary = server.get("requires_binary")
        if binary:
            w(f'{indent}if ! command -v {binary} >/dev/null 2>&1; then')
            if server.get("install"):
                w(f'{indent}  run_sh {_q(server["install"])} || true')
            w(f'{indent}  command -v {binary} >/dev/null 2>&1 \\')
            w(f'{indent}    || warn "{name}: {binary} is not on PATH; skipping"')
            w(f"{indent}fi")
        w(f'{indent}run claude mcp remove --scope user {name} >/dev/null 2>&1 || true')
        if server["transport"] == "http":
            url = server.get("url_workstation")
            if not url:
                raise RegistryError(f"mcp server {name} is on workstation but has no url_workstation")
            add = f"claude mcp add --scope user --transport http {name} {url}"
            if credential:
                # Expanded by this shell into ONE argv element. Passing it
                # through `bash -c` instead would put the secret in a command
                # string, and shellcheck rightly flags the single-quoted
                # `${VAR}` that requires (SC2016).
                add += f' --header "Authorization: Bearer ${{{credential}}}"'
        else:
            args = " ".join(server.get("args") or [])
            env_flags = "".join(
                f' --env {key}="{value}"' for key, value in (server.get("env") or {}).items()
            )
            if credential:
                env_flags += f' --env {credential}="${{{credential}}}"'
            add = f"claude mcp add --scope user{env_flags} {name} -- {server['command']} {args}".rstrip()
        w(f'{indent}if run_redacted "claude mcp add {name}" {add}; then')
        w(f'{indent}  ok "{name} registered"')
        w(f"{indent}else")
        w(f'{indent}  fail "{name} registration failed"')
        w(f"{indent}fi")
        if guard_open:
            w("    fi")
    w("")
    w("    # Verify the VALUE, not the exit codes above: ask Claude what it")
    w("    # actually has registered and name anything expected but absent.")
    w('    if [ "${CHECK_ONLY}" != 1 ]; then')
    w("      registered=$(claude mcp list 2>/dev/null || true)")
    w("      for want in \\")
    expected = [
        server["name"]
        for server in _entries(data, "mcp_servers")
        if on_surface(server, "workstation") and server.get("enabled") is not False
    ]
    for name in expected:
        w(f"        {name} \\")
    w("        ; do")
    w('        case "${registered}" in')
    w('          *"${want}"*) ;;')
    w('          *) warn "MCP server ${want} is not in \\`claude mcp list\\` output" ;;')
    w("        esac")
    w("      done")
    w("    fi")
    w("  else")
    w('    log "MCP registration skipped (--no-mcp)"')
    w("  fi")
    w("fi")
    w("")

    # --- first-party skills ---
    w("# -----------------------------------------------------------------")
    w("# 5. First-party skills that ship in this repository.")
    w("#")
    w("# Copied, not fetched. Multi-file skills, so the whole tree moves and")
    w("# the destination is replaced rather than merged -- a stale script left")
    w("# behind from an older version is worse than a missing one.")
    w("# -----------------------------------------------------------------")
    local_skills = [s for s in _entries(data, "local_skills") if on_surface(s, "workstation")]
    if local_skills:
        w('log "first-party skills"')
    for skill in local_skills:
        name = skill["name"]
        source = skill["path"]
        w("")
        for chunk in _wrap(str(skill.get("purpose") or "").strip(), 64):
            w(f"# {chunk}")
        w(f'src="${{KIT_ROOT}}/{source}"')
        w('if [ ! -f "${src}/SKILL.md" ]; then')
        w(f'  fail "{name}: ${{src}}/SKILL.md is missing; nothing to install"')
        w("else")
        for agent in skill.get("agents") or ["claude"]:
            home = "CLAUDE_HOME" if agent == "claude" else "CODEX_HOME"
            w(f'  dest="${{{home}}}/skills/{name}"')
            w('  if [ "${CHECK_ONLY}" = 1 ]; then')
            w('    log "would replace ${dest}"')
            w("  else")
            w('    rm -rf "${dest}"')
            w('    mkdir -p "$(dirname "${dest}")"')
            w('    cp -R "${src}" "${dest}"')
            w("    # Verify the value: the copy is only useful if SKILL.md and")
            w("    # every script actually landed.")
            w('    if [ -f "${dest}/SKILL.md" ] \\')
            w('       && [ "$(find "${src}" -type f | wc -l)" = "$(find "${dest}" -type f | wc -l)" ]; then')
            w(f'      ok "{name} -> ${{dest}}"')
            w("    else")
            w(f'      fail "{name}: ${{dest}} is incomplete after copy"')
            w("    fi")
            w("  fi")
        w("fi")
    w("")

    # --- retired hooks ---
    w("# -----------------------------------------------------------------")
    w("# 6. Retired hooks.")
    w("#")
    w("# The estate ships no agent hooks. install-agents.sh owns the purge;")
    w("# this only reports a machine that still has them so the operator")
    w("# knows to run it.")
    w("# -----------------------------------------------------------------")
    w('settings="${CLAUDE_CONFIG_DIR:-$HOME/.claude}/settings.json"')
    retired_pattern = "|".join(
        (
            "pre-tool-use-edit-recall",
            "pre-tool-use-git-commit-capture",
            "stop-session-digest",
            "kb-stop-digest",
            "user-prompt-submit-recall",
        ),
    )
    w(f'if [ -f "${{settings}}" ] && grep -qE "{retired_pattern}" "${{settings}}"; then')
    w(
        '  warn "retired knowledge hooks are still wired in ${settings}; '
        'run install-agents.sh to purge them"',
    )
    w("else")
    w('  ok "no retired knowledge hooks in ${settings}"')
    w("fi")
    w("")
    w("# -----------------------------------------------------------------")
    w("log \"summary: ${failures} failure(s), ${warnings} warning(s)\"")
    w('[ "${failures}" = 0 ]')
    return "\n".join(out) + "\n"


def _q(command: str) -> str:
    """Single-quote a command for embedding in the generated bash."""
    return "'" + command.replace("'", "'\\''") + "'"


def _wrap(text: str, width: int) -> list[str]:
    if not text:
        return []
    words = text.split()
    lines: list[str] = []
    current = ""
    for word in words:
        candidate = f"{current} {word}".strip()
        if len(candidate) > width and current:
            lines.append(current)
            current = word
        else:
            current = candidate
    if current:
        lines.append(current)
    return lines


# ---------------------------------------------------------------------------
# Driver
# ---------------------------------------------------------------------------


def artifacts(data: dict[str, Any]) -> dict[Path, tuple[str, int]]:
    return {
        SETUP_SCRIPT: (render_setup_script(data), 0o755),
        HERMES_SOURCES: (render_hermes_sources(data), 0o644),
        HERMES_MCP: (render_hermes_mcp(data), 0o644),
    }


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    group = parser.add_mutually_exclusive_group(required=True)
    group.add_argument("--write", action="store_true", help="write the generated artifacts")
    group.add_argument("--check", action="store_true", help="fail if any artifact is stale")
    args = parser.parse_args(argv)

    try:
        data = load_registry()
    except RegistryError as exc:
        print(f"registry invalid: {exc}", file=sys.stderr)
        return 1

    stale: list[str] = []
    for relative, (content, mode) in artifacts(data).items():
        target = KIT_ROOT / relative
        current = target.read_text() if target.is_file() else None
        if args.check:
            if current != content:
                stale.append(str(relative))
            continue
        target.parent.mkdir(parents=True, exist_ok=True)
        if current != content:
            target.write_text(content)
        target.chmod(mode)

    if args.check:
        if stale:
            print(
                "registry render check failed; stale artifacts: " + ", ".join(stale),
                file=sys.stderr,
            )
            print("run: uv run python scripts/render_registry.py --write", file=sys.stderr)
            return 1
        print("registry render check passed")
        return 0

    print(f"rendered {len(artifacts(data))} registry artifacts")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
