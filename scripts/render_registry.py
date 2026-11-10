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
import json
import re
import sys
import tomllib
from pathlib import Path
from typing import Any

import yaml

KIT_ROOT = Path(__file__).resolve().parent.parent
REGISTRY_PATH = KIT_ROOT / "registry" / "estate-tooling.yaml"

SETUP_SCRIPT = Path("installer/setup-workstation.sh")
CONTAINER_SETUP_SCRIPT = Path("installer/setup-container.sh")
# Hand-written, sourced by SETUP_SCRIPT: keeps workstation_connect forwards alive.
PORT_FORWARD_HELPER = Path("installer/port-forward-agent.sh")
CLOUD_HELPER = Path("installer/cloud-session.sh")
# Where a released setup-workstation.sh fetches its kit bundle: the release
# assets of its own tag, which scripts/publish-installer-artifacts.sh stamps
# in place of the placeholder (scripts/verify_release_asset.py owns both).
RELEASE_DOWNLOAD_BASE = "https://github.com/JorisJonkers-dev/agent-kit/releases/download"
RELEASE_TAG_PLACEHOLDER = "@AGENT_KIT_RELEASE_TAG@"
SKILLS_BUNDLE_ASSET = "agent-kit-skills.tar.gz"
HERMES_SOURCES = Path("registry/generated/hermes/skills-sources.conf")
HERMES_MCP = Path("registry/generated/hermes/mcp-servers.yaml")
HERMES_LOCAL_MCP = Path("registry/generated/hermes/mcp-servers.local.yaml")
VIBE_MCP = Path("registry/generated/vibe/mcp-servers.toml")
VIBE_PRIVATE_MCP = Path("registry/generated/vibe-private/mcp-servers.toml")
ANTIGRAVITY_MCP = Path("registry/generated/antigravity/mcp_config.json")
OPENCODE_MCP = Path("registry/generated/opencode/opencode.json")
# One read-only `critic-<model>` Vibe agent file per vibe_critic_agents entry,
# and one read-only `delegate-*` file per vibe_delegate_agents entry, in the
# same directory: both are agents the kit launches by name.
VIBE_CRITIC_AGENTS_DIR = Path("registry/generated/vibe/agents")
VIBE_DELEGATE_AGENTS_DIR = VIBE_CRITIC_AGENTS_DIR
# The runner's search server goes by `packet` in a critic's launch, so Vibe
# names its tools `packet_<tool>` (ADR-0006). They exist only while it runs.
CRITIC_SEARCH_TOOLS = ("packet_grep", "packet_list", "packet_symbol")
CRITIC_TOOLS = ("read_file", *CRITIC_SEARCH_TOOLS)
VIBE_CRITIC_AGENT_KEYS = {"name", "model", "input_price", "output_price", "thinking", "surfaces", "note"}
# A delegate agent pins no model: the gateway passes `--model <alias>`.
VIBE_DELEGATE_AGENT_KEYS = {"name", "surfaces", "note"}
# What a read-only delegate may do: read a file and search the working
# directory. Vibe 3.0.4 has no listing tool, so `grep` is how it finds files.
DELEGATE_TOOLS = ("read_file", "grep")
# What a writing delegate may add: create or overwrite a file and edit one in
# place. There is no shell tool among them: the gateway runs the check itself.
DELEGATE_WRITE_TOOLS = ("write_file", "edit")
# The one delegate agent that may write; every other `delegate-*` is read-only.
WRITE_DELEGATE_AGENT = "delegate-write"
# Mistral bills cached prompt tokens at this share of the input price
# (docs.mistral.ai/studio/conversations/advanced/prompt-caching). Every critic
# model runs through Mistral, so its agent file prices cached input this way.
VIBE_CACHED_INPUT_SHARE = 0.1
# Vibe's standard thinking levels (vibe/config_values.py THINKING_LEVELS).
VIBE_THINKING_LEVELS = ("off", "low", "medium", "high", "xhigh", "max")
# Antigravity expands no environment variables, so a token-bearing server is
# reached through this stdio proxy, which reads the token from the secret store.
ANTIGRAVITY_PROXY = "$HOME/.local/bin/antigravity-mcp-proxy"
# Seconds Vibe waits for an MCP server to start, unless its entry sets startup_timeout.
VIBE_STARTUP_TIMEOUT = 30
VIBE_HOOKS = Path("installer/vibe-hooks.toml")
VIBE_NO_SIGNATURE = Path("scripts/vibe-no-commit-signature.py")
# Removed from ${VIBE_HOME}/hooks.toml on every run.
RETIRED_VIBE_HOOKS = ("agent-kit-retain-private",)
SECRETS_TABLE = Path("registry/generated/workstation-secrets.json")

# The agents a local_skills entry's `agents:` may name, and the homes each one
# reads skills from. A name outside this table is a registry typo, not a home.
SKILL_AGENT_HOMES = {
    "claude": ["CLAUDE_HOME"],
    "codex": ["CODEX_HOME"],
    "vibe": ["VIBE_HOME", "VIBE_PRIVATE_HOME"],
    "antigravity": ["ANTIGRAVITY_HOME", "ANTIGRAVITY_CLI_HOME"],
    "opencode": ["OPENCODE_HOME"],
}

RETIRED_SKILL_NAME = re.compile(r"[A-Za-z0-9][A-Za-z0-9._-]*")

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


# Names Claude Code accepts only for GitHub sources from Anthropic, never for a folder.
RESERVED_MARKETPLACES = {"claude-plugins-official", "claude-code-plugins", "claude-code-marketplace",
                         "anthropic-marketplace", "anthropic-plugins", "agent-skills", "life-sciences"}


def declared_name(data: dict[str, Any], market: str) -> str:
    """The name a marketplace is declared under, and its plugins are installed as."""
    entry = next((m for m in _entries(data, "marketplaces") if m["name"] == market), {})
    return str(entry.get("declare_as") or market)


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
        user_config = plugin.get("user_config")
        if user_config is not None:
            rel = str(user_config.get("path") or "")
            if not rel or rel.startswith(("/", "~")) or ".." in rel.split("/"):
                raise RegistryError(f"plugin {plugin.get('name')} user_config.path must be relative to $HOME")
            if not isinstance(user_config.get("set"), dict) or not user_config["set"]:
                raise RegistryError(f"plugin {plugin.get('name')} user_config.set must be a non-empty mapping")
            if "'" in json.dumps(user_config["set"]):
                raise RegistryError(
                    f"plugin {plugin.get('name')} user_config.set may not contain a single quote",
                )
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
        str(src.get("repo", "")).rstrip("/").rsplit("/", 1)[-1].lower(): src
        for src in _entries(data, "skill_sources")
    }
    names = {m.get("name") for m in _entries(data, "marketplaces")}
    for market in _entries(data, "marketplaces"):
        alias = market.get("declare_as")
        if market.get("name") in RESERVED_MARKETPLACES and (not alias or alias in RESERVED_MARKETPLACES):
            raise RegistryError(
                f"marketplace {market.get('name')} has a name Claude Code reserves for GitHub sources; "
                "give it a declare_as that is not reserved, which setup declares the pinned folder as",
            )
        if alias and alias in names:
            raise RegistryError(f"marketplace {market.get('name')} declares as {alias}, another marketplace's name")
        pin = str(market.get("commit") or "")
        if not re.fullmatch(r"[0-9a-f]{40}", pin):
            raise RegistryError(
                f"marketplace {market.get('name')} must pin a full 40-char commit, the reviewed one",
            )
        if not str(market.get("ref") or ""):
            raise RegistryError(
                f"marketplace {market.get('name')} must name the ref its pin follows, a branch or a tag",
            )
        shared = reviewed_by_repo.get(str(market.get("repo", "")).rsplit("/", 1)[-1].lower())
        if shared and pin != shared.get("commit"):
            raise RegistryError(
                f"marketplace {market.get('name')} pins {pin} but the skill source for the same "
                f"upstream pins {shared.get('commit')}; the reviewed commit is the one to use",
            )
        if shared and market.get("ref") != shared.get("ref"):
            raise RegistryError(
                f"marketplace {market.get('name')} follows ref {market.get('ref')} but the skill source "
                f"for the same upstream follows {shared.get('ref')}; Renovate moves both only on one ref",
            )
    for plugin in _entries(data, "plugins"):
        if "commit" in plugin:
            raise RegistryError(
                f"plugin {plugin.get('name')} pins a commit; pin its marketplace instead, "
                "which decides the commit every plugin in it installs at",
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
        # The workstation renderer mirrors Hermes' sync-skills init
        # container (clone at the ref, verify the commit, install every
        # SKILL.md it finds), which only implements the "*" selector today.
        # A narrower selector on a workstation surface would silently
        # install the wrong set rather than the one the registry declared,
        # so refuse it at render time instead.
        if "workstation" in (source.get("surfaces") or []) and source.get("selector") != "*":
            raise RegistryError(
                f"skill source {name} targets workstation with selector "
                f"{source.get('selector')!r}; only selector: \"*\" is supported "
                "on the workstation surface today",
            )

    for server in _entries(data, "mcp_servers"):
        name = server.get("name")
        transport = server.get("transport")
        if transport not in {"http", "stdio"}:
            raise RegistryError(f"mcp server {name} transport must be http or stdio")
        if transport == "stdio" and not server.get("command"):
            raise RegistryError(f"stdio mcp server {name} must name a command")
        if session_only(server) and (
            transport != "http" or server.get("surfaces") != ["workstation"] or server.get("cloud") is not False
        ):
            raise RegistryError(
                f"session-only mcp server {name} must be http, on the workstation surface only, with cloud: false",
            )
        if "enabled_for" in server:
            if not session_only(server):
                raise RegistryError(f"mcp server {name}: enabled_for only applies to a session-only server")
            for agent, label in (("antigravity", "Antigravity"), ("opencode", "OpenCode")):
                if agent in (server["enabled_for"] or []):
                    raise RegistryError(
                        f"mcp server {name}: enabled_for may not name {agent}; "
                        f"{label} never gets Private Memory",
                    )
            if not set(server["enabled_for"] or []) <= ALWAYS_ON_AGENTS:
                raise RegistryError(
                    f"mcp server {name}: enabled_for may only name {sorted(ALWAYS_ON_AGENTS)}, "
                    f"not {server['enabled_for']}",
                )

    ca = data.get("coding_agents")
    if ca:
        if not re.fullmatch(r"\d+\.\d+\.\d+", str(ca.get("version", ""))):
            raise RegistryError(
                "coding_agents.version must be an exact x.y.z: autoUpdate is off, so the pin is the version",
            )
        if "apiToken" in (ca.get("config") or {}):
            raise RegistryError("coding_agents.config may not hold apiToken; it is read from HINDSIGHT_API_TOKEN")

    if proxy := data.get("package_proxy"):
        base = str(proxy.get("base") or "")
        if not base.startswith("https://") or base.endswith("/"):
            raise RegistryError("package_proxy.base must be an https URL with no trailing slash")
        if not base.removeprefix("https://").replace(".", "").replace("-", "").isalnum():
            raise RegistryError("package_proxy.base must be a plain host https URL")
        # GOPROXY takes a bare URL (no trailing slash); the directory-style
        # registries (npm, pypi, cargo) are roots and must end in /.
        endpoints = {name: proxy.get(name) for name in ("go", "npm", "pypi", "cargo")}
        if any(value is None for value in endpoints.values()):
            raise RegistryError("package_proxy needs a go, npm, pypi and cargo endpoint")
        for name, value in endpoints.items():
            url = str(value).removeprefix("sparse+")
            if not url.startswith(base + "/"):
                raise RegistryError(f"package_proxy endpoint {url!r} must sit under package_proxy.base")
            if name != "go" and not url.endswith("/"):
                raise RegistryError(f"package_proxy endpoint {url!r} must end in / (registry roots are directories)")
        maven = dict(proxy.get("maven") or {})
        if set(maven) != {"main", "spring_milestone", "sonatype_snapshots", "google_maven"}:
            raise RegistryError(
                "package_proxy.maven takes main, spring_milestone, sonatype_snapshots and google_maven, all of them",
            )
        for field, url in maven.items():
            if not str(url).startswith(base + "/maven/"):
                raise RegistryError(f"package_proxy.maven.{field} ({url}) must sit under base/maven/")
            if not str(url).endswith("/"):
                raise RegistryError(f"package_proxy.maven.{field} ({url}) must end in /")
        first_party = dict(proxy.get("first_party") or {})
        if set(first_party) != {"npm_scope", "npm_registry", "maven_github", "go_private"}:
            raise RegistryError(
                "package_proxy.first_party takes npm_scope, npm_registry, maven_github and go_private, all of them",
            )
        for field, value in first_party.items():
            if not str(value).strip():
                raise RegistryError(f"package_proxy.first_party.{field} must be set")
        if not first_party["npm_scope"].startswith("@"):
            raise RegistryError("package_proxy.first_party.npm_scope must be an npm scope like @owner")
        for field in ("vault_path", "vault_field", "token_file", "verify_url"):
            if not str(proxy.get(field) or "").strip():
                raise RegistryError(f"package_proxy.{field} must be set")
        token_file = str(proxy["token_file"])
        if token_file.startswith(("/", "~")) or ".." in token_file.split("/"):
            raise RegistryError("package_proxy.token_file must be relative to XDG_CONFIG_HOME")
        if not str(proxy["verify_url"]).startswith(base + "/"):
            raise RegistryError("package_proxy.verify_url must sit under package_proxy.base")

    for server in _entries(data, "mcp_servers"):
        if "claude_allow_tools" not in server:
            continue
        if not isinstance(server["claude_allow_tools"], bool):
            raise RegistryError(f"mcp server {server.get('name')}: claude_allow_tools must be true or false")
        if server["claude_allow_tools"] and "workstation" not in (server.get("surfaces") or []):
            raise RegistryError(
                f"mcp server {server.get('name')}: claude_allow_tools needs the workstation surface, "
                "where setup registers it for Claude Code",
            )

    deny = data.get("claude_private_deny") or []
    if not isinstance(deny, list) or not all(isinstance(r, str) and r.endswith(")") for r in deny):
        raise RegistryError("claude_private_deny must be a list of Claude permission rules, like Read(~/.vibe/**)")

    for secret in _entries(data, "workstation_secrets"):
        if secret.get("retired"):
            if set(secret) != {"env", "retired"}:
                raise RegistryError(f"retired workstation secret {secret.get('env')} takes only env and retired")
            continue
        kinds = [k for k in ("value", "grant", "paste", "login") if k in secret]
        if not secret.get("env") or len(kinds) != 1:
            raise RegistryError(
                f"workstation secret {secret.get('env')} needs an env and one of value, grant, paste, login",
            )
        # Rendered into a double-quoted shell string: plain words only.
        if "login" in secret and not (
            isinstance(secret["login"], str) and re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9 ._/=-]*", secret["login"])
        ):
            raise RegistryError(
                f"workstation secret {secret['env']}: login must be a plain command (letters, digits, - _ . / =)",
            )
        if kinds != ["value"] and not str(secret.get("check") or "").startswith("https://"):
            raise RegistryError(f"workstation secret {secret['env']} needs an https check url")
        if "gate" in secret and (secret["gate"] != "passphrase" or secret.get("export") is not False):
            raise RegistryError(f"workstation secret {secret['env']}: gate must be passphrase, with export: false")
        if not isinstance(secret.get("export"), bool):
            raise RegistryError(f"workstation secret {secret['env']} must say export: true or false")

    for skill in _entries(data, "local_skills"):
        path = KIT_ROOT / str(skill.get("path") or "")
        pending = skill.get("pending")
        if pending is not None and not re.fullmatch(r"https://github\.com/[^/\s]+/[^/\s]+/issues/\d+", str(pending)):
            raise RegistryError(
                f"local skill {skill.get('name')} pending must be the URL of the issue that builds it",
            )
        if pending and (path / "SKILL.md").is_file():
            raise RegistryError(
                f"local skill {skill.get('name')} has its SKILL.md now; drop pending: so it installs",
            )
        if not pending and not (path / "SKILL.md").is_file():
            raise RegistryError(
                f"local skill {skill.get('name')} has no SKILL.md at {skill.get('path')}",
            )
        unknown_agents = set(skill.get("agents") or []) - set(SKILL_AGENT_HOMES)
        if unknown_agents:
            raise RegistryError(
                f"local skill {skill.get('name')} names unknown agents {sorted(unknown_agents)}; "
                f"agents: may only name {sorted(SKILL_AGENT_HOMES)}",
            )
        if "hermes" in (skill.get("surfaces") or []):
            raise RegistryError(
                f"local skill {skill.get('name')} cannot target hermes: sync-skills clones "
                "public git URLs with no credential and this repository is private",
            )

    _validate_vibe_critic_agents(data)
    _validate_vibe_delegate_agents(data)

    for key in ("clis", "plugins", "language_servers", "mcp_servers"):
        for item in _entries(data, key):
            if "cloud" in item and not isinstance(item["cloud"], bool):
                raise RegistryError(f"{key} entry {_label(item)} cloud: must be true or false")
    for server in _entries(data, "language_servers"):
        for field in ("runtime", "nixos"):
            if field in server and not (isinstance(server[field], str) and server[field]):
                raise RegistryError(
                    f"language server {server.get('plugin')} {field}: must be a non-empty string",
                )
        if "install_linux" in server and not (server["install_linux"] and server.get("install")):
            raise RegistryError(
                f"language server {server.get('plugin')} install_linux must be a command, "
                "beside an install for the other platforms",
            )

    for server in _entries(data, "mcp_servers"):
        if "auth" in server and not (
            server["auth"] == "oauth" and server.get("transport") == "http" and not server.get("credential")
        ):
            raise RegistryError(
                f"mcp server {server.get('name')} auth: oauth is the only auth kind, for an http server "
                "with no credential",
            )
        if server.get("install_pinned") and not (
            pinned_source(server).get("datasource") == "npm" and server.get("requires_binary")
        ):
            raise RegistryError(
                f"mcp server {server.get('name')} install_pinned needs an npm container pin "
                "or pin: block, and a requires_binary",
            )

    # A cask is installed by Homebrew, so it needs neither a binary to probe
    # nor a shell install command; everything else needs both.
    for cli in _entries(data, "clis"):
        if cli.get("cask"):
            continue
        if not cli.get("binary"):
            raise RegistryError(f"cli {cli.get('name')} must name a binary, or a cask that installs it")
        if on_surface(cli, "workstation") and not cli.get("install"):
            raise RegistryError(f"cli {cli.get('name')} must name an install command, or a cask")
    for key in ("plugins", "language_servers", "mcp_servers", "skill_sources", "local_skills"):
        for item in _entries(data, key):
            for field in ("cask", "macos_only"):
                if field in item:
                    raise RegistryError(f"{key} entry {_label(item)} may not set {field}: it is a clis: key")

    for retired_skill in _entries(data, "retired_skills"):
        rname = retired_skill.get("name")
        if not isinstance(rname, str) or not RETIRED_SKILL_NAME.fullmatch(rname):
            raise RegistryError(f"retired_skills entry needs a plain `name` (letters, digits, . _ -): {rname!r}")

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


def pinned_source(server: dict[str, Any]) -> dict[str, Any]:
    """Where an `install_pinned` server's npm pin lives: its own `pin:`, else its container pin."""
    return dict(server.get("pin") or server.get("container") or {})


def workstation_pins(data: dict[str, Any]) -> list[dict[str, Any]]:
    """The `pin:` blocks of workstation-only pinned installs, which Renovate moves like container pins."""
    return [dict(s["pin"]) for s in _entries(data, "mcp_servers") if s.get("install_pinned") and s.get("pin")]


def on_surface(item: dict[str, Any], surface: str) -> bool:
    return surface in (item.get("surfaces") or [])


def on_cloud(item: dict[str, Any]) -> bool:
    return item.get("cloud", True) is not False


def managed_secrets(data: dict[str, Any]) -> set[str]:
    """Variables agent-kit-keys gets and stores; what they gate reports through it."""
    return {str(e["env"]) for e in _entries(data, "workstation_secrets") if not e.get("retired")}


def _env_report(data: dict[str, Any], message: str, var: str, indent: str) -> str:
    if var not in managed_secrets(data):
        return f'{indent}warn "{message}"'
    return "\n".join([
        f'{indent}if [ "${{CLOUD}}" = 1 ]; then',
        f'{indent}  warn "{message}"',
        f"{indent}else",
        f'{indent}  log "{var} is not set (see tokens and keys)"',
        f"{indent}fi",
    ])


def retired_servers(data: dict[str, Any]) -> list[str]:
    """MCP servers the registry retired: `enabled: false` or on no surface at all."""
    return [
        str(s["name"])
        for s in _entries(data, "mcp_servers")
        if s.get("enabled") is False or not (s.get("surfaces") or [])
    ]


def session_only(server: dict[str, Any]) -> bool:
    """Installed switched off; on only in a session opened for it."""
    return server.get("session_only") is True


# The launchers a session-only server may be on for. vibe-private is the only
# launcher behind the passphrase gate, so it alone may hold Private Memory.
ALWAYS_ON_AGENTS = {"vibe-private"}


def enabled_for(server: dict[str, Any], agent: str) -> bool:
    """Whether a session-only server is on for this agent anyway."""
    return agent in (server.get("enabled_for") or [])


def _label(item: dict[str, Any]) -> str:
    return str(item.get("name") or item.get("plugin"))


def _unalias(data: dict[str, Any], plugin: str, market: str, indent: str) -> list[str]:
    """Uninstall a copy installed under the marketplace's own name before it moved to its alias."""
    if declared_name(data, market) == market:
        return []
    old = f"{plugin}@{market}"
    return [
        f'{indent}case "${{installed_plugins-}}" in *\'"{old}"\'*)',
        f'{indent}  run claude plugin uninstall {old} >/dev/null 2>&1 \\',
        f'{indent}    && ok "{plugin}: removed the copy installed as {old}" ;;',
        f"{indent}esac",
    ]


def _pinned_guard(block: list[str], market: str, name: str, indent: str) -> list[str]:
    """Skip an install whose marketplace could not be put at its pinned commit."""
    return [
        f'{indent}case " ${{unpinned_markets[*]-}} " in *" {market} "*)',
        f'{indent}  warn "{name}: not installed: its marketplace is not at its pinned commit" ;;',
        f"{indent}*)",
        *["  " + line for line in block],
        f"{indent}  ;;",
        f"{indent}esac",
    ]


def _cloud_guard(block: list[str], item: dict[str, Any], indent: str) -> list[str]:
    """Wrap a rendered block so `--cloud` skips an entry marked `cloud: false`."""
    if on_cloud(item):
        return block
    return [
        f'{indent}if [ "${{CLOUD}}" = 1 ]; then',
        f'{indent}  log "{_label(item)}: not set up in cloud mode"',
        f"{indent}else",
        *[f"  {line}" if line else line for line in block],
        f"{indent}fi",
    ]


def _validate_vibe_critic_agents(data: dict[str, Any]) -> None:
    seen: set[str] = set()
    for agent in _entries(data, "vibe_critic_agents"):
        name = str(agent.get("name") or "")
        if not re.fullmatch(r"critic-[a-z0-9]+(-[a-z0-9]+)*", name):
            raise RegistryError(f"vibe critic agent {name!r} must be named critic-<model>, in lower case")
        if name in seen:
            raise RegistryError(f"vibe critic agent {name} is declared twice")
        seen.add(name)
        unknown = set(agent) - VIBE_CRITIC_AGENT_KEYS
        if unknown:
            raise RegistryError(
                f"vibe critic agent {name} sets {sorted(unknown)}; an entry names only its model and "
                "prices, and the read-only settings are the same for every critic",
            )
        if not isinstance(agent.get("model"), str) or not agent["model"]:
            raise RegistryError(f"vibe critic agent {name} must name its model")
        for field in ("input_price", "output_price"):
            price = agent.get(field)
            if isinstance(price, bool) or not isinstance(price, int | float) or price <= 0:
                raise RegistryError(
                    f"vibe critic agent {name} {field} must be the model's real price per million tokens: "
                    "Vibe computes cost from it, and 0 turns every price cap off",
                )
        if "thinking" in agent and agent["thinking"] not in VIBE_THINKING_LEVELS:
            raise RegistryError(f"vibe critic agent {name} thinking must be one of {list(VIBE_THINKING_LEVELS)}")
        if agent.get("surfaces") not in ([], ["workstation"]):
            raise RegistryError(
                f"vibe critic agent {name} surfaces must be [workstation], or [] to retire it",
            )


def vibe_critic_agents(data: dict[str, Any]) -> list[dict[str, Any]]:
    """The declared critic agents; a retired one has ``surfaces: []``."""
    return [a for a in _entries(data, "vibe_critic_agents") if on_surface(a, "workstation")]


def retired_vibe_critic_agents(data: dict[str, Any]) -> list[str]:
    return [str(a["name"]) for a in _entries(data, "vibe_critic_agents") if not on_surface(a, "workstation")]


def _validate_vibe_delegate_agents(data: dict[str, Any]) -> None:
    seen: set[str] = set()
    for agent in _entries(data, "vibe_delegate_agents"):
        name = str(agent.get("name") or "")
        if not re.fullmatch(r"delegate-[a-z0-9]+(-[a-z0-9]+)*", name):
            raise RegistryError(f"vibe delegate agent {name!r} must be named delegate-<what it may do>, in lower case")
        if name in seen:
            raise RegistryError(f"vibe delegate agent {name} is declared twice")
        seen.add(name)
        unknown = set(agent) - VIBE_DELEGATE_AGENT_KEYS
        if unknown:
            raise RegistryError(
                f"vibe delegate agent {name} sets {sorted(unknown)}; an entry names only itself, "
                "and the read-only settings are the same for every delegate",
            )
        if agent.get("surfaces") not in ([], ["workstation"]):
            raise RegistryError(
                f"vibe delegate agent {name} surfaces must be [workstation], or [] to retire it",
            )


def vibe_delegate_agents(data: dict[str, Any]) -> list[dict[str, Any]]:
    """The declared delegate agents; a retired one has ``surfaces: []``."""
    return [a for a in _entries(data, "vibe_delegate_agents") if on_surface(a, "workstation")]


def retired_vibe_delegate_agents(data: dict[str, Any]) -> list[str]:
    return [str(a["name"]) for a in _entries(data, "vibe_delegate_agents") if not on_surface(a, "workstation")]


def render_vibe_delegate_agent(agent: dict[str, Any]) -> str:
    """One delegate agent file, a Vibe config overlay for ``vibe --agent <name>``.

    Every setting is fixed here, so no entry can widen a delegate. Its tool
    list is what keeps shells, skills and subagents from it, and writes too
    unless it is ``delegate-write``, which adds ``write_file`` and ``edit``
    (set to ``always``, as a headless run has nobody to ask);
    ``outside_workdir = "deny"`` keeps it inside the directory the gateway
    points it at, and ``include_project_context = false`` keeps the home's
    ``AGENTS.md`` out of its prompt. It pins no model and sets no
    ``default_agent``: the gateway passes ``--model <alias>``, and a plain
    ``vibe`` never starts with it.
    """
    name = str(agent["name"])
    writes = name == WRITE_DELEGATE_AGENT
    tools = (*DELEGATE_TOOLS, *DELEGATE_WRITE_TOOLS) if writes else DELEGATE_TOOLS
    description = (
        "Write delegate for the model gateway: edits files in its worktree"
        if writes
        else "Read-only delegate for the model gateway"
    )
    lines = [
        f"# {GENERATED_BANNER}",
        f"# A {'writing' if writes else 'read-only'} delegate of the model gateway, launched by it as",
        f"# `vibe --agent {name} --model <alias>`. Installed into the shared Vibe home only.",
        "",
        f"display_name = {_toml_str(name.replace('-', ' ').title())}",
        f'description = "{description}"',
        'safety = "safe"',
        "enabled_tools = [" + ", ".join(_toml_str(tool) for tool in tools) + "]",
        'enabled_skills = ["re:^$"]',
        'outside_workdir = "deny"',
        "enable_subagents = false",
        "include_project_context = false",
        "",
    ]
    if writes:
        for tool in DELEGATE_WRITE_TOOLS:
            lines += [f"[tools.{tool}]", 'permission = "always"', ""]
    lines += ["[session_logging]", "enabled = false"]
    return "\n".join(lines) + "\n"


def cached_input_price(agent: dict[str, Any]) -> float:
    """The agent's price per million cached input tokens: 10% of its input price, rounded clear of float noise.

    Without it Vibe bills a cached token at the full input rate, and a critic,
    which re-reads its conversation every turn, has most of its input cached.
    """
    return round(float(agent["input_price"]) * VIBE_CACHED_INPUT_SHARE, 10)


def render_vibe_critic_agent(agent: dict[str, Any]) -> str:
    """One read-only critic agent file, a Vibe config overlay for ``vibe --agent <name>``.

    Every setting but the model and its prices is fixed here, so no entry can
    widen a critic. Its tool list (``read_file`` and the runner's search
    server's three tools, ADR-0006) is what keeps both writes and skills from
    it; ``include_project_context = false`` keeps the home's ``AGENTS.md`` out
    of its prompt. It leaves MCP to the runner's launch, which switches it off
    for a critic that reads alone and on, with the runner's search server as
    its one live server, for a critic that searches. The search tools are
    approved without asking, since headless Vibe refuses every call that asks.
    It sets no ``default_agent``, so it never becomes the agent a plain
    ``vibe`` starts with.
    """
    name = str(agent["name"])
    model = str(agent["model"])
    lines = [
        f"# {GENERATED_BANNER}",
        "# A read-only critic of the critique pipeline, launched by its runner as",
        f"# `vibe --agent {name}`. Installed into the shared Vibe home only.",
        "",
        f"display_name = {_toml_str(name.replace('-', ' ').title())}",
        f"description = {_toml_str(f'Read-only critic pinned to {model}')}",
        'safety = "safe"',
        f"active_model = {_toml_str(name)}",
        "enabled_tools = [" + ", ".join(_toml_str(tool) for tool in CRITIC_TOOLS) + "]",
        'enabled_skills = ["re:^$"]',
        'outside_workdir = "deny"',
        "enable_subagents = false",
        "include_project_context = false",
        "",
        *[line for tool in CRITIC_SEARCH_TOOLS for line in (f"[tools.{tool}]", 'permission = "always"', "")],
        "[session_logging]",
        "enabled = false",
        "",
        "[[models]]",
        f"name = {_toml_str(model)}",
        'provider = "mistral"',
        f"alias = {_toml_str(name)}",
        f"input_price = {agent['input_price']!r}",
        f"output_price = {agent['output_price']!r}",
        f"cached_input_price = {cached_input_price(agent)!r}",
    ]
    if "thinking" in agent:
        lines.append(f"thinking = {_toml_str(str(agent['thinking']))}")
    return "\n".join(lines) + "\n"


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
    seen: set[str] = set()
    for name, item in _container_candidates(data):
        # Language servers declare no surfaces; for them the block alone decides.
        if "surfaces" in item and bool(item.get("container")) != on_surface(item, "container"):
            raise RegistryError(
                f"{name}: a container: block and the container surface go together",
            )
        if not item.get("container"):
            continue
        if name in seen:
            raise RegistryError(f"container tool name {name!r} is declared twice")
        seen.add(name)

    tools = _container_tool_views(data)
    for tool in tools.values():
        name = tool["name"]
        if not tool["version"] or tool["version"] in {"latest", "None"}:
            raise RegistryError(f"container tool {name} must pin a version, not {tool['version']!r}")
        for field in ("datasource", "package", "install", "version_command"):
            if not tool[field]:
                raise RegistryError(f"container tool {name} must name its {field}")
        if "${VERSION}" not in tool["install"]:
            raise RegistryError(f"container tool {name} install must use ${{VERSION}}")
        for required in tool["requires"]:
            if required not in tools:
                raise RegistryError(f"container tool {name} requires unknown tool {required!r}")


def _container_tool_views(data: dict[str, Any]) -> dict[str, dict[str, Any]]:
    """Each container block merged with the fields it inherits from its entry."""
    tools: dict[str, dict[str, Any]] = {}
    for name, item in _container_candidates(data):
        block = item.get("container")
        if not block:
            continue
        tools[name] = {
            "name": name,
            "binary": block.get("binary") or item.get("binary") or item.get("requires_binary") or name,
            "datasource": block.get("datasource"),
            "package": block.get("package"),
            "version": str(block.get("version") or ""),
            "install": " ".join(str(block.get("install") or "").split()),
            "version_command": block.get("version_command") or item.get("version_command"),
            "requires": list(block.get("requires") or []),
        }
    return tools


def container_tools(data: dict[str, Any]) -> list[dict[str, Any]]:
    """The container tools in install order: registry order, after their requirements."""
    tools = _container_tool_views(data)
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
            # credential_hermes: false -- gateway reaches this by NetworkPolicy alone, no header.
            if credential and server.get("credential_hermes", True):
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
        if not on_surface(server, "workstation") or session_only(server):
            continue
        if server.get("enabled") is False or len(server.get("surfaces") or []) == 0:
            continue
        # Local Hermes has no sign-in flow for an OAuth server.
        if server.get("auth") == "oauth":
            continue
        name = server["name"]
        lines.append(f"  {name}:")
        if server["transport"] == "http":
            url = server.get("url_workstation")
            if not url:
                raise RegistryError(f"mcp server {name} is on workstation but has no url_workstation")
            lines.append(f'    url: "{url}"')
            credential = server.get("credential")
            if credential:
                # Local Hermes interpolates ${CRED} from its own env at connect time.
                lines.append("    headers:")
                lines.append(f'      Authorization: "Bearer ${{{credential}}}"')
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


def _toml_str(value: str) -> str:
    return json.dumps(value)


def vibe_servers(data: dict[str, Any], launcher: str = "vibe") -> list[dict[str, Any]]:
    """Servers a Vibe home gets: the workstation fleet, session-only ones only if enabled for its launcher.

    ``vibe`` is the shared Vibe home (vibe and vibe-study); ``vibe-private``
    is vibe-private's own home.
    """
    return [
        s
        for s in _entries(data, "mcp_servers")
        if s.get("enabled") is not False
        and on_surface(s, "workstation")
        and (not session_only(s) or enabled_for(s, launcher))
    ]


def render_vibe_mcp(data: dict[str, Any]) -> str:
    """The shared Vibe home's servers: merged into ``~/.vibe/config.toml``. Never Private Memory."""
    return _render_vibe_mcp(data, "vibe", "~/.vibe/config.toml")


def render_vibe_private_mcp(data: dict[str, Any]) -> str:
    """vibe-private's servers: the shared fleet plus Private Memory, merged into ``~/.vibe-private/config.toml``."""
    return _render_vibe_mcp(data, "vibe-private", "~/.vibe-private/config.toml")


def _render_vibe_mcp(data: dict[str, Any], launcher: str, config: str) -> str:
    """Render the ``[[mcp_servers]]`` entries merged into a Vibe home's config.toml.

    Credentials are named, never written: Vibe's static auth reads the token
    from ``api_key_env`` at connect time. Merged by ``scripts/vibe-merge-mcp.py``.
    """
    lines = [
        f"# {GENERATED_BANNER}",
        "#",
        f"# Mistral Vibe's MCP servers, merged into {config} by",
        "# scripts/vibe-merge-mcp.py. Servers named here are replaced on every",
        "# run; any other server in that file is left alone.",
    ]
    for server in vibe_servers(data, launcher):
        name = server["name"]
        lines += ["", "[[mcp_servers]]", f"name = {_toml_str(name)}"]
        if server["transport"] == "http":
            url = server.get("url_workstation")
            if not url:
                raise RegistryError(f"mcp server {name} is on workstation but has no url_workstation")
            lines += ['transport = "streamable-http"', f"url = {_toml_str(url)}"]
            if server.get("timeout"):
                lines.append(f"tool_timeout_sec = {float(server['timeout'])}")
                lines.append(f"startup_timeout_sec = {float(server.get('startup_timeout', VIBE_STARTUP_TIMEOUT))}")
            if server.get("auth") == "oauth":
                # Vibe signs in itself (dynamic client registration) and keeps the token.
                lines += ["", "[mcp_servers.auth]", 'type = "oauth"', "scopes = []"]
            credential = server.get("credential")
            if credential:
                lines += [
                    "",
                    "[mcp_servers.auth]",
                    'type = "static"',
                    f"api_key_env = {_toml_str(credential)}",
                    'api_key_header = "Authorization"',
                    'api_key_format = "Bearer {token}"',
                ]
        else:
            args = ", ".join(_toml_str(str(a)) for a in (server.get("args") or []))
            lines += ['transport = "stdio"', f"command = {_toml_str(server['command'])}", f"args = [{args}]"]
            if server.get("timeout"):
                lines.append(f"tool_timeout_sec = {float(server['timeout'])}")
                lines.append(f"startup_timeout_sec = {float(server.get('startup_timeout', VIBE_STARTUP_TIMEOUT))}")
            # The MCP SDK gives a stdio server only a safe allowlist of the parent
            # environment plus this table. Static values only: a credential would
            # be written to disk, and every stdio credential here is optional.
            env = server.get("env") or {}
            if env:
                lines += ["", "[mcp_servers.env]", *(f"{k} = {_toml_str(str(v))}" for k, v in env.items())]
    return "\n".join(lines) + "\n"


def antigravity_servers(data: dict[str, Any]) -> list[dict[str, Any]]:
    """Antigravity's servers: the shared workstation fleet, never Private Memory."""
    return vibe_servers(data, "vibe")


def render_antigravity_mcp(data: dict[str, Any]) -> str:
    """Render Antigravity's global ``mcp_config.json``.

    Antigravity expands no environment variables, so a server that needs a
    token is registered as the stdio proxy rather than with a ``headers``
    block: the proxy reads the token from the agent-kit secret store at
    start-up, and no secret value reaches this file. Merged into
    ``~/.gemini/config/mcp_config.json`` by ``scripts/antigravity-merge-mcp.py``.
    """
    servers: dict[str, Any] = {}
    for server in antigravity_servers(data):
        name = server["name"]
        if server["transport"] == "http":
            url = server.get("url_workstation")
            if not url:
                raise RegistryError(f"mcp server {name} is on workstation but has no url_workstation")
            credential = server.get("credential")
            if credential:
                # /bin/sh -c: a GUI-launched app has no ~/.local/bin on PATH.
                entry: dict[str, Any] = {
                    "command": "/bin/sh",
                    "args": ["-c", f"exec \"{ANTIGRAVITY_PROXY}\" --url {_q(str(url))} --secret {credential}"],
                }
            else:
                entry = {"serverUrl": url}
        else:
            entry = {"command": server["command"], "args": [str(a) for a in (server.get("args") or [])]}
            env = server.get("env") or {}
            if env:
                entry["env"] = {str(k): str(v) for k, v in env.items()}
        if server.get("timeout"):
            entry["timeoutSeconds"] = server["timeout"]
        servers[name] = entry
    return json.dumps({"mcpServers": servers}, indent=2) + "\n"


def opencode_servers(data: dict[str, Any]) -> list[dict[str, Any]]:
    """OpenCode's servers: the shared workstation fleet, never Private Memory."""
    return vibe_servers(data, "vibe")


def render_opencode_mcp(data: dict[str, Any]) -> str:
    """Render the ``mcp`` block merged into OpenCode 2's global ``opencode.json``.

    OpenCode 2 nests servers under ``mcp.servers``. It substitutes
    ``{env:NAME}`` when it loads the file, so a credential is named, never
    written: the header carries ``Bearer {env:<credential>}`` and OAuth
    discovery is off for it. An ``auth: oauth`` server is left to OpenCode's
    own sign-in (``opencode mcp auth <name>``). Timeouts are milliseconds.
    Merged into ``~/.config/opencode/opencode.json`` by
    ``scripts/opencode-merge-mcp.py``.
    """
    servers: dict[str, Any] = {}
    for server in opencode_servers(data):
        name = server["name"]
        entry: dict[str, Any]
        if server["transport"] == "http":
            url = server.get("url_workstation")
            if not url:
                raise RegistryError(f"mcp server {name} is on workstation but has no url_workstation")
            entry = {"type": "remote", "url": url}
            credential = server.get("credential")
            if credential:
                entry["oauth"] = False
                entry["headers"] = {"Authorization": f"Bearer {{env:{credential}}}"}
        else:
            entry = {"type": "local", "command": [server["command"], *(str(a) for a in server.get("args") or [])]}
            # Static values only, as for Vibe: every stdio credential here is optional.
            env = server.get("env") or {}
            if env:
                entry["environment"] = {str(k): str(v) for k, v in env.items()}
        if server.get("timeout"):
            entry["timeout"] = int(max(server["timeout"], server.get("startup_timeout") or 0) * 1000)
        servers[name] = entry
    return json.dumps({"$schema": "https://opencode.ai/config.json", "mcp": {"servers": servers}}, indent=2) + "\n"


MANIFEST_PATH = KIT_ROOT / "manifest.yaml"
# The critique hooks' scripts, in each agent home's copy of the critique skill.
# Where every hook's script lives in an agent home: skills/<skill>/hooks/<name>.sh.
HOOK_SKILLS = "skills"
# A PreToolUse hook gates shell commands only; SessionStart takes no matcher.
CRITIQUE_HOOK_MATCHERS = {"PreToolUse": "Bash"}
# Seconds: a backstop to the runner's own deadline below one second (ADR-0002).
CRITIQUE_HOOK_TIMEOUT = 5
# The hook file setup merges into, per agent the manifest may name.
CRITIQUE_HOOK_HOMES = {
    "claude": ("CLAUDE_HOME", "settings.json", "Claude Code"),
    "codex": ("CODEX_HOME", "hooks.json", "Codex"),
}


def critique_hooks(manifest_path: Path = MANIFEST_PATH) -> list[dict[str, Any]]:
    """The hooks manifest.yaml declares, in its order: name, event, matcher, agents and skill.

    The manifest is the one place the hook names live, and its validator
    allows only the names an ADR allows (ADR-0002, ADR-0009), so setup
    installs nothing else. `skill` is the first-party skill whose `hooks/`
    holds the script (critique unless the entry names another); `matcher`
    overrides the event's default, either for every agent or per agent as a
    mapping (Codex has no EnterWorktree tool to match).
    """
    hooks = yaml.safe_load(manifest_path.read_text()).get("hooks") or {}
    return [
        {
            "name": name,
            "event": entry["event"],
            "matcher": entry.get("matcher") or CRITIQUE_HOOK_MATCHERS.get(entry["event"]),
            "agents": list(entry["agents"]),
            "skill": entry.get("skill") or "critique",
        }
        for name, entry in hooks.items()
    ]


def _agent_matcher(matcher: str | dict[str, str] | None, agent: str) -> str | None:
    """A hook's matcher for one agent: the same string for all, or that agent's entry of a mapping."""
    if isinstance(matcher, dict):
        return matcher.get(agent)
    return matcher


# Merges the critique hooks into one agent's hook file by NAME, or removes
# them. A hook is found by its command naming `<name>.sh`, the way the
# retired-hook purge finds its own, so the operator's hooks are never touched.
# Each hook is one matcher group of its own; an existing group is replaced in
# place, because Codex keys a hook's trust by its position in the file.
CRITIQUE_HOOKS_PY = r'''
import json
import os
import pathlib
import sys

path = pathlib.Path(os.environ["AK_HOOKS_FILE"])
home = os.environ["AK_HOOKS_HOME"]
mode = os.environ["AK_HOOKS_MODE"]
check_only = os.environ["AK_CHECK_ONLY"] == "1"
spec = json.loads(os.environ["AK_HOOKS_SPEC"])
timeout = int(os.environ["AK_HOOKS_TIMEOUT"])


def names(handler, name):
    command = handler.get("command", "") if isinstance(handler, dict) else ""
    if not isinstance(command, str):
        return False
    return any(token.rsplit("/", 1)[-1] == name + ".sh" for token in command.replace('"', " ").split())


def wanted(name, matcher, skill):
    script = "%s/skills/%s/hooks/%s.sh" % (home, skill, name)
    group = {"matcher": matcher} if matcher else {}
    group["hooks"] = [{"type": "command", "command": '/bin/sh "%s"' % script, "timeout": timeout}]
    return group


text = path.read_text() if path.exists() else None
if text is None and mode == "remove":
    print("unchanged")
    raise SystemExit(0)
try:
    data = json.loads(text or "{}")
except json.JSONDecodeError:
    print("%s is not valid JSON" % path, file=sys.stderr)
    raise SystemExit(2)
if not isinstance(data, dict) or not isinstance(data.get("hooks", {}), dict):
    print("%s has no hooks mapping setup can merge into" % path, file=sys.stderr)
    raise SystemExit(2)
before = json.loads(json.dumps(data))
hooks = data.setdefault("hooks", {})

for name, event, matcher, skill in spec:
    want = wanted(name, matcher, skill) if mode == "install" else None
    placed = False
    for current in list(hooks):
        groups = hooks[current]
        if not isinstance(groups, list):
            continue
        kept = []
        for group in groups:
            handlers = group.get("hooks") if isinstance(group, dict) else None
            if not isinstance(handlers, list) or not any(names(h, name) for h in handlers):
                kept.append(group)
                continue
            others = [h for h in handlers if not names(h, name)]
            if want is not None and not placed and current == event and not others:
                kept.append(want)
                placed = True
            elif others:
                kept.append(dict(group, hooks=others))
        if kept or not groups:
            hooks[current] = kept
        else:
            hooks.pop(current)
    if want is not None and not placed:
        hooks.setdefault(event, []).append(want)
if not hooks and before.get("hooks") != {}:
    data.pop("hooks", None)

if data == before:
    print("unchanged")
    raise SystemExit(0)
if check_only:
    print("would-change")
    raise SystemExit(0)
path.parent.mkdir(parents=True, exist_ok=True)
# Written in place, so a profile's settings.json symlink stays a symlink.
path.write_text(json.dumps(data, indent=2) + "\n")

back = json.loads(path.read_text()).get("hooks", {})
for name, event, matcher, skill in spec:
    found = [
        (current, group)
        for current, groups in back.items()
        if isinstance(groups, list)
        for group in groups
        if isinstance(group, dict) and any(names(h, name) for h in group.get("hooks", []))
    ]
    expected = [(event, wanted(name, matcher, skill))] if mode == "install" else []
    if found != expected:
        print("%s: %s is wired as %r, not %r" % (path, name, found, expected), file=sys.stderr)
        raise SystemExit(1)
print("changed")
'''


# Drops every Claude project and Codex `[projects."<root>"]` table at or under
# the runner's workspace root from the files named on the command line, and
# prints each file it changed (under --check: would change). Python 3.9 safe:
# macOS's /usr/bin/python3 runs it. A Codex table goes with the blank lines
# that set it off; comments just above the next table stay with that table.
RUNNER_WORKSPACE_PY = r'''
import json
import os
import re
import sys
from pathlib import Path

ROOT = os.environ["AK_RUNNER_WORKSPACE"].rstrip("/")
CHECK_ONLY = os.environ["AK_CHECK_ONLY"] == "1"
TABLE = re.compile(r"^\s*\[\[?[^\[\]=,]+\]\]?\s*(?:#.*)?$")
PROJECT = re.compile(r"""^\s*\[\s*projects\s*\.\s*(?:"((?:[^"\\]|\\.)*)"|'([^']*)')\s*\]\s*(?:#.*)?$""")
LOOSE = re.compile(r"^\s*(?:#.*)?$")


def leaked(project):
    return project == ROOT or project.startswith(ROOT + "/")


def write(path, text):
    target = path.resolve()
    mode = target.stat().st_mode & 0o777
    staged = target.with_name(target.name + ".agent-kit-tmp")
    staged.write_text(text)
    os.chmod(str(staged), mode)
    os.replace(str(staged), str(target))


def claude(path):
    try:
        text = path.read_text()
        data = json.loads(text)
    except (OSError, ValueError):
        return False
    projects = data.get("projects") if isinstance(data, dict) else None
    if not isinstance(projects, dict):
        return False
    gone = [name for name in projects if leaked(name)]
    if gone and not CHECK_ONLY:
        for name in gone:
            del projects[name]
        write(path, json.dumps(data, indent=2, ensure_ascii=False) + ("\n" if text.endswith("\n") else ""))
    return bool(gone)


def codex(path):
    try:
        lines = path.read_text().splitlines(True)
    except OSError:
        return False
    kept, held, dropping, found = [], [], False, False
    for line in lines:
        if TABLE.match(line):
            if dropping:
                kept.extend(held)
            held = []
            match = PROJECT.match(line)
            name = None
            if match:
                name = match.group(1) if match.group(1) is not None else match.group(2)
            dropping = name is not None and leaked(name)
            if dropping:
                found = True
                while kept and not kept[-1].strip():
                    kept.pop()
                continue
        elif dropping:
            held = held + [line] if LOOSE.match(line) else []
            continue
        kept.append(line)
    if found and not CHECK_ONLY:
        write(path, "".join(kept))
    return found


for argument in dict.fromkeys(sys.argv[1:]):
    path = Path(argument)
    if path.is_file() and (codex(path) if path.suffix == ".toml" else claude(path)):
        print(argument)
'''


def _leaked_runner_workspace_trust_fn(w: Any) -> None:
    """`purge_leaked_runner_workspace_trust <root>`: the runner's workspace trust in Claude and Codex configs."""
    w("# -----------------------------------------------------------------")
    w("# Run outside its container, the runner entrypoint once trusted its")
    w("# workspace root in the invoking user's .claude.json (a Claude project)")
    w('# and Codex config.toml (a [projects."<root>"] table). Where that root')
    w("# does not exist on this machine, the trust is that leak and nothing else,")
    w("# so it goes from every Claude profile setup manages and from Codex,")
    w("# together with the entries for the repositories under it.")
    w("# -----------------------------------------------------------------")
    w("purge_leaked_runner_workspace_trust() {")
    w('  local root="$1" dir out rc changed')
    w("  local files=()")
    w('  [ -e "${root}" ] && return 0')
    w("  command -v python3 >/dev/null 2>&1 || return 0")
    w('  for dir in "${CLAUDE_PROFILE_DIRS[@]}"; do')
    w('    if [ "${dir}" = "${CLAUDE_HOME}" ] && [ -z "${CLAUDE_CONFIG_DIR:-}" ]; then')
    w('      files+=("${HOME}/.claude.json")')
    w("    else")
    w('      files+=("${dir}/.claude.json")')
    w("    fi")
    w("  done")
    w('  files+=("${CODEX_HOME}/config.toml")')
    # Through a file, as the critique hooks do: bash 3.2 misparses a heredoc in $(...).
    w('  out="$(mktemp)"')
    w('  AK_RUNNER_WORKSPACE="${root}" AK_CHECK_ONLY="${CHECK_ONLY}" \\')
    w('    python3 - "${files[@]}" >"${out}" <<\'RUNNER_WORKSPACE\'')
    for line in RUNNER_WORKSPACE_PY.strip("\n").split("\n"):
        w(line)
    w("RUNNER_WORKSPACE")
    w("  rc=$?")
    w("  changed=\"$(paste -s -d ' ' \"${out}\")\"")
    w('  rm -f "${out}"')
    w('  if [ "${rc}" != 0 ]; then')
    w('    fail "could not check the Claude and Codex configs for the runner\'s ${root} trust"')
    w("    return 0")
    w("  fi")
    w('  [ -n "${changed}" ] || return 0')
    w('  if [ "${CHECK_ONLY}" = 1 ]; then')
    w('    warn "the runner\'s ${root} project trust leaked into ${changed}; run setup to remove it"')
    w("  else")
    w('    ok "fixed: removed the runner\'s leaked ${root} project trust from ${changed}"')
    w("  fi")
    w("}")
    w("")


# The no-agent-attribution rule, one marked block in each agent's global
# instruction file. Plain words only: it is written through a quoted heredoc.
ATTRIBUTION_RULE_TEXT = (
    "Commits, pull requests and issues carry the owner's identity only. Never add a",
    "Co-Authored-By trailer, a Generated by or Generated with line, a session link, or",
    "any other credit to an AI agent, a model or a model provider, and never name one as",
    "the author or reviewer of a change. A git hook refuses such commits.",
    "Never describe AI use in a pull request, commit or issue either: not which agent",
    "or model wrote, reviewed, coordinated or ran any part of the work, and not how",
    "the work was delegated. Describe the change, why, and how it was checked.",
)
# The instruction files that get the rule wherever setup runs; the rest
# (Hermes, OpenCode, each Vibe home, Antigravity) get it where setup writes
# their other instructions.
ATTRIBUTION_RULE_FILES = ('"${CLAUDE_HOME}/CLAUDE.md"', '"${CODEX_HOME}/AGENTS.md"')
ATTRIBUTION_RULE_LOCAL_FILES = ('"${HOME}/.hermes/SOUL.md"',)


def _no_agent_attribution_fns(w: Any) -> None:
    """The no-attribution functions: commits and PRs carry the owner's identity only."""
    w("# -----------------------------------------------------------------")
    w("# The owner's identity only: no commit or PR may credit an AI agent or a")
    w("# model provider. Each agent with a setting for its own attribution gets")
    w("# it turned off (Claude Code here, Mistral Vibe in each Vibe home), and")
    w("# each agent's global instruction file carries the rule. The git hook")
    w("# catches the rest, whichever agent or person wrote the message:")
    w("# installer/git-hooks/no-agent-attribution and its allowlist,")
    w("# installer/git-hooks/allowed-identities, wired as config hooks")
    w("# (hook.<name>.command), which run next to a repository's own hooks")
    w("# instead of replacing them as core.hooksPath would.")
    w("# git commit/push --no-verify still skips it.")
    w("# -----------------------------------------------------------------")
    w("claude_no_attribution() {")
    w("  local profile_dir")
    w('  for profile_dir in "${CLAUDE_PROFILE_DIRS[@]}"; do')
    off = '{"attribution": {"commit": "", "pr": ""}, "includeCoAuthoredBy": false}'
    w(f'    if _merge_user_config "${{profile_dir}}/settings.json" \'{off}\'; then')
    w('      [ "${CHECK_ONLY}" = 1 ] || ok "no commit or PR attribution in ${profile_dir}/settings.json"')
    w("    else")
    w('      fail "could not turn off commit and PR attribution in ${profile_dir}/settings.json"')
    w("    fi")
    w("  done")
    w("}")
    w("")
    w("# Mistral Vibe tells its model to sign every commit unless")
    w("# include_commit_signature is false in the home's config.toml.")
    w("vibe_no_commit_signature() {")
    w('  local home="$1"')
    w('  if [ "${CHECK_ONLY}" = 1 ]; then')
    w('    log "would set include_commit_signature = false in ${home}/config.toml"')
    w("    return 0")
    w("  fi")
    w(f'  uv run --no-project --with tomlkit python "${{KIT_ROOT}}/{VIBE_NO_SIGNATURE}" \\')
    w('    --config "${home}/config.toml" >/dev/null \\')
    w('    && ok "no Vibe commit signature in ${home}/config.toml" \\')
    w('    || fail "could not turn off the Vibe commit signature in ${home}/config.toml"')
    w("}")
    w("")
    w("# The rule against agent attribution: one marked block in an agent's global")
    w("# instruction file. Setup owns only that block; the text around it stays the user's.")
    w("attribution_rule() {")
    w('  local file="$1"')
    w('  if [ "${CHECK_ONLY}" = 1 ]; then')
    w('    log "would write the no-agent-attribution rule in ${file}"')
    w("    return 0")
    w("  fi")
    w('  mkdir -p "$(dirname "${file}")"')
    w(f'  python3 "${{KIT_ROOT}}/{INSTRUCTION_BLOCK}" "${{file}}" no-agent-attribution <<\'BLOCK\' >/dev/null \\')
    w('    && ok "no-agent-attribution rule -> ${file}" \\')
    w('    || warn "no-agent-attribution rule: ${file} left alone; fix its block, then re-run setup"')
    for line in ATTRIBUTION_RULE_TEXT:
        w(line)
    w("BLOCK")
    w("}")
    w("")
    w("# True when git, configured as above, refuses a commit that credits an")
    w("# agent and accepts one that does not. Run in a scratch repository.")
    w("_git_no_agent_attribution_works() {")
    w("  local repo rc=1")
    w('  repo="$(mktemp -d)" || return 1')
    w('  if git -C "${repo}" init -q \\')
    who = "-c user.name=check -c user.email=check@example.invalid -c commit.gpgsign=false"
    w(f'    && ! git -C "${{repo}}" {who} commit -q --allow-empty \\')
    w('      -m "check" -m "Co-Authored-By: Mistral Vibe <vibe@mistral.ai>" >/dev/null 2>&1 \\')
    w(f'    && git -C "${{repo}}" {who} commit -q --allow-empty \\')
    w('      -m "check" >/dev/null 2>&1; then')
    w("    rc=0")
    w("  fi")
    w('  rm -rf "${repo}"')
    w('  return "${rc}"')
    w("}")
    w("")
    w("git_no_agent_attribution() {")
    w('  local dir="${HOME}/.config/agent-kit/git-hooks" event')
    w('  local dest="${dir}/no-agent-attribution"')
    w("  if ! command -v git >/dev/null 2>&1; then")
    w('    warn "git is not on PATH; the no-agent-attribution git hook is not installed"')
    w("    return 0")
    w("  fi")
    w('  if [ "${CHECK_ONLY}" = 1 ]; then')
    w('    log "would install the no-agent-attribution hook and its allowlist in ${dir} (commit-msg, pre-push)"')
    w("    return 0")
    w("  fi")
    w('  if ! mkdir -p "${dir}" \\')
    w('    || ! install -m 0755 "${KIT_ROOT}/installer/git-hooks/no-agent-attribution" "${dest}" \\')
    w('    || ! install -m 0644 "${KIT_ROOT}/installer/git-hooks/allowed-identities" "${dir}/allowed-identities"; then')
    w('    fail "could not install the no-agent-attribution git hook in ${dir}"')
    w("    return 0")
    w("  fi")
    w("  for event in commit-msg pre-push; do")
    w('    if ! git config --global "hook.agent-kit-no-attribution-${event}.command" "\\"${dest}\\" ${event}" \\')
    w('      || ! git config --global "hook.agent-kit-no-attribution-${event}.event" "${event}"; then')
    w('      fail "could not register the no-agent-attribution ${event} hook in ~/.gitconfig"')
    w("      return 0")
    w("    fi")
    w("  done")
    w("  # A stubbed git (the test harness) prints no version; there is nothing to check.")
    w('  case "$(git --version 2>/dev/null)" in')
    w('    "git version "*) ;;')
    w('    *) ok "no-agent-attribution git hook registered for commit-msg and pre-push"; return 0 ;;')
    w("  esac")
    w("  if _git_no_agent_attribution_works; then")
    w('    ok "no-agent-attribution git hook refuses commits that credit an agent"')
    w("  else")
    w(
        '    fail "the no-agent-attribution git hook is registered but did not refuse a test commit;'
        ' $(git --version) may not run hooks from config (hook.<name>.command)"'
    )
    w("  fi")
    w("}")
    w("")

def _leaked_runner_git_config_fn(w: Any) -> None:
    """`purge_leaked_runner_git_config`: the runner's git settings that leaked into a workstation's ~/.gitconfig."""
    w("# -----------------------------------------------------------------")
    w("# The runner entrypoint (runner/template/entrypoint.sh) once")
    w("# wrote its global git settings wherever it ran. On a workstation they")
    w("# name the credential helper agent-gh-app, which only the runner image")
    w("# ships, so every push printed \"git: 'credential-agent-gh-app' is not a")
    w('# git command". This removes that helper and the two settings the same')
    w("# run wrote with it: credential.useHttpPath and safe.directory /workspace.")
    w("# Neither goes without the helper: alone, either may be the owner's own.")
    w("# Where git-credential-agent-gh-app is on PATH the settings are real.")
    w("# -----------------------------------------------------------------")
    w("purge_leaked_runner_git_config() {")
    w("  local helpers removed")
    w("  command -v git >/dev/null 2>&1 || return 0")
    w('  helpers="$(git config --global --get-all credential.helper 2>/dev/null || true)"')
    w("  printf '%s\\n' \"${helpers}\" | grep -qxF agent-gh-app || return 0")
    w("  command -v git-credential-agent-gh-app >/dev/null 2>&1 && return 0")
    w('  if [ "${CHECK_ONLY}" = 1 ]; then')
    w(
        '    warn "your ~/.gitconfig names the runner\'s credential helper agent-gh-app, '
        'which is not on PATH; run setup to remove it"',
    )
    w("    return 0")
    w("  fi")
    w("  if ! git config --global --unset-all credential.helper '^agent-gh-app$'; then")
    w('    fail "could not remove the runner\'s credential helper agent-gh-app from ~/.gitconfig"')
    w("    return 0")
    w("  fi")
    w('  removed="credential.helper agent-gh-app"')
    w("  git config --global --unset-all credential.useHttpPath '^true$' 2>/dev/null \\")
    w('    && removed="${removed}, credential.useHttpPath"')
    w("  git config --global --unset-all safe.directory '^/workspace$' 2>/dev/null \\")
    w('    && removed="${removed}, safe.directory /workspace"')
    w('  ok "fixed: removed the runner\'s leaked git settings from ~/.gitconfig (${removed})"')
    w("}")
    w("")


def _critique_hooks_fn(w: Any) -> None:
    """`critique_hooks install|remove`: the three critique hooks, merged by name into Claude Code and Codex."""
    hooks = critique_hooks()
    w("# -----------------------------------------------------------------")
    w("# The critique hooks (docs/adr/0002-exactly-three-critique-hooks.md):")
    w("# merged by NAME into Claude Code's settings.json and Codex's")
    w("# hooks.json, so the operator's own hooks survive, and removed by")
    w("# --uninstall. Each command is a fixed path into the agent home's")
    w("# critique skill, never a version, so an upgrade leaves the definition")
    w("# Codex trusted unchanged. Codex still runs them only once the owner")
    w("# trusts them in /hooks; /flow setup checks that.")
    w("# -----------------------------------------------------------------")
    w("_critique_hooks() {")
    w('  local mode="$1" home="$2" file="$3" label="$4" spec="$5" names="$6"')
    w('  local path="${home}/${file}" out rc name')
    w("  if ! command -v python3 >/dev/null 2>&1; then")
    w('    warn "python3 is not on PATH; cannot merge the critique hooks into ${path}"')
    w("    return 0")
    w("  fi")
    w('  if [ "${mode}" = install ] && [ "${CHECK_ONLY}" != 1 ]; then')
    w("    for name in ${names}; do")
    w(f'      if [ ! -f "${{home}}/{HOOK_SKILLS}/${{name}}.sh" ]; then')
    missing = f"${{home}}/{HOOK_SKILLS}/${{name}}.sh is missing"
    w(f'        fail "critique hooks: {missing}; not wired into ${{path}}"')
    w("        return 0")
    w("      fi")
    w("    done")
    w("  fi")
    # The result word goes through a file: bash 3.2 misparses a heredoc
    # inside $(...) whose body has unbalanced quotes.
    w('  out="$(mktemp)"')
    w('  AK_HOOKS_FILE="${path}" AK_HOOKS_HOME="${home}" AK_HOOKS_MODE="${mode}" \\')
    w(f'    AK_CHECK_ONLY="${{CHECK_ONLY}}" AK_HOOKS_SPEC="${{spec}}" AK_HOOKS_TIMEOUT={CRITIQUE_HOOK_TIMEOUT} \\')
    w('    python3 - >"${out}" <<\'CRITIQUE_HOOKS\'')
    for line in CRITIQUE_HOOKS_PY.strip("\n").split("\n"):
        w(line)
    w("CRITIQUE_HOOKS")
    w("  rc=$?")
    w('  name="$(cat "${out}")"')
    w('  rm -f "${out}"')
    w('  out="${name}"')
    w('  case "${rc}:${mode}:${out}" in')
    w('    0:install:changed) ok "critique hooks installed in ${label} (${path})" ;;')
    w('    0:remove:changed) ok "critique hooks removed from ${label} (${path})" ;;')
    w('    0:install:unchanged) ok "critique hooks already installed in ${label} (${path})" ;;')
    w('    0:remove:unchanged) ok "no critique hooks in ${label} (${path})" ;;')
    w('    0:install:would-change) log "would install the critique hooks in ${label} (${path})" ;;')
    w('    0:remove:would-change) log "would remove the critique hooks from ${label} (${path})" ;;')
    w('    2:*) warn "critique hooks: ${path} left untouched; fix it, then re-run setup" ;;')
    w('    *) fail "critique hooks: merging into ${path} failed" ;;')
    w("  esac")
    w("}")
    w("")
    w("critique_hooks() {")
    for agent, (home_var, file, label) in CRITIQUE_HOOK_HOMES.items():
        mine = [h for h in hooks if agent in h["agents"]]
        if not mine:
            continue
        spec = json.dumps(
            [
                [h["name"], h["event"], _agent_matcher(h["matcher"], agent), h["skill"]]
                for h in mine
            ],
            separators=(",", ":"),
        )
        names = " ".join(f'{h["skill"]}/hooks/{h["name"]}' for h in mine)
        w(f'  _critique_hooks "$1" "${{{home_var}}}" {file} {_q(label)} {_q(spec)} {_q(names)}')
    w("}")
    w("")


def _retired_vibe_hooks() -> str:
    return " ".join(f"--remove {n}" for n in RETIRED_VIBE_HOOKS)


def _agents_pointer_text(w: Any, private: bool) -> None:
    """The AGENTS.md a Vibe home gets. Only vibe-private's mentions Private Memory."""
    w("${marker}")
    w("# Agent contract")
    w("")
    w("The estate-wide conventions live in one place:")
    w("https://github.com/JorisJonkers-dev/workspace/blob/main/CLAUDE.md")
    w("")
    w("Read it before doing anything non-trivial in an estate repository.")
    w("")
    w("Multi-ticket builds (/implement-spec, a council run, a /flow feature in implementation):"
      " keep the build tracker current with the build-tracker skill.")
    w("")
    w("## Memory")
    w("")
    # Plain names: this is an unquoted heredoc, so a backtick would run a command.
    if not private:
        w("One Hindsight bank is connected: memory-api, for estate and programming")
        w("knowledge. At the start of a task, recall from it. Every agent reads it,")
        w("so never retain personal notes into it.")
        return
    w("Two Hindsight banks are connected. At the start of a task, recall from both:")
    w("memory-private for personal context (people, plans, private notes) and")
    w("memory-api for estate and programming knowledge.")
    w("")
    w("Save to memory-private only what is worth keeping long-term: facts about")
    w("people and the user, plans, decisions and their reasons, and anything the")
    w("user asks you to remember. Not small talk, not passing task details, never")
    w("secrets. Update an existing memory rather than adding a near-duplicate.")
    w("Never retain personal notes into memory-api: Hermes and every other agent")
    w("read it.")


def declared_servers(data: dict[str, Any], surface: str) -> list[str]:
    """MCP servers a local surface (claude, codex, vibe, vibe-private or opencode) is declared to hold.

    ``vibe`` is the shared Vibe home. A session-only server (Private Memory)
    is declared only for the launcher it is enabled for. OpenCode gets no
    ``hindsight`` server: Hindsight Coding Agents installs only for Claude
    Code and Codex.
    """
    if surface == "opencode":
        return [str(s["name"]) for s in opencode_servers(data)]
    if surface in ("vibe", "vibe-private"):
        names = [str(s["name"]) for s in vibe_servers(data, surface)]
    else:
        retired = set(retired_servers(data))
        names = [str(s["name"]) for s in _entries(data, "mcp_servers")
                 if on_surface(s, "workstation") and s["name"] not in retired and not session_only(s)]
    if data.get("coding_agents"):
        names.append("hindsight")
    return names


def declared_plugins(data: dict[str, Any]) -> list[str]:
    """Plugin ids the registry declares, as Claude Code shows them (name@marketplace)."""
    official = declared_name(data, "claude-plugins-official")
    return [f"{p['name']}@{declared_name(data, p['marketplace'])}" for p in _entries(data, "plugins")] + [
        f"{s['plugin']}@{official}" for s in _entries(data, "language_servers")
    ]


def _converge_plugins(data: dict[str, Any], w: Any) -> None:
    retired = data.get("retired_plugins") or []
    markets = ",".join(declared_name(data, m["name"]) for m in _entries(data, "marketplaces"))
    w("# Removes retired and undeclared marketplaces and plugins; the keep file spares undeclared ones.")
    w("converge_plugins() {")
    w("  local out rc=0 line")
    w('  local args=(--keep "${HOME}/.config/agent-kit/keep.yaml")')
    w('  [ "${CHECK_ONLY}" = 1 ] && args+=(--check)')
    w("  if ! command -v uv >/dev/null 2>&1; then")
    w('    warn "uv is not on PATH; plugins not converged"')
    w("    return 0")
    w("  fi")
    w('  out=$(uv run --no-project --with pyyaml --with tomlkit python "${KIT_ROOT}/scripts/converge.py" plugins \\')
    w(f'    "${{args[@]}}" --declared-markets {markets} \\')
    w(f"    --declared-plugins {','.join(declared_plugins(data))} \\")
    w(f"    --retired-markets {','.join(str(r['marketplace']) for r in retired)} \\")
    w(f"    --retired-plugins {','.join(str(r['ref']) for r in retired)} 2>&1) || rc=$?")
    w('  while IFS= read -r line; do [ -n "${line}" ] && log "${line}"; done <<< "${out}"')
    w('  [ "${rc}" = 0 ] || fail "plugins did not converge (see above)"')
    w("}")
    w("")


def _converge_mcp(data: dict[str, Any], w: Any) -> None:
    w("# Removes retired and undeclared MCP servers from every surface; the keep file spares undeclared ones.")
    w("converge_mcp() {")
    w("  local out rc=0 line dir")
    w('  local args=(--keep "${HOME}/.config/agent-kit/keep.yaml" --claude-home "${CLAUDE_HOME}")')
    w('  for dir in "${CLAUDE_PROFILE_DIRS[@]}"; do args+=(--claude "${dir}"); done')
    w('  command -v codex >/dev/null 2>&1 && args+=(--codex "${CODEX_HOME}/config.toml")')
    w("  # After a failed split the shared home's Private Memory entry must stay: it")
    w("  # is what marks the home for the next run's split.")
    w('  [ "${VIBE_HOMES_READY}" = 1 ] \\')
    w('    && args+=(--vibe "${VIBE_HOME}/config.toml" --vibe-private "${VIBE_PRIVATE_HOME}/config.toml")')
    w('  [ "${OPENCODE_READY}" = 1 ] && [ "${OPENCODE_V1}" != 1 ] && command -v opencode >/dev/null 2>&1 \\')
    w('    && args+=(--opencode "${OPENCODE_HOME}/opencode.json")')
    w('  [ "${CHECK_ONLY}" = 1 ] && args+=(--check)')
    w("  if ! command -v uv >/dev/null 2>&1; then")
    w('    warn "uv is not on PATH; MCP servers not converged"')
    w("    return 0")
    w("  fi")
    w('  out=$(uv run --no-project --with pyyaml --with tomlkit python "${KIT_ROOT}/scripts/converge.py" mcp \\')
    w('    "${args[@]}" \\')
    for surface in ("claude", "codex", "vibe", "vibe-private", "opencode"):
        w(f"    --declared {surface}={','.join(declared_servers(data, surface))} \\")
    private = ",".join(str(s["name"]) for s in _entries(data, "mcp_servers") if session_only(s))
    w(f'    --retired {",".join(retired_servers(data))} --private {private} 2>&1) || rc=$?')
    w('  while IFS= read -r line; do [ -n "${line}" ] && log "${line}"; done <<< "${out}"')
    w('  [ "${rc}" = 0 ] || fail "MCP servers did not converge (see above)"')
    w("}")
    w("")


def _critic_agents_step(data: dict[str, Any], w: Any) -> None:
    """The shell function that converges the critic agents in one Vibe home.

    Each declared agent's file is replaced, so a hand edit is undone; a retired
    one's is removed; any other agent in the directory is the owner's and is
    left alone. One line per file keeps every path setup reads from the kit
    literal, which is how the bundle test sees it.
    """
    w("# The read-only critic agents the critique runner launches as")
    w("# `vibe --agent critic-<model>`. Called for the shared Vibe home only:")
    w("# vibe-private never runs critics. Nothing here touches config.toml, so")
    w("# no critic becomes the default agent.")
    w("_vibe_critic_agents() {")
    w('  local home="$1" src dest')
    for agent in vibe_critic_agents(data):
        name = agent["name"]
        w(f'  src="${{KIT_ROOT}}/{VIBE_CRITIC_AGENTS_DIR}/{name}.toml"')
        w(f'  dest="${{home}}/agents/{name}.toml"')
        w('  if [ "${CHECK_ONLY}" = 1 ]; then')
        w('    log "would write ${dest}"')
        w('  elif mkdir -p "${home}/agents" && cp "${src}" "${dest}" && cmp -s "${src}" "${dest}"; then')
        w(f'    ok "Vibe critic agent {name} -> ${{dest}}"')
        w("  else")
        w(f'    fail "Vibe critic agent {name}: ${{dest}} does not match ${{src}}"')
        w("  fi")
    for name in retired_vibe_critic_agents(data):
        w(f'  dest="${{home}}/agents/{name}.toml"')
        w('  if [ ! -e "${dest}" ]; then')
        w("    :")
        w('  elif [ "${CHECK_ONLY}" = 1 ]; then')
        w('    log "would remove ${dest}"')
        w("  else")
        w('    rm -f "${dest}"')
        w(f'    ok "removed retired Vibe critic agent {name}"')
        w("  fi")
    w("  return 0")
    w("}")
    w("")


def _delegate_agents_step(data: dict[str, Any], w: Any) -> None:
    """The shell function that converges the delegate agents in one Vibe home.

    Each declared agent's file is replaced, so a hand edit is undone; a retired
    one's is removed; any other agent in the directory is the owner's and is
    left alone. One line per file keeps every path setup reads from the kit
    literal, which is how the bundle test sees it.
    """
    w("# The delegate agents the model gateway launches as")
    w("# `vibe --agent delegate-<what it may do>`. Called for the shared Vibe")
    w("# home only: vibe-private never delegates. Nothing here touches")
    w("# config.toml, so no delegate becomes the default agent.")
    w("_vibe_delegate_agents() {")
    w('  local home="$1" src dest')
    for agent in vibe_delegate_agents(data):
        name = agent["name"]
        w(f'  src="${{KIT_ROOT}}/{VIBE_DELEGATE_AGENTS_DIR}/{name}.toml"')
        w(f'  dest="${{home}}/agents/{name}.toml"')
        w('  if [ "${CHECK_ONLY}" = 1 ]; then')
        w('    log "would write ${dest}"')
        w('  elif mkdir -p "${home}/agents" && cp "${src}" "${dest}" && cmp -s "${src}" "${dest}"; then')
        w(f'    ok "Vibe delegate agent {name} -> ${{dest}}"')
        w("  else")
        w(f'    fail "Vibe delegate agent {name}: ${{dest}} does not match ${{src}}"')
        w("  fi")
    for name in retired_vibe_delegate_agents(data):
        w(f'  dest="${{home}}/agents/{name}.toml"')
        w('  if [ ! -e "${dest}" ]; then')
        w("    :")
        w('  elif [ "${CHECK_ONLY}" = 1 ]; then')
        w('    log "would remove ${dest}"')
        w("  else")
        w('    rm -f "${dest}"')
        w(f'    ok "removed retired Vibe delegate agent {name}"')
        w("  fi")
    w("  return 0")
    w("}")
    w("")


def opencode_plugin_ids(data: dict[str, Any]) -> list[str]:
    """The enabled Claude Code plugins whose skills OpenCode gets, as Claude shows them."""
    return [
        f"{p['name']}@{declared_name(data, p['marketplace'])}"
        for p in _entries(data, "plugins")
        if p.get("enabled") is not False
    ]


def _opencode_plugin_skills_step(data: dict[str, Any], w: Any) -> None:
    """Copy the enabled Claude plugins' skills into OpenCode, then reload its service.

    OpenCode reads ``~/.claude/skills`` but not Claude's plugin cache, so a
    plugin's skills reach it only as copies. Hooks, agents and commands stay
    Claude's: OpenCode has no format that runs them.
    """
    ids = ",".join(opencode_plugin_ids(data))
    w("# -----------------------------------------------------------------")
    w("# 8b. OpenCode: the skills of the enabled Claude Code plugins, copied")
    w("# from where Claude installed them. A plugin's hooks, agents and commands")
    w("# stay Claude's. Then the background service reloads, because it reads")
    w("# its config and skills once, at start.")
    w("# -----------------------------------------------------------------")
    w('if [ "${OPENCODE_READY}" = 1 ]; then')
    w('  log "OpenCode plugin skills"')
    w("  if command -v claude >/dev/null 2>&1; then")
    w('    opencode_skill_args=(--skills-dir "${OPENCODE_HOME}/skills" --plugins ' + _q(ids) + ")")
    w('    [ "${CHECK_ONLY}" = 1 ] && opencode_skill_args+=(--check)')
    w('    opencode_skill_rc=0')
    w('    opencode_skill_out="$(python3 "${KIT_ROOT}/scripts/opencode-plugin-skills.py" \\')
    w('      "${opencode_skill_args[@]}" 2>&1)" || opencode_skill_rc=$?')
    w('    while IFS= read -r line; do [ -n "${line}" ] && log "${line}"; done <<< "${opencode_skill_out}"')
    w('    [ "${opencode_skill_rc}" = 0 ] || fail "OpenCode plugin skills did not all land (see above)"')
    w("  else")
    w('    warn "claude not on PATH; OpenCode gets no plugin skills"')
    w("  fi")
    w('  if [ "${CHECK_ONLY}" != 1 ] && command -v opencode >/dev/null 2>&1 \\')
    w("     && opencode service status 2>/dev/null | grep -q '^http'; then")
    w("    if opencode service restart >/dev/null 2>&1; then")
    w('      ok "OpenCode service restarted"')
    w("    else")
    w('      warn "opencode service restart failed; restart it to load the new config"')
    w("    fi")
    w("  fi")
    w("fi")
    w("")


def claude_allow_rules(data: dict[str, Any]) -> list[str]:
    """Claude permission rules for MCP servers with ``claude_allow_tools: true``.

    ``mcp__<server>__*`` matches every tool the server offers, so Claude Code
    runs them without asking. Only servers setup registers for Claude Code (the
    workstation surface) qualify.
    """
    return [
        f"mcp__{server['name']}__*"
        for server in _entries(data, "mcp_servers")
        if server.get("claude_allow_tools") is True and "workstation" in (server.get("surfaces") or [])
    ]


# The model gateway's pointer text, one marked block in each client's global
# instruction file. Plain words only: it is written through a quoted heredoc.
GATEWAY_POINTER_TEXT = (
    "The model gateway is the `model-gateway` command on PATH. `model-gateway models`",
    "lists the client models on this machine, each with its cost class and difficulty tier.",
    "The `delegate` skill, which is coming later, plans delegations with it.",
)
INSTRUCTION_BLOCK = Path("scripts/instruction-block.py")
PACKAGE_PROXY_MERGE = Path("scripts/package-proxy-merge.py")


def _gateway_pointer_fn(w: Any) -> None:
    """The gateway_pointer function: writes or removes setup's marked block in one instruction file."""
    w("# The model gateway's pointer: one marked block in a client's global instruction")
    w("# file. Setup owns only that block; the text around it stays the user's.")
    w("gateway_pointer() {")
    w('  local file="$1" action="${2:-write}"')
    w('  if [ "${CHECK_ONLY}" = 1 ]; then')
    w('    log "would ${action} the model-gateway pointer in ${file}"')
    w("    return 0")
    w("  fi")
    w('  if [ "${action}" = remove ]; then')
    w('    [ -f "${file}" ] || return 0')
    w(f'    python3 "${{KIT_ROOT}}/{INSTRUCTION_BLOCK}" "${{file}}" model-gateway --remove >/dev/null \\')
    w('      && ok "model-gateway pointer removed from ${file}" \\')
    w('      || warn "model-gateway pointer: ${file} left alone; fix its block, then re-run setup"')
    w("    return 0")
    w("  fi")
    w('  mkdir -p "$(dirname "${file}")"')
    w(f'  python3 "${{KIT_ROOT}}/{INSTRUCTION_BLOCK}" "${{file}}" model-gateway <<\'BLOCK\' \\')
    w('    && ok "model-gateway pointer -> ${file}" \\')
    w('    || warn "model-gateway pointer: ${file} left alone; fix its block, then re-run setup"')
    for line in GATEWAY_POINTER_TEXT:
        w(line)
    w("BLOCK")
    w("}")
    w("")


def _gateway_step(w: Any, declared: bool) -> None:
    """The model gateway's command and pointers, or their retirement when it is not declared here."""
    w("# -----------------------------------------------------------------")
    w("# The model gateway: the `model-gateway` command on PATH, and a pointer to it in")
    w("# each client's global instruction file. A gateway that is not declared on this")
    w("# surface is retired: its command and its pointers are removed, if setup wrote them.")
    w("# -----------------------------------------------------------------")
    if declared:
        w("install_kit_command model-gateway")
        w('gateway_pointer "${CLAUDE_HOME}/CLAUDE.md"')
        w('gateway_pointer "${CODEX_HOME}/AGENTS.md"')
        w('if [ "${CLOUD}" != 1 ]; then')
        w('  gateway_pointer "${HOME}/.hermes/SOUL.md"')
        w('  gateway_pointer "${XDG_CONFIG_HOME:-${HOME}/.config}/opencode/AGENTS.md"')
        w("fi")
    else:
        w("retire_kit_command model-gateway")
        for home_file in (
            '"${CLAUDE_HOME}/CLAUDE.md"',
            '"${CODEX_HOME}/AGENTS.md"',
            '"${VIBE_HOME}/AGENTS.md"',
            '"${VIBE_PRIVATE_HOME}/AGENTS.md"',
            '"${ANTIGRAVITY_HOME}/AGENTS.md"',
            '"${HOME}/.hermes/SOUL.md"',
            '"${XDG_CONFIG_HOME:-${HOME}/.config}/opencode/AGENTS.md"',
        ):
            w(f"gateway_pointer {home_file} remove")
    w("")


# The projects layout's rule, one marked block in each client's global
# instruction file (docs/adr/0009-projects-layout-guard.md). Plain words only:
# it is written through a quoted heredoc.
LAYOUT_POINTER_TEXT = (
    "Clones live at `~/Projects/<owner>/<repo>` (GitHub owner, exact case). Use an existing",
    "clone instead of cloning again, even into /tmp. Worktrees live at",
    "`~/Projects/<owner>/worktrees/<repo>/<name>`, never inside the repository:",
    "`git -C ~/Projects/<owner>/<repo> worktree add ~/Projects/<owner>/worktrees/<repo>/<name> -b <branch>`.",
    "In Claude Code, create the worktree that way, then call `EnterWorktree` with `path`, never `name`.",
    "The projects-layout skill has the details.",
)
LAYOUT_POINTER_FILES = (
    '"${CLAUDE_HOME}/CLAUDE.md"',
    '"${CODEX_HOME}/AGENTS.md"',
    '"${HOME}/.hermes/SOUL.md"',
    '"${XDG_CONFIG_HOME:-${HOME}/.config}/opencode/AGENTS.md"',
)


def _layout_pointer_fn(w: Any) -> None:
    """The layout_pointer function: writes or removes the projects-layout block in one instruction file."""
    w("# The projects layout's rule: one marked block in a client's global instruction")
    w("# file. Setup owns only that block; the text around it stays the user's.")
    w("layout_pointer() {")
    w('  local file="$1" action="${2:-write}"')
    w('  if [ "${CHECK_ONLY}" = 1 ]; then')
    w('    log "would ${action} the projects-layout rule in ${file}"')
    w("    return 0")
    w("  fi")
    w('  if [ "${action}" = remove ]; then')
    w('    [ -f "${file}" ] || return 0')
    w(f'    python3 "${{KIT_ROOT}}/{INSTRUCTION_BLOCK}" "${{file}}" projects-layout --remove >/dev/null \\')
    w('      && ok "projects-layout rule removed from ${file}" \\')
    w('      || warn "projects-layout rule: ${file} left alone; fix its block, then re-run setup"')
    w("    return 0")
    w("  fi")
    w('  mkdir -p "$(dirname "${file}")"')
    w(f'  python3 "${{KIT_ROOT}}/{INSTRUCTION_BLOCK}" "${{file}}" projects-layout <<\'BLOCK\' \\')
    w('    && ok "projects-layout rule -> ${file}" \\')
    w('    || warn "projects-layout rule: ${file} left alone; fix its block, then re-run setup"')
    for line in LAYOUT_POINTER_TEXT:
        w(line)
    w("BLOCK")
    w("}")
    w("")


def _layout_step(w: Any, declared: bool) -> None:
    """The projects-layout rule in each client's instruction file, or its removal when not declared."""
    w("# -----------------------------------------------------------------")
    w("# The projects layout: its rule in each client's global instruction file.")
    w("# A cloud session has no ~/Projects, so it gets no rule. A layout that is")
    w("# not declared on this surface is retired: setup removes the blocks it wrote.")
    w("# -----------------------------------------------------------------")
    if declared:
        w('if [ "${CLOUD}" != 1 ]; then')
        for home_file in LAYOUT_POINTER_FILES:
            w(f"  layout_pointer {home_file}")
        w("fi")
    else:
        for home_file in LAYOUT_POINTER_FILES:
            w(f"layout_pointer {home_file} remove")
    w("")


def _attribution_rule_step(w: Any) -> None:
    """The rule against agent attribution in each agent's global instruction file."""
    w("# -----------------------------------------------------------------")
    w("# No agent attribution: the rule in each agent's global instruction file.")
    w("# Vibe, Antigravity and OpenCode get it with their other instructions above.")
    w("# -----------------------------------------------------------------")
    for home_file in ATTRIBUTION_RULE_FILES:
        w(f"attribution_rule {home_file}")
    w('if [ "${CLOUD}" != 1 ]; then')
    for home_file in ATTRIBUTION_RULE_LOCAL_FILES:
        w(f"  attribution_rule {home_file}")
    w("fi")


# The mirror Maven gets: every external repository, but not the first-party
# `github` one (the mirror exclusion Gradle's isAllowed() also grants).
MAVEN_MIRROR_OF = "external:*,!github"

# The Gradle init script, copied from the estate's setup-java-gradle action
# (github-workflows, actions/setup-java-gradle/action.yml): every third-party
# Maven repository URL is rewritten in place to the proxy, nothing is added
# or removed. The one change: the action read the token from the environment,
# this script reads
# it through packagesProxyToken() -- the Gradle property `packagesProxyToken`
# (-PpackagesProxyToken on the command line, else the gradle.properties in
# the Gradle user home, read with java.util.Properties), else the
# PACKAGES_PROXY_TOKEN environment variable, else an empty string. The init
# script never holds the key itself, so it is written world-readable. The
# four maven URLs and the two allowed hosts are %(...)s placeholders filled
# from the registry (package_proxy.maven, and the base/first-party hosts),
# so the registry stays the one place the URLs live.
GRADLE_INIT_KTS = """\
// Route dependency resolution through the packages proxy. First-party GitHub
// Packages and local (file) repositories stay as they are; every other Maven
// repository keeps its place but has its URL rewritten to the proxy's
// equivalent, with the proxy's credentials. Nothing is added or removed, so
// repository order, the settings-versus-project choice and
// FAIL_ON_PROJECT_REPOS all behave as the build declared them.
fun proxyUrlFor(url: java.net.URI): String? {
    val host = url.host ?: return null
    if (url.scheme == "file" || host == "%(github_host)s" || host == "%(proxy_host)s") return null
    val path = url.path ?: ""
    return when {
        host == "repo.spring.io" && path.startsWith("/milestone") -> "%(spring_milestone)s"
        host.endsWith("sonatype.org") ||
            host == "central.sonatype.com" && path.contains("snapshots") -> "%(sonatype_snapshots)s"
        host == "maven.google.com" || host == "dl.google.com" -> "%(google_maven)s"
        else -> "%(main)s" // Maven Central, the Gradle Plugin Portal, and any other mirror of Central
    }
}

fun RepositoryHandler.usePackagesProxy() {
    all {
        val repo = this as? MavenArtifactRepository ?: return@all
        val target = runCatching { proxyUrlFor(repo.url) }.getOrNull() ?: return@all
        repo.setUrl(target)
        repo.packagesProxyAuthentication()
    }
}

// The token is never in this file (it is world-readable): Gradle takes it
// from the property `packagesProxyToken` -- the -PpackagesProxyToken of the
// command line, else the gradle.properties of the Gradle user home -- or
// from the environment, and a build without any of them runs keyless.
fun packagesProxyToken(): String {
    (gradle.startParameter.projectProperties["packagesProxyToken"] as? String)
        ?.takeIf { it.isNotEmpty() }?.let { return it }
    val userHomeProperties = java.io.File(gradle.gradleUserHomeDir, "gradle.properties")
    if (userHomeProperties.isFile) {
        val properties = java.util.Properties()
        userHomeProperties.inputStream().use { properties.load(it) }
        val fromUserHome = properties.getProperty("packagesProxyToken")
        if (!fromUserHome.isNullOrEmpty()) return fromUserHome
    }
    return System.getenv("PACKAGES_PROXY_TOKEN") ?: ""
}

fun MavenArtifactRepository.packagesProxyAuthentication() {
    credentials(HttpHeaderCredentials::class) {
        name = "Authorization"
        value = "Bearer " + packagesProxyToken()
    }
    authentication {
        create<HttpHeaderAuthentication>("header")
    }
}

settingsEvaluated {
    pluginManagement.repositories.usePackagesProxy()
    dependencyResolutionManagement.repositories.usePackagesProxy()
}

beforeProject {
    buildscript.repositories.usePackagesProxy()
    repositories.usePackagesProxy()
}
"""


def _package_proxy_step(w: Any, data: dict[str, Any]) -> None:
    """The packages proxy setup step: resolve the key, verify it, store it,
    then write the files that point the tools at the proxy.

    The step runs right after ``tokens_and_keys``: everything after it reads
    the proxy through the config files this step writes. The key is resolved
    from PACKAGES_PROXY_TOKEN, the vault secret or the stored token file (in
    that order); the token file is written only once the key verifies. The
    writers own the netrc's proxy machine entry, go's env file, one marked
    block each of the npmrc, the uv and pip configs and the gradle
    properties, two files of the cargo home, the Gradle init script and
    Maven's settings.xml (create-only) -- nothing else in those files.
    Everything runs under the calling shell's own ``log``/``warn`` helpers,
    and the key never reaches argv or a log line.
    """
    proxy = data.get("package_proxy") or {}
    verify_url = str(proxy.get("verify_url") or "")
    token_file = str(proxy.get("token_file") or "")
    vault_path = str(proxy.get("vault_path") or "")
    vault_field = str(proxy.get("vault_field") or "")
    base = str(proxy.get("base") or "")
    npm_url = str(proxy.get("npm") or "")
    go_url = str(proxy.get("go") or "")
    go_private = str((proxy.get("first_party") or {}).get("go_private") or "")
    pypi_url = str(proxy.get("pypi") or "")
    cargo_url = str(proxy.get("cargo") or "")
    maven = {name: str(url) for name, url in (proxy.get("maven") or {}).items()}
    if not (
        verify_url and token_file and vault_path and vault_field
        and base and npm_url and go_url and pypi_url and cargo_url and go_private
        and maven.get("main") and maven.get("spring_milestone")
        and maven.get("sonatype_snapshots") and maven.get("google_maven")
    ):
        return
    # netrc keys on the bare host; npm's token line is the registry URL with
    # the scheme taken off, in npmrc's //host/path/ form.
    host = base.removeprefix("https://")
    npm_auth = "//" + npm_url.removeprefix("https://")
    # The Gradle init script's two allowed maven hosts: the first-party
    # GitHub Packages host, and the proxy's own (a repository a build script
    # already points at the proxy is not re-routed through it twice).
    github_maven_host = str((proxy.get("first_party") or {}).get("maven_github") or "")
    github_maven_host = github_maven_host.removeprefix("https://").split("/")[0]
    init_kts = GRADLE_INIT_KTS % {
        **maven,
        "github_host": github_maven_host,
        "proxy_host": host,
    }
    w("# -----------------------------------------------------------------")
    w("# The packages proxy")
    w("# -----------------------------------------------------------------")
    w("")
    w("# pp_resolve_key prints the proxy key (always captured, never logged),")
    w("# in order: PACKAGES_PROXY_TOKEN, the vault secret, the stored token file.")
    w("# The last resort is a prompt, and only in an interactive run.")
    w("pp_resolve_key() {")
    w('  local root="$1"')
    w(f'  local token="${{XDG_CONFIG_HOME:-${{root}}/.config}}/{token_file}"')
    w('  local key=""')
    w('  if [ -n "${PACKAGES_PROXY_TOKEN:-}" ]; then')
    w('    printf \'%s\' "${PACKAGES_PROXY_TOKEN}"')
    w("    return 0")
    w("  fi")
    w('  if command -v vault >/dev/null 2>&1 && vault token lookup >/dev/null 2>&1; then')
    w(f'    key="$(vault kv get -mount=secret -field={vault_field} {vault_path} 2>/dev/null)"')
    w('    if [ -n "${key}" ]; then')
    w("      printf '%s' \"${key}\"")
    w("      return 0")
    w("    fi")
    w("  fi")
    w('  if [ -s "${token}" ]; then')
    w('    printf \'%s\' "$(cat "${token}" 2>/dev/null)"')
    w("    return 0")
    w("  fi")
    w("  if interactive; then")
    w("    printf 'packages proxy key: ' >&2")
    w('    read -rs key <"${PROMPT_TTY}"')
    w("    printf '\\n' >&2")
    w('    if [ -n "${key}" ]; then')
    w("      printf '%s' \"${key}\"")
    w("      return 0")
    w("    fi")
    w("  fi")
    w("  return 1")
    w("}")
    w("")
    w("# pp_store_key writes the key to the token file, atomically and 0600: a")
    w("# temp file under umask 077 in the same directory, then a rename. A file")
    w("# that already holds the key is only chmodded back to 0600.")
    w("pp_store_key() {")
    w('  local root="$1" key="$2"')
    w('  local config_root="${XDG_CONFIG_HOME:-${root}/.config}"')
    w(f'  local dest="${{config_root}}/{token_file}"')
    w('  case "${config_root}" in')
    w('    "${root}"|"${root}"/*) ;;')
    w("    *)")
    w('      warn "packages proxy: the config root is not under ${root}, so the key is not stored"')
    w("      return 1 ;;")
    w("  esac")
    w('  if [ -f "${dest}" ] && [ "$(cat "${dest}" 2>/dev/null)" = "${key}" ]; then')
    w('    chmod 600 "${dest}" 2>/dev/null || true')
    w("    return 0")
    w("  fi")
    w("  (")
    w("    umask 077")
    w('    mkdir -p "$(dirname "${dest}")"')
    w('    local tmp')
    w('    tmp="$(mktemp "${dest}.tmp.XXXXXX")"')
    w('    printf \'%s\' "${key}" >"${tmp}"')
    w('    mv -f "${tmp}" "${dest}"')
    w("  )")
    w('  chmod 600 "${dest}"')
    w("}")
    w("")
    w("# pp_write_netrc owns exactly the proxy machine entry of ROOT/.netrc.")
    w("# netrc has no comment markers, so there is no marked block to own:")
    w("# an existing proxy entry, single- or multi-line, is replaced where it")
    w("# stands (the entry runs to the next machine/default/token line) and")
    w("# every other line keeps as the user wrote it. The first change is")
    w("# backed up once to .netrc.bak-agent-kit, the file ends up 0600, and")
    w("# the key travels in shell memory only, never on argv.")
    w("pp_write_netrc() {")
    w('  local root="$1" key="$2"')
    w(f'  local netrc="${{root}}/.netrc" host="{host}"')
    w('  local entry="machine ${host} login packages password ${key}"')
    w('  local text="" line="" first="" second="" skip=0 found=0 tmp')
    w("  local nl=$'\\n'")
    w('  if [ "${CHECK_ONLY}" = 1 ]; then')
    w('    log "would set up the ${host} machine entry in ${netrc}"')
    w("    return 0")
    w("  fi")
    w('  if [ -f "${netrc}" ]; then')
    w('    while IFS= read -r line || [ -n "${line}" ]; do')
    w('      read -r first second rest <<< "${line}"')
    w('      if [ "${skip}" = 1 ]; then')
    w('        case "${first}" in')
    w("          login|password|account|force) continue ;;")
    w("        esac")
    w('        skip=0')
    w("      fi")
    w('      if [ "${first}" = machine ] && [ "${second}" = "${host}" ]; then')
    w('        text="${text}${entry}${nl}"')
    w('        skip=1')
    w('        found=1')
    w("        continue")
    w("      fi")
    w('      text="${text}${line}${nl}"')
    w('    done < "${netrc}"')
    w("  fi")
    w('  if [ "${found}" = 0 ]; then')
    w('    text="${text}${entry}${nl}"')
    w("  fi")
    w('  if [ -f "${netrc}" ] && [ "$(cat "${netrc}")" = "${text%"${nl}"}" ]; then')
    w('    chmod 600 "${netrc}" 2>/dev/null || true')
    w('    log "packages proxy: ${netrc} is already set up"')
    w("    return 0")
    w("  fi")
    w('  if [ -f "${netrc}" ] && [ ! -f "${netrc}.bak-agent-kit" ]; then')
    w('    cp "${netrc}" "${netrc}.bak-agent-kit"')
    w("  fi")
    w("  (")
    w("    umask 077")
    w('    tmp="$(mktemp "${netrc}.tmp.XXXXXX")"')
    w("    printf '%s\\n' \"${text%\"${nl}\"}\" > \"${tmp}\"")
    w('    mv -f "${tmp}" "${netrc}"')
    w("  )")
    w('  chmod 600 "${netrc}"')
    w('  log "packages proxy: ${netrc} is set up"')
    w("}")
    w("")
    w("# pp_write_go points go at the proxy -- but only when go is on PATH,")
    w("# and only into this root's env file: GOENV names it (ROOT/.config/go/env")
    w("# by default), and a GOENV outside the root is refused outright. No")
    w("# GOSUMDB and no ,direct: the proxy is the only source. CHECK_ONLY")
    w("# never calls go.")
    w("pp_write_go() {")
    w('  local root="$1"')
    w('  local goenv="${GOENV:-${root}/.config/go/env}"')
    w('  if [ "${CHECK_ONLY}" = 1 ]; then')
    w('    log "would point go at the packages proxy through ${goenv}"')
    w("    return 0")
    w("  fi")
    w('  if ! command -v go >/dev/null 2>&1; then')
    w('    log "packages proxy: go is not on PATH, its env stays as it is"')
    w("    return 0")
    w("  fi")
    w('  case "${goenv}" in')
    w('    "${root}"|"${root}"/*) ;;')
    w("    *)")
    w('      warn "packages proxy: GOENV is not under ${root}, go env left alone"')
    w("      return 0 ;;")
    w("  esac")
    w('  mkdir -p "$(dirname "${goenv}")"')
    w(f'  if ! GOENV="${{goenv}}" go env -w "GOPROXY={go_url}" "GOPRIVATE={go_private}"; then')
    w("    return 1")
    w("  fi")
    w('  ok "packages proxy: go env points at the proxy (${goenv})"')
    w("}")
    w("")
    w("# pp_write_npmrc sets the proxy registry and its token in ONE marked")
    w("# block of ROOT/.npmrc, through the merge helper: everything outside the")
    w("# block keeps as the user wrote it (the first-party scope and its GitHub")
    w("# token line included), a registry= line outside the block refuses the")
    w("# merge outright, and the file ends up 0600. The token rides stdin,")
    w("# never argv or a log line.")
    w("pp_write_npmrc() {")
    w('  local root="$1" key="$2"')
    w('  local npmrc="${root}/.npmrc"')
    w(f'  local helper="${{KIT_ROOT}}/{PACKAGE_PROXY_MERGE}"')
    w('  if [ "${CHECK_ONLY}" = 1 ]; then')
    w('    log "would set up the packages registry and token in ${npmrc}"')
    w("    return 0")
    w("  fi")
    w(f"  printf 'registry={npm_url}\\n{npm_auth}:_authToken=%s\\n' \"${{key}}\" \\")
    w("    | python3 \"${helper}\" \"${npmrc}\" packages-proxy --mode 600 --refuse '^registry=' >/dev/null")
    w("  case $? in")
    w('    0) ok "packages proxy: ${npmrc} is set up" ;;')
    w('    3) warn "packages proxy: ${npmrc} has its own registry= line, nothing written" ;;')
    w("    *) return 1 ;;")
    w("  esac")
    w("}")
    w("")
    w("# pp_write_uv points uv at the proxy through ONE marked block of the uv")
    w("# config: a default index served by the proxy, capped at a week of")
    w("# package age. A user exclude-newer outside the block refuses the")
    w("# merge, and everything else in the file keeps as the user wrote it.")
    w("# The config root is XDG_CONFIG_HOME when it sits under the root, else")
    w("# ROOT/.config -- a config root outside the root is never written.")
    w("pp_write_uv() {")
    w('  local root="$1"')
    w('  local config_root="${XDG_CONFIG_HOME:-${root}/.config}"')
    w('  local uv_toml="${config_root}/uv/uv.toml"')
    w(f'  local helper="${{KIT_ROOT}}/{PACKAGE_PROXY_MERGE}"')
    w('  if [ "${CHECK_ONLY}" = 1 ]; then')
    w('    log "would set up the packages pypi index in ${uv_toml}"')
    w("    return 0")
    w("  fi")
    w('  case "${config_root}" in')
    w('    "${root}"|"${root}"/*) ;;')
    w("    *)")
    w('      warn "packages proxy: the config root is not under ${root}, uv is left alone"')
    w("      return 0 ;;")
    w("  esac")
    w(f'  printf \'[[index]]\\nurl = "{pypi_url}"\\ndefault = true\\nexclude-newer = "7 days"\\n\' \\')
    w("    | python3 \"${helper}\" \"${uv_toml}\" packages-proxy --validate toml \\")
    w("      --refuse '^exclude-newer' >/dev/null")
    w("  case $? in")
    w('    0) ok "packages proxy: ${uv_toml} is set up" ;;')
    w('    3) warn "packages proxy: ${uv_toml} has its own exclude-newer, nothing written" ;;')
    w("    *) return 1 ;;")
    w("  esac")
    w("}")
    w("")
    w("# pp_write_pip points pip at the proxy through ONE marked block of")
    w("# pip.conf: index-url under [global]. A [global] section the user")
    w("# already wrote must not come through twice, so the helper is asked on")
    w("# a throwaway copy (its --refuse reads the copy without this block)")
    w("# whether one stands outside the block; when it does the merged block")
    w("# carries the bare line, which [global] above already owns. A user")
    w("# index-url outside the block refuses the merge outright.")
    w("pp_write_pip() {")
    w('  local root="$1"')
    w('  local config_root="${XDG_CONFIG_HOME:-${root}/.config}"')
    w('  local pip_conf="${config_root}/pip/pip.conf" body probe')
    w(f'  local helper="${{KIT_ROOT}}/{PACKAGE_PROXY_MERGE}"')
    w('  if [ "${CHECK_ONLY}" = 1 ]; then')
    w('    log "would set up the packages pypi index in ${pip_conf}"')
    w("    return 0")
    w("  fi")
    w('  case "${config_root}" in')
    w('    "${root}"|"${root}"/*) ;;')
    w("    *)")
    w('      warn "packages proxy: the config root is not under ${root}, pip is left alone"')
    w("      return 0 ;;")
    w("  esac")
    w(f"  body=$'[global]\\nindex-url = {pypi_url}'")
    w('  if [ -f "${pip_conf}" ]; then')
    w('    probe="$(mktemp "${pip_conf}.probe.XXXXXX")"')
    w('    cp "${pip_conf}" "${probe}"')
    w('    python3 "${helper}" "${probe}" packages-proxy --remove \\')
    w("      --refuse '^\\[global\\]' >/dev/null 2>&1 </dev/null")
    w("    case $? in")
    w(f"      3) body=$'index-url = {pypi_url}' ;;")
    w("    esac")
    w('    rm -f "${probe}" "${probe}.bak-agent-kit"')
    w("  fi")
    w('  printf \'%s\\n\' "${body}" \\')
    w("    | python3 \"${helper}\" \"${pip_conf}\" packages-proxy --validate ini \\")
    w("      --refuse '^index-url' >/dev/null")
    w("  case $? in")
    w('    0) ok "packages proxy: ${pip_conf} is set up" ;;')
    w('    3) warn "packages proxy: ${pip_conf} has its own index-url, nothing written" ;;')
    w("    *) return 1 ;;")
    w("  esac")
    w("}")
    w("")
    w("# pp_write_cargo points cargo at the proxy through TWO marked blocks of")
    w("# the cargo home: the registry replacement in config.toml (no key in")
    w("# it, so no --mode) and the login token in credentials.toml (written")
    w("# 0600, its key riding stdin). CARGO_HOME counts only when it is set")
    w("# AND under the root; anything else falls back to ROOT/.cargo, so a")
    w("# writer never writes outside its root. A table this step must own")
    w("# that already stands outside the block refuses that file's merge --")
    w("# and the other file still gets its block.")
    w("pp_write_cargo() {")
    w('  local root="$1" key="$2"')
    w('  local cargo_home="${root}/.cargo"')
    w('  case "${CARGO_HOME:-}" in')
    w('    "") ;;')
    w('    "${root}"|"${root}"/*) cargo_home="${CARGO_HOME}" ;;')
    w('    *) log "packages proxy: CARGO_HOME is not under ${root}, cargo is configured in ${cargo_home}" ;;')
    w("  esac")
    w('  local cargo_config="${cargo_home}/config.toml"')
    w('  local credentials="${cargo_home}/credentials.toml"')
    w(f'  local helper="${{KIT_ROOT}}/{PACKAGE_PROXY_MERGE}"')
    w("  local guard='^\\[(source\\.(crates-io|jorisjonkers-packages)|'\\")
    w("$'registries\\.jorisjonkers-packages|registry)\\]'")
    w('  if [ "${CHECK_ONLY}" = 1 ]; then')
    w('    log "would set up the cargo registry in ${cargo_config} and its token in ${credentials}"')
    w("    return 0")
    w("  fi")
    w("  {")
    w("    printf '[source.crates-io]\\nreplace-with = \"jorisjonkers-packages\"\\n\\n'")
    w(f"    printf '[source.jorisjonkers-packages]\\nregistry = \"{cargo_url}\"\\n\\n'")
    w(f"    printf '[registries.jorisjonkers-packages]\\nindex = \"{cargo_url}\"\\n\\n'")
    w("    printf '[registry]\\nglobal-credential-providers = [\"cargo:token\"]\\n'")
    w("    printf 'global-min-publish-age = \"7 days\"\\n'")
    w("  } | python3 \"${helper}\" \"${cargo_config}\" packages-proxy --validate toml \\")
    w('    --refuse "${guard}" >/dev/null')
    w("  case $? in")
    w('    0) ok "packages proxy: ${cargo_config} is set up" ;;')
    w('    3) warn "packages proxy: ${cargo_config} has its own cargo registry tables, nothing written" ;;')
    w("    *) return 1 ;;")
    w("  esac")
    w("  printf '[registries.jorisjonkers-packages]\\ntoken = \"Bearer %s\"\\n' \"${key}\" \\")
    w("    | python3 \"${helper}\" \"${credentials}\" packages-proxy --validate toml --mode 600 \\")
    w("      --refuse '^\\[registries\\.jorisjonkers-packages\\]' >/dev/null")
    w("  case $? in")
    w('    0) ok "packages proxy: ${credentials} is set up" ;;')
    w('    3) warn "packages proxy: ${credentials} has its own registry token, nothing written" ;;')
    w("    *) return 1 ;;")
    w("  esac")
    w("}")
    w("")
    w("# pp_write_gradle writes Gradle's packages-proxy init script -- copied")
    w("# from the estate's setup-java-gradle action, with only the token")
    w("# source changed -- plus the packagesProxyToken Gradle property that")
    w("# carries the key. The init script holds no key (its Kotlin reads the")
    w("# token from the property or the environment at build time), so it is")
    w("# written 0644 through a quoted heredoc (no shell expansion inside")
    w("# it); gradle.properties holds the key and is written 0600 through")
    w("# the merge helper, its key riding stdin -- a packagesProxyToken= line")
    w("# outside the block refusing the merge outright. GRADLE_USER_HOME")
    w("# counts only when it is set AND under the root; anything else falls")
    w("# back to ROOT/.gradle, so the writer never writes outside its root.")
    w("pp_write_gradle() {")
    w('  local root="$1" key="$2"')
    w('  local gradle_home="${root}/.gradle"')
    w('  case "${GRADLE_USER_HOME:-}" in')
    w('    "") ;;')
    w('    "${root}"|"${root}"/*) gradle_home="${GRADLE_USER_HOME}" ;;')
    w('    *) log "packages proxy: GRADLE_USER_HOME is not under ${root}, gradle is configured in ${gradle_home}" ;;')
    w("  esac")
    w('  local init_script="${gradle_home}/init.d/packages-proxy.init.gradle.kts"')
    w('  local gradle_props="${gradle_home}/gradle.properties"')
    w(f'  local helper="${{KIT_ROOT}}/{PACKAGE_PROXY_MERGE}"')
    w("  local tmp")
    w('  if [ "${CHECK_ONLY}" = 1 ]; then')
    w('    log "would write the gradle init script ${init_script}"')
    w('    log "would set up the gradle token in ${gradle_props}"')
    w("    return 0")
    w("  fi")
    w('  mkdir -p "$(dirname "${init_script}")"')
    w("  (")
    w("    umask 077")
    w('    tmp="$(mktemp "${init_script}.tmp.XXXXXX")"')
    w("    cat >\"${tmp}\" <<'KTS'")
    for line in init_kts.splitlines():
        w(line)
    w("KTS")
    w('    mv -f "${tmp}" "${init_script}"')
    w("  )")
    w('  chmod 644 "${init_script}"')
    w('  ok "packages proxy: ${init_script} is set up"')
    w("  printf 'packagesProxyToken=%s\\n' \"${key}\" \\")
    w("    | python3 \"${helper}\" \"${gradle_props}\" packages-proxy --mode 600 \\")
    w("      --refuse '^packagesProxyToken=' >/dev/null")
    w("  case $? in")
    w('    0) ok "packages proxy: ${gradle_props} is set up" ;;')
    w('    3) warn "packages proxy: ${gradle_props} has its own packagesProxyToken, nothing written" ;;')
    w("    *) return 1 ;;")
    w("  esac")
    w("}")
    w("")
    w("# pp_write_maven: Maven's settings.xml, create-only. A root with no")
    w("# .m2/settings.xml gets one -- written through a temp file under umask")
    w("# 077 and renamed, so the key is never on disk world-readable -- with")
    w("# the proxy mirror (everything external but github) and its server; a")
    w("# root with its own settings.xml is never edited: the snippet to add")
    w("# is logged with the password as the PACKAGES_PROXY_KEY placeholder,")
    w("# so the run's output never carries the key either.")
    w("pp_write_maven() {")
    w('  local root="$1" key="$2"')
    w('  local settings="${root}/.m2/settings.xml"')
    w("  local tmp")
    w('  if [ "${CHECK_ONLY}" = 1 ]; then')
    w('    log "would set up the packages proxy mirror and server in ${settings}"')
    w("    return 0")
    w("  fi")
    w('  if [ -f "${settings}" ]; then')
    w('    log "packages proxy: ${settings} already exists, add this to it (PACKAGES_PROXY_KEY stands for the key):"')
    w("    cat <<'XML'")
    w("  <mirrors>")
    w("    <mirror>")
    w("      <id>packages-proxy</id>")
    w(f"      <url>{maven['main']}</url>")
    w(f"      <mirrorOf>{MAVEN_MIRROR_OF}</mirrorOf>")
    w("    </mirror>")
    w("  </mirrors>")
    w("  <servers>")
    w("    <server>")
    w("      <id>packages-proxy</id>")
    w("      <username>packages</username>")
    w("      <password>PACKAGES_PROXY_KEY</password>")
    w("    </server>")
    w("  </servers>")
    w("XML")
    w("    return 0")
    w("  fi")
    w('  mkdir -p "$(dirname "${settings}")"')
    w("  (")
    w("    umask 077")
    w('    tmp="$(mktemp "${settings}.tmp.XXXXXX")"')
    w('    cat >"${tmp}" <<XML')
    w('<?xml version="1.0" encoding="UTF-8"?>')
    w("<settings>")
    w("  <mirrors>")
    w("    <mirror>")
    w("      <id>packages-proxy</id>")
    w(f"      <url>{maven['main']}</url>")
    w(f"      <mirrorOf>{MAVEN_MIRROR_OF}</mirrorOf>")
    w("    </mirror>")
    w("  </mirrors>")
    w("  <servers>")
    w("    <server>")
    w("      <id>packages-proxy</id>")
    w("      <username>packages</username>")
    w("      <password>${key}</password>")
    w("    </server>")
    w("  </servers>")
    w("</settings>")
    w("XML")
    w('    mv -f "${tmp}" "${settings}"')
    w("  )")
    w('  chmod 600 "${settings}"')
    w('  ok "packages proxy: ${settings} is set up"')
    w("}")
    w("")
    w("# package_proxy_setup: refuse without a home to write into, skip a cloud")
    w("# session, report only under --check; then resolve the key, verify it,")
    w("# store it and call the eight writers -- netrc, go env, npmrc, uv, pip,")
    w("# cargo, gradle and maven -- each guarded, so a writer that fails warns")
    w("# without stopping the run; a missing or refused key warns and skips --")
    w("# no other step depends on it. The verify call carries the key in a")
    w('# stdin ("-K -"), never on argv or in a log line.')
    w("package_proxy_setup() {")
    w('  local root="${1:-}" key')
    w('  if [ -z "${root}" ] || [ "${root}" = "/" ]; then')
    w('    warn "packages proxy: no home directory to configure for the packages proxy, skipped"')
    w("    return 1")
    w("  fi")
    w('  if [ "${CLOUD}" = 1 ]; then')
    w('    log "packages proxy: the cloud session brings its own"')
    w("    return 0")
    w("  fi")
    w('  if [ "${CHECK_ONLY}" = 1 ]; then')
    w(f'    log "would look up the packages proxy key and verify it against {verify_url}"')
    w(f'    log "would store it under {token_file} and write .netrc, go env, .npmrc, uv, pip, cargo, gradle and maven"')
    w("    return 0")
    w("  fi")
    w('  if ! key="$(pp_resolve_key "${root}")"; then')
    w('    warn "packages proxy: no key found (PACKAGES_PROXY_TOKEN, vault or the prompt), skipped"')
    w("    return 0")
    w("  fi")
    w('  if ! printf \'header = "Authorization: Bearer %s"\\n\' "${key}" \\')
    w(f'    | curl -sf -o /dev/null -K - {verify_url}; then')
    w(f'    warn "packages proxy: the key was refused by {verify_url}, nothing was written"')
    w("    return 0")
    w("  fi")
    w('  pp_store_key "${root}" "${key}" || return 1')
    w('  pp_write_netrc "${root}" "${key}" \\')
    w('    || warn "packages proxy: the netrc writer failed, the run continues"')
    w('  pp_write_go "${root}" \\')
    w('    || warn "packages proxy: the go env writer failed, the run continues"')
    w('  pp_write_npmrc "${root}" "${key}" \\')
    w('    || warn "packages proxy: the npmrc writer failed, the run continues"')
    w('  pp_write_uv "${root}" \\')
    w('    || warn "packages proxy: the uv writer failed, the run continues"')
    w('  pp_write_pip "${root}" \\')
    w('    || warn "packages proxy: the pip writer failed, the run continues"')
    w('  pp_write_cargo "${root}" "${key}" \\')
    w('    || warn "packages proxy: the cargo writer failed, the run continues"')
    w('  pp_write_gradle "${root}" "${key}" \\')
    w('    || warn "packages proxy: the gradle writer failed, the run continues"')
    w('  pp_write_maven "${root}" "${key}" \\')
    w('    || warn "packages proxy: the maven writer failed, the run continues"')
    w('  ok "packages proxy: the key is verified and stored"')
    w("}")
    w("")


def render_setup_script(data: dict[str, Any]) -> str:
    out: list[str] = []
    w = out.append
    gateway_declared = any(
        skill.get("name") == "model-gateway" and on_surface(skill, "workstation")
        for skill in _entries(data, "local_skills")
    )
    layout_declared = any(
        skill.get("name") == "projects-layout" and on_surface(skill, "workstation")
        for skill in _entries(data, "local_skills")
    )

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
    w("#   ./setup-workstation.sh --no-prompt     never ask; report what needs a hand")
    w("#   ./setup-workstation.sh --cloud         a Claude Code cloud environment's")
    w("#                                          setup script (docs/CLOUD.md)")
    w("#   ./setup-workstation.sh --uninstall     remove what a PREVIOUS install.sh /")
    w("#                                          install-agents.sh install wrote,")
    w("#                                          then purge retired knowledge hooks,")
    w("#                                          remove the critique hooks and exit")
    w("#")
    w("# Run outside an agent-kit checkout (`curl | bash`), it fetches the kit")
    w("# bundle attached to its own GitHub release and re-runs the copy inside it.")
    w("# AGENT_KIT_SKILLS_BUNDLE_URL overrides where that bundle comes from.")
    w("#")
    w("# Secrets are read from the environment and never written here:")
    for server in _entries(data, "mcp_servers"):
        if on_surface(server, "workstation") and server.get("credential"):
            w(f"#   {server['credential']}  -> the {server['name']} MCP server")
    for plugin in _entries(data, "plugins"):
        for var in (plugin.get("requires_env") or {}):
            w(f"#   {var}  -> the {plugin['name']} plugin")
    help_lines = len(out)
    w("")
    w("set -uo pipefail")
    w("")
    w("CHECK_ONLY=0")
    w("DO_LSP=1")
    w("DO_MCP=1")
    w("DO_PROFILES=1")
    w("CLOUD=0")
    w("UNINSTALL=0")
    w("PROMPT=1")
    w("PROMPT_TTY=/dev/tty")
    w("failures=0")
    w("warnings=0")
    w("todo_failures=()           # Every failure, listed again at the end")
    w("todo_warnings=()           # Every warning, listed again at the end")
    w("")
    w("# Track what needs attention after the run")
    w("skipped_mcp_servers=()     # MCP servers skipped due to missing credentials")
    w("skipped_mcp_names=()       # Their names, so the registration check skips them")
    w("unpinned_markets=()        # Marketplaces not at their pin: their plugins are not installed")
    w("unshared_profile_paths=()  # Shared surfaces a secondary profile did not get")
    w("missing_lsp_binaries=()    # Language servers with missing binaries")
    w("disabled_plugins=()        # Plugins disabled (missing binary or on purpose)")
    w("missing_plugin_env=()      # Plugins whose required env vars are unset")
    w("plugin_drift=()            # Plugins whose commit drifted")
    w("new_binaries=()            # Binaries installed during this run")
    w("")
    w('ORIG_ARGS=("$@")')
    w('while [ "$#" -gt 0 ]; do')
    w("  case \"$1\" in")
    w("    --check) CHECK_ONLY=1 ;;")
    w("    --no-lsp) DO_LSP=0 ;;")
    w("    --no-mcp) DO_MCP=0 ;;")
    w("    --no-profiles) DO_PROFILES=0 ;;")
    w("    --no-prompt) PROMPT=0 ;;")
    w("    --cloud) CLOUD=1; DO_PROFILES=0 ;;")
    w("    --uninstall) UNINSTALL=1 ;;")
    w(f"    --help|-h) sed -n '2,{help_lines}p' \"$0\"; exit 0 ;;")
    w('    *) echo "unknown option: $1" >&2; exit 64 ;;')
    w("  esac")
    w("  shift")
    w("done")
    w("")
    w("log()  { printf 'setup: %s\\n' \"$*\"; }")
    w("ok()   { printf 'setup:   ok    %s\\n' \"$*\"; }")
    w("warn() { printf 'setup:   warn  %s\\n' \"$*\" >&2; warnings=$((warnings + 1)); todo_warnings+=(\"$*\"); }")
    w("fail() { printf 'setup:   FAIL  %s\\n' \"$*\" >&2; failures=$((failures + 1)); todo_failures+=(\"$*\"); }")
    w("")
    w("# Everything the run left for a person to do: failures first, each once.")
    w("print_todo() {")
    w("  local all=() unique=() entry seen i=0")
    w('  for entry in ${todo_failures[@]+"${todo_failures[@]}"}; do all+=("FAIL ${entry}"); done')
    w('  for entry in ${todo_warnings[@]+"${todo_warnings[@]}"}; do all+=("${entry}"); done')
    w('  for entry in ${all[@]+"${all[@]}"}; do')
    w('    for seen in ${unique[@]+"${unique[@]}"}; do')
    w('      [ "${seen}" = "${entry}" ] && continue 2')
    w("    done")
    w('    unique+=("${entry}")')
    w("  done")
    w('  if [ "${#unique[@]}" = 0 ]; then')
    w('    log "Nothing left to do."')
    w("    return 0")
    w("  fi")
    w('  log "Still to do (${#unique[@]}):"')
    w('  for entry in "${unique[@]}"; do')
    w("    i=$((i + 1))")
    w('    log "  ${i}. ${entry}"')
    w("  done")
    w("}")
    w("")
    w("# Asks only with someone at a terminal: never under --check, --cloud, CI or --no-prompt.")
    w("interactive() {")
    w('  [ "${PROMPT}" = 1 ] && [ "${CHECK_ONLY}" != 1 ] && [ "${CLOUD}" != 1 ] && [ -t 1 ] \\')
    w("    && { : </dev/tty; } 2>/dev/null")
    w("}")
    w("")
    w("# Tokens and keys first: everything after this reads them. In a terminal")
    w("# agent-kit-keys signs you in, mints what is missing and stores it; either way")
    w("# env.sh is rewritten and loaded, so this run registers what needs them.")
    w("tokens_and_keys() {")
    w('  local table="${KIT_ROOT}/registry/generated/workstation-secrets.json"')
    w('  local helper="${KIT_ROOT}/scripts/agent-kit-keys.py"')
    w("  local var state")
    w('  if [ "${CLOUD}" = 1 ]; then')
    w('    log "tokens and keys: the cloud session brings its own"')
    w("    return 0")
    w("  fi")
    w('  log "tokens and keys"')
    w("  if interactive; then")
    w('    python3 "${helper}" --table "${table}" ensure <"${PROMPT_TTY}" \\')
    w('      || warn "tokens and keys: the walk-through stopped early; run setup again to continue"')
    w('  elif [ "${CHECK_ONLY}" != 1 ]; then')
    w('    python3 "${helper}" --table "${table}" write-env \\')
    w('      || warn "tokens and keys: could not write ~/.config/agent-kit/env.sh"')
    w("  fi")
    w("  while read -r var state; do")
    w('    case "${var} ${state}" in')
    w('      "shell missing")')
    w('        warn "your shell does not load ~/.config/agent-kit/env.sh; run setup in a terminal to add it" ;;')
    for secret in _entries(data, "workstation_secrets"):
        if secret.get("login") and not secret.get("retired"):
            # Setup cannot sign in for you: say which command does.
            w(f'      "{secret["env"]} missing")')
            w(f'        warn "{secret["env"]} is missing; run  {secret["login"]}  in a terminal" ;;')
            w(f'      "{secret["env"]} invalid")')
            w(f'        warn "{secret["env"]} is refused by its host; run  {secret["login"]}  in a terminal again" ;;')
    w('      *" missing") warn "${var} is missing; run setup in a terminal and it gets it for you" ;;')
    w('      *" invalid") warn "${var} is refused by its host; run setup in a terminal to replace it" ;;')
    w('      *" unchecked") log "${var}: its host did not answer, so it was not checked" ;;')
    w('      *" ok") ok "${var}" ;;')
    w("    esac")
    w('  done < <(python3 "${helper}" --table "${table}" status)')
    w('  if [ -f "${HOME}/.config/agent-kit/env.sh" ]; then')
    w("    # shellcheck disable=SC1091")
    w('    . "${HOME}/.config/agent-kit/env.sh"')
    w("    # Its vibe and memory-private wrappers are for your shell, not for this run.")
    w("    unset -f vibe memory-private 2>/dev/null || true")
    w("  fi")
    w("}")
    w("")
    _package_proxy_step(w, data)
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
    w("# Sets the given top-level keys in a JSON config under $HOME, keeping")
    w("# every other key. Used for plugins that read a user config file.")
    w("_merge_user_config() {")
    w('  if [ "${CHECK_ONLY}" = 1 ]; then')
    w('    log "would set $2 in $1"')
    w("    return 0")
    w("  fi")
    w("  python3 - \"$1\" \"$2\" <<'PY'")
    w("import json, pathlib, sys")
    w("path, want = pathlib.Path(sys.argv[1]), json.loads(sys.argv[2])")
    w("data = json.loads(path.read_text() or '{}') if path.exists() else {}")
    w("data.update(want)")
    w("path.parent.mkdir(parents=True, exist_ok=True)")
    w('path.write_text(json.dumps(data, indent=2) + "\\n")')
    w("back = json.loads(path.read_text())")
    w("if any(back.get(k) != v for k, v in want.items()):")
    w('    raise SystemExit(f"{path}: {want} did not read back")')
    w("PY")
    w("}")
    w("")
    rules = json.dumps(data.get("claude_private_deny") or [])
    w("# Joins the private-bank deny rules to each Claude profile's own; nothing else changes.")
    w("claude_private_denies() {")
    w('  [ "${CLOUD}" = 1 ] && return 0')
    w('  local profile_dir')
    w('  for profile_dir in "${CLAUDE_PROFILE_DIRS[@]}"; do')
    w('    if [ "${CHECK_ONLY}" = 1 ]; then')
    w('      log "would add the private-bank deny rules to ${profile_dir}/settings.json"')
    w("      continue")
    w("    fi")
    w(f"    python3 - \"${{profile_dir}}/settings.json\" '{rules}' <<'PY' \\")
    w('      && ok "private-bank deny rules in ${profile_dir}/settings.json" \\')
    w('      || fail "could not add the private-bank deny rules to ${profile_dir}/settings.json"')
    w("import json, pathlib, sys")
    w("path, rules = pathlib.Path(sys.argv[1]), json.loads(sys.argv[2])")
    w("data = json.loads(path.read_text() or '{}') if path.exists() else {}")
    w("deny = data.setdefault('permissions', {}).setdefault('deny', [])")
    w("missing = [r for r in rules if r not in deny]")
    w("if missing:")
    w("    deny.extend(missing)")
    w("    path.parent.mkdir(parents=True, exist_ok=True)")
    w("    path.write_text(json.dumps(data, indent=2) + '\\n')")
    w("back = json.loads(path.read_text())['permissions']['deny']")
    w("if any(r not in back for r in rules):")
    w("    raise SystemExit(f'{path}: deny rules did not read back')")
    w("PY")
    w("  done")
    w("}")
    w("")
    allows = json.dumps(claude_allow_rules(data))
    w("# Joins the allow rules for MCP servers whose every tool runs without a")
    w("# prompt (registry: claude_allow_tools) to each Claude profile's own; nothing else changes.")
    w("claude_mcp_allows() {")
    w('  local profile_dir')
    w('  for profile_dir in "${CLAUDE_PROFILE_DIRS[@]}"; do')
    w('    if [ "${CHECK_ONLY}" = 1 ]; then')
    w('      log "would add the MCP allow rules to ${profile_dir}/settings.json"')
    w("      continue")
    w("    fi")
    w(f"    python3 - \"${{profile_dir}}/settings.json\" '{allows}' <<'PY' \\")
    w('      && ok "MCP allow rules in ${profile_dir}/settings.json" \\')
    w('      || fail "could not add the MCP allow rules to ${profile_dir}/settings.json"')
    w("import json, pathlib, sys")
    w("path, rules = pathlib.Path(sys.argv[1]), json.loads(sys.argv[2])")
    w("data = json.loads(path.read_text() or '{}') if path.exists() else {}")
    w("allow = data.setdefault('permissions', {}).setdefault('allow', [])")
    w("missing = [r for r in rules if r not in allow]")
    w("if missing:")
    w("    allow.extend(missing)")
    w("    path.parent.mkdir(parents=True, exist_ok=True)")
    w("    path.write_text(json.dumps(data, indent=2) + '\\n')")
    w("back = json.loads(path.read_text()).get('permissions', {}).get('allow', [])")
    w("if any(r not in back for r in rules):")
    w("    raise SystemExit(f'{path}: allow rules did not read back')")
    w("PY")
    w("  done")
    w("}")
    w("")
    ca = data.get("coding_agents")
    if ca:
        url = ca["config"]["apiUrl"]
        want = json.dumps(ca["config"], sort_keys=True).replace("'", "'\\''")
        w("# Hindsight Coding Agents, pinned and locked to recall only (registry: coding_agents).")
        w("hindsight_coding_agents() {")
        w("  if ! command -v npx >/dev/null 2>&1; then")
        w('    warn "npx is not on PATH; Hindsight Coding Agents not installed"')
        w("    return 0")
        w("  fi")
        w('  if [ "${CHECK_ONLY}" = 1 ]; then')
        w(f'    log "would install {ca["package"]}@{ca["version"]} for {" and ".join(ca["harnesses"])}"')
        w("    return 0")
        w("  fi")
        w(f'  if npx -y {ca["package"]}@{ca["version"]} install {" ".join(ca["harnesses"])} '
          f'--server self-hosted --api-url {url} </dev/null >/dev/null 2>&1; then')
        w(f'    ok "Hindsight Coding Agents {ca["version"]} wired into {" and ".join(ca["harnesses"])}"')
        w("  else")
        w('    warn "Hindsight Coding Agents install failed"')
        w("    return 0")
        w("  fi")
        w('  local config="${HOME}/.hindsight/coding-agent.json"')
        w(f"  _merge_user_config \"${{config}}\" '{want}' \\")
        w('    && ok "Hindsight Coding Agents locked to recall only (${config})" \\')
        w('    || warn "could not write ${config}"')
        w("  # The token comes from HINDSIGHT_API_TOKEN; a copied one never stays on disk.")
        w('  python3 - "${config}" <<\'PY\' || warn "could not check ${config} for a token"')
        w("import json, pathlib, sys")
        w("path = pathlib.Path(sys.argv[1])")
        w("data = json.loads(path.read_text() or '{}') if path.exists() else {}")
        w("if data.pop('apiToken', None) is not None:")
        w("    path.write_text(json.dumps(data, indent=2) + '\\n')")
        w("PY")
        w("  # Its installer registers the memory tools in the default profile only.")
        w("  run claude_each_profile claude mcp remove --scope user hindsight >/dev/null 2>&1 || true")
        w("  run claude_each_profile claude mcp add --scope user hindsight --env HINDSIGHT_MCP_HARNESS=claude-code \\")
        w('    -- node "${HOME}/.hindsight/coding-agents/dist/mcp-server.js" >/dev/null 2>&1 \\')
        w('    || warn "could not register the Hindsight memory tools in every Claude profile"')
        w("}")
        w("")
    w("# Whether a marketplace is declared from the given folder (not tracking GitHub).")
    w("_marketplace_from_folder() {")
    w('  claude plugin marketplace list --json 2>/dev/null | python3 -c \'')
    w("import json, os, sys")
    w("name, path = sys.argv[1], os.path.realpath(sys.argv[2])")
    w('ok = any(m.get("name") == name and m.get("source") == "directory"')
    w('         and os.path.realpath(m.get("path", "")) == path for m in json.load(sys.stdin))')
    w("sys.exit(0 if ok else 1)")
    w('\' "$1" "$2"')
    w("  # end _marketplace_from_folder")
    w("}")
    w("")
    w("# Rebuilds ROOT/ALIAS as the checkout ROOT/NAME under another name: its own")
    w("# marketplace.json with the name replaced, and symlinks to everything else.")
    w("_marketplace_alias() {")
    w('  local root="$1" name="$2" alias="$3" entry base')
    w('  [ -n "${alias}" ] && [ "${alias}" != "${name}" ] || return 1')
    w('  rm -rf "${root:?}/${alias}"')
    w('  mkdir -p "${root}/${alias}/.claude-plugin" || return 1')
    w('  for entry in "${root}/${name}"/* "${root}/${name}"/.[!.]* "${root}/${name}"/.claude-plugin/*; do')
    w('    [ -e "${entry}" ] || continue')
    w('    base="${entry#"${root}/${name}"/}"')
    w('    case "${base}" in .git|.claude-plugin|.claude-plugin/marketplace.json) continue ;; esac')
    w('    ln -s "${root}/${name}/${base}" "${root}/${alias}/${base}" || return 1')
    w("  done")
    w('  python3 - "${root}/${name}/.claude-plugin/marketplace.json" \\')
    w('    "${root}/${alias}/.claude-plugin/marketplace.json" "${alias}" <<\'PY\'')
    w("import json, sys")
    w("source, dest, alias = sys.argv[1:]")
    w("manifest = json.load(open(source))")
    w("manifest['name'] = alias")
    w("json.dump(manifest, open(dest, 'w'), indent=2)")
    w("PY")
    w("  # end _marketplace_alias")
    w("}")
    w("")
    w("# Checks a marketplace out at its pinned commit and declares it from that folder.")
    w("# Claude Code tracks a GitHub marketplace's default branch; a folder holds still.")
    w("# A reserved name is declared as ALIAS, from a renamed copy of the checkout.")
    w("ensure_marketplace() {")
    w('  local name="$1" repo="$2" commit="$3" alias="${4:-}"')
    w('  local root="${AGENT_KIT_MARKETPLACES:-${HOME}/.local/share/agent-kit/marketplaces}"')
    w('  local dir="${root}/${name}" declared="${4:-$1}" declared_dir="${root}/${4:-$1}"')
    w('  local base="${AGENT_KIT_GIT_BASE:-https://github.com}" moved=0')
    w('  if [ "${CHECK_ONLY}" = 1 ]; then')
    w('    log "would pin marketplace ${name} at ${commit:0:12}"')
    w("    return 0")
    w("  fi")
    w('  if [ "$(git -C "${dir}" rev-parse HEAD 2>/dev/null)" != "${commit}" ]; then')
    w('    mkdir -p "${dir}"')
    w('    [ -d "${dir}/.git" ] || git -C "${dir}" init -q')
    w('    git -C "${dir}" remote remove origin 2>/dev/null || true')
    w('    git -C "${dir}" remote add origin "${base}/${repo}"')
    w('    if ! git -C "${dir}" fetch -q --depth 1 origin "${commit}" 2>/dev/null \\')
    w('       || ! git -C "${dir}" -c advice.detachedHead=false checkout -q --force FETCH_HEAD; then')
    w("      # Fail closed: only a folder at an earlier pin may keep serving plugins.")
    w('      if _marketplace_from_folder "${declared}" "${declared_dir}"; then')
    w('        warn "marketplace ${name}: could not fetch ${repo} at ${commit:0:12}; still at its previous pin"')
    w("      else")
    w('        unpinned_markets+=("${name}")')
    w('        warn "marketplace ${name}: could not fetch ${repo} at ${commit:0:12}; its plugins are not installed"')
    w("      fi")
    w("      return 0")
    w("    fi")
    w("    moved=1")
    w("  fi")
    w('  if [ "$(git -C "${dir}" rev-parse HEAD)" != "${commit}" ]; then')
    w('    fail "marketplace ${name}: the checkout is not at ${commit:0:12}"')
    w('    unpinned_markets+=("${name}")')
    w("    return 0")
    w("  fi")
    w('  if [ -n "${alias}" ] && ! _marketplace_alias "${root}" "${name}" "${alias}"; then')
    w('    unpinned_markets+=("${name}")')
    w('    warn "marketplace ${name}: could not build ${declared_dir}; its plugins are not installed"')
    w("    return 0")
    w("  fi")
    w('  if _marketplace_from_folder "${declared}" "${declared_dir}"; then')
    w('    if [ "${moved}" = 1 ]; then')
    w('      run claude plugin marketplace update "${declared}" >/dev/null 2>&1 \\')
    w('        || warn "marketplace ${name}: could not refresh after moving to ${commit:0:12}"')
    w("    fi")
    w('    ok "marketplace ${name} pinned at ${commit:0:12}"')
    w("  else")
    w("    # Removing the upstream-tracking declaration uninstalls its plugins; the")
    w("    # plugins step reinstalls them from the pinned folder. A reserved name is")
    w("    # left alone: it cannot be declared from a folder, so its plugins would be lost.")
    w('    [ -n "${alias}" ] || run claude plugin marketplace remove "${name}" >/dev/null 2>&1 || true')
    w('    if run claude plugin marketplace add "${declared_dir}" >/dev/null 2>&1; then')
    w('      ok "marketplace ${name} pinned at ${commit:0:12} (${alias:+declared as ${alias}, }was tracking upstream)"')
    w("    else")
    w('      unpinned_markets+=("${name}")')
    w('      warn "marketplace ${name}: could not declare it from ${declared_dir}; its plugins are not installed"')
    w("    fi")
    w("  fi")
    w("  # end ensure_marketplace")
    w("}")
    w("")
    w("# The commit a plugin installs at under a pinned marketplace: the marketplace")
    w("# commit for a plugin in its repo, or the sha its entry pins for an external one.")
    w("expected_plugin_commit() {")
    w('  python3 - "$1" "$2" "$3" <<\'PY\'')
    w("import json, pathlib, sys")
    w("folder, name, commit = sys.argv[1:]")
    w("try:")
    w("    manifest = json.loads((pathlib.Path(folder) / '.claude-plugin' / 'marketplace.json').read_text())")
    w("except (OSError, ValueError):")
    w("    raise SystemExit(0)")
    w("for plugin in manifest.get('plugins', []):")
    w("    if plugin.get('name') == name:")
    w("        source = plugin.get('source')")
    w("        print(source.get('sha', '') if isinstance(source, dict) else commit)")
    w("        break")
    w("PY")
    w("  # end expected_plugin_commit")
    w("}")
    w("")
    w("# Removes a retired kit command, but only one setup wrote.")
    w("retire_kit_command() {")
    w('  local dest="${HOME}/.local/bin/$1"')
    w('  grep -qxF "# managed by agent-kit setup-workstation.sh" "${dest}" 2>/dev/null || return 0')
    w('  if [ "${CHECK_ONLY}" = 1 ]; then')
    w('    log "would remove retired ${dest}"')
    w("    return 0")
    w("  fi")
    w('  rm -f "${dest}" && ok "$1: retired, removed ${dest}"')
    w("}")
    w("")
    w("# Copies a kit command onto PATH. A file there that setup did not write is left alone.")
    w("install_kit_command() {")
    w('  local name="$1"')
    w('  local src="${KIT_ROOT}/installer/${name}" dest="${HOME}/.local/bin/${name}"')
    w('  local marker="# managed by agent-kit setup-workstation.sh"')
    w('  if [ -e "${dest}" ] && ! grep -qxF "${marker}" "${dest}"; then')
    w('    warn "${dest} exists and was not written by setup; left alone"')
    w("    return 0")
    w("  fi")
    w('  if [ "${CHECK_ONLY}" = 1 ]; then')
    w('    log "would install ${dest}"')
    w("    return 0")
    w("  fi")
    w('  mkdir -p "$(dirname "${dest}")"')
    w('  install -m 0755 "${src}" "${dest}" && cmp -s "${src}" "${dest}" \\')
    w('    && ok "${name} -> ${dest}" \\')
    w('    || fail "${name}: could not install ${dest}"')
    w("}")
    w("")
    w("# Points an agent home at the estate conventions, unless the user wrote their own.")
    w("# $1 is the AGENTS.md to write; $2 is shared (Shared Memory only) or private.")
    w("_agents_pointer() {")
    w('  local file="$1" kind="$2"')
    w('  local marker="<!-- managed by agent-kit setup-workstation.sh -->"')
    w('  if [ -f "${file}" ] && ! grep -qF "${marker}" "${file}"; then')
    w('    log "${file} is the user\'s own; left alone"')
    w("    return 0")
    w("  fi")
    w('  if [ "${CHECK_ONLY}" = 1 ]; then')
    w('    log "would write ${file}"')
    w("    return 0")
    w("  fi")
    w('  mkdir -p "$(dirname "${file}")"')
    w('  if [ "${kind}" = private ]; then')
    w('    cat > "${file}" <<EOF')
    _agents_pointer_text(w, private=True)
    w("EOF")
    w("  else")
    w('    cat > "${file}" <<EOF')
    _agents_pointer_text(w, private=False)
    w("EOF")
    w("  fi")
    w('  ok "AGENTS.md pointer -> ${file}"')
    w("}")
    w("")
    w("# Points a Vibe home at the estate conventions.")
    w("# $2 is shared (vibe, vibe-study: Shared Memory only) or private (vibe-private).")
    w("_vibe_agents_pointer() {")
    w('  _agents_pointer "${1}/AGENTS.md" "$2"')
    w("}")
    w("")
    _gateway_pointer_fn(w)
    _layout_pointer_fn(w)
    w("# Every chat model the Mistral login can use, kept current in one Vibe home.")
    w("_vibe_models() {")
    w('  local home="$1"')
    # keyring reads Vibe's key from the Secret Service on Linux.
    sync = 'uv run --no-project --with tomlkit --with keyring python "${KIT_ROOT}/scripts/vibe-sync-models.py"'
    w(f'  if VIBE_HOME="${{home}}" {sync} \\')
    w('      --config "${home}/config.toml" --has-key; then')
    w(f'    VIBE_HOME="${{home}}" {sync} \\')
    w('      --config "${home}/config.toml" \\')
    w('      || warn "Vibe model sync failed (${home})"')
    w("  else")
    w('    warn "vibe: not signed in; run vibe and sign in with your Mistral account, then re-run setup"')
    w("  fi")
    w("}")
    w("")
    w("# The kit's hooks into one Vibe home; extra arguments are --remove NAME pairs.")
    w("_vibe_hooks() {")
    w('  local home="$1"; shift')
    w('  uv run --no-project --with tomlkit python "${KIT_ROOT}/scripts/vibe-merge-mcp.py" \\')
    w(f'    "${{KIT_ROOT}}/{VIBE_HOOKS}" --key hooks --config "${{home}}/hooks.toml" "$@" \\')
    w('    && ok "Vibe hooks merged (${home})" \\')
    w('    || fail "Vibe hooks merge failed (${home})"')
    w("}")
    w("")
    w("# Once: the single Vibe home of before held sessions that recalled Private")
    w("# Memory, so it becomes vibe-private's and the shared home starts fresh.")
    w("# Until it has run, the shared home's Private Memory entry is what marks it as")
    w("# one to split: a failed split keeps setup out of both Vibe homes this run.")
    w("_vibe_split_homes() {")
    w("  local args=() out rc=0 line")
    w('  [ "${CHECK_ONLY}" = 1 ] && args+=(--check)')
    split = '"${KIT_ROOT}/scripts/vibe-split-homes.py" --shared "${VIBE_HOME}" --private "${VIBE_PRIVATE_HOME}"'
    w(f"  out=$(python3 {split} \\")
    w('    ${args[@]+"${args[@]}"} 2>&1) || rc=$?')
    w('  while IFS= read -r line; do [ -n "${line}" ] && log "${line}"; done <<< "${out}"')
    w('  if [ "${rc}" != 0 ]; then')
    w("    VIBE_HOMES_READY=0")
    w('    fail "could not split the Vibe home; both Vibe homes are left alone until it can"')
    w("  fi")
    w("}")
    w("")
    w("# vibe-private's own Vibe home: the shared fleet plus Private Memory. Its")
    w("# sessions never share a directory with the password-free launchers.")
    w("_vibe_private_home() {")
    w('  if [ "${CHECK_ONLY}" = 1 ]; then')
    w('    log "would merge vibe-private MCP servers into ${VIBE_PRIVATE_HOME}/config.toml"')
    w("  else")
    w('    uv run --no-project --with tomlkit python "${KIT_ROOT}/scripts/vibe-merge-mcp.py" \\')
    w(f'      "${{KIT_ROOT}}/{VIBE_PRIVATE_MCP}" --config "${{VIBE_PRIVATE_HOME}}/config.toml" \\')
    w('      && ok "vibe-private MCP servers merged" \\')
    w('      || fail "vibe-private MCP merge failed"')
    w('    _vibe_models "${VIBE_PRIVATE_HOME}"')
    w(f'    _vibe_hooks "${{VIBE_PRIVATE_HOME}}" {_retired_vibe_hooks()}')
    w("  fi")
    w('  _vibe_agents_pointer "${VIBE_PRIVATE_HOME}" private')
    w('  gateway_pointer "${VIBE_PRIVATE_HOME}/AGENTS.md"')
    w('  attribution_rule "${VIBE_PRIVATE_HOME}/AGENTS.md"')
    w('  vibe_no_commit_signature "${VIBE_PRIVATE_HOME}"')
    w("}")
    w("")
    _critic_agents_step(data, w)
    _delegate_agents_step(data, w)
    w("# A shell -c wrapper for the pipe-into-sh installers the upstreams publish.")
    w("run_sh() {")
    w('  if [ "${CHECK_ONLY}" = 1 ]; then')
    w('    log "would run: $1"')
    w("    return 0")
    w("  fi")
    w('  bash -c "$1"')
    w("}")
    w("")
    w("# Cloud mode registers a ${VAR} that Claude Code expands per session, keeping tokens out of the snapshot.")
    w("secret_ref() {")
    w('  if [ "${CLOUD}" = 1 ]; then')
    w("    # shellcheck disable=SC2016")
    w("    printf '${%s%s}' \"$1\" \"${2:+:-}\"")
    w("  else")
    w("    printf '%s' \"${!1:-}\"")
    w("  fi")
    w("}")
    w("")
    w("# Runs argv once per Claude profile, each with its own config root.")
    w("# MCP servers live in a profile's own .claude.json, so the fleet has to")
    w("# be registered per profile; plugins and skills do not, because those")
    w("# directories are shared by symlink.")
    w("claude_each_profile() {")
    w("  local dir rc=0")
    w('  for dir in "${CLAUDE_PROFILE_DIRS[@]}"; do')
    w('    claude_in_profile "${dir}" "$@" || rc=1')
    w("  done")
    w('  return "${rc}"')
    w("}")
    w("")
    w("# An `auth: oauth` server is registered by URL only: each client signs in")
    w("# itself and keeps its own token, so setup stores none. This reports every")
    w("# client that has not signed in yet, under --check too.")
    w("oauth_status() {")
    w('  local name="$1" dir')
    w("  if command -v claude >/dev/null 2>&1; then")
    w('    for dir in "${CLAUDE_PROFILE_DIRS[@]}"; do')
    w('      claude_in_profile "${dir}" claude mcp get "${name}" 2>/dev/null | grep -q Connected \\')
    w('        || warn "${name}: needs sign-in (claude ${dir}): run /mcp in Claude and pick ${name}"')
    w("    done")
    w("  fi")
    w("  if command -v codex >/dev/null 2>&1; then")
    w("    codex mcp list --json 2>/dev/null | uv run --no-project python -c \\")
    signed_in = (
        'import json,sys; sys.exit(0 if any(s.get("name") == sys.argv[1] and s.get("auth_status")'
        ' not in (None, "not_logged_in") for s in json.load(sys.stdin)) else 1)'
    )
    w(f"      '{signed_in}' \\")
    w('      "${name}" || warn "${name}: needs sign-in (codex): run codex mcp login ${name}"')
    w("  fi")
    w("  # Vibe keeps an MCP OAuth token in the keychain as mcp-oauth:<name>:tokens.")
    w("  if command -v vibe >/dev/null 2>&1 && command -v security >/dev/null 2>&1; then")
    w('    security find-generic-password -s ai.mistral.vibe -a "mcp-oauth:${name}:tokens" >/dev/null 2>&1 \\')
    w('      || warn "${name}: needs sign-in (vibe): run /mcp login ${name} in Vibe"')
    w("  fi")
    w("}")
    w("")
    w("# Antigravity signs in through a browser once, so an unsigned machine only")
    w("# fails later, in a non-interactive run. `agy models` lists the account's")
    w("# models without calling one, so it spends no quota, and asks for a sign-in")
    w("# when there is none. Stdin is closed and a perl alarm ends it after 20s;")
    w("# signed in it answers in about a second. Only the CLI can be asked: the")
    w("# desktop app on its own is not checked. Reported under --check too.")
    w("report_antigravity_signin() {")
    w("  command -v agy >/dev/null 2>&1 || return 0")
    w('  local out rc=0 lower line=""')
    w("  if command -v perl >/dev/null 2>&1; then")
    w("    out=\"$(perl -e 'alarm shift; exec @ARGV or exit 127' 20 agy models </dev/null 2>&1)\" || rc=$?")
    w("  else")
    w('    out="$(agy models </dev/null 2>&1)" || rc=$?')
    w("  fi")
    w('  if [ "${rc}" = 0 ]; then')
    w("    return 0")
    w("  fi")
    w("  lower=\"$(printf '%s' \"${out}\" | tr '[:upper:]' '[:lower:]')\"")
    w('  case "${lower}" in')
    w('    *"sign in"*)')
    w('      warn "antigravity: needs sign-in: run agy and sign in with your Google account"')
    w("      return 0 ;;")
    w("  esac")
    w("  # 142 is SIGALRM: the alarm ended a call that did not answer.")
    w('  if [ "${rc}" = 142 ]; then')
    w('    warn "antigravity: agy models failed: no answer within 20s"')
    w("    return 0")
    w("  fi")
    w("  while IFS= read -r line; do")
    w('    case "${line}" in')
    w("      *[![:space:]]*) [ \"${line}\" = 'Fetching available models...' ] || break ;;")
    w("    esac")
    w('    line=""')
    w('  done <<< "${out}"')
    w('  warn "antigravity: agy models failed: ${line:-exit ${rc}}"')
    w("}")
    w("")
    _converge_mcp(data, w)
    _converge_plugins(data, w)
    w("# An explicit CLAUDE_CONFIG_DIR=~/.claude moves .claude.json inside it, where a bare `claude` never looks.")
    w("claude_in_profile() {")
    w('  local dir="$1"; shift')
    w('  if [ "${dir}" = "${CLAUDE_HOME}" ] && [ -z "${CLAUDE_CONFIG_DIR:-}" ]; then')
    w('    "$@"')
    w("  else")
    w('    CLAUDE_CONFIG_DIR="${dir}" "$@"')
    w("  fi")
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
    w("# The work profile lives in the default Claude directory, whatever")
    w("# CLAUDE_CONFIG_DIR the calling shell carries (a personal session exports")
    w("# its own). Dropping it keeps every claude call below on that directory.")
    w('CLAUDE_HOME="${HOME}/.claude"')
    w('if [ -n "${CLAUDE_CONFIG_DIR:-}" ] && [ "${CLAUDE_CONFIG_DIR%/}" != "${CLAUDE_HOME}" ]; then')
    w(
        '  echo "setup: ignoring CLAUDE_CONFIG_DIR=${CLAUDE_CONFIG_DIR};'
        ' the work profile installs into ${CLAUDE_HOME}" >&2'
    )
    w("fi")
    w("unset CLAUDE_CONFIG_DIR")
    w('CODEX_HOME="${CODEX_HOME:-$HOME/.codex}"')
    w('VIBE_HOME="${VIBE_HOME:-$HOME/.vibe}"')
    w("# vibe-private exports its own home as VIBE_HOME; setup run from inside a")
    w("# vibe-private session must still treat ~/.vibe as the shared home.")
    w('[ "${VIBE_HOME%/}" = "${HOME}/.vibe-private" ] && VIBE_HOME="${HOME}/.vibe"')
    w("# vibe-private's own Vibe home; the vibe-private launcher uses the same path.")
    w('VIBE_PRIVATE_HOME="${HOME}/.vibe-private"')
    w("# Antigravity's own global config, shared by the app, the CLI and the IDE.")
    w("# Never ~/.gemini itself: Gemini CLI reads the AGENTS.md and GEMINI.md there.")
    w('ANTIGRAVITY_HOME="${HOME}/.gemini/config"')
    w("# The CLI reads skills from its own folder as well as the global one.")
    w('ANTIGRAVITY_CLI_HOME="${HOME}/.gemini/antigravity-cli"')
    w("# 0 where Antigravity is not a supported surface: setup then skips it whole,")
    w("# rather than leaving half of it behind in ~/.gemini.")
    w("ANTIGRAVITY_READY=0")
    w('[ "$(uname -s)" = Darwin ] && ANTIGRAVITY_READY=1')
    w("# OpenCode 2's global config: opencode.json, AGENTS.md and skills/.")
    w('OPENCODE_HOME="${XDG_CONFIG_HOME:-$HOME/.config}/opencode"')
    w("# 0 on a cloud run: OpenCode is a workstation surface only.")
    w("OPENCODE_READY=1")
    w('[ "${CLOUD}" = 1 ] && OPENCODE_READY=0')
    w("# 1 once the opencode on PATH reports a 1.x release: its MCP config is then left alone.")
    w("OPENCODE_V1=0")
    w("# 0 after a failed split of the Vibe home: nothing then touches either home.")
    w("VIBE_HOMES_READY=1")
    w("")
    w("# The release this copy came from. Publishing a release stamps its tag here;")
    w("# a copy taken from a checkout keeps the placeholder.")
    w(f"AGENT_KIT_RELEASE_TAG='{RELEASE_TAG_PLACEHOLDER}'")
    w("")
    w("# Outside a checkout, re-run the copy in the release's kit bundle; --uninstall needs none of it.")
    w("use_kit_bundle() {")
    w(f'  [ -f "${{KIT_ROOT}}/{REGISTRY_PATH.relative_to(KIT_ROOT)}" ] && return 0')
    w('  if [ -n "${AGENT_KIT_BUNDLE_STAGE:-}" ]; then')
    w('    fail "the kit bundle at ${AGENT_KIT_BUNDLE_STAGE} is not a complete kit"')
    w("    exit 1")
    w("  fi")
    w('  local url="${AGENT_KIT_SKILLS_BUNDLE_URL:-}"')
    w("  # The bundle of this script's own release, never `latest`: installer and")
    w("  # bundle then always match, even while a newer release is publishing.")
    w('  if [ -z "${url}" ]; then')
    w('    case "${AGENT_KIT_RELEASE_TAG}" in')
    w(f'      v[0-9]*) url="{RELEASE_DOWNLOAD_BASE}/${{AGENT_KIT_RELEASE_TAG}}/{SKILLS_BUNDLE_ASSET}" ;;')
    w("      *)")
    w('        fail "this copy carries no release tag, so it has no kit bundle;" \\')
    w('          "set AGENT_KIT_SKILLS_BUNDLE_URL or run from a checkout"')
    w("        exit 1")
    w("        ;;")
    w("    esac")
    w("  fi")
    w('  local stage expected actual')
    w('  stage="$(mktemp -d)"')
    w('  log "not an agent-kit checkout; fetching the kit bundle from ${url}"')
    w("  # The checksum first, then the tarball under a URL keyed by it: a release")
    w("  # asset ignores the query, but an override host behind a CDN may cache the")
    w("  # tarball longer than its .sha256 and serve the previous release.")
    w('  if ! curl -fsSL -o "${stage}/kit.tar.gz.sha256" "${url}.sha256"; then')
    w('    fail "kit bundle unavailable at ${url}; set AGENT_KIT_SKILLS_BUNDLE_URL or run from a checkout"')
    w("    exit 1")
    w("  fi")
    w("  expected=\"$(awk '{print $1}' \"${stage}/kit.tar.gz.sha256\")\"")
    w('  if ! curl -fsSL -o "${stage}/kit.tar.gz" "${url}?sha256=${expected}"; then')
    w('    fail "kit bundle unavailable at ${url}; set AGENT_KIT_SKILLS_BUNDLE_URL or run from a checkout"')
    w("    exit 1")
    w("  fi")
    w("  if command -v sha256sum >/dev/null 2>&1; then")
    w("    actual=\"$(sha256sum \"${stage}/kit.tar.gz\" | awk '{print $1}')\"")
    w("  else")
    w("    actual=\"$(shasum -a 256 \"${stage}/kit.tar.gz\" | awk '{print $1}')\"")
    w("  fi")
    w('  if [ -z "${expected}" ] || [ "${expected}" != "${actual}" ]; then')
    w('    fail "kit bundle at ${url} failed its sha256 check" \\')
    w('      "(expected ${expected:-<empty>}, got ${actual}); refusing to run it"')
    w("    exit 1")
    w("  fi")
    w('  if ! tar -xzf "${stage}/kit.tar.gz" -C "${stage}" \\')
    w('     || [ ! -f "${stage}/installer/setup-workstation.sh" ]; then')
    w('    fail "kit bundle at ${url} does not carry installer/setup-workstation.sh"')
    w("    exit 1")
    w("  fi")
    w('  ok "kit bundle verified (sha256 ${actual})"')
    w("  # Under curl | bash the rest of this script is still on stdin: drain it so no")
    w("  # command below reads it as input and curl is not cut off mid-write.")
    w("  [ -t 0 ] || cat >/dev/null")
    w('  AGENT_KIT_BUNDLE_STAGE="${stage}" exec bash "${stage}/installer/setup-workstation.sh" \\')
    w('    ${ORIG_ARGS[@]+"${ORIG_ARGS[@]}"} </dev/null')
    w("}")
    w('[ "${UNINSTALL}" = 1 ] || use_kit_bundle')
    w('if [ -n "${AGENT_KIT_BUNDLE_STAGE:-}" ]; then')
    w("  trap 'rm -rf \"${AGENT_KIT_BUNDLE_STAGE}\"' EXIT")
    w("fi")
    w("")
    w("# Where install_linux commands install: system-wide when writable, as on a cloud VM.")
    w("if [ -w /usr/local/bin ]; then")
    w('  AK_BIN_DIR=/usr/local/bin AK_OPT_DIR=/usr/local/lib/agent-kit')
    w("else")
    w('  AK_BIN_DIR="$HOME/.local/bin" AK_OPT_DIR="$HOME/.local/lib/agent-kit"')
    w("fi")
    w("export AK_BIN_DIR AK_OPT_DIR")
    w("")
    w("# ensure_port_forward: keeps the loopback of a workstation_connect MCP")
    w("# server alive (a launchd agent on macOS).")
    w(f'. "${{KIT_ROOT}}/{PORT_FORWARD_HELPER}"')
    w("")
    w('if [ "${CLOUD}" = 1 ]; then')
    w(f'  . "${{KIT_ROOT}}/{CLOUD_HELPER}"')
    w("  cloud_prepare")
    w("fi")
    w("")
    w('CLAUDE_HOOKS_DIR="${CLAUDE_HOME}/hooks"')
    w('CODEX_HOOKS_DIR="${CODEX_HOME}/hooks"')
    w("")

    retired_pattern = "|".join(
        (
            "pre-tool-use-edit-recall",
            "pre-tool-use-git-commit-capture",
            "stop-session-digest",
            "kb-stop-digest",
            "user-prompt-submit-recall",
            "kb-user-prompt-recall",
        ),
    )

    # --- retired-hook purge, callable from a normal run AND --uninstall ---
    w("# -----------------------------------------------------------------")
    w("# The only agent hooks the estate ships are the critique hooks below")
    w("# (ADR-0002). This purges the three")
    w("# retired knowledge-recall hook groups (edit recall, git-commit")
    w("# capture, session digest) from Claude settings.json by COMMAND")
    w("# BASENAME, so an operator's own hooks survive, and removes the now-dead")
    w("# hook script files themselves. Runs on every setup AND on --uninstall,")
    w("# because a machine set up by the retired install.sh/install-agents.sh")
    w("# is exactly the one that still has these.")
    w("# -----------------------------------------------------------------")
    w("purge_retired_hooks() {")
    w("  # Claude's settings and Codex's hooks.json share the hook layout. Codex's file")
    w("  # now also holds Hindsight Coding Agents' hooks, so only the retired ones go.")
    w("  local settings")
    w('  for settings in "${CLAUDE_HOME}/settings.json" "${CODEX_HOME}/hooks.json"; do')
    w('  if [ ! -e "${settings}" ]; then')
    w('    ok "no retired knowledge hooks (${settings} does not exist)"')
    w('  elif ! command -v python3 >/dev/null 2>&1; then')
    w('    warn "python3 is not on PATH; cannot purge retired hooks from ${settings}"')
    w('  elif [ "${CHECK_ONLY}" = 1 ]; then')
    w(f'    if grep -qE {_q(retired_pattern)} "${{settings}}" 2>/dev/null; then')
    w('      warn "retired knowledge hooks are still wired in ${settings}"')
    w("    else")
    w('      ok "no retired knowledge hooks in ${settings}"')
    w("    fi")
    w("  else")
    w('    if AK_SETTINGS_FILE="${settings}" python3 - <<'"'"'PURGE'"'"'')
    w("import json")
    w("import os")
    w("import pathlib")
    w("import sys")
    w("")
    w('path = pathlib.Path(os.environ["AK_SETTINGS_FILE"])')
    w("")
    w("RETIRED = {")
    w('    "pre-tool-use-edit-recall.sh",')
    w('    "pre-tool-use-git-commit-capture.sh",')
    w('    "stop-session-digest.sh",')
    w('    "kb-stop-digest.sh",')
    w('    "kb-user-prompt-recall.sh",')
    w('    "user-prompt-submit-recall.sh",')
    w("}")
    w("")
    w("try:")
    w('    data = json.loads(path.read_text() or "{}")')
    w("except json.JSONDecodeError:")
    w('    print("purge: %s is not valid JSON; left untouched" % path, file=sys.stderr)')
    w("    raise SystemExit(0)")
    w("if not isinstance(data, dict):")
    w("    raise SystemExit(0)")
    w("")
    w('hooks = data.get("hooks")')
    w("if not isinstance(hooks, dict):")
    w("    raise SystemExit(0)")
    w("")
    w("")
    w("def retired(group):")
    w('    for hook in group.get("hooks", []) if isinstance(group, dict) else []:')
    w('        command = hook.get("command", "") if isinstance(hook, dict) else ""')
    w('        for token in command.replace(\'"\', " ").split():')
    w('            if token.rsplit("/", 1)[-1] in RETIRED:')
    w("                return True")
    w("    return False")
    w("")
    w("")
    w("removed = 0")
    w("for event in list(hooks):")
    w("    groups = hooks.get(event)")
    w("    if not isinstance(groups, list):")
    w("        continue")
    w("    kept = [g for g in groups if not retired(g)]")
    w("    removed += len(groups) - len(kept)")
    w("    if kept:")
    w("        hooks[event] = kept")
    w("    else:")
    w("        hooks.pop(event, None)")
    w("")
    w("if hooks:")
    w('    data["hooks"] = hooks')
    w("else:")
    w('    data.pop("hooks", None)')
    w("")
    w('path.write_text(json.dumps(data, indent=2) + "\\n")')
    w("")
    w("back = path.read_text()")
    w("still = sorted(name for name in RETIRED if name in back)")
    w("if still:")
    w('    print("purge: FAILED, still referenced: %s" % ", ".join(still), file=sys.stderr)')
    w("    raise SystemExit(1)")
    w('print("purge: removed %d retired hook group(s); none remain" % removed)')
    w("PURGE")
    w("    then")
    w('      ok "retired knowledge hooks purged from ${settings}"')
    w("    else")
    w('      fail "purge of retired knowledge hooks in ${settings} failed"')
    w("    fi")
    w("  fi")
    w("  done")
    w("")
    w("  local stale")
    w('  for stale in \\')
    w('    "${CLAUDE_HOOKS_DIR}/pre-tool-use-edit-recall.sh" \\')
    w('    "${CLAUDE_HOOKS_DIR}/pre-tool-use-git-commit-capture.sh" \\')
    w('    "${CLAUDE_HOOKS_DIR}/stop-session-digest.sh" \\')
    w('    "${CLAUDE_HOOKS_DIR}/user-prompt-submit-recall.sh" \\')
    w('    "${CODEX_HOOKS_DIR}/pre-tool-use-edit-recall.sh" \\')
    w('    "${CODEX_HOOKS_DIR}/pre-tool-use-git-commit-capture.sh" \\')
    w('    "${CODEX_HOOKS_DIR}/kb-stop-digest.sh" \\')
    w('    "${CODEX_HOOKS_DIR}/kb-user-prompt-recall.sh"')
    w("  do")
    w('    if [ -e "${stale}" ]; then')
    w('      if [ "${CHECK_ONLY}" = 1 ]; then')
    w('        log "would remove retired hook file ${stale}"')
    w("      else")
    w('        rm -f "${stale}"')
    w('        log "removed retired hook file ${stale}"')
    w("      fi")
    w("    fi")
    w("  done")
    w('  if [ "${CHECK_ONLY}" != 1 ]; then')
    w('    rmdir "${CLAUDE_HOOKS_DIR}" "${CODEX_HOOKS_DIR}" 2>/dev/null || true')
    w("  fi")
    w("}")
    w("")
    _no_agent_attribution_fns(w)
    _leaked_runner_git_config_fn(w)
    _leaked_runner_workspace_trust_fn(w)
    _critique_hooks_fn(w)

    # --- --uninstall: remove what a PREVIOUS install.sh/install-agents.sh wrote ---
    council_dir = KIT_ROOT / "skills" / "workstation" / "council"
    council_relative = sorted(
        str(p.relative_to(council_dir)) for p in council_dir.rglob("*") if p.is_file()
    )
    speckit_names = [
        "analyze",
        "checklist",
        "clarify",
        "constitution",
        "implement",
        "plan",
        "specify",
        "tasks",
        "taskstoissues",
    ]
    legacy_shared_skill_names = [
        "topics",
        "audit",
        "kb-first",
        "token-economy",
        "agent-session-bootstrap",
        "grill-me",
    ]
    legacy_claude_paths = (
        [f"commands/speckit.{n}.md" for n in speckit_names]
        + [f"skills/{n}/SKILL.md" for n in legacy_shared_skill_names]
        + [f"skills/council/{p}" for p in council_relative]
        + [".knowledge-system-version"]
    )
    legacy_codex_paths = (
        [f"skills/speckit-{n}/SKILL.md" for n in speckit_names]
        + [f"skills/{n}/SKILL.md" for n in legacy_shared_skill_names]
        + [f"skills/council/{p}" for p in council_relative]
        + [".knowledge-system-version"]
    )
    w("# -----------------------------------------------------------------")
    w("# --uninstall: remove every file the retired install.sh /")
    w("# install-agents.sh wrote (base skills, Spec Kit commands/skills, the")
    w("# knowledge-system version manifest), then purge retired hooks, remove")
    w("# the critique hooks from Claude Code and Codex, and exit. A normal run")
    w("# removes their MCP entries (converge_mcp). Beyond the critique hooks it")
    w("# does not touch anything this script itself manages -- a re-run with no")
    w("# flags reinstalls the current local_skills and the hooks afterward.")
    w("# -----------------------------------------------------------------")
    w('if [ "${UNINSTALL}" = 1 ]; then')
    w('  log "uninstalling files from a previous install.sh / install-agents.sh"')
    w("  legacy_remove() {")
    w('    if [ ! -e "$1" ]; then return 0; fi')
    w('    if [ "${CHECK_ONLY}" = 1 ]; then')
    w('      log "would remove $1"')
    w("    else")
    w('      rm -f "$1"')
    w('      log "removed $1"')
    w("    fi")
    w("  }")
    for rel in legacy_claude_paths:
        w(f'  legacy_remove "${{CLAUDE_HOME}}/{rel}"')
    for rel in legacy_codex_paths:
        w(f'  legacy_remove "${{CODEX_HOME}}/{rel}"')
    w("  purge_retired_hooks")
    w("  critique_hooks remove")
    w('  log "uninstall complete: ${failures} failure(s), ${warnings} warning(s)"')
    w('  [ "${failures}" = 0 ]')
    w("  exit $?")
    w("fi")
    w("")

    # --- CLIs ---
    w("# -----------------------------------------------------------------")
    w("# 1. Command-line tools.")
    w("# -----------------------------------------------------------------")
    w("tokens_and_keys")
    if data.get("package_proxy"):
        # ${HOME:-} so the step, not set -u, reports a missing HOME: the
        # guard inside warns and returns; nothing else in this step is read.
        w("")
        w('package_proxy_setup "${HOME:-}"')
    w("")
    w('log "command-line tools"')
    for cli in _entries(data, "clis"):
        if not on_surface(cli, "workstation"):
            continue
        name = cli["name"]
        cask = cli.get("cask")
        binary = cli.get("binary")
        version_command = cli.get("version_command")
        provides = cli.get("provides_binaries") or ([binary] if binary else [])
        w("")
        for chunk in _wrap(str(cli.get("purpose") or "").strip(), 64):
            w(f"# {chunk}")
        block: list[str] = []
        b = block.append
        if cask:
            # Homebrew owns the cask, so presence is the cask list rather than a
            # binary on PATH (the desktop app has none), and `brew upgrade` is a
            # no-op when the cask is already current.
            present = f"brew list --cask {cask} >/dev/null 2>&1"
            install = f"run brew install --cask {cask}"
            update = f"run brew upgrade --cask {cask}"
            label = name
            failed = f"{name}: brew install --cask {cask} did not install it; is Homebrew on PATH?"
        else:
            install_cmd = str(cli.get("install") or "")
            if not install_cmd:
                raise RegistryError(f"cli {name} must name an install command")
            present = f"command -v {binary} >/dev/null 2>&1"
            install = f"run_sh {_q(install_cmd)}"
            update = f"run_sh {_q(str(cli.get('update') or install_cmd))}"
            label = str(binary)
            failed = f"{binary} still not on PATH after install; open a new shell and re-run"
        b(f"if {present}; then")
        b(f"  {update}")
        if version_command:
            b(f'  ok "{binary} $({version_command} 2>/dev/null | head -1)"')
        else:
            b(f'  ok "{label} present"')
        b("else")
        b(f"  {install}")
        b('  if [ "${CHECK_ONLY}" = 1 ]; then')
        b(f'    log "{label} would be installed"')
        b(f"  elif {present}; then")
        b(f'    ok "{label} installed"')
        # Track newly installed binaries
        for bin_name in provides:
            b(f'    new_binaries+=("{bin_name}")')
        b("  else")
        b(f'    fail "{failed}"')
        b("  fi")
        b("fi")
        for var, why in (cli.get("requires_env") or {}).items():
            why_escaped = str(why).replace('"', '\\"')
            b(f'if [ -z "${{{var}:-}}" ]; then')
            b(_env_report(data, f'{name}: {var} is not set ({why_escaped}); export it and re-run', var, "  "))
            b(f'  missing_plugin_env+=("{name}: export {var}")')
            b("fi")
        if cli.get("macos_only"):
            # The cask is macOS-only; elsewhere the block is skipped with a notice.
            block = [
                'if [ "$(uname -s)" = Darwin ]; then',
                *[f"  {line}" if line else line for line in block],
                "else",
                f'  warn "{name}: macOS only; skipped on $(uname -s)"',
                "fi",
            ]
        out.extend(_cloud_guard(block, cli, ""))
    w("")
    w("report_antigravity_signin")
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
    w("# Claude Code may not read or run anything that touches Private Memory's")
    w("# files: the Vibe homes (their session logs) and the gated token. Against accidents,")
    w("# not a determined agent; the passphrase is the boundary.")
    w("claude_private_denies")
    w("")
    w("# MCP servers the registry trusts to run every tool without a prompt.")
    w("claude_mcp_allows")
    w("")
    w("# Claude Code credits neither itself nor its model in commits or PRs.")
    w("claude_no_attribution")
    w("")
    w("# Before anything touches a Vibe home (MCP merge, converge, skills), and")
    w("# whether or not vibe is on PATH yet.")
    w('[ "${CLOUD}" = 1 ] || _vibe_split_homes')
    w("")
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
    cloud_markets = {"claude-plugins-official"} if _entries(data, "language_servers") else set()
    cloud_markets |= {p["marketplace"] for p in _entries(data, "plugins") if on_cloud(p)}
    for market in _entries(data, "marketplaces"):
        alias = f' {market["declare_as"]}' if market.get("declare_as") else ""
        block = [f'  ensure_marketplace {market["name"]} {market["repo"]} {market["commit"]}{alias}']
        out.extend(_cloud_guard(block, {**market, "cloud": market["name"] in cloud_markets}, "  "))
    w("")
    w('  log "plugins"')
    if any(m.get("declare_as") for m in _entries(data, "marketplaces")):
        w('  installed_plugins="$(claude plugin list --json 2>/dev/null || true)"')
    for plugin in _entries(data, "plugins"):
        ref = f'{plugin["name"]}@{declared_name(data, plugin["marketplace"])}'
        w(f'  # {plugin["name"]}: ' + " ".join(str(plugin.get("purpose") or "").split())[:200])
        block = _unalias(data, plugin["name"], plugin["marketplace"], "  ")
        b = block.append
        b(f'  if run claude plugin install {ref} --yes --scope user; then')
        # Do NOT run unconditional update - that causes drift. Only on explicit request.
        if plugin.get("enabled", True):
            b(f'    run claude plugin enable {ref} >/dev/null 2>&1 || true')
            b(f'    ok "{ref} installed and enabled"')
        else:
            b(f'    run claude plugin disable {ref} >/dev/null 2>&1 || true')
            b(f'    ok "{ref} installed, left disabled on purpose"')
            b(f'    disabled_plugins+=("{plugin["name"]}: disabled on purpose in registry")')
        b("  else")
        b(f'    warn "{ref} install failed"')
        b("  fi")
        user_config = plugin.get("user_config")
        if user_config:
            want = json.dumps(user_config["set"], sort_keys=True)
            b(f'  _merge_user_config "${{HOME}}/{user_config["path"]}" \'{want}\' \\')
            b(f'    && ok "{plugin["name"]}: {user_config["path"]} set" \\')
            b(f'    || warn "{plugin["name"]}: could not update {user_config["path"]}"')
        # Unset: the plugin falls back silently, so warn instead of trusting the default.
        requires_env = plugin.get("requires_env") or {}
        for var, purpose in requires_env.items():
            purpose_escaped = str(purpose).replace('"', '\\"')
            b(f'  if [ -z "${{{var}:-}}" ]; then')
            b(_env_report(data, f'{plugin["name"]}: {var} is not set ({purpose_escaped}); export it and re-run',
                          var, "    "))
            b(f'    missing_plugin_env+=("{plugin["name"]}: export {var}")')
            b("  fi")
        guarded = _pinned_guard(block, plugin["marketplace"], plugin["name"], "  ")
        out.extend(_cloud_guard(guarded, plugin, "  "))
    w("")
    if data.get("coding_agents"):
        w("  hindsight_coding_agents")
    w("  # memory-private on|off|status [DIR]: keeps a directory's sessions out of")
    w("  # Shared Memory (HINDSIGHT_DISABLED=true in its local settings).")
    w("  install_kit_command memory-private")
    w("  # agent-kit-private: the passphrase gate in front of the Private Memory token.")
    w("  install_kit_command agent-kit-private")
    w("  # The Vibe launchers. Plain vibe is Vibe itself: the shared Vibe home, the")
    w("  # personal login, Shared Memory. vibe-study is the same home with the Study")
    w("  # account; vibe-private has its own home and Private Memory, behind the gate.")
    w("  # Workstation only: a cloud session gets plain vibe.")
    w('  if [ "${CLOUD}" != 1 ]; then')
    w("    install_kit_command vibe-study")
    w("    install_kit_command vibe-private")
    w("    # Antigravity's MCP config cannot hold a token, so Shared Memory reaches")
    w("    # it through this stdio proxy, which reads the token from the keychain.")
    w('    if [ "${ANTIGRAVITY_READY}" = 1 ]; then')
    w("      install_kit_command antigravity-mcp-proxy")
    w("    fi")
    w("  fi")
    w("  # Retired: Vibe saves to memory by tool now, not every turn.")
    w("  retire_kit_command vibe-retain-private")
    w("")
    w("  # ---------------------------------------------------------------")
    w("  # 4. Plugin drift detection.")
    w("  #")
    w("  # Each marketplace is checked out at its pinned commit, which decides")
    w("  # the commit every plugin in it should be at. A plugin installed before")
    w("  # (or off) its pin is updated to it; only one that will not move is")
    w("  # reported.")
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
        markets = {m["name"]: m for m in _entries(data, "marketplaces")}
        for plugin in _entries(data, "plugins"):
            market = markets[plugin["marketplace"]]
            ref = f'{plugin["name"]}@{declared_name(data, plugin["marketplace"])}'
            folder = f'"${{AGENT_KIT_MARKETPLACES:-${{HOME}}/.local/share/agent-kit/marketplaces}}/{market["name"]}"'
            w(f'    expected_commit="$(expected_plugin_commit {folder} {plugin["name"]} {market["commit"]})"')
            w(f'    installed_commit="$(installed_plugin_commit "${{manifest}}" "{ref}")"')
            w('    if [ -z "${expected_commit}" ]; then')
            w(f'      warn "{plugin["name"]}: its marketplace entry pins no commit, so it cannot be pinned"')
            w('    elif [ -z "${installed_commit}" ]; then')
            w(f'      log "{plugin["name"]}: no commit recorded; drift not checked"')
            w('    elif [ "${installed_commit}" = "${expected_commit}" ]; then')
            w(f'      ok "{plugin["name"]}: at its pinned commit"')
            w("    else")
            w(f"      run claude plugin update {ref} >/dev/null 2>&1 || true")
            w(f'      installed_commit="$(installed_plugin_commit "${{manifest}}" "{ref}")"')
            w('      if [ "${installed_commit}" = "${expected_commit}" ]; then')
            w(f'        ok "{plugin["name"]}: moved to its pinned commit ${{expected_commit:0:12}}"')
            w("      else")
            w(f'        warn "{plugin["name"]}: installed ${{installed_commit:0:12}} '
              'will not move to its pinned ${expected_commit:0:12}"')
            w(f'        plugin_drift+=("{plugin["name"]}: ${{installed_commit:0:12}} vs ${{expected_commit:0:12}}")')
            w("      fi")
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
        ref = f"{plugin}@{declared_name(data, 'claude-plugins-official')}"
        enabled = server.get("enabled", True)
        w("")
        w(f'    # {plugin} -> {binary} ({", ".join(server.get("languages") or [])})')
        block = _unalias(data, plugin, "claude-plugins-official", "    ")
        b = block.append
        b(f'    if run claude plugin install {ref} --yes --scope user; then')
        runtime = server.get("runtime")
        nixos_pkg = server.get("nixos")
        # A server that needs a runtime (jdtls runs `java`) is usable only
        # when both are on PATH; the launcher alone answers no request.
        present = f"command -v {binary} >/dev/null 2>&1"
        if runtime:
            present += f" && command -v {runtime} >/dev/null 2>&1"
        b(f"      if {present}; then")
        b(f'        run claude plugin enable {ref} >/dev/null 2>&1 || true')
        if enabled:
            b(f'        ok "{plugin}: {binary} on PATH, plugin enabled"')
        else:
            b(f'        ok "{plugin}: {binary} on PATH, so plugin enabled (off by default)"')
        if runtime:
            b(f"      elif command -v {binary} >/dev/null 2>&1; then")
            b(f'        run claude plugin disable {ref} >/dev/null 2>&1 || true')
            b(f'        warn "{plugin}: {binary} is on PATH but {runtime} is not; plugin left disabled"')
            b(f'        missing_lsp_binaries+=("{plugin}: install {runtime}, which {binary} runs")')
        b("      else")
        b(f'        run claude plugin disable {ref} >/dev/null 2>&1 || true')
        if not enabled:
            # Off by default: installing the binary by hand is the opt-in,
            # so setup never installs it.
            b(f'        log "{plugin}: off by default; install {binary} to enable it"')
            b("      fi")
            b("    else")
            b(f'      warn "{ref} install failed"')
            b("    fi")
            guarded = _pinned_guard(block, "claude-plugins-official", plugin, "    ")
            out.extend(_cloud_guard(guarded, server, "    "))
            continue
        toolchain = server.get("toolchain")
        if toolchain:
            # No toolchain means no work in that language here: a fact, not a to-do.
            b(f"        if ! command -v {toolchain} >/dev/null 2>&1; then")
            b(f'          log "{plugin}: no {toolchain} on this machine; plugin left off"')
            b("        else")
        install_at = len(block)
        if nixos_pkg:
            # NixOS cannot run brew, gem, go install or rustup into its store:
            # nix-config declares the binary, setup only reports it.
            b('        if [ -e "${AK_NIXOS_MARKER:-/etc/NIXOS}" ]; then')
            needs = f"pkgs.{nixos_pkg}" + (f" and a {runtime}" if runtime else "")
            b(
                f'          warn "{plugin}: {binary} is absent; on NixOS declare {needs} '
                'in nix-config; plugin left disabled"'
            )
            b(f'          missing_lsp_binaries+=("{plugin}: declare {needs} in nix-config")')
            b("        else")
        nix_at = len(block)
        # Attempt install if one is configured
        if lsp_install:
            if server.get("install_linux"):
                b('        if [ "$(uname -s)" = Linux ]; then')
                b(f"          lsp_cmd={_q(server['install_linux'])}")
                b("        else")
                b(f"          lsp_cmd={_q(lsp_install)}")
                b("        fi")
            else:
                b(f"        lsp_cmd={_q(lsp_install)}")
            b('        run_sh "${lsp_cmd}" || true')
            b(f"        if {present}; then")
            b(f'          run claude plugin enable {ref} >/dev/null 2>&1 || true')
            b(f'          ok "{plugin}: {binary} installed and enabled"')
            b("        else")
            b(f'          warn "{plugin}: {binary} is absent; plugin left disabled (install missing: ${{lsp_cmd}})"')
            b(f'          missing_lsp_binaries+=("{plugin}: install with: ${{lsp_cmd}}")')
            b("        fi")
        else:
            msg = (
                f'"{plugin}: {binary} is absent and ships with its platform '
                f'toolchain; plugin left disabled"'
            )
            b(f'        warn {msg}')
            b(f'        missing_lsp_binaries+=("{plugin}: missing from toolchain (no install command)")')
        if nixos_pkg:
            block[nix_at:] = ["  " + line for line in block[nix_at:]]
            b("        fi")
        if toolchain:
            block[install_at:] = ["  " + line for line in block[install_at:]]
            b("        fi")
        b("      fi")
        b("    else")
        b(f'      warn "{ref} install failed"')
        b("    fi")
        guarded = _pinned_guard(block, "claude-plugins-official", plugin, "    ")
        out.extend(_cloud_guard(guarded, server, "    "))
    w("  else")
    w('    log "language servers skipped (--no-lsp)"')
    w("  fi")
    w("")

    # Once, in the primary profile: plugins/ and settings.json are shared.
    w("  converge_plugins")
    w("")
    # --- MCP ---
    w("  # ---------------------------------------------------------------")
    w("  # 6. MCP servers, registered for Claude Code at user scope.")
    w("  #")
    w("  # The registry is authoritative: declared servers are ensured, then")
    w("  # converge_mcp removes retired and undeclared ones (see CONTEXT.md).")
    w("  #")
    w("  # Every claude call here runs once per profile: a server lives in the")
    w("  # profile's own .claude.json, which is the one file two profiles must")
    w("  # not share, so the fleet is registered into each of them.")
    w("  # ---------------------------------------------------------------")
    w('  if [ "${DO_MCP}" = 1 ]; then')
    w('    log "MCP servers"')
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
        if session_only(server):
            # Private Memory lives in vibe-private's Vibe home alone. Earlier
            # versions installed it for Claude Code (a --mcp-config file) and
            # Codex (switched off); an upgrade removes both.
            cleanup = [
                "    if command -v claude >/dev/null 2>&1; then",
                f"      run claude_each_profile claude mcp remove --scope user {name} >/dev/null 2>&1 || true",
                "    fi",
                f'    run rm -f "${{XDG_CONFIG_HOME:-${{HOME}}/.config}}/agent-kit/{name}.mcp.json"',
                "    if command -v codex >/dev/null 2>&1; then",
                f"      run codex mcp remove {name} >/dev/null 2>&1 || true",
                "    fi",
            ]
            out.extend(_cloud_guard(cleanup, server, "    "))
            continue
        # A REQUIRED credential gates registration; an OPTIONAL one never does
        # (the server self-manages auth -- e.g. overleaf via olcli -- so an
        # unset override must not skip it, that was the #52 regression). In
        # cloud mode nothing gates: the value arrives with the session.
        block = []
        b = block.append
        guard_open = False
        if credential and not optional_cred:
            b(f'    if [ -z "${{{credential}:-}}" ] && [ "${{CLOUD}}" != 1 ]; then')
            if credential in managed_secrets(data):
                b(f'      log "{name}: {credential} is not set; not registered (see tokens and keys)"')
            else:
                b(f'      warn "{name}: {credential} is not set; skipping (export it and re-run)"')
            b(f'      skipped_mcp_servers+=("{name}: export {credential}")')
            b(f"      skipped_mcp_names+=({name})")
            b("    else")
            guard_open = True
        indent = "      " if guard_open else "    "
        binary = server.get("requires_binary")
        if binary and server.get("install_pinned"):
            source = pinned_source(server)
            pin = f'{source["package"]}@{source["version"]}'
            install = " && ".join(c for c in (f'npm install -g "{pin}"', server.get("post_install")) if c)
            b(f'{indent}if ! npm ls -g --depth=0 "{pin}" >/dev/null 2>&1; then')
            b(f"{indent}  run_sh {_q(install)} || true")
            b(f"{indent}fi")
            b(f'{indent}command -v {binary} >/dev/null 2>&1 \\')
            b(f'{indent}  || warn "{name}: {binary} is not on PATH; skipping"')
        elif binary:
            b(f'{indent}if ! command -v {binary} >/dev/null 2>&1; then')
            if server.get("install"):
                b(f'{indent}  run_sh {_q(server["install"])} || true')
            b(f'{indent}  command -v {binary} >/dev/null 2>&1 \\')
            b(f'{indent}    || warn "{name}: {binary} is not on PATH; skipping"')
            b(f"{indent}fi")
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
            b("")
            b(f"    # {name} reaches the cluster via a kubectl port-forward of the")
            b(f"    # ClusterIP service {svc}.{ns} (no ingress route exists for it).")
            b(f"    ensure_port_forward {name} {ns} {svc} {lport} {rport}")
        # --- Claude Code ---
        b(f"{indent}if command -v claude >/dev/null 2>&1; then")
        b(f'{indent}  run claude_each_profile claude mcp remove --scope user {name} >/dev/null 2>&1 || true')
        if server["transport"] == "http":
            url = server.get("url_workstation")
            if not url:
                raise RegistryError(f"mcp server {name} is on workstation but has no url_workstation")
            add = f"claude mcp add --scope user {name} --transport http {url}"
            if credential:
                add += f' --header "Authorization: Bearer $(secret_ref {credential})"'
        else:
            args = " ".join(server.get("args") or [])
            env_flags = _mcp_env_flags(server, credential, optional_cred, claude=True)
            # Name BEFORE the --env flags: claude 2.x rejects the redeclared
            # order (`--env K=V` then the name) with "missing required argument
            # 'commandOrUrl'". Verified against claude 2.1.267.
            add = f"claude mcp add --scope user {name}{env_flags} -- {server['command']} {args}".rstrip()
        b(f'{indent}  if run_redacted "claude mcp add {name}" claude_each_profile {add}; then')
        b(f'{indent}    ok "{name} registered (claude)"')
        b(f"{indent}  else")
        b(f'{indent}    fail "{name} registration failed (claude)"')
        b(f"{indent}  fi")
        b(f"{indent}fi")
        # --- Codex ---
        b(f"{indent}if command -v codex >/dev/null 2>&1; then")
        if server.get("auth") == "oauth":
            # `codex mcp add --url` starts the OAuth flow at once and waits for a
            # browser, so the helper writes the entry and codex reads it back.
            b(f'{indent}  if run uv run --no-project --with tomlkit python '
              f'"${{KIT_ROOT}}/scripts/codex-add-url-mcp.py" '
              f'"${{CODEX_HOME:-${{HOME}}/.codex}}/config.toml" {name} {server["url_workstation"]} \\')
            b(f"{indent}    && run codex mcp get {name} >/dev/null; then")
        else:
            b(f"{indent}  run codex mcp remove {name} >/dev/null 2>&1 || true")
            if server["transport"] == "http":
                url = server.get("url_workstation")
                add = f"codex mcp add {name} --url {url}"
                if credential:
                    # Unlike claude, codex reads this env var at its own runtime.
                    add += f" --bearer-token-env-var {credential}"
            else:
                args = " ".join(server.get("args") or [])
                env_flags = _mcp_env_flags(server, credential, optional_cred)
                add = f"codex mcp add {name}{env_flags} -- {server['command']} {args}".rstrip()
            b(f'{indent}  if run_redacted "codex mcp add {name}" {add}; then')
        b(f'{indent}    ok "{name} registered (codex)"')
        b(f"{indent}  else")
        b(f'{indent}    fail "{name} registration failed (codex)"')
        b(f"{indent}  fi")
        b(f"{indent}fi")
        if server.get("auth") == "oauth":
            b(f"{indent}oauth_status {name}")
        if guard_open:
            b("    fi")
        out.extend(_cloud_guard(block, server, "    "))
    w("")
    # --- local Hermes: merge the workstation-flavoured mcp_servers block ---
    w("    # Local Hermes reads its MCP servers from ~/.hermes/config.yaml.")
    w("    # `hermes mcp add` is interactive (probes + prompts), so the setup")
    w("    # script merges the generated workstation block via a helper instead.")
    w("    if command -v hermes >/dev/null 2>&1; then")
    w("      if [ \"${CHECK_ONLY}\" = 1 ]; then")
    w("        log \"would merge local Hermes MCP servers\"")
    w("      else")
    w("        uv run --no-project --with pyyaml python \"${KIT_ROOT}/scripts/hermes-merge-mcp.py\" \\")
    w(f"          \"${{KIT_ROOT}}/{HERMES_LOCAL_MCP}\" \\")
    w("          || fail \"local Hermes MCP merge failed\"")
    retired = retired_servers(data)
    if retired:
        # The merge only adds and replaces, so retired servers go in a separate pass.
        removes = " ".join(f"--remove {name}" for name in retired)
        w("        uv run --no-project --with pyyaml python \"${KIT_ROOT}/scripts/hermes-merge-mcp.py\" \\")
        w(f"          \"${{KIT_ROOT}}/{HERMES_LOCAL_MCP}\" {removes} \\")
        w("          || fail \"local Hermes retired MCP cleanup failed\"")
    hermes_off_cloud = [
        s["name"]
        for s in _entries(data, "mcp_servers")
        if on_surface(s, "workstation") and not on_cloud(s) and not session_only(s)
    ]
    if hermes_off_cloud:
        removes = " ".join(f"--remove {name}" for name in hermes_off_cloud)
        w('        if [ "${CLOUD}" = 1 ]; then')
        w("          uv run --no-project --with pyyaml python \"${KIT_ROOT}/scripts/hermes-merge-mcp.py\" \\")
        w(f"            \"${{KIT_ROOT}}/{HERMES_LOCAL_MCP}\" {removes} \\")
        w("            || fail \"local Hermes MCP cleanup failed\"")
        w("        fi")
    w("      fi")
    w("    else")
    w("      warn \"hermes not on PATH; local Hermes MCP config not merged\"")
    w("    fi")
    w("    # Mistral Vibe reads [[mcp_servers]] from ${VIBE_HOME}/config.toml and has")
    w("    # no `mcp add`; the helper replaces the servers it manages, keeps the rest.")
    w("    # Two Vibe homes: the shared one (vibe, vibe-study) never holds Private")
    w("    # Memory; vibe-private's own home is the only one that does.")
    w('    if command -v vibe >/dev/null 2>&1 && [ "${VIBE_HOMES_READY}" = 1 ]; then')
    w('      if [ "${CHECK_ONLY}" = 1 ]; then')
    w('        log "would merge Vibe MCP servers into ${VIBE_HOME}/config.toml"')
    w("      else")
    w("        vibe_remove=()")
    vibe_off_cloud = [s["name"] for s in vibe_servers(data) if not on_cloud(s)]
    if vibe_off_cloud:
        removes = " ".join(f"--remove {name}" for name in vibe_off_cloud)
        w(f'        [ "${{CLOUD}}" = 1 ] && vibe_remove+=({removes})')
    w('        uv run --no-project --with tomlkit python "${KIT_ROOT}/scripts/vibe-merge-mcp.py" \\')
    # A shared home from before the split still lists Private Memory: drop it.
    private_only = "".join(
        f" --remove {s['name']}" for s in _entries(data, "mcp_servers")
        if session_only(s) and s not in vibe_servers(data)
    )
    remove_args = '${vibe_remove[@]+"${vibe_remove[@]}"}'
    w(f'          "${{KIT_ROOT}}/{VIBE_MCP}" --config "${{VIBE_HOME}}/config.toml"{private_only} {remove_args} \\')
    w('          && ok "Vibe MCP servers merged" \\')
    w('          || fail "Vibe MCP merge failed"')
    w('        _vibe_models "${VIBE_HOME}"')
    w(f"        vibe_hooks_remove=({_retired_vibe_hooks()})")
    hook_names =[h["name"] for h in tomllib.loads((KIT_ROOT / VIBE_HOOKS).read_text()).get("hooks", [])]
    if hook_names:
        cloud_removes = " ".join(f"--remove {n}" for n in hook_names)
        w(f'        [ "${{CLOUD}}" = 1 ] && vibe_hooks_remove+=({cloud_removes})')
    hooks_remove = '${vibe_hooks_remove[@]+"${vibe_hooks_remove[@]}"}'
    w(f'        _vibe_hooks "${{VIBE_HOME}}" {hooks_remove}')
    w("      fi")
    w('      _vibe_agents_pointer "${VIBE_HOME}" shared')
    w('      gateway_pointer "${VIBE_HOME}/AGENTS.md"')
    w('      attribution_rule "${VIBE_HOME}/AGENTS.md"')
    w('      vibe_no_commit_signature "${VIBE_HOME}"')
    w('      [ "${CLOUD}" = 1 ] || _vibe_private_home')
    w("    else")
    w('      warn "vibe not on PATH, or its home is not split yet; Vibe MCP config not merged"')
    w("    fi")
    w("    # Antigravity reads one global mcp_config.json, shared by the app, the CLI")
    w("    # and the IDE. `agy mcp add` writes one server at a time, so the fragment")
    w("    # is merged in one pass. It names the stdio proxy for any server that")
    w("    # needs a token, so no secret reaches that file. Private Memory is removed")
    w("    # even when someone added it by hand: no keep file keeps it here.")
    w('    if [ "${CLOUD}" != 1 ] && [ "${ANTIGRAVITY_READY}" = 1 ] && command -v agy >/dev/null 2>&1; then')
    w('      if [ "${CHECK_ONLY}" = 1 ]; then')
    w('        log "would merge Antigravity MCP servers into ${ANTIGRAVITY_HOME}/mcp_config.json"')
    w("      elif python3 \"${KIT_ROOT}/scripts/antigravity-merge-mcp.py\" \\")
    antigravity_private = [
        str(s["name"]) for s in _entries(data, "mcp_servers")
        if session_only(s) and s not in antigravity_servers(data)
    ]
    removes = "".join(f" --remove {name}" for name in [*retired_servers(data), *antigravity_private])
    w(f'        "${{KIT_ROOT}}/{ANTIGRAVITY_MCP}" --config "${{ANTIGRAVITY_HOME}}/mcp_config.json"{removes}; then')
    w('        ok "Antigravity MCP servers merged"')
    w("      else")
    w('        fail "Antigravity MCP merge failed"')
    w("      fi")
    w("    fi")
    w("    # Antigravity's global rules file. Never ~/.gemini/AGENTS.md or")
    w("    # GEMINI.md: Gemini CLI reads those.")
    w('    if [ "${CLOUD}" != 1 ] && [ "${ANTIGRAVITY_READY}" = 1 ]; then')
    w('      _agents_pointer "${ANTIGRAVITY_HOME}/AGENTS.md" shared')
    w('      gateway_pointer "${ANTIGRAVITY_HOME}/AGENTS.md"')
    w('      attribution_rule "${ANTIGRAVITY_HOME}/AGENTS.md"')
    w("    fi")
    w("    # OpenCode 2 reads mcp.servers from its global opencode.json and fills in")
    w("    # {env:NAME} at load, so the fragment names each credential and holds")
    w("    # none. OpenCode 1 reads another shape, so a V1 binary is named, not fed.")
    w('    if [ "${OPENCODE_READY}" = 1 ] && command -v opencode >/dev/null 2>&1; then')
    w('      opencode_version="$(opencode --version 2>/dev/null | head -1)"')
    w("      # V2 prints `opencode v2.2.5`; match the last word, with or without its v.")
    w('      opencode_release="${opencode_version##* }"')
    w('      case "${opencode_release#v}" in')
    w("        1.*)")
    w("          OPENCODE_V1=1")
    w('          warn "opencode: ${opencode_version} is OpenCode 1; its MCP config is left alone;'
      ' agent-kit configures OpenCode 2 (brew install opencode)" ;;')
    w("      esac")
    w('      if [ "${OPENCODE_V1}" = 1 ]; then')
    w("        :")
    w('      elif [ "${CHECK_ONLY}" = 1 ]; then')
    w('        log "would merge OpenCode MCP servers into ${OPENCODE_HOME}/opencode.json"')
    w("      elif python3 \"${KIT_ROOT}/scripts/opencode-merge-mcp.py\" \\")
    opencode_private = [
        str(s["name"]) for s in _entries(data, "mcp_servers")
        if session_only(s) and s not in opencode_servers(data)
    ]
    removes = "".join(f" --remove {name}" for name in [*retired_servers(data), *opencode_private])
    w(f'        "${{KIT_ROOT}}/{OPENCODE_MCP}" --config "${{OPENCODE_HOME}}/opencode.json"{removes}; then')
    w('        ok "OpenCode MCP servers merged"')
    w("      else")
    w('        fail "OpenCode MCP merge failed"')
    w("      fi")
    w("    fi")
    w("    # OpenCode's global rules file; it reads ~/.claude/CLAUDE.md only without one.")
    w('    if [ "${OPENCODE_READY}" = 1 ]; then')
    w('      _agents_pointer "${OPENCODE_HOME}/AGENTS.md" shared')
    w('      attribution_rule "${OPENCODE_HOME}/AGENTS.md"')
    w("    fi")
    w("    converge_mcp")
    w("    # Verify the VALUE, not the exit codes: ask Claude what it")
    w("    # actually has, and check every expected server is present.")
    w('    if [ "${CHECK_ONLY}" != 1 ]; then')
    w("     # Once per profile: each one answers for its own .claude.json.")
    w('     for profile_dir in "${CLAUDE_PROFILE_DIRS[@]}"; do')
    w('      registered=$(claude_in_profile "${profile_dir}" claude mcp list 2>/dev/null || true)')
    w("      # Check all expected servers are registered")
    expected = [
        server
        for server in _entries(data, "mcp_servers")
        if (
            on_surface(server, "workstation")
            and server.get("enabled") is not False
            and len(server.get("surfaces") or []) > 0
            and not session_only(server)
        )
    ]
    w("      expected_servers=(" + " ".join(s["name"] for s in expected if on_cloud(s)) + ")")
    off_cloud = [s["name"] for s in expected if not on_cloud(s)]
    if off_cloud:
        w('      if [ "${CLOUD}" != 1 ]; then')
        w("        expected_servers+=(" + " ".join(off_cloud) + ")")
        w("      fi")
    w('      for want in "${expected_servers[@]}"; do')
    w('        case " ${skipped_mcp_names[*]-} " in *" ${want} "*) continue ;; esac')
    w('        case "${registered}" in')
    w('          *"${want}"*) ;;')
    w('          *) warn "MCP server ${want} is not registered in ${profile_dir}" ;;')
    w("        esac")
    w("      done")
    w("     done")
    w("    fi")
    w("  else")
    w('    log "MCP registration skipped (--no-mcp)"')
    w("  fi")
    w("fi")
    w("")

    _gateway_step(w, gateway_declared)
    _layout_step(w, layout_declared)
    _attribution_rule_step(w)
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
        if skill.get("pending"):
            # Registered before the issue that writes it has landed.
            w(f'  log "{name}: not built yet ({skill["pending"]}); skipped"')
        else:
            w(f'  fail "{name}: ${{src}}/SKILL.md is missing; nothing to install"')
        w("else")
        homes = [
            h for agent in skill.get("agents") or ["claude"]
            for h in SKILL_AGENT_HOMES[agent]
        ]
        for home in homes:
            w(f'  dest="${{{home}}}/skills/{name}"')
            if home in ("ANTIGRAVITY_HOME", "ANTIGRAVITY_CLI_HOME"):
                # Antigravity is a macOS-only surface: a cloud run never builds
                # it, and neither does a Linux one, which skips it entirely.
                w('  if [ "${CLOUD}" = 1 ] || [ "${ANTIGRAVITY_READY}" != 1 ]; then')
                w("    :")
                w('  elif [ "${CHECK_ONLY}" = 1 ]; then')
            elif home == "OPENCODE_HOME":
                # OpenCode is workstation-only: a cloud run never builds its home.
                w('  if [ "${OPENCODE_READY}" != 1 ]; then')
                w("    :")
                w('  elif [ "${CHECK_ONLY}" = 1 ]; then')
            elif home == "VIBE_PRIVATE_HOME":
                # vibe-private's home is workstation-only: a cloud run never builds it.
                w('  if [ "${CLOUD}" = 1 ] || [ "${VIBE_HOMES_READY}" != 1 ]; then')
                w("    :")
                w('  elif [ "${CHECK_ONLY}" = 1 ]; then')
            elif home == "VIBE_HOME":
                # A failed split leaves both Vibe homes alone this run.
                w('  if [ "${VIBE_HOMES_READY}" != 1 ]; then')
                w("    :")
                w('  elif [ "${CHECK_ONLY}" = 1 ]; then')
            else:
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

    # --- retired first-party skills ---
    retired_skills = _entries(data, "retired_skills")
    if retired_skills:
        w("# Retired first-party skills (registry retired_skills:): a renamed skill's")
        w("# old directory is removed from every agent home, not left beside the new one.")
        w('log "retired skills"')
        for retired_skill in retired_skills:
            rname = retired_skill["name"]
            for home in (h for homes in SKILL_AGENT_HOMES.values() for h in homes):
                w(f'if [ -d "${{{home}}}/skills/{rname}" ]; then')
                w(f'  dest="${{{home}}}/skills/{rname}"')
                guard = {
                    "ANTIGRAVITY_HOME": '[ "${CLOUD}" = 1 ] || [ "${ANTIGRAVITY_READY}" != 1 ]',
                    "ANTIGRAVITY_CLI_HOME": '[ "${CLOUD}" = 1 ] || [ "${ANTIGRAVITY_READY}" != 1 ]',
                    "VIBE_PRIVATE_HOME": '[ "${CLOUD}" = 1 ] || [ "${VIBE_HOMES_READY}" != 1 ]',
                    "VIBE_HOME": '[ "${VIBE_HOMES_READY}" != 1 ]',
                }.get(home)
                if guard:
                    w(f"  if {guard}; then")
                    w("    :")
                    w('  elif [ "${CHECK_ONLY}" = 1 ]; then')
                else:
                    w('  if [ "${CHECK_ONLY}" = 1 ]; then')
                w('    log "would remove retired skill ${dest}"')
                w("  else")
                w('    rm -rf "${dest}"')
                w(f'    ok "removed retired skill {rname} from ${{dest}}"')
                w("  fi")
                w("fi")
        w("")

    # --- the critique pipeline's Vibe critic agents ---
    if _entries(data, "vibe_critic_agents"):
        w("# The read-only Vibe critic agents, into the shared Vibe home only. Like")
        w("# its skills, a failed split leaves the home alone this run.")
        w('if [ "${VIBE_HOMES_READY}" = 1 ]; then')
        w('  log "Vibe critic agents"')
        w('  _vibe_critic_agents "${VIBE_HOME}"')
        w("fi")
        w("")

    # --- the model gateway's Vibe delegate agents ---
    if _entries(data, "vibe_delegate_agents"):
        w("# The read-only Vibe delegate agents, into the shared Vibe home only.")
        w('if [ "${VIBE_HOMES_READY}" = 1 ]; then')
        w('  log "Vibe delegate agents"')
        w('  _vibe_delegate_agents "${VIBE_HOME}"')
        w("fi")
        w("")

    # --- vendored third-party skills (skill_sources on the workstation surface) ---
    w("# -----------------------------------------------------------------")
    w("# 8. Vendored third-party skills.")
    w("#")
    w("# Mirrors Hermes' sync-skills init container: clone the pinned ref,")
    w("# verify the commit sha (a mismatch is a supply-chain event, so it")
    w('# fails loudly rather than installing something else), then install')
    w("# every SKILL.md the source publishes into the Claude, Codex and")
    w("# OpenCode skills folders.")
    w("# -----------------------------------------------------------------")
    vendor_sources = [s for s in _entries(data, "skill_sources") if on_surface(s, "workstation")]
    if vendor_sources:
        w('log "vendored skills"')
    for source in vendor_sources:
        vname = source["name"]
        repo = source["repo"]
        ref = source["ref"]
        commit = source["commit"]
        w("")
        for chunk in _wrap(str(source.get("note") or "").strip(), 64):
            w(f"# {chunk}")
        w(f'# {vname}: {repo}@{ref}')
        w('if [ "${CHECK_ONLY}" = 1 ]; then')
        w(f'  log "would clone {repo}@{ref} and install its skills ({vname})"')
        w("else")
        w("  if ! command -v git >/dev/null 2>&1; then")
        w(f'    fail "{vname}: git is not on PATH; cannot vendor its skills"')
        w("  else")
        w('    vendor_stage="$(mktemp -d)"')
        w(f'    if git clone --quiet --depth 1 --branch {_q(ref)} {_q(repo)} "${{vendor_stage}}" >/dev/null 2>&1 \\')
        w(f'       && [ "$(git -C "${{vendor_stage}}" rev-parse HEAD 2>/dev/null)" = {_q(commit)} ]; then')
        w('      vendor_skills_root="${vendor_stage}"')
        w('      [ -d "${vendor_stage}/skills" ] && vendor_skills_root="${vendor_stage}/skills"')
        w('      vendor_count=0')
        w('      while IFS= read -r vendor_skill_md; do')
        w('        vendor_dir="$(dirname "${vendor_skill_md}")"')
        w('        vendor_name="$(basename "${vendor_dir}")"')
        w('        vendor_homes=("${CLAUDE_HOME}" "${CODEX_HOME}")')
        w('        [ "${OPENCODE_READY}" = 1 ] && vendor_homes+=("${OPENCODE_HOME}")')
        w('        for vendor_home in "${vendor_homes[@]}"; do')
        w('          vendor_dest="${vendor_home}/skills/${vendor_name}"')
        w('          rm -rf "${vendor_dest}"')
        w('          mkdir -p "$(dirname "${vendor_dest}")"')
        w('          cp -R "${vendor_dir}" "${vendor_dest}"')
        w("        done")
        w("        vendor_count=$((vendor_count + 1))")
        w('      done < <(find "${vendor_skills_root}" -name SKILL.md)')
        w('      if [ "${vendor_count}" -gt 0 ]; then')
        w(f'        ok "{vname}: installed ${{vendor_count}} skill(s) from {repo}@{ref}"')
        w("      else")
        w(f'        fail "{vname}: cloned {repo}@{ref} but found no SKILL.md under it"')
        w("      fi")
        w("    else")
        w(
            f'      fail "{vname}: could not clone {repo}@{ref} at commit {commit} '
            '(supply-chain mismatch or network failure)"',
        )
        w("    fi")
        w('    rm -rf "${vendor_stage}"')
        w("  fi")
        w("fi")
    w("")

    _opencode_plugin_skills_step(data, w)

    # --- retired hooks ---
    w("# -----------------------------------------------------------------")
    w("# 9. Agent hooks. Purge the retired ones (see purge_retired_hooks above),")
    w("# not just report them: a machine set up by the retired install.sh /")
    w("# install-agents.sh is exactly the one that still has these. Then merge")
    w("# in the critique hooks, after step 7 copied the scripts they run.")
    w("# -----------------------------------------------------------------")
    w("purge_retired_hooks")
    w("purge_leaked_runner_git_config")
    w("git_no_agent_attribution")
    w("purge_leaked_runner_workspace_trust /workspace")
    w('log "critique hooks"')
    w("critique_hooks install")
    w("")
    w('if [ "${CLOUD}" = 1 ]; then')
    w("  cloud_finish")
    w("fi")
    w("")
    w("# -----------------------------------------------------------------")
    w("# 10. Summary: what changed and what still needs attention.")
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
    w("# Report shared surfaces a secondary profile did not get")
    w('if [ "${#unshared_profile_paths[@]}" -gt 0 ]; then')
    w('  log "Profile surfaces not shared:"')
    w('  for entry in "${unshared_profile_paths[@]}"; do')
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
    w("# Final summary. What needs a hand is in the Still to do list below, once.")
    w("# Every item that needs a hand raised a warning or a failure, so these decide.")
    w('if [ "${failures}" = 0 ] && [ "${warnings}" = 0 ]; then')
    w('  log "Setup complete: everything is ready"')
    w("else")
    w("  log \"Setup summary: ${failures} failure(s), ${warnings} warning(s)\"")
    w("fi")
    w("print_todo")
    w("")
    w('[ "${failures}" = 0 ]')
    return "\n".join(out) + "\n"


def render_container_setup_script(data: dict[str, Any]) -> str:
    base = data.get("container_base") or {}
    tools = container_tools(data)
    bootstrap = [str(p) for p in base.get("apt_bootstrap") or []]
    packages = [str(p) for p in base.get("apt_packages") or []]
    header = [
        "#!/usr/bin/env bash",
        f"# {GENERATED_BANNER}",
        "#",
        "# Installs the agents image's tools at their registry pins. Root, build time, no secrets.",
        "#",
        "# Usage:",
        "#   ./setup-container.sh           install everything, then verify (root)",
        "#   ./setup-container.sh --check   verify only: every tool present at its pin",
    ]
    out: list[str] = [*header, ""]
    w = out.append

    w("set -euo pipefail")
    w("")
    w("CHECK_ONLY=0")
    w('case "${1:-}" in')
    w("  --check) CHECK_ONLY=1 ;;")
    w('  "") ;;')
    w(f"  --help|-h) sed -n '2,{len(header)}p' \"$0\"; exit 0 ;;")
    w('  *) echo "unknown option: $1" >&2; exit 64 ;;')
    w("esac")
    w("")
    w("failures=0")
    w("log()  { printf 'setup-container: %s\\n' \"$*\"; }")
    w("ok()   { printf 'setup-container:   ok    %s\\n' \"$*\"; }")
    w("fail() { printf 'setup-container:   FAIL  %s\\n' \"$*\" >&2; failures=$((failures + 1)); }")
    w("")
    w("export DEBIAN_FRONTEND=noninteractive")
    for key, value in (base.get("environment") or {}).items():
        w(f"export {key}={_q(str(value))}")
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
    w("# Checks the reported version, so a stale binary earlier on PATH fails.")
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

    w('if [ "${CHECK_ONLY}" = 0 ]; then')
    w('  if [ "$(id -u)" != 0 ]; then')
    w('    echo "setup-container: must run as root (image build time); use --check otherwise" >&2')
    w("    exit 77")
    w("  fi")
    w("")
    w('  log "apt repositories"')
    w("  apt-get update")
    if bootstrap:
        w(f"  apt-get install -y --no-install-recommends {' '.join(bootstrap)}")
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
    for path in base.get("readable_paths") or []:
        w(f"  [ ! -e {_q(str(path))} ] || chmod -R a+rX {_q(str(path))}")
    w("  npm cache clean --force >/dev/null 2>&1 || true")
    w("  rm -rf /var/lib/apt/lists/* /root/.cache /root/.npm /tmp/*")
    w("fi")
    w("")
    w('log "verify"')
    for package in [*bootstrap, *packages]:
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


def _mcp_env_flags(
    server: dict[str, Any], credential: str | None, optional_cred: bool, claude: bool = False,
) -> str:
    """Build the ``--env KEY=VALUE`` fragment for a stdio MCP server.

    Shared by the Claude and Codex registration commands. Emits one flag per
    static ``env`` entry, plus the credential when the server declares one.

    An OPTIONAL credential is emitted as ``${VAR:-}``. The script runs under
    ``set -u``, and a bare ``${VAR}`` for an unset variable does not expand to
    an empty string there -- it aborts the run, taking every step after it
    with it. A required credential is already guarded by a ``-z`` test that
    skips the registration, so it is only ever expanded when it is set.

    Claude gets the credential through ``secret_ref``, which is the same value
    on a workstation and a ``${VAR}`` reference in cloud mode.
    """
    env = server.get("env") or {}
    flags = "".join(f' --env {key}="{value}"' for key, value in env.items())
    if credential and claude:
        optional = " optional" if optional_cred else ""
        flags += f' --env {credential}="$(secret_ref {credential}{optional})"'
    elif credential:
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
    """Plugins are pinned through their marketplaces, so drift is checked when there are any."""
    return bool(_entries(data, "marketplaces") and _entries(data, "plugins"))


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
        VIBE_MCP: (render_vibe_mcp(data), 0o644),
        VIBE_PRIVATE_MCP: (render_vibe_private_mcp(data), 0o644),
        ANTIGRAVITY_MCP: (render_antigravity_mcp(data), 0o644),
        OPENCODE_MCP: (render_opencode_mcp(data), 0o644),
        SECRETS_TABLE: (json.dumps(_entries(data, "workstation_secrets"), indent=2) + "\n", 0o644),
        **{
            VIBE_CRITIC_AGENTS_DIR / f"{agent['name']}.toml": (render_vibe_critic_agent(agent), 0o644)
            for agent in vibe_critic_agents(data)
        },
        **{
            VIBE_DELEGATE_AGENTS_DIR / f"{agent['name']}.toml": (render_vibe_delegate_agent(agent), 0o644)
            for agent in vibe_delegate_agents(data)
        },
    }


def _leftover_agent_files(data: dict[str, Any], root: Path) -> list[Path]:
    """Files in the agents' directory that no declared critic or delegate renders."""
    directory = root / VIBE_CRITIC_AGENTS_DIR
    if not directory.is_dir():
        return []
    rendered = set(artifacts(data))
    return sorted(
        path.relative_to(root) for path in directory.iterdir()
        if path.is_file() and path.relative_to(root) not in rendered
    )


def stale_artifacts(data: dict[str, Any], root: Path = KIT_ROOT) -> list[str]:
    """The generated artifacts under ``root`` that differ from a fresh render."""
    stale = []
    for relative, (content, _mode) in artifacts(data).items():
        target = root / relative
        if (target.read_text() if target.is_file() else None) != content:
            stale.append(str(relative))
    # A retired critic's or delegate's file would otherwise linger, and ship.
    stale.extend(str(path) for path in _leftover_agent_files(data, root))
    return stale


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

    if args.check:
        stale = stale_artifacts(data)
        if stale:
            print(
                "registry render check failed; stale artifacts: " + ", ".join(stale),
                file=sys.stderr,
            )
            print("run: uv run python scripts/render_registry.py --write", file=sys.stderr)
            return 1
        print("registry render check passed")
        return 0

    for relative, (content, mode) in artifacts(data).items():
        target = KIT_ROOT / relative
        current = target.read_text() if target.is_file() else None
        target.parent.mkdir(parents=True, exist_ok=True)
        if current != content:
            target.write_text(content)
        target.chmod(mode)
    for leftover in _leftover_agent_files(data, KIT_ROOT):
        (KIT_ROOT / leftover).unlink()

    print(f"rendered {len(artifacts(data))} registry artifacts")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
