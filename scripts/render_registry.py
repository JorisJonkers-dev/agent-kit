#!/usr/bin/env python3
"""Render the estate tooling registry into its generated artifacts.

One file is edited by hand -- ``registry/estate-tooling.yaml``. Everything
below is generated from it:

* ``installer/setup-workstation.sh``            -- local Claude Code + Codex + Hermes
* ``installer/setup-container.sh``              -- the agents image, at build time
* ``registry/generated/hermes/skills-sources.conf`` -- Hermes ``sources.conf``
* ``registry/generated/hermes/mcp-servers.yaml``    -- Hermes ``mcp_servers:`` (gateway)
* ``registry/generated/hermes/mcp-servers.local.yaml`` -- local Hermes ``mcp_servers:``

``--check`` renders into memory and compares, so CI fails on a hand edit to a
generated file or on a registry change that was never rendered. ``--write``
writes them.
"""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path
from typing import Any

import yaml

KIT_ROOT = Path(__file__).resolve().parent.parent
REGISTRY_PATH = KIT_ROOT / "registry" / "estate-tooling.yaml"

SETUP_SCRIPT = Path("installer/setup-workstation.sh")
CONTAINER_SETUP_SCRIPT = Path("installer/setup-container.sh")
# Hand-written, sourced by SETUP_SCRIPT: keeps workstation_connect forwards alive.
PORT_FORWARD_HELPER = Path("installer/port-forward-agent.sh")
HERMES_SOURCES = Path("registry/generated/hermes/skills-sources.conf")
HERMES_MCP = Path("registry/generated/hermes/mcp-servers.yaml")
HERMES_LOCAL_MCP = Path("registry/generated/hermes/mcp-servers.local.yaml")

GENERATED_BANNER = "GENERATED FROM registry/estate-tooling.yaml -- DO NOT EDIT."

# Paths under a Claude Code config root that hold ONE profile's own state.
# Sharing any of them between profiles merges the histories the profiles
# exist to keep apart, so a `shared_paths:` naming one is refused.
PROFILE_PRIVATE_PATHS = frozenset(
    {
        ".claude.json",
        ".credentials.json",
        "history.jsonl",
        "projects",
        "sessions",
        "shell-snapshots",
        "statsig",
        "todos",
    },
)


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
    known_surfaces = {"workstation", "hermes", "runner", "container"}

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

    # A plugin and a skill source can wrap the SAME upstream -- caveman is
    # both. When they do, one commit is reviewed and the other must not
    # disagree, or the workstation quietly tracks something Hermes rejects.
    reviewed_by_repo = {
        str(src.get("repo", "")).rstrip("/").rsplit("/", 1)[-1].lower(): src.get("commit")
        for src in _entries(data, "skill_sources")
    }
    for plugin in _entries(data, "plugins"):
        pin = plugin.get("commit")
        shared = reviewed_by_repo.get(str(plugin.get("name", "")).lower())
        if pin and shared and pin != shared:
            raise RegistryError(
                f"plugin {plugin.get('name')} pins {pin} but the skill source for the same "
                f"upstream pins {shared}; the reviewed commit is the one to use",
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

    _validate_claude_profiles(data)
    _validate_container(data)

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


def _validate_claude_profiles(data: dict[str, Any]) -> None:
    profiles = _entries(data, "claude_profiles")
    if not profiles:
        raise RegistryError("claude_profiles must declare at least the primary profile")

    names = [p.get("name") for p in profiles]
    if len(set(names)) != len(names):
        raise RegistryError("claude_profiles names must be unique")

    primaries = [p for p in profiles if p.get("primary")]
    if len(primaries) != 1:
        raise RegistryError("claude_profiles must declare exactly one primary profile")

    for profile in profiles:
        name = profile.get("name")
        # The name becomes the claude-<name> launcher, so it has to be a
        # plain command name.
        if not re.fullmatch(r"[a-z0-9][a-z0-9-]*", str(name or "")):
            raise RegistryError(
                f"claude profile name {name!r} must be lowercase letters, digits and dashes",
            )
        if not profile.get("config_dir"):
            raise RegistryError(f"claude profile {name} must name a config_dir")
        if profile.get("primary"):
            if profile.get("shares_from") or profile.get("shared_paths"):
                raise RegistryError(
                    f"claude profile {name} is the primary and shares from nothing",
                )
            continue

        # A secondary profile that shares nothing is a second install, not a
        # second login into the same setup -- and that is what the operator
        # asked the registry for.
        parent = profile.get("shares_from")
        if parent != primaries[0].get("name"):
            raise RegistryError(
                f"claude profile {name} must share from the primary profile "
                f"{primaries[0].get('name')!r}, not {parent!r}",
            )
        shared = profile.get("shared_paths") or []
        if not shared:
            raise RegistryError(f"claude profile {name} must name the paths it shares")
        for rel in shared:
            rel = str(rel)
            if rel.startswith("/") or ".." in Path(rel).parts:
                raise RegistryError(
                    f"claude profile {name} shares {rel!r}; shared paths are relative "
                    "to the config root and may not escape it",
                )
            if rel in PROFILE_PRIVATE_PATHS:
                raise RegistryError(
                    f"claude profile {name} shares {rel!r}, which is that profile's own "
                    "history or credentials; sharing it defeats the separate profile",
                )


CONTAINER_SOURCES = ("clis", "mcp_servers", "language_servers")


def _container_candidates(data: dict[str, Any]) -> list[tuple[str, dict[str, Any]]]:
    """Every registry entry that could carry a ``container:`` block, with its name."""
    found = []
    for key in CONTAINER_SOURCES:
        for item in _entries(data, key):
            found.append((str(item.get("name") or item.get("plugin")), item))
    return found


def _validate_container(data: dict[str, Any]) -> None:
    candidates = _container_candidates(data)
    blocks = {name: item["container"] for name, item in candidates if item.get("container")}

    for name, item in candidates:
        # Language servers declare no surfaces; for them the block alone decides.
        if "surfaces" in item and bool(item.get("container")) != on_surface(item, "container"):
            raise RegistryError(
                f"{name}: a container: block and the container surface go together",
            )

    for name, block in blocks.items():
        version = str(block.get("version") or "")
        if not version or version == "latest":
            raise RegistryError(f"container tool {name} must pin a version, not {version!r}")
        for field in ("datasource", "package", "install", "version_command"):
            if not block.get(field) and not (field == "version_command" and _item(data, name).get(field)):
                raise RegistryError(f"container tool {name} must name its {field}")
        if "${VERSION}" not in str(block["install"]):
            raise RegistryError(f"container tool {name} install must use ${{VERSION}}")
        for required in block.get("requires") or []:
            if required not in blocks:
                raise RegistryError(f"container tool {name} requires unknown tool {required!r}")


def _item(data: dict[str, Any], name: str) -> dict[str, Any]:
    return next(item for candidate, item in _container_candidates(data) if candidate == name)


def container_tools(data: dict[str, Any]) -> list[dict[str, Any]]:
    """The container tools in install order: registry order, after their requirements."""
    tools: dict[str, dict[str, Any]] = {}
    for name, item in _container_candidates(data):
        block = item.get("container")
        if not block:
            continue
        tools[name] = {
            "name": name,
            "binary": block.get("binary") or item.get("binary") or item.get("requires_binary") or name,
            "datasource": block["datasource"],
            "package": block["package"],
            "version": str(block["version"]),
            "install": " ".join(str(block["install"]).split()),
            "version_command": block.get("version_command") or item.get("version_command"),
            "requires": list(block.get("requires") or []),
        }

    ordered: list[dict[str, Any]] = []
    placed: set[str] = set()

    def place(name: str, trail: tuple[str, ...]) -> None:
        if name in placed:
            return
        if name in trail:
            raise RegistryError(f"container tools require each other in a cycle: {' -> '.join(trail)}")
        for required in tools[name]["requires"]:
            place(required, (*trail, name))
        placed.add(name)
        ordered.append(tools[name])

    for name in tools:
        place(name, ())
    return ordered


def claude_profiles(data: dict[str, Any]) -> list[dict[str, Any]]:
    """Every Claude Code profile, the primary first."""
    profiles = _entries(data, "claude_profiles")
    return sorted(profiles, key=lambda p: not p.get("primary"))


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


def render_hermes_local_mcp(data: dict[str, Any]) -> str:
    """Render the WORKSTATION-FLAVOURED ``mcp_servers:`` block for local Hermes.

    ``render_hermes_mcp`` targets the in-cluster gateway: it uses the
    ``hermes_command`` (npx, because the image has no global packages), the
    ``url_hermes`` and ``@CRED@`` placeholders for Vault substitution. Local
    Hermes on a workstation is different: ``olcli-mcp``/``npx`` sit on PATH, so
    the database ``command``/``args`` apply directly, and the target is the
    user's own ``~/.hermes/config.yaml`` rather than a ConfigMap. Credentials
    are rendered as ``${CRED}`` placeholders that Hermes interpolates at
    connect time from its secret scope / environment -- never written to disk.

    The block is merged into the local config by ``scripts/hermes-merge-mcp.py``
    (generated setup calls it once after registering the CLI agents).
    """
    lines = [
        f"# {GENERATED_BANNER}",
        "#",
        "# WORKSTATION surface for LOCAL Hermes (vs the in-cluster gateway, which",
        "# is registry/generated/hermes/mcp-servers.yaml). Merged into",
        "# ~/.hermes/config.yaml by scripts/hermes-merge-mcp.py.",
        "#",
        "mcp_servers:",
    ]
    for server in _entries(data, "mcp_servers"):
        if not on_surface(server, "workstation"):
            continue
        if server.get("enabled") is False or len(server.get("surfaces") or []) == 0:
            continue
        name = server["name"]
        lines.append(f"  {name}:")
        if server["transport"] == "http":
            url = server.get("url_workstation")
            if not url:
                raise RegistryError(f"mcp server {name} is on workstation but has no url_workstation")
            lines.append(f'    url: "{url}"')
        else:
            lines.append(f'    command: "{server["command"]}"')
            args = server.get("args") or []
            rendered_args = ", ".join(f'"{a}"' for a in args) if args else ""
            lines.append(f"    args: [{rendered_args}]")
            env = server.get("env") or {}
            credential = server.get("credential")
            if env or (credential and not server.get("credential_optional")):
                lines.append("    env:")
                for key, value in env.items():
                    lines.append(f'      {key}: "{value}"')
                if credential:
                    # Hermes interpolates ${VAR} from its secret scope / env at
                    # connect time; no plaintext secret is written to disk.
                    lines.append(f"      {credential}: \"${{{credential}}}\"")
        if server.get("timeout"):
            lines.append(f"    timeout: {server['timeout']}")
        lines.append("")
    return "\n".join(lines).rstrip() + "\n"


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
    w("#   ./setup-workstation.sh --no-profiles   only the primary Claude profile")
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
    w("DO_PROFILES=1")
    w("failures=0")
    w("warnings=0")
    w("")
    w("# Track what needs attention after the run")
    w("skipped_mcp_servers=()     # MCP servers skipped due to missing credentials")
    w("unshared_profile_paths=()  # Shared surfaces a secondary profile did not get")
    w("missing_lsp_binaries=()    # Language servers with missing binaries")
    w("disabled_plugins=()        # Plugins disabled (missing binary or on purpose)")
    w("plugin_drift=()            # Plugins whose commit drifted")
    w("new_binaries=()            # Binaries installed during this run")
    w("")
    w('while [ "$#" -gt 0 ]; do')
    w("  case \"$1\" in")
    w("    --check) CHECK_ONLY=1 ;;")
    w("    --no-lsp) DO_LSP=0 ;;")
    w("    --no-mcp) DO_MCP=0 ;;")
    w("    --no-profiles) DO_PROFILES=0 ;;")
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
    w("# Runs argv once per Claude profile, each with its own config root.")
    w("# MCP servers live in a profile's own .claude.json, so the fleet has to")
    w("# be registered per profile; plugins and skills do not, because those")
    w("# directories are shared by symlink.")
    w("claude_each_profile() {")
    w("  local dir rc=0")
    w('  for dir in "${CLAUDE_PROFILE_DIRS[@]}"; do')
    w('    CLAUDE_CONFIG_DIR="${dir}" "$@" || rc=1')
    w("  done")
    w('  return "${rc}"')
    w("}")
    w("")
    w("# Links one shared surface of the primary profile into a secondary one.")
    w("# A real file or directory already sitting at the destination is left")
    w("# alone: that is someone's own config, and a symlink cannot give back")
    w("# what replacing it would lose.")
    w("link_profile_path() {")
    w('  local primary="$1" secondary="$2" rel="$3"')
    w('  local src="${primary}/${rel}" dest="${secondary}/${rel}"')
    w("  # Linking a root onto itself replaces every shared surface with a")
    w("  # symlink to itself, and the profile stops loading anything.")
    w('  if [ "${primary}" = "${secondary}" ] || [ "${primary}" -ef "${secondary}" ]; then')
    w('    fail "profile: ${secondary} is the primary root; refusing to link it onto itself"')
    w("    return 0")
    w("  fi")
    w("  # A shared DIRECTORY that the primary does not have yet is created, so")
    w("  # the link exists before the thing it points at does and whatever")
    w("  # writes there later reaches both profiles. A shared FILE is not")
    w("  # invented: an empty settings.json would look like a real answer.")
    w('  if [ ! -e "${src}" ]; then')
    w('    case "${rel}" in')
    w('      *.*)')
    w('        warn "profile: ${src} does not exist yet; ${rel} not shared"')
    w('        unshared_profile_paths+=("${dest}: ${src} does not exist")')
    w("        return 0")
    w("        ;;")
    w('      *)')
    w('        if [ "${CHECK_ONLY}" = 1 ]; then')
    w('          log "would create ${src}"')
    w("        else")
    w('          mkdir -p "${src}"')
    w("        fi")
    w("        ;;")
    w("    esac")
    w("  fi")
    w('  if [ -L "${dest}" ] && [ "$(readlink "${dest}")" = "${src}" ]; then')
    w('    ok "profile: ${dest} -> ${src}"')
    w("    return 0")
    w("  fi")
    w('  if [ -e "${dest}" ] && [ ! -L "${dest}" ]; then')
    w('    warn "profile: ${dest} exists and is not a symlink; left alone"')
    w('    unshared_profile_paths+=("${dest}: real file or directory, not replaced")')
    w("    return 0")
    w("  fi")
    w('  if [ "${CHECK_ONLY}" = 1 ]; then')
    w('    log "would link ${dest} -> ${src}"')
    w("    return 0")
    w("  fi")
    w('  mkdir -p "$(dirname "${dest}")"')
    w('  if ln -sfn "${src}" "${dest}"; then')
    w('    ok "profile: ${dest} -> ${src}"')
    w("  else")
    w('    fail "profile: could not link ${dest} -> ${src}"')
    w("  fi")
    w("}")
    w("")
    w("# Writes the claude-<profile> launcher for one secondary profile, so the")
    w("# profile is a command rather than an environment variable to remember.")
    w("# A file at that path that this script did not write is left alone.")
    w("install_profile_launcher() {")
    w('  local name="$1" dir="$2"')
    w('  local bin_dir="${CLAUDE_LAUNCHER_DIR:-$HOME/.local/bin}"')
    w('  local launcher="${bin_dir}/claude-${name}"')
    w('  local marker="# managed by agent-kit setup-workstation.sh"')
    w("  local content")
    w("  content=\"$(printf '%s\\n' '#!/usr/bin/env bash' \"${marker}\" \\")
    w('    "# Claude Code with the ${name} profile config root." \\')
    w("    \"export CLAUDE_CONFIG_DIR=$(printf '%q' \"${dir}\")\" \\")
    w("    'exec claude \"$@\"')\"")
    w('  if [ -e "${launcher}" ] && ! grep -qxF "${marker}" "${launcher}"; then')
    w('    warn "profile: ${launcher} exists and was not written by setup; left alone"')
    w('    unshared_profile_paths+=("${launcher}: not a managed launcher, not replaced")')
    w("    return 0")
    w("  fi")
    w('  if [ -x "${launcher}" ] && [ "$(cat "${launcher}")" = "${content}" ]; then')
    w('    ok "profile: ${launcher}"')
    w("  elif [ \"${CHECK_ONLY}\" = 1 ]; then")
    w('    log "would write ${launcher}"')
    w("  elif mkdir -p \"${bin_dir}\" && printf '%s\\n' \"${content}\" > \"${launcher}\" \\")
    w('    && chmod 755 "${launcher}"; then')
    w('    ok "profile: wrote ${launcher}"')
    w("  else")
    w('    fail "profile: could not write ${launcher}"')
    w("    return 0")
    w("  fi")
    w('  case ":${PATH}:" in')
    w('    *":${bin_dir}:"*) ;;')
    w('    *) warn "profile: ${bin_dir} is not on PATH; claude-${name} will not be found" ;;')
    w("  esac")
    w("}")
    w("")
    w("# This script lives in <kit>/installer, and the first-party skills it")
    w("# copies live in <kit>/skills.")
    w('KIT_ROOT="$(cd "$(dirname "$0")/.." && pwd)"')
    w('CLAUDE_HOME="${CLAUDE_CONFIG_DIR:-$HOME/.claude}"')
    w('CODEX_HOME="${CODEX_HOME:-$HOME/.codex}"')
    w("")
    w("# ensure_port_forward: keeps the loopback of a workstation_connect MCP")
    w("# server alive (a launchd agent on macOS).")
    w(f'. "${{KIT_ROOT}}/{PORT_FORWARD_HELPER}"')
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
        provides = cli.get("provides_binaries") or [binary]
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
        # Track newly installed binaries
        for bin_name in provides:
            w(f'    new_binaries+=("{bin_name}")')
        w("  else")
        w(f'    fail "{binary} still not on PATH after install; open a new shell and re-run"')
        w("  fi")
        w("fi")
    w("")

    # --- Claude profiles ---
    #
    # Emitted BEFORE anything that writes into a config root: the secondary
    # profiles have to exist, and their shared surfaces have to be symlinks
    # into the primary, before plugins or MCP servers are written anywhere.
    w("# -----------------------------------------------------------------")
    w("# 2. Claude Code profiles.")
    w("#")
    w("# A profile is a config root: CLAUDE_CONFIG_DIR moves credentials AND")
    w("# conversation history together, so a second login is a second root.")
    w("# The primary keeps the default location; every secondary shares the")
    w("# surfaces named in the registry by symlinking back into it, and keeps")
    w("# its own projects/, sessions/, history.jsonl and .claude.json.")
    w("# -----------------------------------------------------------------")
    profiles = claude_profiles(data)
    primary = profiles[0]
    secondaries = profiles[1:]
    w(f'# {primary["name"]}: ' + " ".join(str(primary.get("purpose") or "").split())[:200])
    w('CLAUDE_PROFILE_DIRS=("${CLAUDE_HOME}")')
    w("")
    if secondaries:
        w('if [ "${DO_PROFILES}" = 1 ]; then')
        w('  log "claude profiles"')
        w('  ok "${CLAUDE_HOME} (primary)"')
        for profile in secondaries:
            name = profile["name"]
            config_dir = profile["config_dir"]
            w("")
            for chunk in _wrap(" ".join(str(profile.get("purpose") or "").split()), 62):
                w(f"  # {chunk}")
            w(f'  profile_dir="{config_dir}"')
            # Run from inside a secondary profile's session, CLAUDE_CONFIG_DIR
            # makes that secondary the primary too. Everything below would
            # then point the root at itself, so stop before touching it.
            w('  if [ "${profile_dir}" = "${CLAUDE_HOME}" ] || [ "${profile_dir}" -ef "${CLAUDE_HOME}" ]; then')
            w(f'    fail "{name}: CLAUDE_CONFIG_DIR points at ${{profile_dir}}; re-run with it unset"')
            w("  else")
            w('    if [ "${CHECK_ONLY}" = 1 ] && [ ! -d "${profile_dir}" ]; then')
            w('      log "would create ${profile_dir}"')
            w("    else")
            w('      mkdir -p "${profile_dir}"')
            w("    fi")
            for rel in profile["shared_paths"]:
                w(f'    link_profile_path "${{CLAUDE_HOME}}" "${{profile_dir}}" "{rel}"')
            w('    CLAUDE_PROFILE_DIRS+=("${profile_dir}")')
            w(f'    install_profile_launcher "{name}" "${{profile_dir}}"')
            # The login is the operator's to do: it is interactive, and the
            # whole point of the second root is that it holds a DIFFERENT
            # account, which this script has no way to choose.
            w("")
            w('    if [ -s "${profile_dir}/.claude.json" ]; then')
            w(f'      ok "{name}: ${{profile_dir}} is set up"')
            w("    else")
            w(f'      log "{name}: log in with  claude-{name}  (its own account)"')
            w("    fi")
            w("  fi")
        w("else")
        w('  log "secondary claude profiles skipped (--no-profiles)"')
        w("fi")
        w("")

    # --- Marketplaces ---
    w("# -----------------------------------------------------------------")
    w("# 3. Claude Code marketplaces and plugins.")
    w("#")
    w("# Installed into the PRIMARY profile only. plugins/ is a shared")
    w("# surface and enabledPlugins lives in the shared settings.json, so a")
    w("# second install per profile would write the same state twice.")
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
        # Do NOT run unconditional update - that causes drift. Only on explicit request.
        if plugin.get("enabled", True):
            w(f'    run claude plugin enable {ref} >/dev/null 2>&1 || true')
            w(f'    ok "{ref} installed and enabled"')
        else:
            w(f'    run claude plugin disable {ref} >/dev/null 2>&1 || true')
            w(f'    ok "{ref} installed, left disabled on purpose"')
            w(f'    disabled_plugins+=("{plugin["name"]}: disabled on purpose in registry")')
        w("  else")
        w(f'    warn "{ref} install failed"')
        w("  fi")
    w("")
    w("  # ---------------------------------------------------------------")
    w("  # 4. Plugin drift detection.")
    w("  #")
    w("  # The CLI does not support pinning plugins to a commit, so pins in")
    w("  # the registry are advisory. Compare installed commits against")
    w("  # registry expectations and report drift.")
    w("  # ---------------------------------------------------------------")
    if _has_plugin_pins(data):
        w('  manifest="${CLAUDE_HOME}/plugins/installed_plugins.json"')
        w('  if [ -f "${manifest}" ]; then')
        w('    log "checking plugin commit drift"')
        w("")
        w("    # The reader is a helper taking the manifest path and the plugin")
        w("    # ref as ARGV, with SINGLE-quoted python source. An earlier")
        w("    # version inlined double-quoted python inside a double-quoted")
        w("    # shell string, so the inner quotes closed the shell string and")
        w("    # python got mangled source -- and `2>/dev/null || true` hid the")
        w("    # SyntaxError, so every plugin silently reported no drift.")
        w("    installed_plugin_commit() {")
        w("      python3 -c '")
        w("import json, sys")
        w("try:")
        w("    manifest = json.load(open(sys.argv[1]))")
        w("except Exception:")
        w("    sys.exit(0)")
        w('installs = manifest.get("plugins", {}).get(sys.argv[2], [])')
        w("for install in installs:")
        w('    if install.get("scope") == "user":')
        w('        print(install.get("gitCommitSha", ""))')
        w("        break")
        w("' \\")
        w('        "$1" "$2" 2>/dev/null')
        w("    }")
        w("")
        for plugin in _entries(data, "plugins"):
            if not plugin.get("commit"):
                continue
            ref = f'{plugin["name"]}@{plugin["marketplace"]}'
            expected_commit = plugin["commit"]
            w(f'    installed_commit="$(installed_plugin_commit "${{manifest}}" "{ref}")"')
            w('    if [ -z "${installed_commit}" ]; then')
            w(f'      log "{plugin["name"]}: no user-scope commit recorded; drift not checked"')
            w(f'    elif [ "${{installed_commit}}" != "{expected_commit}" ]; then')
            w(f'      warn "{plugin["name"]}: installed {{installed_commit}} '
              f'differs from the registry\'s {expected_commit[:12]}"'.replace(
                  "{installed_commit}", "${installed_commit}"))
            w(f'      plugin_drift+=("{plugin["name"]}: ${{installed_commit}} vs {expected_commit[:12]}")')
            w("    else")
            w(f'      ok "{plugin["name"]}: at the expected commit"')
            w("    fi")
        w("  fi")
    w("")

    # --- Language servers ---
    w("  # ---------------------------------------------------------------")
    w("  # 5. Language servers: the plugin AND the binary it drives.")
    w("  #")
    w("  # An LSP plugin with no binary on PATH registers no tools and says")
    w("  # nothing about it. Install the plugin, but leave it disabled until")
    w("  # the binary is on PATH. This enables it on a second run once the")
    w("  # binary is installed.")
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
        # Always check binary presence to determine enable/disable state
        w(f'      if command -v {binary} >/dev/null 2>&1; then')
        if enabled:
            w(f'        run claude plugin enable {ref} >/dev/null 2>&1 || true')
            w(f'        ok "{plugin}: {binary} on PATH, plugin enabled"')
        else:
            w(f'        run claude plugin enable {ref} >/dev/null 2>&1 || true')
            w(f'        ok "{plugin}: {binary} on PATH, plugin enabled (was disabled in registry)"')
        w("      else")
        w(f'        run claude plugin disable {ref} >/dev/null 2>&1 || true')
        # Attempt install if one is configured
        if lsp_install:
            w(f'        run_sh {_q(lsp_install)} || true')
            w(f'        if command -v {binary} >/dev/null 2>&1; then')
            w(f'          run claude plugin enable {ref} >/dev/null 2>&1 || true')
            w(f'          ok "{plugin}: {binary} installed and enabled"')
            w("        else")
            escaped_install = lsp_install.replace('"', '\\"')
            w(f'          warn "{plugin}: {binary} is absent; plugin left disabled (install missing: {lsp_install})"')
            w(f'          missing_lsp_binaries+=("{plugin}: install with: {escaped_install}")')
            w("        fi")
        else:
            msg = (
                f'"{plugin}: {binary} is absent and ships with its platform '
                f'toolchain; plugin left disabled"'
            )
            w(f'        warn {msg}')
            w(f'        missing_lsp_binaries+=("{plugin}: missing from toolchain (no install command)")')
        w("      fi")
        w("    else")
        w(f'      warn "{ref} install failed"')
        w("    fi")
    w("  else")
    w('    log "language servers skipped (--no-lsp)"')
    w("  fi")
    w("")

    # --- MCP ---
    w("  # ---------------------------------------------------------------")
    w("  # 6. MCP servers, registered for Claude Code at user scope.")
    w("  #")
    w("  # The registry is authoritative: servers with `surfaces: []` or")
    w("  # `enabled: false` are removed. Others are ensured. Plugin-provided")
    w("  # servers (plugin:*:*) and hand-added ones are left untouched.")
    w("  #")
    w("  # Every claude call here runs once per profile: a server lives in the")
    w("  # profile's own .claude.json, which is the one file two profiles must")
    w("  # not share, so the fleet is registered into each of them.")
    w("  # ---------------------------------------------------------------")
    w('  if [ "${DO_MCP}" = 1 ]; then')
    w('    log "MCP servers"')
    # Remove first, and consider EVERY registry server regardless of surface.
    #
    # Filtering on `on_surface(..., "workstation")` before this pass was the
    # bug it was meant to fix: `knowledge` is retired with `surfaces: []`, so
    # it is on no surface, so it was skipped -- and it stayed registered and
    # CONNECTED to the knowledge base being retired, bearer token and all.
    # A retired server is precisely the one that reaches no surface.
    for server in _entries(data, "mcp_servers"):
        name = server["name"]
        # RETIRED only: `enabled: false` or no surfaces at all. A server that
        # is active for hermes or runner but not for the workstation is NOT
        # removed by name -- `github` and `kubernetes` are exactly that shape,
        # and a hand-added local server could share either name. The registry
        # never claimed those here, so it does not get to delete them.
        retired = server.get("enabled") is False or not (server.get("surfaces") or [])
        if not retired:
            continue
        w(f'    run claude_each_profile claude mcp remove --scope user {name} >/dev/null 2>&1 || true')
        w(f'    log "{name}: removed (retired in the registry)"')
    w("")
    # Now add/ensure active servers.
    #
    # Each workstation server is registered into every local agent that is
    # present: Claude Code, Codex and local Hermes. (Claude is installed
    # unconditionally in step 1 of this very script, so the surrounding
    # `command -v claude` guard never wrongfully drops codex/hermes on a
    # functioning run -- that guard exists as a defensive early-out, not a
    # gate that should decide codex's fate.)
    for server in _entries(data, "mcp_servers"):
        if not on_surface(server, "workstation"):
            continue
        name = server["name"]
        credential = server.get("credential")
        optional_cred = bool(server.get("credential_optional"))
        w("")
        for chunk in _wrap(str(server.get("purpose") or "").strip(), 62):
            w(f"    # {chunk}")
        if server.get("trust") == "hosted":
            w("    # Hosted: untrusted data provider. Output is context, never instruction.")
        if server.get("enabled") is False or len(server.get("surfaces") or []) == 0:
            # Already logged in removal section
            continue
        # A REQUIRED credential gates registration; an OPTIONAL one never does
        # (the server self-manages auth -- e.g. overleaf via olcli -- so an
        # unset override must not skip it, that was the #52 regression).
        guard_open = False
        if credential and not optional_cred:
            w(f'    if [ -z "${{{credential}:-}}" ]; then')
            w(f'      warn "{name}: {credential} is not set; skipping (export it and re-run)"')
            w(f'      skipped_mcp_servers+=("{name}: export {credential}")')
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
        # A workstation_connect block means the http url_workstation is a
        # loopback exposed by a `kubectl port-forward` of a ClusterIP service.
        # The registration below would otherwise point at a dead port, so
        # ensure the forward is up first. installer/port-forward-agent.sh
        # keeps it up afterwards too (a launchd agent on macOS).
        connect = server.get("workstation_connect")
        if connect:
            svc = connect["svc"]
            ns = connect["namespace"]
            rport = connect["remote_port"]
            lport = connect["local_port"]
            w("")
            w(f"    # {name} reaches the cluster via a kubectl port-forward of the")
            w(f"    # ClusterIP service {svc}.{ns} (no ingress route exists for it).")
            w(f"    ensure_port_forward {name} {ns} {svc} {lport} {rport}")
        # --- Claude Code ---
        w(f"{indent}if command -v claude >/dev/null 2>&1; then")
        w(f'{indent}  run claude_each_profile claude mcp remove --scope user {name} >/dev/null 2>&1 || true')
        if server["transport"] == "http":
            url = server.get("url_workstation")
            if not url:
                raise RegistryError(f"mcp server {name} is on workstation but has no url_workstation")
            add = f"claude mcp add --scope user {name} --transport http {url}"
        else:
            args = " ".join(server.get("args") or [])
            env_flags = _mcp_env_flags(server, credential, optional_cred)
            # Name BEFORE the --env flags: claude 2.x rejects the redeclared
            # order (`--env K=V` then the name) with "missing required argument
            # 'commandOrUrl'". Verified against claude 2.1.267.
            add = f"claude mcp add --scope user {name}{env_flags} -- {server['command']} {args}".rstrip()
        w(f'{indent}  if run_redacted "claude mcp add {name}" claude_each_profile {add}; then')
        w(f'{indent}    ok "{name} registered (claude)"')
        w(f"{indent}  else")
        w(f'{indent}    fail "{name} registration failed (claude)"')
        w(f"{indent}  fi")
        w(f"{indent}fi")
        # --- Codex ---
        w(f"{indent}if command -v codex >/dev/null 2>&1; then")
        w(f"{indent}  run codex mcp remove {name} >/dev/null 2>&1 || true")
        if server["transport"] == "http":
            url = server.get("url_workstation")
            add = f"codex mcp add {name} --url {url}".rstrip()
        else:
            args = " ".join(server.get("args") or [])
            env_flags = _mcp_env_flags(server, credential, optional_cred)
            add = f"codex mcp add {name}{env_flags} -- {server['command']} {args}".rstrip()
        w(f'{indent}  if run_redacted "codex mcp add {name}" {add}; then')
        w(f'{indent}    ok "{name} registered (codex)"')
        w(f"{indent}  else")
        w(f'{indent}    fail "{name} registration failed (codex)"')
        w(f"{indent}  fi")
        w(f"{indent}fi")
        if guard_open:
            w("    fi")
    w("")
    # --- local Hermes: merge the workstation-flavoured mcp_servers block ---
    w("    # Local Hermes reads its MCP servers from ~/.hermes/config.yaml.")
    w("    # `hermes mcp add` is interactive (probes + prompts), so the setup")
    w("    # script merges the generated workstation block via a helper instead.")
    w("    if command -v hermes >/dev/null 2>&1; then")
    w("      if [ \"${CHECK_ONLY}\" = 1 ]; then")
    w("        log \"would merge local Hermes MCP servers\"")
    w("      else")
    w("        uv run --directory \"${KIT_ROOT}\" python \\")
    w(f"          scripts/hermes-merge-mcp.py \"${{KIT_ROOT}}/{HERMES_LOCAL_MCP}\"")
    w("      fi")
    w("    else")
    w("      warn \"hermes not on PATH; local Hermes MCP config not merged\"")
    w("    fi")
    w("    # Verify the VALUE, not the exit codes: ask Claude what it")
    w("    # actually has. Check expected servers are present, and report")
    w("    # any unexpected ones (but leave plugin-provided and hand-added).")
    w('    if [ "${CHECK_ONLY}" != 1 ]; then')
    w("     # Once per profile: each one answers for its own .claude.json.")
    w('     for profile_dir in "${CLAUDE_PROFILE_DIRS[@]}"; do')
    w('      registered=$(CLAUDE_CONFIG_DIR="${profile_dir}" claude mcp list 2>/dev/null || true)')
    w("      # Check all expected servers are registered")
    w("      for want in \\")
    expected = [
        server["name"]
        for server in _entries(data, "mcp_servers")
        if (
            on_surface(server, "workstation")
            and server.get("enabled") is not False
            and len(server.get("surfaces") or []) > 0
        )
    ]
    for name in expected:
        w(f"        {name} \\")
    w("        ; do")
    w('        case "${registered}" in')
    w('          *"${want}"*) ;;')
    w('          *) warn "MCP server ${want} is not registered in ${profile_dir}" ;;')
    w("        esac")
    w("      done")
    w("      # Report unknown servers (but ignore plugin-provided and hand-added ones)")
    w("      # One name per line, taken from the start of the line up to the first")
    w("      # colon. NOT a lookahead: BSD grep has no PCRE, so `(?=:)` is a")
    w('      # "repetition-operator operand invalid" error and the whole check')
    w("      # silently inspected nothing on macOS.")
    w('      echo "${registered}" | sed -n \'s/^\\([a-zA-Z0-9_:-]*\\):[[:space:]].*/\\1/p\' '
      '| sort -u | while read -r found; do')
    w('        case "${found}" in')
    # All registry-owned servers
    registry_servers = [s["name"] for s in _entries(data, "mcp_servers") if on_surface(s, "workstation")]
    for name in registry_servers:
        w(f'          {name}) ;;')
    # Plugin-provided and hand-added servers
    w('          plugin:*|idea|rubymine) ;;')
    w('          *) warn "unknown MCP server ${found} in ${profile_dir} '
      '-- hand-added or from a removed registry entry?" ;;')
    w("        esac")
    w("      done")
    w("     done")
    w("    fi")
    w("  else")
    w('    log "MCP registration skipped (--no-mcp)"')
    w("  fi")
    w("fi")
    w("")

    # --- first-party skills ---
    w("# -----------------------------------------------------------------")
    w("# 7. First-party skills that ship in this repository.")
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
    w("# 8. Retired hooks.")
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
    w("# 9. Summary: what changed and what still needs attention.")
    w("# -----------------------------------------------------------------")
    w("")
    w("log \"Summary of findings:\"")
    w("")
    w("# Report newly installed binaries that need a new shell")
    w('if [ "${#new_binaries[@]}" -gt 0 ]; then')
    w('  log "Binaries installed during this run (may need a new shell):"')
    w('  for bin in "${new_binaries[@]}"; do')
    w('    log "  - ${bin}"')
    w("  done")
    w('  log ""')
    w("fi")
    w("")
    w("# Report MCP servers skipped due to missing credentials")
    w('if [ "${#skipped_mcp_servers[@]}" -gt 0 ]; then')
    w('  log "MCP servers not registered (missing credentials):"')
    w('  for entry in "${skipped_mcp_servers[@]}"; do')
    w('    log "  - ${entry}"')
    w("  done")
    w('  log ""')
    w("fi")
    w("")
    w("# Report shared surfaces a secondary profile did not get")
    w('if [ "${#unshared_profile_paths[@]}" -gt 0 ]; then')
    w('  log "Profile surfaces not shared:"')
    w('  for entry in "${unshared_profile_paths[@]}"; do')
    w('    log "  - ${entry}"')
    w("  done")
    w('  log ""')
    w("fi")
    w("")
    w("# Report language servers with missing binaries")
    w('if [ "${#missing_lsp_binaries[@]}" -gt 0 ]; then')
    w('  log "Language servers with missing binaries:"')
    w('  for entry in "${missing_lsp_binaries[@]}"; do')
    w('    log "  - ${entry}"')
    w("  done")
    w('  log ""')
    w("fi")
    w("")
    w("# Report plugins left disabled")
    w('if [ "${#disabled_plugins[@]}" -gt 0 ]; then')
    w('  log "Plugins left disabled:"')
    w('  for entry in "${disabled_plugins[@]}"; do')
    w('    log "  - ${entry}"')
    w("  done")
    w('  log ""')
    w("fi")
    w("")
    w("# Report plugins with drifted commits")
    w('if [ "${#plugin_drift[@]}" -gt 0 ]; then')
    w('  log "Plugins with drifted commits:"')
    w('  for entry in "${plugin_drift[@]}"; do')
    w('    log "  - ${entry}"')
    w("  done")
    w('  log ""')
    w("fi")
    w("")
    w("# Final summary")
    w('if [ "${failures}" = 0 ] && [ "${warnings}" = 0 ] \\')
    w('   && [ "${#skipped_mcp_servers[@]}" = 0 ] && [ "${#missing_lsp_binaries[@]}" = 0 ] \\')
    w('   && [ "${#disabled_plugins[@]}" = 0 ] && [ "${#plugin_drift[@]}" = 0 ]; then')
    w('  log "Setup complete: everything is ready"')
    w("else")
    w("  log \"Setup summary: ${failures} failure(s), ${warnings} warning(s)\"")
    w("fi")
    w("")
    w('[ "${failures}" = 0 ]')
    return "\n".join(out) + "\n"


def render_container_setup_script(data: dict[str, Any]) -> str:
    base = data.get("container_base") or {}
    tools = container_tools(data)
    out: list[str] = []
    w = out.append

    w("#!/usr/bin/env bash")
    w(f"# {GENERATED_BANNER}")
    w("#")
    w("# Installs every tool the agents image gives its Agent Sessions, at the")
    w("# versions the registry pins. Runs as root at image build time, where no")
    w("# secret exists; anything that needs a credential happens at container start.")
    w("#")
    w("# Usage:")
    w("#   ./setup-container.sh           install everything, then verify (root)")
    w("#   ./setup-container.sh --check   verify only: every tool present at its pin")
    w("")
    w("set -euo pipefail")
    w("")
    w("CHECK_ONLY=0")
    w('case "${1:-}" in')
    w("  --check) CHECK_ONLY=1 ;;")
    w('  "") ;;')
    w("  --help|-h) sed -n '2,12p' \"$0\"; exit 0 ;;")
    w('  *) echo "unknown option: $1" >&2; exit 64 ;;')
    w("esac")
    w("")
    w("failures=0")
    w("log()  { printf 'setup-container: %s\\n' \"$*\"; }")
    w("ok()   { printf 'setup-container:   ok    %s\\n' \"$*\"; }")
    w("fail() { printf 'setup-container:   FAIL  %s\\n' \"$*\" >&2; failures=$((failures + 1)); }")
    w("")
    w("# Shared, world-readable locations, so the non-root agent user can run")
    w("# what root installed.")
    w("export UV_TOOL_DIR=/opt/uv/tools UV_TOOL_BIN_DIR=/usr/local/bin UV_PYTHON_INSTALL_DIR=/opt/uv/python")
    w("export PLAYWRIGHT_BROWSERS_PATH=/ms-playwright")
    w("export DEBIAN_FRONTEND=noninteractive")
    w("")
    w('case "$(uname -m)" in')
    w("  x86_64|amd64) DEB_ARCH=amd64 GNU_ARCH=x86_64 ;;")
    w("  aarch64|arm64) DEB_ARCH=arm64 GNU_ARCH=aarch64 ;;")
    w('  *) echo "setup-container: unsupported architecture $(uname -m)" >&2; exit 1 ;;')
    w("esac")
    w("export DEB_ARCH GNU_ARCH")
    w("")
    w("install_tool() {")
    w('  log "install $1 $2"')
    w('  VERSION="$2" bash -euo pipefail -c "$3"')
    w("}")
    w("")
    w("# Verifies the value, not the exit code: the tool must report the pinned")
    w("# version, so a stale binary earlier on PATH fails the check.")
    w("check_tool() {")
    w('  local binary="$1" version="${2#v}" version_command="$3" output')
    w('  if ! command -v "${binary}" >/dev/null 2>&1; then')
    w('    fail "${binary}: not on PATH"')
    w("    return 0")
    w("  fi")
    w('  output="$(bash -c "${version_command}" 2>&1 || true)"')
    w('  if printf \'%s\\n\' "${output}" | grep -Eq "(^|[^0-9.])${version//./\\\\.}([^0-9.]|$)"; then')
    w('    ok "${binary} ${version}"')
    w("  else")
    w('    fail "${binary}: expected ${version}, got: $(printf \'%s\\n\' "${output}" | head -1)"')
    w("  fi")
    w("}")
    w("")
    w("check_apt() {")
    w('  if dpkg -s "$1" >/dev/null 2>&1; then')
    w('    ok "apt $1"')
    w("  else")
    w('    fail "apt $1: not installed"')
    w("  fi")
    w("}")
    w("")

    packages = [str(p) for p in base.get("apt_packages") or []]
    w('if [ "${CHECK_ONLY}" = 0 ]; then')
    w('  if [ "$(id -u)" != 0 ]; then')
    w('    echo "setup-container: must run as root (image build time); use --check otherwise" >&2')
    w("    exit 77")
    w("  fi")
    w("")
    w('  log "apt repositories"')
    w("  apt-get update")
    w("  apt-get install -y --no-install-recommends ca-certificates curl gnupg")
    w("  # shellcheck source=/dev/null")
    w('  codename="$(. /etc/os-release && echo "${VERSION_CODENAME}")"')
    for repo in base.get("apt_repositories") or []:
        name = repo["name"]
        keyring = f"/usr/share/keyrings/{name}.gpg"
        w(f'  curl -fsSL {_q(repo["key_url"])} | gpg --dearmor -o {keyring}')
        w(
            f'  echo "deb [signed-by={keyring}] {repo["url"]} ${{codename}} {repo["components"]}" '
            f"> /etc/apt/sources.list.d/{name}.list",
        )
    w("  apt-get update")
    w('  log "apt packages"')
    w("  apt-get install -y --no-install-recommends \\")
    for package in packages:
        w(f"    {package} \\")
    w("    ;")
    w("")
    w("  # Single quotes on purpose: install_tool expands ${VERSION} per tool.")
    w("  # shellcheck disable=SC2016")
    w("  {")
    for tool in tools:
        w(f"  install_tool {tool['name']} {_q(tool['version'])} {_q(tool['install'])}")
    w("  }")
    w("")
    w("  chmod -R a+rX /opt/uv")
    w("  npm cache clean --force >/dev/null 2>&1 || true")
    w("  rm -rf /var/lib/apt/lists/* /root/.cache /root/.npm /tmp/*")
    w("fi")
    w("")
    w('log "verify"')
    for package in packages:
        w(f"check_apt {package}")
    for tool in tools:
        w(f"check_tool {tool['binary']} {_q(tool['version'])} {_q(tool['version_command'])}")
    w("")
    w('if [ "${failures}" != 0 ]; then')
    w('  log "${failures} check(s) failed"')
    w("  exit 1")
    w("fi")
    w('log "every tool is at its pinned version"')
    return "\n".join(out) + "\n"


def _mcp_env_flags(server: dict[str, Any], credential: str | None, optional_cred: bool) -> str:
    """Build the ``--env KEY=VALUE`` fragment for a stdio MCP server.

    Shared by the Claude and Codex registration commands. Emits one flag per
    static ``env`` entry, plus the credential when the server declares one.

    An OPTIONAL credential is emitted as ``${VAR:-}``. The script runs under
    ``set -u``, and a bare ``${VAR}`` for an unset variable does not expand to
    an empty string there -- it aborts the run, taking every step after it
    with it. A required credential is already guarded by a ``-z`` test that
    skips the registration, so it is only ever expanded when it is set.
    """
    env = server.get("env") or {}
    flags = "".join(f' --env {key}="{value}"' for key, value in env.items())
    if credential:
        default = ":-" if optional_cred else ""
        flags += f' --env {credential}="${{{credential}{default}}}"'
    return flags


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


def _has_plugin_pins(data: dict[str, Any]) -> bool:
    """Check if any plugins in the registry have commit pins."""
    return any(plugin.get("commit") for plugin in _entries(data, "plugins"))


# ---------------------------------------------------------------------------
# Driver
# ---------------------------------------------------------------------------


def artifacts(data: dict[str, Any]) -> dict[Path, tuple[str, int]]:
    return {
        SETUP_SCRIPT: (render_setup_script(data), 0o755),
        CONTAINER_SETUP_SCRIPT: (render_container_setup_script(data), 0o755),
        HERMES_SOURCES: (render_hermes_sources(data), 0o644),
        HERMES_MCP: (render_hermes_mcp(data), 0o644),
        HERMES_LOCAL_MCP: (render_hermes_local_mcp(data), 0o644),
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
