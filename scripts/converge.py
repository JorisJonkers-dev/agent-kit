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
agents go even when the keep file names them.

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
        --keep PATH [--settings PATH] [--registry PATH] [--check]
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

import tomlkit
import yaml

DECLARED_SOURCES = ("plugin:",)

KIT_ROOT = Path(__file__).resolve().parent.parent
REGISTRY_PATH = KIT_ROOT / "registry" / "estate-tooling.yaml"

# The Gold Band instance whose settings.json this kit converges, and the app
# process whose running state makes a write unsafe.
GOLD_BAND_INSTANCE = "gold-band-personal"
GOLD_BAND_PROCESS = "gold-band-desktop"
GOLD_BAND_RUNNING_ENV = "AGENT_KIT_GOLD_BAND_RUNNING"
GOLD_BAND_ID = re.compile(r"^[a-z0-9]+(-[a-z0-9]+)*$")
# An adapter env value is a reference the app resolves at launch, never a
# literal: a secret in settings.json is a secret in a file.
GOLD_BAND_ENV_REF = re.compile(r"^\$(?:[A-Za-z_][A-Za-z0-9_]*|\{[A-Za-z_][A-Za-z0-9_]*\})$")
GOLD_BAND_PROMPT_DELIVERY = ("none", "meta-append")
GOLD_BAND_AGENT_FIELDS = (
    "id",
    "binary",
    "adapter",
    "icon",
    "primaryAgentDir",
    "projectPrimaryAgentDir",
    "compatibleAgentDirs",
    "systemPromptDelivery",
    "externalSessionSyncSupported",
    "externalSessionSyncEnabled",
)
GOLD_BAND_ADAPTER_FIELDS = ("displayName", "command", "args", "env")
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


def _gold_band_settings_path() -> Path:
    """``${GOLD_BAND_HOME:-~/.gold-band}/settings.json``."""
    home = os.environ.get("GOLD_BAND_HOME") or str(Path.home() / ".gold-band")
    return Path(home) / "settings.json"


def _gold_band_registry(path: Path) -> tuple[list[dict[str, Any]], set[str], set[str]]:
    """The declared agents, the retired ids and the ids the personal instance may hold."""
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
    personal = instances.get(GOLD_BAND_INSTANCE) if isinstance(instances, dict) else None
    allowed = personal.get("agents") if isinstance(personal, dict) else None
    if not isinstance(allowed, list) or not all(isinstance(name, str) for name in allowed):
        raise _CannotConverge(
            f"{path}: gold_band_instances.{GOLD_BAND_INSTANCE}.agents must be a list of ids"
        )
    return agents, set(retired), set(allowed)


def _gold_band_adapter_problems(name: str, adapter: Any) -> list[str]:
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
    if any("codex" in word.lower() for word in words):
        problems.append(f"{name}: Codex is refused in adapter.command or adapter.args")
    return problems


def _gold_band_problems(
    declared: list[dict[str, Any]], allowed: set[str], retired: set[str] = frozenset()
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
        if "codex" in raw.lower():
            problems.append(f"{raw}: Codex is refused on the personal instance")
        if raw in retired:
            problems.append(f"{raw}: declared and retired at once")
        if raw not in allowed:
            problems.append(f"{raw}: not in gold_band_instances.{GOLD_BAND_INSTANCE}.agents")
        problems += [f"{raw}: unknown field {key}" for key in agent if key not in GOLD_BAND_AGENT_FIELDS]
        problems += _gold_band_adapter_problems(raw, agent.get("adapter"))
        delivery = agent.get("systemPromptDelivery")
        if delivery is not None and delivery not in GOLD_BAND_PROMPT_DELIVERY:
            problems.append(f"{raw}: systemPromptDelivery must be one of {', '.join(GOLD_BAND_PROMPT_DELIVERY)}")
    return problems


def _gold_band_expand(value: str) -> str:
    """A leading ``~/`` becomes the home directory; the app expands nothing itself."""
    return str(Path.home()) + value[1:] if value.startswith("~/") else value


def _gold_band_entry(agent: dict[str, Any]) -> dict[str, Any]:
    """The settings entry for a declared agent: its written fields, expanded."""
    entry = {
        key: value
        for key, value in agent.items()
        if key in GOLD_BAND_AGENT_FIELDS and key not in ("id", "binary")
    }
    adapter = entry.get("adapter")
    if not isinstance(adapter, dict):
        return entry
    adapter = dict(adapter)
    if agent["id"] in GOLD_BAND_BUILT_INS:
        adapter.pop("command", None)
        adapter.pop("args", None)
    if "command" in adapter:
        adapter["command"] = _gold_band_expand(adapter["command"])
    if "args" in adapter:
        adapter["args"] = [_gold_band_expand(arg) for arg in adapter["args"]]
    if "env" in adapter:
        adapter["env"] = {key: _gold_band_expand(value) for key, value in adapter["env"].items()}
    entry["adapter"] = adapter
    return entry


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


def _gold_band_running() -> bool:
    """Whether the desktop app holds settings.json open, so a write would be lost."""
    flag = os.environ.get(GOLD_BAND_RUNNING_ENV)
    if flag is not None:
        return flag == "1"
    try:
        pattern = rf"(^|/){GOLD_BAND_PROCESS}( |$)"
        return subprocess.run(["pgrep", "-f", pattern], capture_output=True, check=False).returncode == 0
    except OSError:
        return False


def gold_band(args: argparse.Namespace) -> int:
    settings = args.settings or _gold_band_settings_path()
    try:
        declared, retired, allowed = _gold_band_registry(args.registry)
        problems = _gold_band_problems(declared, allowed, retired)
        if problems:
            for problem in problems:
                print(f"gold-band-agents: {problem}")
            return 1
        if not args.check and _gold_band_running():
            print("gold-band-agents: Gold Band is running; quit it and re-run setup (nothing was written)")
            return 3
        doc, current = _gold_band_document(settings)
    except _CannotConverge as exc:
        print(f"gold-band-agents: cannot converge: {exc}")
        return 3
    keep = _keep(args.keep)

    installed: dict[str, dict[str, Any]] = {}
    skipped: list[tuple[str, str]] = []
    for agent in declared:
        binary = agent.get("binary")
        if binary is not None and shutil.which(binary) is None:
            skipped.append((agent["id"], binary))
            continue
        installed[agent["id"]] = _gold_band_entry(agent)

    # A skipped agent is declared, so its existing entry stays exactly as it is.
    skipped_ids = {name for name, _ in skipped}
    merged: dict[str, Any] = {}
    removed: list[str] = []
    for name, value in current.items():
        if name in installed:
            merged[name] = installed[name]
        elif name in retired or "codex" in name.lower():
            removed.append(name)
        elif name in skipped_ids or name in keep:
            merged[name] = value
        else:
            removed.append(name)
    added = [name for name in installed if name not in current]
    merged.update({name: installed[name] for name in added})
    changed = [name for name in merged if name in current and merged[name] != current[name]]

    for name, binary in skipped:
        print(f"skipped: {name} ({binary} is not installed)")
    # The map keeps its slot among the other top-level keys; a new one is appended.
    new_doc = {**doc, "agents": merged}
    if new_doc == doc:
        print("settings unchanged")
        return 0
    for name in added:
        print(f"added {name}")
    for name in changed:
        print(f"changed {name}")
    for name in removed:
        print(f"removed {name}")
    if args.check:
        return 0

    if settings.is_file():
        shutil.copy2(settings, settings.with_name(settings.name + ".bak"))
    settings.parent.mkdir(parents=True, exist_ok=True)
    # Written beside it and renamed, so a starting Gold Band never reads half a file.
    staged = settings.with_name(settings.name + ".agent-kit-tmp")
    staged.write_text(json.dumps(new_doc, indent=2) + "\n")
    os.chmod(staged, 0o600)
    os.replace(staged, settings)
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
    g.add_argument("--registry", type=Path, default=REGISTRY_PATH)
    for each in (m, p, g):
        each.add_argument("--keep", type=Path, required=True)
        each.add_argument("--check", action="store_true")
    args = parser.parse_args(argv)
    return {"mcp": mcp, "plugins": plugins, "gold-band": gold_band}[args.surface](args)


if __name__ == "__main__":
    sys.exit(main())
