#!/usr/bin/env python3
# managed by agent-kit setup-workstation.sh
"""Run a harness on a LiteLLM route, and write each harness's provider entries (agent-kit#737).

Two subcommands, both driven by the table ``scripts/render_registry.py`` renders
(``registry/generated/litellm/harnesses.json``): the proxy URL and, per route, the
provider name (``litellm-<agent>``), the route's key variable and its models.

    litellm-harness.py configure <codex|hermes|opencode|vibe> --table T --config PATH [--check]
    litellm-harness.py run <claude|codex|hermes|opencode|vibe> <route> [--model ID] [-- ARGS...]

``configure`` writes one provider per route into the harness's own config file: Codex
``[model_providers.litellm-<agent>]`` in config.toml, Hermes ``providers:`` in config.yaml,
OpenCode ``provider`` in opencode.json, Vibe ``[[providers]]`` and ``[[models]]`` in the
config.toml of a Vibe home that is neither ``~/.vibe`` nor ``~/.vibe-private``. Setup owns
every entry named ``litellm-*``: it replaces those the table names, removes the rest, and
keeps everything else the owner wrote. No file holds a key, only the *name* of the
variable that does.

``run`` reads the route's key from the secret store (``agent-kit-secret get <KEY_ENV>``,
the value on the helper's stdout) and starts the harness with it in the child's
environment, never in argv and never printed. Claude Code has no config entry: the launcher
sets ``ANTHROPIC_BASE_URL`` and ``ANTHROPIC_AUTH_TOKEN`` for that one process, so the
default profile never holds either. Exit codes: 2 for a missing key, table or route, or a
config file this will not touch.
"""

from __future__ import annotations

import argparse
import copy
import json
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import Any

PREFIX = "litellm-"
CONFIGURABLE = ("codex", "hermes", "opencode", "vibe")
HARNESSES = ("claude", *CONFIGURABLE)
SCHEMA = "https://opencode.ai/config.json"
# Homes the Vibe provider entries must never be written into.
FORBIDDEN_VIBE_HOMES = (".vibe", ".vibe-private")


class HarnessError(Exception):
    """A refusal the caller reports as one line and exit status 2."""


def provider_name(route: dict[str, Any]) -> str:
    return PREFIX + str(route["agent"])


def route_named(table: dict[str, Any], agent: str) -> dict[str, Any]:
    for route in table.get("routes") or []:
        if route["agent"] == agent:
            found: dict[str, Any] = route
            return found
    known = ", ".join(str(route["agent"]) for route in table.get("routes") or [])
    raise HarnessError(f"no LiteLLM route {agent!r}; the registry has: {known}")


def model_ids(route: dict[str, Any]) -> list[str]:
    return [str(model["id"]) for model in route.get("models") or []]


# ----- the entry each harness gets, by provider name ------------------------


def codex_entries(table: dict[str, Any]) -> dict[str, dict[str, Any]]:
    return {
        provider_name(route): {
            "name": f"LiteLLM {route['agent']}",
            "base_url": table["base_url"],
            "env_key": route["key_env"],
            "wire_api": "responses",
        }
        for route in table["routes"]
    }


def hermes_entries(table: dict[str, Any]) -> dict[str, dict[str, Any]]:
    out = {}
    for route in table["routes"]:
        entry: dict[str, Any] = {
            "name": provider_name(route),
            "base_url": table["base_url"],
            "key_env": route["key_env"],
            "api_mode": "chat_completions",
        }
        ids = model_ids(route)
        if ids:
            entry["default_model"] = ids[0]
            entry["models"] = {model: {} for model in ids}
        out[provider_name(route)] = entry
    return out


def opencode_entries(table: dict[str, Any]) -> dict[str, dict[str, Any]]:
    return {
        provider_name(route): {
            "npm": "@ai-sdk/openai-compatible",
            "name": f"LiteLLM {route['agent']}",
            "options": {"baseURL": table["base_url"], "apiKey": "{env:" + str(route["key_env"]) + "}"},
            "models": {model: {"name": model} for model in model_ids(route)},
        }
        for route in table["routes"]
    }


def vibe_entries(table: dict[str, Any]) -> tuple[list[dict[str, Any]], list[dict[str, Any]]]:
    providers: list[dict[str, Any]] = []
    models: list[dict[str, Any]] = []
    for route in table["routes"]:
        name = provider_name(route)
        providers.append(
            {
                "name": name,
                "api_base": table["base_url"],
                "api_key_env_var": route["key_env"],
                "api_style": "openai",
                "backend": "generic",
            }
        )
        models.extend({"name": model, "provider": name, "alias": f"{name}/{model}"} for model in model_ids(route))
    return providers, models


# ----- merging --------------------------------------------------------------


def merge_named(existing: dict[str, Any], desired: dict[str, Any]) -> dict[str, Any]:
    """``existing`` with every ``litellm-*`` key replaced by ``desired``; other keys kept."""
    merged = {key: value for key, value in existing.items() if not key.startswith(PREFIX)}
    merged.update(desired)
    return merged


def write_atomically(path: Path, text: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    mode = path.stat().st_mode & 0o777 if path.exists() else 0o600
    fd, tmp = tempfile.mkstemp(dir=path.parent, prefix=f".{path.name}.")
    try:
        with os.fdopen(fd, "w") as handle:
            handle.write(text)
        os.chmod(tmp, mode)
        os.replace(tmp, path)
    except BaseException:
        Path(tmp).unlink(missing_ok=True)
        raise


def merge_codex(text: str, table: dict[str, Any]) -> str:
    import tomlkit

    doc = tomlkit.parse(text)
    providers = doc.get("model_providers")
    if providers is None:
        providers = tomlkit.table(is_super_table=True)
        doc["model_providers"] = providers
    for name in [key for key in providers if key.startswith(PREFIX)]:
        del providers[name]
    for name, entry in codex_entries(table).items():
        providers[name] = entry
    return tomlkit.dumps(doc)


def merge_vibe(text: str, table: dict[str, Any]) -> str:
    import tomlkit

    doc = tomlkit.parse(text)
    providers, models = vibe_entries(table)
    for key, entries, field in (("providers", providers, "name"), ("models", models, "provider")):
        current = doc.get(key)
        kept = [item for item in (current or []) if not str(item.get(field, "")).startswith(PREFIX)]
        array = tomlkit.aot()
        for item in [*kept, *entries]:
            array.append(item if isinstance(item, tomlkit.items.Table) else _toml_table(item))
        doc[key] = array
    return tomlkit.dumps(doc)


def _toml_table(values: dict[str, Any]) -> Any:
    import tomlkit

    out = tomlkit.table()
    for key, value in values.items():
        out[key] = value
    return out


def merge_hermes(text: str, table: dict[str, Any]) -> str:
    import yaml

    data = yaml.safe_load(text) if text.strip() else {}
    if not isinstance(data, dict):
        raise HarnessError("the Hermes config is not a mapping")
    providers = data.get("providers") or {}
    if not isinstance(providers, dict):
        raise HarnessError("its providers field is not a mapping")
    data["providers"] = merge_named(providers, hermes_entries(table))
    return yaml.safe_dump(data, sort_keys=False)


def merge_opencode(text: str, table: dict[str, Any]) -> str:
    data = json.loads(text) if text.strip() else {}
    if not isinstance(data, dict) or not isinstance(data.get("provider", {}), dict):
        raise HarnessError("opencode.json is not an object with an object provider field")
    data.setdefault("$schema", SCHEMA)
    data["provider"] = merge_named(data.get("provider") or {}, opencode_entries(table))
    return json.dumps(data, indent=2) + "\n"


MERGERS = {"codex": merge_codex, "hermes": merge_hermes, "opencode": merge_opencode, "vibe": merge_vibe}


def check_vibe_home(config: Path, home: Path) -> None:
    resolved = config.resolve().parent
    for name in FORBIDDEN_VIBE_HOMES:
        if resolved == (home / name).resolve():
            raise HarnessError(f"{resolved} is a Vibe home this never writes to; use a separate one")
    if "private" in resolved.name:
        raise HarnessError(f"{resolved} looks like a private Vibe home; use a separate one")


def configure(harness: str, table: dict[str, Any], config: Path, check: bool, home: Path) -> str:
    if harness == "vibe":
        check_vibe_home(config, home)
    old = config.read_text() if config.exists() else ""
    try:
        new = MERGERS[harness](old, table)
    except (ValueError, KeyError, HarnessError) as error:
        raise HarnessError(f"{config} was left as it is: {error}") from error
    if new == old:
        return "in place"
    if not check:
        if config.exists():
            shutil.copy2(config, config.with_name(config.name + ".bak"))
        write_atomically(config, new)
    return ("would be written" if check else "written") + f" ({len(table['routes'])} routes)"


# ----- running --------------------------------------------------------------


def plan(
    harness: str,
    route: dict[str, Any],
    table: dict[str, Any],
    key: str,
    args: list[str],
    model: str | None,
    environ: dict[str, str],
    vibe_home: Path,
) -> tuple[list[str], dict[str, str]]:
    """The argv and environment to start the harness with. The key is only ever in the environment."""
    env = dict(environ)
    name = provider_name(route)
    if harness == "claude":
        for gone in ("ANTHROPIC_API_KEY", "CLAUDE_CODE_OAUTH_TOKEN"):
            env.pop(gone, None)
        env["ANTHROPIC_BASE_URL"] = str(table["base_url"]).removesuffix("/v1")
        env["ANTHROPIC_AUTH_TOKEN"] = key
        argv = ["claude", *(["--model", model] if model else [])]
    elif harness == "codex":
        env[route["key_env"]] = key
        argv = ["codex", "-c", f'model_provider="{name}"', *(["-m", model] if model else [])]
    elif harness == "hermes":
        env[route["key_env"]] = key
        argv = ["hermes", "chat", "--provider", name, *(["-m", model] if model else [])]
    elif harness == "opencode":
        env[route["key_env"]] = key
        argv = ["opencode", *(["-m", f"{name}/{model}"] if model else [])]
    else:
        env[route["key_env"]] = key
        env["VIBE_HOME"] = str(vibe_home)
        if model:
            env["VIBE_ACTIVE_MODEL"] = f"{name}/{model}"
        argv = ["vibe"]
    return [*argv, *args], env


def read_key(helper: str, key_env: str) -> str:
    try:
        done = subprocess.run([helper, "get", key_env], capture_output=True, text=True, check=False)
    except OSError as error:
        raise HarnessError(f"the secret store helper {helper} did not run: {error.strerror}") from error
    key = done.stdout.strip()
    if done.returncode != 0 or not key:
        raise HarnessError(f"no key {key_env} in the secret store; run agent-kit setup to mint it")
    return key


def load_table(path: Path) -> dict[str, Any]:
    try:
        table = json.loads(path.read_text())
    except (OSError, ValueError) as error:
        raise HarnessError(f"cannot read the route table {path}: {error}") from error
    if not isinstance(table, dict) or not table.get("base_url") or not isinstance(table.get("routes"), list):
        raise HarnessError(f"{path} is not a LiteLLM route table")
    return table


def default_table() -> Path:
    """The table beside a kit checkout, else the copy setup puts in ~/.config/agent-kit."""
    kit = Path(__file__).resolve().parent.parent / "registry" / "generated" / "litellm" / "harnesses.json"
    installed = (
        Path(os.environ.get("XDG_CONFIG_HOME") or Path.home() / ".config") / "agent-kit" / "litellm-harnesses.json"
    )
    return kit if kit.exists() or not installed.exists() else installed


def default_helper() -> str:
    return str(Path.home() / ".local" / "bin" / "agent-kit-secret")


def main(argv: list[str] | None = None, exec_fn: Any = os.execvpe) -> int:
    argv = sys.argv[1:] if argv is None else argv
    split = argv.index("--") if "--" in argv else len(argv)
    own, passthrough = argv[:split], argv[split + 1 :]
    parser = argparse.ArgumentParser(description="Run a harness on a LiteLLM route.")
    parser.add_argument("--table", type=Path, default=None)
    sub = parser.add_subparsers(dest="command", required=True)
    conf = sub.add_parser("configure")
    conf.add_argument("harness", choices=CONFIGURABLE)
    conf.add_argument("--config", type=Path, required=True)
    conf.add_argument("--check", action="store_true")
    run = sub.add_parser("run")
    run.add_argument("harness", choices=HARNESSES)
    run.add_argument("route")
    run.add_argument("--model")
    run.add_argument("--secret-helper", default=None)
    run.add_argument("--vibe-home", type=Path, default=None)
    args = parser.parse_args(own)
    home = Path(os.environ.get("HOME") or Path.home())
    try:
        table = load_table(args.table or default_table())
        if args.command == "configure":
            print(f"{args.harness}: LiteLLM providers {configure(args.harness, table, args.config, args.check, home)}")
            return 0
        route = route_named(table, args.route)
        if args.model and args.model not in model_ids(route):
            raise HarnessError(f"route {args.route} does not list the model {args.model!r}")
        config_home = Path(os.environ.get("XDG_CONFIG_HOME") or home / ".config")
        vibe_home = args.vibe_home or config_home / "agent-kit" / "vibe-litellm"
        if args.harness == "vibe":
            check_vibe_home(vibe_home / "config.toml", home)
        key = read_key(args.secret_helper or default_helper(), route["key_env"])
        command, env = plan(
            args.harness, route, table, key, passthrough, args.model, dict(copy.copy(os.environ)), vibe_home
        )
        exec_fn(command[0], command, env)
        return 0
    except HarnessError as error:
        print(f"litellm-harness: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
