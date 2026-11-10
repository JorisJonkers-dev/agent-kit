"""Validate the Gold Band instances and the per-machine overlay.

The rules live in the ``gold-band-registry`` crate (scripts/crates/gold-band-registry);
this module only flattens the registry's ``gold_band_instances`` section, and the
overlay when one exists, into the crate's line protocol and runs the crate's
``validate`` over it. A refusal is returned as lines, one per problem.

The overlay is ``~/.config/agent-kit/overlay.yaml``, beside the keep file:

    instances:
      gold-band:
        agents: [codex]
        mcp_servers:
          jira: {command: npx, args: [-y, some-mcp], env: {JIRA_TOKEN: $JIRA_TOKEN}}
        settings: {editor: {theme: dark}}

A missing overlay is an empty one.
"""

from __future__ import annotations

import os
import shutil
import subprocess
import tempfile
from pathlib import Path
from typing import Any

import yaml

KIT_ROOT = Path(__file__).resolve().parent.parent
CARGO_MANIFEST = KIT_ROOT / "scripts" / "Cargo.toml"
CRATE = "gold-band-registry"
BIN_ENV = "GOLD_BAND_REGISTRY_BIN"
DEFAULT_OVERLAY = Path.home() / ".config" / "agent-kit" / "overlay.yaml"
TIMEOUT_S = 600.0

REGISTRY_KEY = "gold_band_instances"
OVERLAY_KEY = "instances"


class InstanceError(RuntimeError):
    """The declaration cannot be checked at all (malformed, or no crate to run)."""


def _flatten(prefix: str, value: Any, out: list[tuple[str, str]]) -> None:
    if isinstance(value, dict):
        for key, item in value.items():
            name = str(key)
            if not name or any(c in name for c in ".\t\r\n"):
                raise InstanceError(
                    f"{prefix or 'declaration'}: key {name!r} is empty or holds a dot or whitespace control"
                )
            _flatten(f"{prefix}.{name}" if prefix else name, item, out)
    elif isinstance(value, list):
        for index, item in enumerate(value):
            _flatten(f"{prefix}.{index}", item, out)
    elif isinstance(value, bool):
        out.append((prefix, "true" if value else "false"))
    elif isinstance(value, (str, int, float)):
        text = str(value)
        if any(c in text for c in "\t\r\n"):
            raise InstanceError(f"{prefix}: value holds a tab or newline")
        out.append((prefix, text))
    else:
        raise InstanceError(f"{prefix}: value must be a string, number, boolean, list or mapping")


def _section(label: str, block: Any, where: str) -> list[str]:
    if not isinstance(block, dict):
        raise InstanceError(f"{where} must be a mapping of instance id to declaration")
    lines = [f"source\t{label}"]
    for instance, body in block.items():
        if "\t" in str(instance) or "\n" in str(instance):
            raise InstanceError(f"{where}: instance id holds a tab or newline")
        lines.append(f"instance\t{instance}")
        if body is None:
            continue
        if not isinstance(body, dict):
            raise InstanceError(f"{where}.{instance} must be a mapping")
        fields: list[tuple[str, str]] = []
        _flatten("", body, fields)
        lines.extend(f"field\t{path}\t{value}" for path, value in fields)
    return lines


def flatten(registry: dict[str, Any], overlay: dict[str, Any] | None) -> str:
    """The crate's line protocol for the registry section and the overlay."""
    if REGISTRY_KEY not in registry:
        raise InstanceError(f"registry has no {REGISTRY_KEY} section")
    lines = _section("registry", registry[REGISTRY_KEY], REGISTRY_KEY)
    if overlay is not None:
        block = overlay.get(OVERLAY_KEY)
        lines += _section("overlay", {} if block is None else block, f"overlay {OVERLAY_KEY}")
    return "\n".join(lines) + "\n"


def load_overlay(path: Path) -> dict[str, Any] | None:
    """The overlay at ``path``, or None when the machine has none."""
    if not path.is_file():
        return None
    data = yaml.safe_load(path.read_text())
    if data is None:
        return {}
    if not isinstance(data, dict):
        raise InstanceError(f"{path} must be a mapping")
    unknown = set(data) - {OVERLAY_KEY}
    if unknown:
        raise InstanceError(f"{path}: unknown top-level keys {sorted(unknown)}; only {OVERLAY_KEY}:")
    return data


def _command(declaration: Path) -> list[str]:
    binary = os.environ.get(BIN_ENV)
    if binary:
        return [binary, "validate", str(declaration)]
    if shutil.which("cargo") is None:
        raise InstanceError(f"cargo is not on PATH and {BIN_ENV} is not set; cannot run the {CRATE} crate")
    return [
        "cargo",
        "run",
        "--quiet",
        "--locked",
        "--manifest-path",
        str(CARGO_MANIFEST),
        "-p",
        CRATE,
        "--",
        "validate",
        str(declaration),
    ]


def validate(registry: dict[str, Any], overlay_path: Path | None = None) -> list[str]:
    """Problems the crate finds in the instances and the overlay; empty when clean."""
    overlay = load_overlay(overlay_path) if overlay_path is not None else None
    text = flatten(registry, overlay)
    with tempfile.TemporaryDirectory() as tmp:
        declaration = Path(tmp) / "declaration.txt"
        declaration.write_text(text)
        try:
            result = subprocess.run(
                _command(declaration), capture_output=True, text=True, timeout=TIMEOUT_S, check=False
            )
        except (OSError, subprocess.TimeoutExpired) as exc:
            raise InstanceError(f"could not run the {CRATE} crate: {exc}") from exc
    if result.returncode == 0:
        return []
    lines = [line for line in result.stderr.splitlines() if line.strip()]
    if result.returncode != 1:
        raise InstanceError(f"the {CRATE} crate failed ({result.returncode}): " + "; ".join(lines))
    return lines
