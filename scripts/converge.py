#!/usr/bin/env python3
"""Converge agent surfaces to the registry and the keep file.

``mcp``: the MCP servers of each Claude profile's ``.claude.json``, Codex's
``config.toml`` and the ``config.toml`` of each Vibe home (the shared one and
vibe-private's), and the ``mcp.servers`` of OpenCode's ``opencode.json``.
``plugins``: Claude's marketplaces and user-scope plugins, which every
profile shares. The setup script renders
the declared and retired names from the registry. A retired entry is removed;
any other entry the surface's declared set does not name is removed unless the
keep file names it. A ``--private`` entry (Private Memory) is removed from every
surface that does not declare it, whatever the keep file says.

``gold-band``: the ``agents`` map of a Gold Band instance's ``settings.json``,
which REPLACES the app's built-in agent list rather than merging with it. The
map is made to hold exactly the registry's declared agents that are installed
plus the kept ones; a declared agent whose ``binary`` is not on PATH is skipped
and reported, and its existing entry is left alone. Codex ids and the retired
agents go even when the keep file names them. An agent that names a
``skill_set`` also gets one skill directory of its own, holding exactly that
set's skills, and its entry points at it.

On the work instance (``--instance gold-band``) ``--overlay PATH`` also applies the
per-machine overlay's ``instances.gold-band``: its ``settings`` are set at their
dotted paths in settings.json and its ``mcp_servers`` become ``contextServers``
entries. The overlay is validated by the gold-band-registry crate first and
refused whole when it fails; it is read, never copied into the registry.

``gold-band-roles``: the ``context/profiles`` directory of the same instance,
one Markdown profile per role the registry declares, rendered from the critique
skill's own role, prompt, schema and lens files. A profile the registry does not
declare is removed unless the keep file names its stem; a stem or id in
``gold_band.retired_roles`` goes even when kept, and the app's own
``pf-builtin-*`` profiles are never touched.

``--check`` prints each removal and changes nothing. A normal run removes the
entries, reads every surface again and fails if a removed name is still there.

Usage::

    uv run --no-project --exclude-newer "7 days" --with pyyaml --with tomlkit python scripts/converge.py mcp \\
        --keep PATH --claude-home DIR [--claude DIR ...] [--codex PATH] [--vibe PATH] [--vibe-private PATH] \\
        [--opencode PATH] --declared SURFACE=NAME,... --retired NAME,... [--private NAME,...] [--check]
    uv run --no-project --exclude-newer "7 days" --with pyyaml --with tomlkit python scripts/converge.py plugins \\
        --keep PATH --declared-markets NAME,... --declared-plugins ID,... \\
        --retired-markets NAME,... --retired-plugins ID,... [--check]
    uv run --no-project --exclude-newer "7 days" --with pyyaml --with tomlkit python scripts/converge.py gold-band \\
        --keep PATH [--settings PATH] [--instance ID] [--overlay PATH] [--registry PATH] [--kit DIR] \\
        [--skills-root DIR] [--check]
    uv run --no-project --exclude-newer "7 days" --with pyyaml --with tomlkit python scripts/converge.py \\
        gold-band-roles --keep PATH [--profiles DIR] [--registry PATH] [--critique DIR] [--closing PATH] [--check]
"""

from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import subprocess
import sys
import tomllib
from collections.abc import Callable
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

import gold_band_instances
import tomlkit
import yaml

DECLARED_SOURCES = ("plugin:",)

KIT_ROOT = Path(__file__).resolve().parent.parent
REGISTRY_PATH = KIT_ROOT / "registry" / "estate-tooling.yaml"

# The Gold Band instance whose settings.json this kit converges, and the app
# process whose running state makes a write unsafe.
GOLD_BAND_INSTANCE = "gold-band-personal"
GOLD_BAND_WORK_INSTANCE = "gold-band"
GOLD_BAND_INSTANCES = (GOLD_BAND_WORK_INSTANCE, GOLD_BAND_INSTANCE)
# Claude Code reads its profile from this variable, and the adapter forwards
# the environment it is given: setting it in the agent entry is what keeps an
# ambient value from switching the identity.
GOLD_BAND_CLAUDE_CONFIG_ENV = "CLAUDE_CONFIG_DIR"
GOLD_BAND_CLAUDE_ID = re.compile(r"^claude(-[a-z0-9]+)*$")
GOLD_BAND_PROCESS_NAMES = ("gold-band-desktop", "Gold Band")
# Exit code of a refusal because an instance runs; setup stops on it.
GOLD_BAND_RUNNING_EXIT = 4
GOLD_BAND_RUNNING_ENV = "AGENT_KIT_GOLD_BAND_RUNNING"
# Gold Band's settings.json key for MCP servers (SettingsConfig.context_servers)
# and the stdio proxy a token-bearing server is registered through: it reads
# the token from the secret store at launch, so no secret reaches settings.json.
GOLD_BAND_SERVERS_KEY = "contextServers"
GOLD_BAND_MCP_PROXY = "$HOME/.local/bin/antigravity-mcp-proxy"
GOLD_BAND_ID = re.compile(r"^[a-z0-9]+(-[a-z0-9]+)*$")
# An adapter env value is a reference the app resolves at launch, never a
# literal: a secret in settings.json is a secret in a file.
GOLD_BAND_ENV_REF = re.compile(r"^\$(?:[A-Za-z_][A-Za-z0-9_]*|\{[A-Za-z_][A-Za-z0-9_]*\})$")
GOLD_BAND_PROMPT_DELIVERY = ("none", "meta-append")
GOLD_BAND_AGENT_FIELDS = (
    "id",
    "binary",
    "instance",
    "config_dir",
    "skill_set",
    "command_from_binary",
    "os",
    "adapter",
    "icon",
    "primaryAgentDir",
    "projectPrimaryAgentDir",
    "compatibleAgentDirs",
    "systemPromptDelivery",
    "externalSessionSyncSupported",
    "externalSessionSyncEnabled",
)
# An agent that names a skill set gets its own directory as its primary agent
# directory, so it may not also declare one of these.
GOLD_BAND_SKILL_SET_REPLACES = ("primaryAgentDir", "projectPrimaryAgentDir", "compatibleAgentDirs")
# Declared in the registry only; never written to settings.json.
GOLD_BAND_REGISTRY_ONLY = ("id", "binary", "instance", "config_dir", "skill_set", "command_from_binary", "os")
# An agent may declare its launch command per OS: ``os: {linux: {...}, mac: {...}}``,
# each a partial override of the agent's own ``binary``, ``command_from_binary``
# and ``adapter``. An agent with ``os`` and no section for the host is skipped.
GOLD_BAND_HOSTS = ("linux", "mac")
GOLD_BAND_OS_FIELDS = ("binary", "command_from_binary", "adapter")
# Where macOS keeps what an agent launches; none of it may be written on Linux.
GOLD_BAND_MAC_PATH = re.compile(
    r"/Applications\b|/opt/homebrew|/usr/local/Cellar|/Library/|/Users/|homebrew|\.app/", re.I
)
GOLD_BAND_ADAPTER_FIELDS = ("displayName", "command", "args", "env")
# A generated profile's front matter carries a fixed stamp, so a second run
# rewrites nothing. The app's own profiles carry this id prefix and are never
# touched, whatever the registry or the keep file says.
GOLD_BAND_ROLE_STAMP = "2025-01-01T00:00:00Z"
GOLD_BAND_ROLE_BUILT_IN = "pf-builtin-"
GOLD_BAND_ROLE_PLAIN = re.compile(r"^[A-Za-z0-9$(][^\n#]*$")
GOLD_BAND_ROLE_FIELDS = ("file", "id", "name", "summary", "source", "lenses")
GOLD_BAND_ROLES_CLOSING = KIT_ROOT / "scripts" / "gold-band" / "roles-closing.md"
# The app strips adapter.command and adapter.args from a built-in id when it
# saves settings.json and re-injects them on load, so setup writes neither.
GOLD_BAND_BUILT_INS = frozenset({
    "claude-acp",
    "codex-acp",
    "cursor",
    "gemini",
    "codebuddy-code",
    "goose",
    "qwen-code",
    "opencode",
    "kimi",
    "amp-acp",
    "pi-acp",
})


class _CannotConverge(Exception):
    """The surface cannot be converged at all: the input is unreadable or malformed."""


@dataclass
class Surface:
    label: str
    kind: str
    path: Path | None = None
    env: dict[str, str] = field(default_factory=dict)

    def names(self) -> set[str]:
        if self.kind == "marketplace":
            return {str(m["name"]) for m in _claude_json("plugin", "marketplace", "list", "--json")}
        if self.kind == "plugin":
            return {str(p["id"]) for p in _claude_json("plugin", "list", "--json") if p.get("scope") == "user"}
        assert self.path is not None
        if not self.path.is_file():
            return set()
        if self.kind == "claude":
            return set(json.loads(self.path.read_text()).get("mcpServers") or {})
        if self.kind == "opencode":
            return set(_opencode_servers(json.loads(self.path.read_text())))
        data = tomllib.loads(self.path.read_text())
        if self.kind == "codex":
            return set(data.get("mcp_servers") or {})
        return {str(s.get("name")) for s in data.get("mcp_servers") or []}

    def remove(self, name: str) -> None:
        commands = {
            "claude": ["claude", "mcp", "remove", "--scope", "user", name],
            "codex": ["codex", "mcp", "remove", name],
            "plugin": ["claude", "plugin", "uninstall", name, "--scope", "user"],
            "marketplace": ["claude", "plugin", "marketplace", "remove", name],
        }
        if self.kind in commands:
            subprocess.run(commands[self.kind], env=self.env or None, capture_output=True, check=False)
            return
        assert self.path is not None
        if self.kind == "opencode":
            config = json.loads(self.path.read_text())
            _opencode_servers(config).pop(name, None)
            staged = self.path.with_name(self.path.name + ".agent-kit-tmp")
            staged.write_text(json.dumps(config, indent=2) + "\n")
            os.replace(staged, self.path)
            return
        doc = tomlkit.parse(self.path.read_text())
        kept = tomlkit.aot()
        for server in doc.get("mcp_servers") or []:
            if str(server.get("name")) != name:
                kept.append(server)
        del doc["mcp_servers"]
        if len(kept):
            doc["mcp_servers"] = kept
        self.path.write_text(tomlkit.dumps(doc))


def _opencode_servers(config: dict[str, Any]) -> dict[str, Any]:
    """OpenCode 2's ``mcp.servers`` table, the live object, or an empty one."""
    mcp = config.get("mcp")
    servers = mcp.get("servers") if isinstance(mcp, dict) else None
    return servers if isinstance(servers, dict) else {}


def _plain_json(path: Path) -> bool:
    """True when ``path`` is missing or parses as JSON."""
    if not path.is_file():
        return True
    try:
        json.loads(path.read_text())
    except json.JSONDecodeError:
        return False
    return True


def _claude_json(*args: str) -> list[dict[str, Any]]:
    out = subprocess.run(["claude", *args], capture_output=True, text=True, check=True).stdout
    entries: list[dict[str, Any]] = json.loads(out)
    return entries


def _tilde(path: Path) -> str:
    home = Path.home()
    return f"~/{path.relative_to(home)}" if path.is_relative_to(home) else str(path)


def _claude(profile: Path, claude_home: Path) -> Surface:
    env = dict(os.environ)
    # The primary profile without CLAUDE_CONFIG_DIR keeps .claude.json in HOME.
    if profile == claude_home and not env.get("CLAUDE_CONFIG_DIR"):
        env.pop("CLAUDE_CONFIG_DIR", None)
        path = Path.home() / ".claude.json"
    else:
        env["CLAUDE_CONFIG_DIR"] = str(profile)
        path = profile / ".claude.json"
    return Surface(f"claude {_tilde(profile)}", "claude", path, env)


def _keep(path: Path) -> set[str]:
    if not path.is_file():
        return set()
    data = yaml.safe_load(path.read_text()) or {}
    keep = data.get("keep") if isinstance(data, dict) else None
    if keep is None:
        return set()
    if not isinstance(keep, list) or not all(isinstance(n, str) for n in keep):
        raise SystemExit(f"converge: {path} must hold a top-level `keep:` list of names; nothing removed")
    return set(keep)


def _names(value: str) -> set[str]:
    return {n for n in value.split(",") if n}


def converge(surfaces: list[Surface], retired: Callable[[Surface, str], bool],
             spared: Callable[[Surface, str], bool], check: bool,
             private: Callable[[Surface, str], bool] = lambda _s, _n: False) -> int:
    """Remove each retired or misplaced private entry, and each other one not spared; then read every surface back."""
    removals: list[tuple[Surface, str, str]] = []
    for surface in surfaces:
        for name in sorted(surface.names()):
            if private(surface, name):
                removals.append((surface, name, "private"))
            elif retired(surface, name):
                removals.append((surface, name, "retired"))
            elif not spared(surface, name):
                removals.append((surface, name, "undeclared"))

    for surface, name, reason in removals:
        if check:
            print(f"would remove {name} ({surface.label}, {reason})")
        else:
            surface.remove(name)
            print(f"removed {name} ({surface.label}, {reason})")

    lingering = [(s, n) for s, n, _ in removals if not check and n in s.names()]
    for surface, name in lingering:
        print(f"{name} is still present ({surface.label})")
    return 1 if lingering else 0


def mcp(args: argparse.Namespace) -> int:
    keep = _keep(args.keep)
    retired = _names(args.retired)
    declared = {kind: _names(names) for kind, _, names in (d.partition("=") for d in args.declared)}
    private = _names(args.private)
    surfaces = [_claude(p, args.claude_home) for p in args.claude]
    if args.codex:
        surfaces.append(Surface("codex", "codex", args.codex))
    if args.vibe:
        surfaces.append(Surface("vibe", "vibe", args.vibe))
    if args.vibe_private:
        surfaces.append(Surface("vibe-private", "vibe-private", args.vibe_private))
    unreadable = 0
    if args.opencode:
        if _plain_json(args.opencode):
            surfaces.append(Surface("opencode", "opencode", args.opencode))
        else:
            # OpenCode accepts comments; rewriting would drop them, and one bad file
            # must not stop every other surface from converging.
            print(f"{_tilde(args.opencode)} is not plain JSON; opencode not converged")
            unreadable = 1
    return converge(
        surfaces,
        lambda _, name: name in retired,
        lambda s, name: name in declared.get(s.kind, set()) or name.startswith(DECLARED_SOURCES) or name in keep,
        args.check,
        lambda s, name: name in private and name not in declared.get(s.kind, set()),
    ) or unreadable


def plugins(args: argparse.Namespace) -> int:
    keep = _keep(args.keep)
    retired = _names(args.retired_plugins) | _names(args.retired_markets)
    declared = _names(args.declared_plugins) | _names(args.declared_markets)

    def spared(_: Surface, name: str) -> bool:
        plugin, _sep, market = name.partition("@")
        return name in declared or bool({name, plugin, market} & keep)

    # Plugins first: removing a marketplace uninstalls its plugins without saying which.
    surfaces = [Surface("claude plugin", "plugin"), Surface("claude marketplace", "marketplace")]
    return converge(surfaces, lambda _, name: name in retired, spared, args.check)


def _gold_band_data_dir(gold_band_home: str | None, fallback_home: str | None = None) -> Path:
    """The app's data dir: `.gold-band` under `GOLD_BAND_HOME` (blank counts as unset), else under the home."""
    if gold_band_home and gold_band_home.strip():
        return Path(gold_band_home) / ".gold-band"
    return Path(fallback_home or Path.home()) / ".gold-band"


def _gold_band_settings_path() -> Path:
    """``${GOLD_BAND_HOME:-~}/.gold-band/settings.json``: the app keeps its data in ``.gold-band`` under that home."""
    return _gold_band_data_dir(os.environ.get("GOLD_BAND_HOME")) / "settings.json"


def _gold_band_registry(
    path: Path, instance: str = GOLD_BAND_INSTANCE
) -> tuple[list[dict[str, Any]], set[str], set[str]]:
    """The declared agents, the retired ids and the ids ``instance`` may hold."""
    try:
        data = yaml.safe_load(path.read_text())
    except (OSError, UnicodeDecodeError, yaml.YAMLError) as exc:
        raise _CannotConverge(f"{path} cannot be read: {exc}") from exc
    if not isinstance(data, dict):
        raise _CannotConverge(f"{path} is not a mapping")
    section = data.get("gold_band")
    if not isinstance(section, dict):
        raise _CannotConverge(f"{path} has no gold_band mapping")
    agents = section.get("agents")
    if not isinstance(agents, list) or not all(isinstance(agent, dict) for agent in agents):
        raise _CannotConverge(f"{path}: gold_band.agents must be a list of mappings")
    retired = section.get("retired_agents")
    if retired is None:
        retired = []
    if not isinstance(retired, list) or not all(isinstance(name, str) for name in retired):
        raise _CannotConverge(f"{path}: gold_band.retired_agents must be a list of ids")
    instances = data.get("gold_band_instances")
    held = instances.get(instance) if isinstance(instances, dict) else None
    allowed = held.get("agents") if isinstance(held, dict) else None
    if not isinstance(allowed, list) or not all(isinstance(name, str) for name in allowed):
        raise _CannotConverge(f"{path}: gold_band_instances.{instance}.agents must be a list of ids")
    return agents, set(retired), set(allowed)


def _gold_band_adapter_problems(name: str, adapter: Any, codex_ok: bool = False) -> list[str]:
    """Problems with an adapter; Codex in its launch is refused unless ``codex_ok`` (the work instance's agent)."""
    if adapter is None:
        return []
    if not isinstance(adapter, dict):
        return [f"{name}: adapter must be a mapping"]
    problems = [f"{name}: unknown adapter field {key}" for key in adapter if key not in GOLD_BAND_ADAPTER_FIELDS]
    command = adapter.get("command")
    if command is not None and not isinstance(command, str):
        problems.append(f"{name}: adapter.command must be a string")
    args = adapter.get("args")
    if args is not None and (not isinstance(args, list) or not all(isinstance(arg, str) for arg in args)):
        problems.append(f"{name}: adapter.args must be a list of strings")
    env = adapter.get("env")
    if env is not None:
        if not isinstance(env, dict) or not all(isinstance(value, str) for value in env.values()):
            problems.append(f"{name}: adapter.env must be a mapping of string to string")
        else:
            problems += [
                f"{name}: adapter.env {key} must be an environment reference, not a literal"
                for key, value in env.items()
                if not GOLD_BAND_ENV_REF.match(value)
            ]
    words = [command] if isinstance(command, str) else []
    words += [arg for arg in args if isinstance(arg, str)] if isinstance(args, list) else []
    if not codex_ok and any("codex" in word.lower() for word in words):
        problems.append(f"{name}: Codex is refused in adapter.command or adapter.args")
    return problems


def _gold_band_instance_of(agent: dict[str, Any]) -> Any:
    """The instance an agent is declared for: the personal one unless ``instance`` says otherwise."""
    return agent.get("instance", GOLD_BAND_INSTANCE)


def _gold_band_everyone_problems(declared: list[dict[str, Any]]) -> list[str]:
    """Problems that span the instances: each agent's instance, and each Claude
    agent's own configuration directory, which no two agents share.

    ``declared`` is every agent of the registry, whatever its instance, so a
    directory shared by the work and the personal agent is caught here.
    """
    problems: list[str] = []
    owners: dict[str, str] = {}
    for agent in declared:
        raw = agent.get("id")
        if not isinstance(raw, str):
            continue
        if _gold_band_instance_of(agent) not in GOLD_BAND_INSTANCES:
            problems.append(f"{raw}: instance must be one of {', '.join(GOLD_BAND_INSTANCES)}")
        directory = agent.get("config_dir")
        if not GOLD_BAND_CLAUDE_ID.match(raw):
            if directory is not None:
                problems.append(f"{raw}: config_dir is only for a Claude agent")
            continue
        adapter = agent.get("adapter")
        env = adapter.get("env") if isinstance(adapter, dict) else None
        if isinstance(env, dict) and GOLD_BAND_CLAUDE_CONFIG_ENV in env:
            problems.append(f"{raw}: adapter.env {GOLD_BAND_CLAUDE_CONFIG_ENV} is set by config_dir, not by hand")
        if directory is None:
            problems.append(f"{raw}: a Claude agent must declare config_dir")
        elif not isinstance(directory, str) or not directory:
            problems.append(f"{raw}: config_dir must be a non-empty string")
        elif not (directory.startswith("/") or directory.startswith("~/")) or ".." in Path(directory).parts:
            problems.append(f"{raw}: config_dir must be an absolute path or start with ~/, without ..")
        else:
            key = os.path.normpath(_gold_band_expand(directory))
            if key in owners:
                problems.append(f"{raw}: config_dir {directory} is already {owners[key]}'s")
            owners.setdefault(key, raw)
    return problems


def _gold_band_host() -> str:
    """``mac`` on macOS, ``linux`` anywhere else; ``AGENT_KIT_GOLD_BAND_HOST`` overrides for tests."""
    forced = os.environ.get("AGENT_KIT_GOLD_BAND_HOST")
    if forced in GOLD_BAND_HOSTS:
        return forced
    return "mac" if sys.platform == "darwin" else "linux"


def _gold_band_for_host(agent: dict[str, Any], host: str) -> dict[str, Any] | None:
    """The agent as it is launched on ``host``, or None when it declares no launch for that OS.

    An agent without ``os`` is the same everywhere. With ``os``, the host's
    section overrides ``binary`` and ``command_from_binary`` and is merged into
    ``adapter`` (its keys win); a host with no section has no launch command.
    """
    resolved = {key: value for key, value in agent.items() if key != "os"}
    sections = agent.get("os")
    if not isinstance(sections, dict):
        return resolved
    section = sections.get(host)
    if not isinstance(section, dict):
        return None
    for key, value in section.items():
        if key == "adapter" and isinstance(value, dict) and isinstance(resolved.get("adapter"), dict):
            resolved["adapter"] = {**resolved["adapter"], **value}
        else:
            resolved[key] = value
    return resolved


def _gold_band_os_problems(raw: str, agent: dict[str, Any]) -> list[str]:
    """Problems with ``os`` and, for the Linux launch, any macOS path in what it would write."""
    problems: list[str] = []
    sections = agent.get("os")
    if "os" in agent:
        if not isinstance(sections, dict) or not sections:
            return [f"{raw}: os must map linux and/or mac to a launch declaration"]
        for host, section in sections.items():
            if host not in GOLD_BAND_HOSTS:
                problems.append(f"{raw}: os.{host} is not one of {', '.join(GOLD_BAND_HOSTS)}")
            elif not isinstance(section, dict):
                problems.append(f"{raw}: os.{host} must be a mapping")
            else:
                problems += [
                    f"{raw}: unknown field os.{host}.{key}" for key in section if key not in GOLD_BAND_OS_FIELDS
                ]
                label = f"{raw} os.{host}"
                if not section:
                    problems.append(f"{label}: must declare binary, command_from_binary or adapter")
                if section.get("command_from_binary") is True and not (section.get("binary") or agent.get("binary")):
                    problems.append(f"{label}: command_from_binary needs a binary")
                if "binary" in section and (not isinstance(section["binary"], str) or not section["binary"]):
                    problems.append(f"{label}: binary must be a non-empty string")
                if "command_from_binary" in section and section["command_from_binary"] is not True:
                    problems.append(f"{label}: command_from_binary must be true")
                problems += _gold_band_adapter_problems(
                    label, section.get("adapter"), _gold_band_instance_of(agent) == GOLD_BAND_WORK_INSTANCE
                )
    linux = _gold_band_for_host(agent, "linux") if not problems else None
    if linux is not None:
        raw_adapter = linux.get("adapter")
        adapter: dict[str, Any] = raw_adapter if isinstance(raw_adapter, dict) else {}
        raw_args = adapter.get("args")
        args: list[Any] = raw_args if isinstance(raw_args, list) else []
        written = [linux.get("binary"), adapter.get("command"), *args]
        if any(isinstance(item, str) and GOLD_BAND_MAC_PATH.search(item) for item in written):
            problems.append(f"{raw}: a macOS path (Homebrew or /Applications) is written on Linux")
    return problems


def _gold_band_problems(
    declared: list[dict[str, Any]],
    allowed: set[str],
    retired: frozenset[str] | set[str] = frozenset(),
    instance: str = GOLD_BAND_INSTANCE,
) -> list[str]:
    """One line per problem with the declared agents; empty when they may be written."""
    problems: list[str] = []
    seen: set[str] = set()
    for agent in declared:
        raw = agent.get("id")
        name = raw if isinstance(raw, str) else "?"
        if not isinstance(raw, str) or not GOLD_BAND_ID.match(raw):
            problems.append(f"{name}: id must match {GOLD_BAND_ID.pattern}")
            continue
        if raw in seen:
            problems.append(f"{raw}: duplicate id")
            continue
        seen.add(raw)
        if instance == GOLD_BAND_INSTANCE and "codex" in raw.lower():
            problems.append(f"{raw}: Codex is refused on the personal instance")
        if raw in retired:
            problems.append(f"{raw}: declared and retired at once")
        if raw not in allowed:
            problems.append(f"{raw}: not in gold_band_instances.{instance}.agents")
        problems += [f"{raw}: unknown field {key}" for key in agent if key not in GOLD_BAND_AGENT_FIELDS]
        if "command_from_binary" in agent and (agent["command_from_binary"] is not True or not agent.get("binary")):
            problems.append(f"{raw}: command_from_binary must be true and needs a binary")
        problems += _gold_band_os_problems(raw, agent)
        problems += _gold_band_adapter_problems(
            raw, agent.get("adapter"), _gold_band_instance_of(agent) == GOLD_BAND_WORK_INSTANCE
        )
        delivery = agent.get("systemPromptDelivery")
        if delivery is not None and delivery not in GOLD_BAND_PROMPT_DELIVERY:
            problems.append(f"{raw}: systemPromptDelivery must be one of {', '.join(GOLD_BAND_PROMPT_DELIVERY)}")
    return problems


def _gold_band_skill_sets(path: Path) -> tuple[Any, dict[str, str]]:
    """The raw ``gold_band.skill_sets`` value and each local skill's source path.

    ``_gold_band_registry`` has already refused a registry that is not a mapping
    with a ``gold_band`` section, so only ``local_skills`` is checked here. The
    sets come back unvalidated: a malformed one is a problem line, so the owner
    sees every problem at once instead of one per run.
    """
    data = yaml.safe_load(path.read_text())
    local = data.get("local_skills")
    if local is None:
        local = []
    if not isinstance(local, list) or not all(isinstance(skill, dict) for skill in local):
        raise _CannotConverge(f"{path}: local_skills must be a list of mappings")
    sources = {
        str(skill["name"]): str(skill.get("path") or "") for skill in local if isinstance(skill.get("name"), str)
    }
    return data["gold_band"].get("skill_sets"), sources


def _gold_band_skill_set_problems(
    declared: list[dict[str, Any]], raw_sets: Any, sources: dict[str, str], kit: Path
) -> tuple[list[str], dict[str, list[str]]]:
    """One line per problem with the declared skill sets, and the sets themselves.

    The sets come back even beside a problem, so the caller can report every
    problem before it refuses to write anything.
    """
    problems: list[str] = []
    if raw_sets is None:
        raw_sets = {}
    if not isinstance(raw_sets, dict):
        return ["skill_sets must be a mapping of set name to skills"], {}
    sets: dict[str, list[str]] = {}
    for raw_name, raw_skills in raw_sets.items():
        name = str(raw_name)
        skills = [skill for skill in raw_skills if isinstance(skill, str)] if isinstance(raw_skills, list) else []
        sets[name] = skills
        if not GOLD_BAND_ID.match(name):
            problems.append(f"{name}: skill set name must match {GOLD_BAND_ID.pattern}")
        if not isinstance(raw_skills, list) or len(skills) != len(raw_skills):
            problems.append(f"{name}: skill set must be a list of skill names")
            continue
        seen: set[str] = set()
        for skill in skills:
            if skill in seen:
                problems.append(f"{name}: duplicate skill {skill}")
                continue
            seen.add(skill)
            if skill not in sources:
                problems.append(f"{name}: {skill} is not in local_skills")
            elif not (kit / sources[skill] / "SKILL.md").is_file():
                problems.append(f"{name}: {skill} has no SKILL.md at {sources[skill]}")
    for agent in declared:
        raw_id = agent.get("id")
        if not isinstance(raw_id, str) or not GOLD_BAND_ID.match(raw_id):
            continue
        chosen = agent.get("skill_set")
        if chosen is None:
            continue
        if not isinstance(chosen, str):
            problems.append(f"{raw_id}: skill_set must be a set name")
            continue
        if chosen not in sets:
            problems.append(f"{raw_id}: skill_set {chosen} is not a declared set")
            continue
        replaced = [key for key in GOLD_BAND_SKILL_SET_REPLACES if key in agent]
        if replaced:
            problems.append(f"{raw_id}: skill_set replaces {', '.join(replaced)}")
    return problems, sets


def _gold_band_expand(value: str) -> str:
    """A leading ``~/`` becomes the home directory; the app expands nothing itself."""
    return str(Path.home()) + value[1:] if value.startswith("~/") else value


def _gold_band_entry(
    agent: dict[str, Any], skills_dir: Path | None = None, binary_path: str | None = None
) -> dict[str, Any]:
    """The settings entry for a declared agent: its written fields, expanded.

    ``skill_set``, ``instance`` and ``config_dir`` are registry-only, so they are
    never written; ``config_dir`` becomes the adapter's ``CLAUDE_CONFIG_DIR``,
    set in the entry so no ambient value can replace it. An agent that names
    one gets its own skill directory as its primary agent directory and no
    compatible ones, since nothing is shared between ids. ``command_from_binary``
    (registry-only) writes the adapter command as the absolute path of the
    installed ``binary`` (``binary_path``), so Gold Band launches that file and
    never fetches the agent at launch.
    """
    entry = {
        key: value
        for key, value in agent.items()
        if key in GOLD_BAND_AGENT_FIELDS and key not in GOLD_BAND_REGISTRY_ONLY
    }
    adapter = entry.get("adapter")
    if agent.get("config_dir") is not None:
        adapter = dict(adapter) if isinstance(adapter, dict) else {}
        adapter["env"] = {**(adapter.get("env") or {}), GOLD_BAND_CLAUDE_CONFIG_ENV: agent["config_dir"]}
    if isinstance(adapter, dict):
        adapter = dict(adapter)
        if agent["id"] in GOLD_BAND_BUILT_INS:
            adapter.pop("command", None)
            adapter.pop("args", None)
        if agent.get("command_from_binary") and binary_path is not None:
            adapter["command"] = binary_path
        if "command" in adapter:
            adapter["command"] = _gold_band_expand(adapter["command"])
        if "args" in adapter:
            adapter["args"] = [_gold_band_expand(arg) for arg in adapter["args"]]
        if "env" in adapter:
            adapter["env"] = {key: _gold_band_expand(value) for key, value in adapter["env"].items()}
        entry["adapter"] = adapter
    if skills_dir is not None:
        entry["primaryAgentDir"] = str(skills_dir)
        entry["compatibleAgentDirs"] = []
    return entry


def _gold_band_server_problems(server: dict[str, Any]) -> list[str]:
    name = server.get("name")
    label = name if isinstance(name, str) else "?"
    if not isinstance(name, str) or not GOLD_BAND_ID.match(name):
        return [f"mcp server {label}: name must match {GOLD_BAND_ID.pattern}"]
    if server.get("transport") == "http":
        if not isinstance(server.get("url_workstation"), str) or not server["url_workstation"]:
            return [f"mcp server {name}: an http server needs a url_workstation"]
    elif server.get("transport") == "stdio":
        if not isinstance(server.get("command"), str) or not server["command"]:
            return [f"mcp server {name}: a stdio server needs a command"]
    else:
        return [f"mcp server {name}: transport must be http or stdio"]
    return []


def _gold_band_server_entry(server: dict[str, Any]) -> dict[str, Any]:
    """The ``contextServers`` entry for one registry server; never holds a secret.

    A server with a credential is the stdio proxy, which reads the token from
    the secret store at launch. The app expands no variable itself, so the
    proxy is started by ``/bin/sh`` (a launcher's PATH holds no ``~/.local/bin``).
    """
    entry: dict[str, Any] = {"id": server["name"], "name": server["name"], "enabled": True}
    if server["transport"] == "http":
        url = server["url_workstation"]
        credential = server.get("credential")
        if credential:
            command = f'exec "{GOLD_BAND_MCP_PROXY}" --url \'{url}\' --secret {credential}'
            entry.update(transport="stdio", command="/bin/sh", args=["-c", command], env={})
        else:
            entry.update(transport="http", url=url, headers={})
    else:
        entry.update(
            transport="stdio",
            command=server["command"],
            args=[str(arg) for arg in server.get("args") or []],
            env={str(key): str(value) for key, value in (server.get("env") or {}).items()},
        )
    return entry


def _gold_band_servers(path: Path, instance: str) -> tuple[dict[str, dict[str, Any]], set[str], list[str]]:
    """The servers ``instance`` registers by id, every id the registry binds to
    any instance, and the problems with the declared ones."""
    try:
        data = yaml.safe_load(path.read_text())
    except (OSError, UnicodeDecodeError, yaml.YAMLError) as exc:
        raise _CannotConverge(f"{path} cannot be read: {exc}") from exc
    servers = data.get("mcp_servers") if isinstance(data, dict) else None
    if servers is None:
        return {}, set(), []
    if not isinstance(servers, list) or not all(isinstance(server, dict) for server in servers):
        raise _CannotConverge(f"{path}: mcp_servers must be a list of mappings")
    declared: dict[str, dict[str, Any]] = {}
    bound: set[str] = set()
    problems: list[str] = []
    for server in servers:
        instances = server.get("instances")
        if instances is None:
            continue
        if not isinstance(instances, list) or not all(isinstance(held, str) for held in instances):
            problems.append(f"mcp server {server.get('name')}: instances must be a list of instance ids")
            continue
        found = _gold_band_server_problems(server)
        problems += found
        if found:
            continue
        bound.add(server["name"])
        if instance in instances:
            declared[server["name"]] = _gold_band_server_entry(server)
    return declared, bound, problems


def _gold_band_current_servers(path: Path, doc: dict[str, Any]) -> list[Any]:
    """The settings document's ``contextServers`` list; absent is empty."""
    current = doc.get(GOLD_BAND_SERVERS_KEY)
    if current is None:
        return []
    if not isinstance(current, list):
        raise _CannotConverge(f"{path}: {GOLD_BAND_SERVERS_KEY} is not a list")
    return current


def _gold_band_merge_servers(
    current: list[Any],
    wanted: dict[str, dict[str, Any]],
    bound: set[str],
    keep: set[str],
    instance: str,
) -> tuple[list[Any], list[str]]:
    """The ``contextServers`` list after converging, and one line per change.

    A declared server is written (an entry that already holds every declared
    field stays as the app saved it). The app's own managed entries stay. A
    server the registry binds to another instance is removed, which is how the
    work instance never holds a personal one, unless the keep file names it.
    The personal instance also drops any other undeclared server, as it does
    an undeclared agent; the work instance leaves the rest to its overlay.
    """
    merged: list[Any] = []
    lines: list[str] = []
    seen: set[str] = set()
    for entry in current:
        name = entry.get("id") if isinstance(entry, dict) else None
        if not isinstance(name, str):
            merged.append(entry)
        elif name in wanted:
            seen.add(name)
            if all(entry.get(key) == value for key, value in wanted[name].items()):
                merged.append(entry)
            else:
                merged.append(wanted[name])
                lines.append(f"changed server {name}")
        elif entry.get("managed") is True or name in keep:
            merged.append(entry)
        elif name in bound or instance == GOLD_BAND_INSTANCE:
            lines.append(f"removed server {name}")
        else:
            merged.append(entry)
    for name, entry in wanted.items():
        if name not in seen:
            merged.append(entry)
            lines.append(f"added server {name}")
    return merged, lines


def _gold_band_document(path: Path) -> tuple[dict[str, Any], dict[str, Any]]:
    """The settings document and its ``agents`` map; a missing file is an empty one."""
    if not path.is_file():
        return {}, {}
    try:
        doc = json.loads(path.read_text())
    except (OSError, json.JSONDecodeError, UnicodeDecodeError) as exc:
        raise _CannotConverge(f"{path} is not a JSON object") from exc
    if not isinstance(doc, dict):
        raise _CannotConverge(f"{path} is not a JSON object")
    agents = doc.get("agents")
    if agents is None:
        return doc, {}
    if not isinstance(agents, dict):
        raise _CannotConverge(f"{path} is not a JSON object")
    return doc, agents


@dataclass(frozen=True)
class _Process:
    """One process of the current user: its pid, executable and, where the OS shows it, GOLD_BAND_HOME."""

    pid: int
    exe: str
    home: str | None


def _processes_proc(root: Path) -> list[_Process]:
    found: list[_Process] = []
    for entry in sorted(root.iterdir()):
        if not entry.name.isdigit():
            continue
        try:
            if entry.stat().st_uid != os.getuid():
                continue
            argv = (entry / "cmdline").read_bytes().split(b"\0")[0].decode(errors="replace")
        except OSError:
            continue
        try:
            env = (entry / "environ").read_bytes().split(b"\0")
        except OSError:
            # Its data directory is hidden, so it counts as holding ours.
            found.append(_Process(int(entry.name), argv, None))
            continue
        values = {e.partition(b"=")[0]: e.partition(b"=")[2].decode(errors="replace") for e in env}
        data = _gold_band_data_dir(values.get(b"GOLD_BAND_HOME"), values.get(b"HOME"))
        found.append(_Process(int(entry.name), argv, str(data)))
    return found


def _processes_ps() -> list[_Process]:
    # macOS: the other macOS user's instance (the work one) is not this user's process.
    out = subprocess.run(
        ["ps", "-U", str(os.getuid()), "-o", "pid=,comm="], capture_output=True, text=True, check=False
    ).stdout
    found: list[_Process] = []
    for line in out.splitlines():
        pid, _, exe = line.strip().partition(" ")
        if pid.isdigit():
            found.append(_Process(int(pid), exe.strip(), None))
    return found


def _gold_band_processes() -> list[_Process]:
    """Every process of the current user, read from /proc where it exists and from ps otherwise."""
    root = Path("/proc")
    try:
        return _processes_proc(root) if (root / "self").exists() else _processes_ps()
    except OSError:
        return []


def _gold_band_running(data_dir: Path, instance: str = GOLD_BAND_INSTANCE) -> str | None:
    """The running instance that holds ``data_dir``, or None.

    An instance is a process whose executable is named like the desktop app,
    matched by name and never by install path, and whose data directory is the
    one being written. Where the OS does not show a process's data directory
    the process counts, since a wrongly held write is worse than a refusal.
    """
    flag = os.environ.get(GOLD_BAND_RUNNING_ENV)
    if flag is not None:
        return f"{instance} (set by {GOLD_BAND_RUNNING_ENV})" if flag == "1" else None
    ours = data_dir.expanduser().resolve()
    for proc in _gold_band_processes():
        if Path(proc.exe).name not in GOLD_BAND_PROCESS_NAMES:
            continue
        if proc.home is not None and Path(proc.home).expanduser().resolve() != ours:
            continue
        return f"{instance} (process {proc.pid}, data {ours})"
    return None


def _gold_band_stopped(tool: str, running: str, written: list[str], pending: list[str]) -> int:
    """Report an instance that started between write groups; a second run finishes the rest."""
    print(f"{tool}: Gold Band instance {running} started during setup; quit it and re-run setup")
    print(f"{tool}: written: {', '.join(written) or 'nothing'}")
    print(f"{tool}: not yet written: {', '.join(pending)}")
    return GOLD_BAND_RUNNING_EXIT


def _gold_band_skill_tree(root: Path) -> dict[str, tuple[str, bytes]]:
    """Every entry under ``root`` as its kind and payload, for an exact comparison.

    A symlink is recorded as its target and never followed: the copy keeps it a
    symlink, so a link out of the kit stays a link.
    """
    tree: dict[str, tuple[str, bytes]] = {}
    for path in sorted(root.rglob("*")):
        relative = path.relative_to(root).as_posix()
        if path.is_symlink():
            tree[relative] = ("link", os.readlink(path).encode())
        elif path.is_dir():
            tree[relative] = ("dir", b"")
        elif path.is_file():
            tree[relative] = ("file", path.read_bytes())
    return tree


def _gold_band_mkdir(path: Path) -> None:
    """``path`` as a directory, 0o700 on the ones this creates."""
    if path.is_dir():
        return
    path.parent.mkdir(parents=True, exist_ok=True)
    path.mkdir(mode=0o700)
    os.chmod(path, 0o700)


def _gold_band_remove(path: Path) -> None:
    """Remove a file, a symlink or a directory tree; a missing path is nothing to do."""
    if path.is_symlink() or path.is_file():
        path.unlink()
    elif path.is_dir():
        shutil.rmtree(path)


def _gold_band_skills(
    root: Path,
    kit: Path,
    wanted: dict[str, list[str]],
    sources: dict[str, str],
    spared: set[str],
    keep: set[str],
    check: bool,
) -> list[str]:
    """Converge one skill directory per agent id; return the lines to print.

    Each directory holds exactly its set's skills, each a verbatim copy of the
    skill directory under the kit. An up-to-date directory is not touched, and a
    directory for an id that is not installed with a set is removed unless the
    id is spared or the keep file names it. Only direct children of ``root`` are
    ever removed.
    """
    lines: list[str] = []
    for name, skills in wanted.items():
        target = root / name
        # A link or a file where the directory belongs is replaced, never followed.
        if target.is_symlink() or (target.exists() and not target.is_dir()):
            if not check:
                _gold_band_remove(target)
            lines.append(f"changed skills {name}")
        held = {child.name for child in target.iterdir()} if target.is_dir() and not target.is_symlink() else set()
        added = [skill for skill in skills if skill not in held]
        changed = [
            skill
            for skill in skills
            if skill in held
            and (
                (target / skill).is_symlink()
                or _gold_band_skill_tree(target / skill) != _gold_band_skill_tree(kit / sources[skill])
            )
        ]
        removed = sorted(held - set(skills))
        if not (added or changed or removed):
            continue
        if not check:
            _gold_band_mkdir(root)
            _gold_band_mkdir(target)
            for skill in [*removed, *added, *changed]:
                _gold_band_remove(target / skill)
            for skill in [*added, *changed]:
                shutil.copytree(kit / sources[skill], target / skill, symlinks=True)
        if added:
            lines.append(f"added skills {name}")
        if changed:
            lines.append(f"changed skills {name}")
        if removed:
            lines.append(f"removed skills {name}")
    if root.is_dir():
        for child in sorted(root.iterdir()):
            if child.name in wanted or child.name in spared or child.name in keep:
                continue
            if not check:
                _gold_band_remove(child)
            lines.append(f"removed skills {child.name}")
    return lines


def _gold_band_quiet(document: dict[str, Any]) -> dict[str, Any]:
    """``document`` with the app's outbound components off; every other key is as it was.

    Keys of the pinned app's ``SettingsConfig`` (src/config/mod.rs): the metrics
    switch ``desktopMetricsEnabled`` (:881), the task-board integration
    ``desktopMulticaEnabled`` (:892), the chat connectors ``imIntegrations.channels[].enabled``
    (:888, src/im/model.rs:485). The updater has no switch: its only setting is
    ``desktopUpdaterUrlOverride`` (:877), an endpoint override, so it is left
    exactly as it is. The app has no feedback setting. A key that is absent stays absent;
    the app defaults each of these to off.
    """
    quiet = dict(document)
    for key in ("desktopMetricsEnabled", "desktopMulticaEnabled"):
        if quiet.get(key) is True:
            quiet[key] = False
    im = quiet.get("imIntegrations")
    channels = im.get("channels") if isinstance(im, dict) else None
    if isinstance(im, dict) and isinstance(channels, list):
        quiet["imIntegrations"] = {
            **im,
            "channels": [
                {**channel, "enabled": False} if isinstance(channel, dict) and channel.get("enabled") is True
                else channel
                for channel in channels
            ],
        }
    return quiet


# Keys of settings.json the overlay may not set: the agent map is the registry's
# alone (the work instance holds exactly its declared agents), and the MCP
# servers come from the overlay's ``mcp_servers``.
GOLD_BAND_OVERLAY_RESERVED = ("agents", "contextServers")


def _gold_band_overlay_block(registry_path: Path, overlay_path: Path) -> dict[str, Any]:
    """The work instance's overlay declaration, checked by the crate; empty when there is none.

    Raises ``_CannotConverge`` for an overlay or a check that cannot be read or
    run, and ``_OverlayRefused`` carrying the crate's problem lines.
    """
    try:
        registry = yaml.safe_load(registry_path.read_text())
        overlay = gold_band_instances.load_overlay(overlay_path)
        if overlay is None:
            return {}
        refused = gold_band_instances.validate(registry, overlay_path)
    except (gold_band_instances.InstanceError, OSError, yaml.YAMLError) as exc:
        raise _CannotConverge(f"the overlay {overlay_path} cannot be checked: {exc}") from exc
    if refused:
        raise _OverlayRefused(refused)
    block = (overlay.get(gold_band_instances.OVERLAY_KEY) or {}).get(GOLD_BAND_WORK_INSTANCE)
    return block if isinstance(block, dict) else {}


class _OverlayRefused(Exception):
    """The overlay breaks a rule; ``problems`` are the crate's lines."""

    def __init__(self, problems: list[str]) -> None:
        super().__init__("; ".join(problems))
        self.problems = problems


def _gold_band_overlay_settings(
    document: dict[str, Any], settings: Any, notes: list[str]
) -> dict[str, Any]:
    """``document`` with each overlay setting set at its dotted path; a copy."""
    result: dict[str, Any] = json.loads(json.dumps(document))
    for path, value in _flat_settings("", settings) if settings else []:
        head, *rest = path.split(".")
        if head in GOLD_BAND_OVERLAY_RESERVED:
            raise _CannotConverge(f"overlay settings.{path}: {head} is not set from the overlay")
        node = result
        for key in [head, *rest][:-1]:
            child = node.setdefault(key, {})
            if not isinstance(child, dict):
                raise _CannotConverge(f"overlay settings.{path}: {key} holds a value that is not a mapping")
            node = child
        last = path.split(".")[-1]
        if last not in node or node[last] != value or type(node[last]) is not type(value):
            node[last] = value
            notes.append(f"overlay: set {path}")
    return result


def _flat_settings(prefix: str, value: Any) -> list[tuple[str, Any]]:
    """The leaves of the overlay's ``settings`` mapping with their dotted paths."""
    if not isinstance(value, dict):
        return [(prefix, value)]
    found: list[tuple[str, Any]] = []
    for key, item in value.items():
        found += _flat_settings(f"{prefix}.{key}" if prefix else str(key), item)
    return found


def _gold_band_overlay_servers(
    document: dict[str, Any], servers: Any, notes: list[str]
) -> dict[str, Any]:
    """``document`` with each overlay MCP server upserted by id into ``contextServers``; a copy."""
    result: dict[str, Any] = json.loads(json.dumps(document))
    current = result.get("contextServers")
    if current is None:
        current = []
    if not isinstance(current, list):
        raise _CannotConverge("settings.json contextServers is not a list")
    for name, body in (servers or {}).items():
        if "url" in body:
            transport: dict[str, Any] = {"transport": "http", "url": body["url"], "headers": body.get("headers") or {}}
        else:
            transport = {
                "transport": "stdio",
                "command": body["command"],
                "args": list(body.get("args") or []),
                "env": dict(body.get("env") or {}),
            }
        entry = {"id": name, "name": name, "enabled": True, **transport, "managed": False}
        for index, held in enumerate(current):
            if isinstance(held, dict) and held.get("id") == name:
                if held != entry:
                    current[index] = entry
                    notes.append(f"overlay: changed mcp server {name}")
                break
        else:
            current.append(entry)
            notes.append(f"overlay: added mcp server {name}")
    if current or "contextServers" in result:
        result["contextServers"] = current
    return result


def gold_band(args: argparse.Namespace) -> int:
    instance = args.instance
    # The default path is the current user's instance. The work instance lives
    # in its own account, so it is never guessed: name its data directory.
    if instance != GOLD_BAND_INSTANCE and not (args.settings or os.environ.get("GOLD_BAND_HOME")):
        print(f"gold-band-agents: {instance} needs --settings or GOLD_BAND_HOME; the default is the personal one")
        return 3
    if args.overlay is not None and instance != GOLD_BAND_WORK_INSTANCE:
        print(f"gold-band-agents: --overlay applies to the work instance {GOLD_BAND_WORK_INSTANCE} only")
        return 3
    settings = args.settings or _gold_band_settings_path()
    skills_root = Path(os.path.abspath(args.skills_root or settings.parent / "skill-sets"))
    try:
        everyone, retired, allowed = _gold_band_registry(args.registry, instance)
        # Only this instance's agents are converged; the others are still held
        # to the configuration-directory rules, which span both instances.
        declared = [agent for agent in everyone if _gold_band_instance_of(agent) == instance]
        raw_sets, sources = _gold_band_skill_sets(args.registry)
        problems = _gold_band_problems(declared, allowed, retired, instance)
        problems += _gold_band_everyone_problems(everyone)
        set_problems, sets = _gold_band_skill_set_problems(declared, raw_sets, sources, args.kit)
        problems += set_problems
        wanted_servers, bound_servers, server_problems = _gold_band_servers(args.registry, instance)
        problems += server_problems
        if problems:
            for problem in problems:
                print(f"gold-band-agents: {problem}")
            return 1
        running = None if args.check else _gold_band_running(settings.parent, instance)
        if running:
            print(
                f"gold-band-agents: Gold Band instance {running} is running; "
                "quit it and re-run setup (nothing was written)"
            )
            return GOLD_BAND_RUNNING_EXIT
        doc, current = _gold_band_document(settings)
        overlay = _gold_band_overlay_block(args.registry, args.overlay) if args.overlay is not None else {}
        current_servers = _gold_band_current_servers(settings, doc)
    except _OverlayRefused as exc:
        for problem in exc.problems:
            print(f"gold-band-agents: overlay: {problem}")
        return 1
    except _CannotConverge as exc:
        print(f"gold-band-agents: cannot converge: {exc}")
        return 3
    # The keep file is the personal instance's: the work instance holds exactly
    # its declared agents, so nothing is kept there beyond a skipped one.
    keep = set() if instance == GOLD_BAND_WORK_INSTANCE else _keep(args.keep)

    installed_ids: set[str] = set()
    skipped: list[tuple[str, str]] = []
    binary_paths: dict[str, str] = {}
    host = _gold_band_host()
    launched: list[dict[str, Any]] = []
    for declared_agent in declared:
        agent = _gold_band_for_host(declared_agent, host)
        if agent is None:
            skipped.append((declared_agent["id"], f"no launch command declared for {host}"))
            continue
        binary = agent.get("binary")
        found = shutil.which(binary) if binary is not None else None
        if binary is not None and found is None:
            skipped.append((agent["id"], f"{binary} is not installed"))
            continue
        launched.append(agent)
        if found is not None:
            binary_paths[agent["id"]] = os.path.abspath(found)
        installed_ids.add(agent["id"])

    # One directory per installed id that names a set, holding exactly that
    # set's skills: no id shares another's directory.
    wanted: dict[str, list[str]] = {
        agent["id"]: sets[agent["skill_set"]]
        for agent in launched
        if agent["id"] in installed_ids and agent.get("skill_set") is not None
    }
    installed: dict[str, dict[str, Any]] = {}
    for agent in launched:
        if agent["id"] in installed_ids:
            directory = skills_root / agent["id"] if agent["id"] in wanted else None
            installed[agent["id"]] = _gold_band_entry(agent, directory, binary_paths.get(agent["id"]))

    # A skipped agent is declared, so its existing entry stays exactly as it is.
    skipped_ids = {name for name, _ in skipped}
    merged: dict[str, Any] = {}
    removed: list[str] = []
    for name, value in current.items():
        if name in installed:
            merged[name] = installed[name]
        elif name in retired or (instance == GOLD_BAND_INSTANCE and "codex" in name.lower()):
            removed.append(name)
        elif name in skipped_ids or name in keep:
            merged[name] = value
        else:
            removed.append(name)
    added = [name for name in installed if name not in current]
    merged.update({name: installed[name] for name in added})
    changed = [name for name in merged if name in current and merged[name] != current[name]]

    for name, reason in skipped:
        print(f"skipped: {name} ({reason})")
    # The skill directories are written before settings.json, and an unchanged
    # settings map never skips them.
    for line in _gold_band_skills(skills_root, args.kit, wanted, sources, skipped_ids, keep - retired, args.check):
        print(line)
    # The map keeps its slot among the other top-level keys; a new one is appended.
    overlay_notes: list[str] = []
    merged_servers, server_lines = _gold_band_merge_servers(
        current_servers, wanted_servers, bound_servers, keep, instance
    )
    new_doc = {**doc, "agents": merged}
    if merged_servers or GOLD_BAND_SERVERS_KEY in doc:
        new_doc[GOLD_BAND_SERVERS_KEY] = merged_servers
    try:
        new_doc = _gold_band_overlay_settings(new_doc, overlay.get("settings"), overlay_notes)
        new_doc = _gold_band_overlay_servers(new_doc, overlay.get("mcp_servers"), overlay_notes)
    except _CannotConverge as exc:
        print(f"gold-band-agents: cannot converge: {exc}")
        return 3
    # Quiet last, so an overlay cannot turn the outbound components back on.
    new_doc = _gold_band_quiet(new_doc)
    if new_doc == doc:
        # An unchanged map still leaves the data directory owner-only.
        if not args.check and settings.parent.is_dir():
            os.chmod(settings.parent, 0o700)
        print("settings unchanged")
        return 0
    for name in added:
        print(f"added {name}")
    for name in changed:
        print(f"changed {name}")
    for name in removed:
        print(f"removed {name}")
    for line in server_lines:
        print(line)
    for note in overlay_notes:
        print(note)
    if args.check:
        return 0

    # The one write group of this surface: the guard runs again right before it.
    running = _gold_band_running(settings.parent, instance)
    if running:
        return _gold_band_stopped("gold-band-agents", running, [], [settings.name])
    try:
        _gold_band_write_settings(settings, new_doc)
    except OSError as exc:
        print(f"gold-band-agents: settings were not replaced, the old file is in place: {exc}")
        return 1
    return 0


def _gold_band_write_settings(settings: Path, document: dict[str, Any]) -> None:
    """Replace ``settings`` atomically, keeping one backup of what it held.

    The data directory ends owner-only. The new file is written 0o600 beside the
    old one and renamed over it, so a failure at any step leaves the old file
    untouched and no staged file behind; only keys this run changed differ.
    """
    data_dir = settings.parent
    data_dir.mkdir(mode=0o700, parents=True, exist_ok=True)
    os.chmod(data_dir, 0o700)
    backup = settings.with_name(settings.name + ".bak")
    staged = settings.with_name(settings.name + ".agent-kit-tmp")
    staged_backup = settings.with_name(settings.name + ".bak.agent-kit-tmp")
    try:
        if settings.is_file():
            shutil.copy2(settings, staged_backup)
            os.chmod(staged_backup, 0o600)
            os.replace(staged_backup, backup)
        staged.unlink(missing_ok=True)
        fd = os.open(staged, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        with os.fdopen(fd, "w", encoding="utf-8") as handle:
            handle.write(json.dumps(document, indent=2) + "\n")
            handle.flush()
            os.fsync(handle.fileno())
        os.chmod(staged, 0o600)
        os.replace(staged, settings)
    except BaseException:
        staged.unlink(missing_ok=True)
        staged_backup.unlink(missing_ok=True)
        raise


def _gold_band_roles_registry(path: Path) -> tuple[str | None, list[dict[str, Any]], set[str]]:
    """The roles source, the declared roles and the retired stems and ids."""
    try:
        data = yaml.safe_load(path.read_text())
    except (OSError, UnicodeDecodeError, yaml.YAMLError) as exc:
        raise _CannotConverge(f"{path} cannot be read: {exc}") from exc
    if not isinstance(data, dict):
        raise _CannotConverge(f"{path} is not a mapping")
    section = data.get("gold_band")
    if not isinstance(section, dict):
        raise _CannotConverge(f"{path} has no gold_band mapping")
    source = section.get("roles_source")
    if source is not None and not isinstance(source, str):
        raise _CannotConverge(f"{path}: gold_band.roles_source must be a path")
    roles = section.get("roles")
    if not isinstance(roles, list) or not all(isinstance(role, dict) for role in roles):
        raise _CannotConverge(f"{path}: gold_band.roles must be a list of mappings")
    retired = section.get("retired_roles")
    if retired is None:
        retired = []
    if not isinstance(retired, list) or not all(isinstance(name, str) for name in retired):
        raise _CannotConverge(f"{path}: gold_band.retired_roles must be a list of stems or ids")
    return source, roles, set(retired)


def _gold_band_role_problems(declared: list[dict[str, Any]], retired: set[str]) -> list[str]:
    """One line per problem with the declared roles; empty when they may be written."""
    problems: list[str] = []
    seen_files: set[str] = set()
    seen_ids: set[str] = set()
    for role in declared:
        raw_file = role.get("file")
        stem = raw_file if isinstance(raw_file, str) else "?"
        if not isinstance(raw_file, str) or not GOLD_BAND_ID.match(raw_file):
            problems.append(f"{stem}: file must match {GOLD_BAND_ID.pattern}")
        elif raw_file.startswith(GOLD_BAND_ROLE_BUILT_IN):
            problems.append(f"{stem}: file must not name a built-in profile")
        elif raw_file in seen_files:
            problems.append(f"{stem}: duplicate file")
        else:
            seen_files.add(raw_file)
        raw_id = role.get("id")
        name = raw_id if isinstance(raw_id, str) else stem
        if not isinstance(raw_id, str) or not raw_id.startswith("pf-"):
            problems.append(f"{name}: id must start with pf-")
        elif raw_id.startswith(GOLD_BAND_ROLE_BUILT_IN):
            problems.append(f"{name}: id must not name a built-in profile")
        elif raw_id in seen_ids:
            problems.append(f"{name}: duplicate id")
        else:
            seen_ids.add(raw_id)
        problems += [
            f"{name}: {key} must be a string"
            for key in ("name", "summary", "source")
            if not isinstance(role.get(key), str)
        ]
        # name and summary go into the front matter unquoted, so each must stay one plain YAML scalar.
        problems += [
            f"{name}: {key} must be a plain single-line scalar"
            for key in ("name", "summary")
            if isinstance(role.get(key), str) and not GOLD_BAND_ROLE_PLAIN.match(role[key])
        ]
        lenses = role.get("lenses")
        if not isinstance(lenses, list) or not all(isinstance(lens, str) for lens in lenses):
            problems.append(f"{name}: lenses must be a list of names")
        if (isinstance(raw_file, str) and raw_file in retired) or (isinstance(raw_id, str) and raw_id in retired):
            problems.append(f"{name}: declared and retired at once")
        problems += [f"{name}: unknown field {key}" for key in role if key not in GOLD_BAND_ROLE_FIELDS]
    return problems


def _gold_band_role_text(path: Path) -> str:
    """A source file's text; a missing or undecodable one cannot be converged."""
    try:
        return path.read_text(encoding="utf-8")
    except (OSError, UnicodeDecodeError) as exc:
        raise _CannotConverge(f"{path}: {exc}") from exc


def _gold_band_role_read(path: Path) -> str | None:
    """A profile's text, or None when it cannot be decoded."""
    try:
        return path.read_text(encoding="utf-8")
    except (OSError, UnicodeDecodeError):
        return None


def _gold_band_role_section(text: str) -> str:
    """One section: the text, a closing newline, then the blank line that separates it."""
    return text + ("" if text.endswith("\n") else "\n") + "\n"


def render_gold_band_role(critique: Path, role: dict[str, Any], closing: str) -> str:
    """One profile file: the front matter, then the role, the shared prompts, the schema, its lenses and the closing.

    The sources are concatenated verbatim, never templated: a brace in a role or
    a lens is a brace in the profile.
    """
    header = (
        "---\n"
        f"id: {role['id']}\n"
        f"name: {role['name']}\n"
        f"summary: {role['summary']}\n"
        f"createdAt: {GOLD_BAND_ROLE_STAMP}\n"
        f"updatedAt: {GOLD_BAND_ROLE_STAMP}\n"
        "dynamicTemplate: false\n"
        "---\n"
    )
    schema = _gold_band_role_text(critique / "schemas" / "finding.schema.json").rstrip()
    sections = [
        _gold_band_role_text(critique / "roles" / role["source"]),
        _gold_band_role_text(critique / "prompts" / "preamble.md"),
        _gold_band_role_text(critique / "prompts" / "output-contract.md"),
        f"```json\n{schema}\n```",
        *(_gold_band_role_text(critique / "lenses" / f"{lens}.md") for lens in role["lenses"]),
        closing,
    ]
    return header + "\n" + "".join(_gold_band_role_section(section) for section in sections)


def _gold_band_role_id(text: str | None) -> str | None:
    """The ``id:`` of a profile's first front matter block, or None."""
    if text is None:
        return None
    lines = text.splitlines()
    if not lines or lines[0].strip() != "---":
        return None
    for line in lines[1:]:
        if line.strip() == "---":
            return None
        key, sep, value = line.partition(":")
        if sep and key.strip() == "id":
            return value.strip() or None
    return None


def _gold_band_role_write(path: Path, text: str) -> None:
    """Written beside it and renamed, so a starting Gold Band never reads half a profile."""
    staged = path.with_name(f".{path.name}.agent-kit-tmp")
    try:
        staged.write_text(text, encoding="utf-8")
        os.replace(staged, path)
    except OSError:
        staged.unlink(missing_ok=True)
        raise


def gold_band_roles(args: argparse.Namespace) -> int:
    profiles = args.profiles or _gold_band_settings_path().parent / "context" / "profiles"
    try:
        source, declared, retired = _gold_band_roles_registry(args.registry)
        problems = _gold_band_role_problems(declared, retired)
        if problems:
            for problem in problems:
                print(f"gold-band-roles: {problem}")
            return 1
        running = None if args.check else _gold_band_running(profiles.parent.parent)
        if running:
            print(
                f"gold-band-roles: Gold Band instance {running} is running; "
                "quit it and re-run setup (nothing was written)"
            )
            return GOLD_BAND_RUNNING_EXIT
        critique = args.critique or (KIT_ROOT / source if source else None)
        if critique is None:
            raise _CannotConverge(f"{args.registry}: gold_band.roles_source must be a path")
        closing = _gold_band_role_text(args.closing or GOLD_BAND_ROLES_CLOSING)
        # Every role is rendered before anything is touched: one unreadable
        # source leaves the directory exactly as it was.
        rendered = {role["file"]: render_gold_band_role(critique, role, closing) for role in declared}
    except _CannotConverge as exc:
        print(f"gold-band-roles: cannot converge: {exc}")
        return 3
    keep = _keep(args.keep)
    existing = (
        {path.stem: path for path in sorted(profiles.glob("*.md")) if path.is_file()} if profiles.is_dir() else {}
    )
    for stem in rendered:
        held = _gold_band_role_id(_gold_band_role_read(existing[stem])) if stem in existing else None
        if held is not None and held.startswith(GOLD_BAND_ROLE_BUILT_IN):
            print(f"gold-band-roles: {stem}: the existing profile is a built-in; nothing was written")
            return 1

    added = [stem for stem in rendered if stem not in existing]
    changed = [
        stem for stem, text in rendered.items() if stem in existing and _gold_band_role_read(existing[stem]) != text
    ]
    removed: list[tuple[str, str]] = []
    for stem, path in existing.items():
        if stem in rendered:
            continue
        role_id = _gold_band_role_id(_gold_band_role_read(path))
        if role_id is not None and role_id.startswith(GOLD_BAND_ROLE_BUILT_IN):
            continue
        if stem in retired or role_id in retired:
            removed.append((stem, "retired"))
        elif stem in keep or f"{stem}.md" in keep:
            continue
        else:
            removed.append((stem, "undeclared"))

    if not (added or changed or removed):
        print("profiles unchanged")
        return 0
    for stem in added:
        print(f"added {stem}")
    for stem in changed:
        print(f"changed {stem}")
    for stem, reason in removed:
        print(f"removed {stem} ({reason})")
    if args.check:
        return 0

    # Each profile is one write group, and the guard runs before every group.
    groups = [(stem, True) for stem in added + changed] + [(stem, False) for stem, _ in removed]
    written: list[str] = []
    try:
        for index, (stem, write) in enumerate(groups):
            running = _gold_band_running(profiles.parent.parent)
            if running:
                pending = [f"{each}.md" for each, _ in groups[index:]]
                return _gold_band_stopped("gold-band-roles", running, written, pending)
            if write:
                profiles.mkdir(mode=0o700, parents=True, exist_ok=True)
                _gold_band_role_write(profiles / f"{stem}.md", rendered[stem])
            else:
                (profiles / f"{stem}.md").unlink()
            written.append(f"{stem}.md")
    except OSError as exc:
        print(f"gold-band-roles: cannot converge: {exc}")
        return 3
    return 0


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Converge agent surfaces to the registry and the keep file")
    sub = parser.add_subparsers(dest="surface", required=True)
    m = sub.add_parser("mcp")
    m.add_argument("--claude-home", type=Path, required=True)
    m.add_argument("--claude", type=Path, action="append", default=[])
    m.add_argument("--codex", type=Path)
    m.add_argument("--vibe", type=Path)
    m.add_argument("--vibe-private", type=Path)
    m.add_argument("--opencode", type=Path)
    m.add_argument("--declared", action="append", default=[], metavar="SURFACE=NAME,...")
    m.add_argument("--retired", default="")
    m.add_argument("--private", default="", metavar="NAME,...")
    p = sub.add_parser("plugins")
    for flag in ("--declared-markets", "--declared-plugins", "--retired-markets", "--retired-plugins"):
        p.add_argument(flag, default="")
    g = sub.add_parser("gold-band")
    g.add_argument("--settings", type=Path, default=None)
    g.add_argument("--instance", choices=GOLD_BAND_INSTANCES, default=GOLD_BAND_INSTANCE)
    g.add_argument("--registry", type=Path, default=REGISTRY_PATH)
    g.add_argument("--overlay", type=Path, default=None)
    # The kit the local skills are copied from, and where the per-agent skill
    # directories go: beside settings.json unless the owner moves them.
    g.add_argument("--kit", type=Path, default=KIT_ROOT)
    g.add_argument("--skills-root", type=Path, default=None)
    r = sub.add_parser("gold-band-roles")
    r.add_argument("--profiles", type=Path, default=None)
    r.add_argument("--registry", type=Path, default=REGISTRY_PATH)
    r.add_argument("--critique", type=Path, default=None)
    r.add_argument("--closing", type=Path, default=None)
    for each in (m, p, g, r):
        each.add_argument("--keep", type=Path, required=True)
        each.add_argument("--check", action="store_true")
    args = parser.parse_args(argv)
    surfaces: dict[str, Callable[[argparse.Namespace], int]] = {
        "mcp": mcp,
        "plugins": plugins,
        "gold-band": gold_band,
        "gold-band-roles": gold_band_roles,
    }
    return surfaces[args.surface](args)


if __name__ == "__main__":
    sys.exit(main())
