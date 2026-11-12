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

import argparse
import atexit
import os
import shutil
import subprocess
import sys
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
        if any(c in str(instance) for c in "\t\r\n"):
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
    try:
        data = yaml.safe_load(path.read_text())
    except (yaml.YAMLError, UnicodeDecodeError) as exc:
        raise InstanceError(f"{path} cannot be read: {exc}") from exc
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
        "--manifest-path",
        str(_standalone_manifest()),
        "--",
        "validate",
        str(declaration),
    ]


_STANDALONE: Path | None = None


def _standalone_manifest() -> Path:
    """A private copy of the crate that is its own workspace.

    The crate is std-only, so building it alone needs no registry. Building it
    inside the shared workspace would make cargo resolve every other member's
    dependencies too, and so need the crate gate, which a registry check does
    not have.
    """
    global _STANDALONE
    if _STANDALONE is None:
        root = Path(tempfile.mkdtemp(prefix="gold-band-registry-"))
        atexit.register(shutil.rmtree, root, ignore_errors=True)
        source = CARGO_MANIFEST.parent / "crates" / CRATE
        shutil.copytree(source / "src", root / "src")
        (root / "Cargo.toml").write_text((source / "Cargo.toml").read_text() + "\n[workspace]\n")
        _STANDALONE = root / "Cargo.toml"
    return _STANDALONE


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
    if result.returncode != 1 or not lines:
        raise InstanceError(f"the {CRATE} crate failed ({result.returncode}): " + "; ".join(lines))
    return lines


WORK_INSTANCE = "gold-band"
CODE_NAV = "code-nav"


def _expand(root: str) -> str:
    """``~`` and ``$HOME`` resolved against the running user's HOME."""
    return os.path.expandvars(os.path.expanduser(root))


def _root_pairs(args: Any) -> list[str] | None:
    """The paths of an exact ``--root PATH`` list, or None when ``args`` is anything else."""
    if not isinstance(args, list) or not args or len(args) % 2:
        return None
    roots: list[str] = []
    for flag, path in zip(args[::2], args[1::2], strict=True):
        if flag != "--root" or not isinstance(path, str) or not path.strip() or path.startswith("-"):
            return None
        roots.append(path)
    return roots


def code_nav_roots(registry: dict[str, Any], overlay_path: Path) -> list[str]:
    """The roots the work overlay adds to code-nav, expanded, beyond the registry's own.

    Only the work instance's ``mcp_servers.code-nav`` is read: the overlay is
    validated first (the crate refuses ``mcp_servers`` on the personal instance),
    and no other instance or server is looked at. A missing overlay, or one with
    no code-nav entry, adds nothing. An entry that is not ``command: code-nav``
    with ``--root PATH`` pairs only is refused, so an overlay cannot smuggle a
    flag or another binary into the registration.
    """
    overlay = load_overlay(overlay_path)
    if overlay is None:
        return []
    refused = validate(registry, overlay_path)
    if refused:
        raise InstanceError("; ".join(refused))
    block = overlay.get(OVERLAY_KEY) or {}
    servers = ((block.get(WORK_INSTANCE) or {}).get("mcp_servers")) or {}
    entry = servers.get(CODE_NAV)
    if entry is None:
        return []
    roots = _root_pairs(entry.get("args")) if isinstance(entry, dict) and entry.get("command") == CODE_NAV else None
    if roots is None:
        raise InstanceError(
            f"overlay {WORK_INSTANCE}.mcp_servers.{CODE_NAV} must be command {CODE_NAV} with only --root PATH pairs"
        )
    own = {
        _expand(path)
        for server in registry.get("mcp_servers") or []
        if server.get("name") == CODE_NAV
        for path in _root_pairs(server.get("args")) or []
    }
    found: list[str] = []
    for root in map(_expand, roots):
        if root not in own and root not in found:
            found.append(root)
    return found


def names_work_instance(overlay_path: Path) -> bool:
    """Whether the overlay at ``overlay_path`` declares the work instance.

    Only the overlay is read, never the registry. A missing overlay names no
    instance; one that is not a valid overlay file is an error.
    """
    overlay = load_overlay(overlay_path)
    block = (overlay or {}).get(OVERLAY_KEY)
    if block is not None and not isinstance(block, dict):
        raise InstanceError(f"{overlay_path}: {OVERLAY_KEY} must be a mapping of instance id to declaration")
    return WORK_INSTANCE in (block or {})


def main(argv: list[str] | None = None) -> int:
    """``code-nav-roots``: print the overlay's extra code-nav roots, one per line.

    Exit 0 with the roots (none is fine), 1 when the overlay is refused, with the
    reason on stderr. Setup then registers the registry's personal roots alone.

    ``names-work-instance``: print ``yes`` or ``no`` and exit 0; exit 1 with the
    reason on stderr when the overlay cannot be read.
    """
    parser = argparse.ArgumentParser(prog="gold_band_instances")
    parser.add_argument("command", choices=["code-nav-roots", "names-work-instance"])
    parser.add_argument("--registry", type=Path, default=KIT_ROOT / "registry" / "estate-tooling.yaml")
    parser.add_argument("--overlay", type=Path, default=DEFAULT_OVERLAY)
    args = parser.parse_args(argv)
    if args.command == "names-work-instance":
        try:
            print("yes" if names_work_instance(args.overlay) else "no")
        except (InstanceError, OSError) as exc:
            print(f"work instance: {exc}", file=sys.stderr)
            return 1
        return 0
    try:
        registry = yaml.safe_load(args.registry.read_text())
        roots = code_nav_roots(registry, args.overlay)
    except (InstanceError, OSError, yaml.YAMLError) as exc:
        print(f"code-nav roots: {exc}", file=sys.stderr)
        return 1
    for root in roots:
        print(root)
    return 0


if __name__ == "__main__":
    sys.exit(main())
