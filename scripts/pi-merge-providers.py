#!/usr/bin/env python3
"""Write one Pi provider per LiteLLM route into Pi's models.json, keeping the rest of the file.

    pi-merge-providers.py --providers registry/generated/pi/providers.json \\
        --models-json ~/.pi/agent/models.json --state ~/.config/agent-kit/pi-providers.json \\
        --secret-helper ~/.local/bin/agent-kit-secret [--check]

``--providers`` is the rendered table (``scripts/render_registry.py``): the proxy URL,
the API kind and, per route, the provider name (``litellm-<agent>``), the route's key
variable and its models. Each becomes a ``providers`` entry with that ``baseUrl`` and
``api``, one ``models`` entry per model id, and an ``apiKey`` of
``!<secret-helper> get <KEY_ENV>``: Pi runs that command when it sends a request, so
the key is never in models.json, argv or the environment.

Setup owns the providers it names: each is replaced whole on every run. Every other
provider and every other top-level field the owner wrote is kept. The names setup
wrote are recorded in ``--state``, so a provider whose route is gone from the registry
is removed on the next run; one setup never recorded is left alone.

A models.json that is not a JSON object with a ``providers`` object is left as it is
and the script exits 2. ``--check`` changes nothing and reports what a run would do.
Prints one line: what changed, or that the providers are in place.
"""

from __future__ import annotations

import argparse
import json
import os
import shlex
import sys
import tempfile
from pathlib import Path
from typing import Any

SCHEMA = "https://pi.dev/schemas/models.schema.json"


def desired_providers(table: dict[str, Any], secret_helper: str) -> dict[str, dict[str, Any]]:
    """The providers entries setup writes, by name."""
    out: dict[str, dict[str, Any]] = {}
    for provider in table.get("providers") or []:
        command = f"!{shlex.quote(secret_helper)} get {shlex.quote(provider['key_env'])}"
        out[provider["name"]] = {
            "baseUrl": table["base_url"],
            "api": table["api"],
            "apiKey": command,
            "models": [{"id": model} for model in provider["models"]],
        }
    return out


def merge(
    existing: dict[str, Any], desired: dict[str, dict[str, Any]], recorded: set[str]
) -> tuple[dict[str, Any], list[str], list[str], list[str]]:
    """models.json with setup's providers in place: (merged, added, updated, removed)."""
    merged = dict(existing)
    providers = dict(existing.get("providers") or {})
    added = [name for name in desired if name not in providers]
    updated = [name for name in desired if name in providers and providers[name] != desired[name]]
    removed = sorted(name for name in recorded - set(desired) if name in providers)
    for name in removed:
        del providers[name]
    providers.update(desired)
    merged.setdefault("$schema", SCHEMA)
    merged["providers"] = providers
    return merged, added, updated, removed


def read_models_json(path: Path) -> dict[str, Any]:
    """The current models.json, or an empty one; raises ValueError on a file setup must not touch."""
    if not path.exists():
        return {}
    data = json.loads(path.read_text())
    if not isinstance(data, dict):
        raise ValueError("it is not a JSON object")
    if not isinstance(data.get("providers", {}), dict):
        raise ValueError("its providers field is not an object")
    return data


def read_recorded(path: Path) -> set[str]:
    """The provider names setup wrote last time; a missing or unreadable record is empty."""
    try:
        return {str(name) for name in json.loads(path.read_text()).get("providers") or []}
    except (OSError, ValueError, AttributeError):
        return set()


def write_atomically(path: Path, text: str) -> None:
    """Replace ``path`` in one step, keeping its mode when it exists."""
    path.parent.mkdir(parents=True, exist_ok=True)
    mode = path.stat().st_mode & 0o777 if path.exists() else 0o644
    fd, tmp = tempfile.mkstemp(dir=path.parent, prefix=f".{path.name}.")
    try:
        with os.fdopen(fd, "w") as handle:
            handle.write(text)
        os.chmod(tmp, mode)
        os.replace(tmp, path)
    except BaseException:
        Path(tmp).unlink(missing_ok=True)
        raise


def summary(added: list[str], updated: list[str], removed: list[str], check: bool) -> str:
    parts = [
        f"{verb} {', '.join(names)}"
        for verb, names in (("added", added), ("updated", updated), ("removed", removed))
        if names
    ]
    if not parts:
        return "in place"
    return ("would be " if check else "") + "; ".join(parts)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Write the LiteLLM route providers into Pi's models.json.")
    parser.add_argument("--providers", type=Path, required=True)
    parser.add_argument("--models-json", type=Path, required=True)
    parser.add_argument("--state", type=Path, required=True)
    parser.add_argument("--secret-helper", required=True)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args(argv)

    desired = desired_providers(json.loads(args.providers.read_text()), args.secret_helper)
    try:
        existing = read_models_json(args.models_json)
    except ValueError as exc:
        print(f"pi-merge-providers: {args.models_json} left as it is: {exc}", file=sys.stderr)
        return 2
    merged, added, updated, removed = merge(existing, desired, read_recorded(args.state))
    if not args.check:
        if merged != existing:
            write_atomically(args.models_json, json.dumps(merged, indent=2) + "\n")
        record = json.dumps({"providers": sorted(desired)}, indent=2) + "\n"
        if not args.state.exists() or args.state.read_text() != record:
            write_atomically(args.state, record)
    print(summary(added, updated, removed, args.check))
    return 0


if __name__ == "__main__":
    sys.exit(main())
