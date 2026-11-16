"""Delegation profiles: registry/delegation-profiles.yaml rendered into the
model gateway's defaults.

One named profile owns the models a delegation may use and its tool scope
(#773, #774). The registry is validated against its JSON schema and against the
gateway's catalogue (every model is a ``<client>:<model>`` of defaults.json),
then rendered into the ``"profiles"`` section of
skills/workstation/model-gateway/defaults.json, which the Node runner and the
Rust gateway both read. ``render_registry.py --check`` fails when that section
is stale.
"""

from __future__ import annotations

import json
import re
from pathlib import Path
from typing import Any

import validate_manifest
import yaml

KIT_ROOT = Path(__file__).resolve().parent.parent
PROFILES_PATH = KIT_ROOT / "registry" / "delegation-profiles.yaml"
SCHEMA_PATH = KIT_ROOT / "registry" / "delegation-profiles.schema.json"
DEFAULTS_PATH = Path("skills") / "workstation" / "model-gateway" / "defaults.json"

TOOL_KINDS = ("mcp", "skills", "plugins")
FIELD_ORDER = (
    "description",
    "models",
    "pin",
    "ladder",
    "distinct_families",
    "exclude_writer_family",
    "tools",
    "max_spend_usd",
    "unenforced_clients",
)

# The block of defaults.json the render owns: from its key to the line before "settings".
PROFILES_BLOCK = re.compile(r'(?s)  "profiles": \{\n.*?\n  \},\n(?=  "settings")')
INSERT_BEFORE = '  "settings": {'


class ProfilesError(RuntimeError):
    """The profiles registry is malformed in a way that would render a broken default."""


def load_profiles(path: Path = PROFILES_PATH, schema_path: Path = SCHEMA_PATH) -> dict[str, Any]:
    data = yaml.safe_load(path.read_text())
    if not isinstance(data, dict):
        raise ProfilesError(f"{path} must be a mapping")
    schema = json.loads(schema_path.read_text())
    try:
        validate_manifest.validate_json_schema(data, schema, "delegation-profiles", schema)
    except AssertionError as exc:
        raise ProfilesError(f"{path.name}: {exc}") from exc
    return data


def catalogue_keys(defaults: dict[str, Any]) -> set[str]:
    """Every ``<client>:<model>`` the catalogue states; an entry for ``pi`` stands for each ``pi-*`` client."""
    clients = set(defaults["clients"])
    keys: set[str] = set()
    for entry in defaults["models"]:
        client = entry["client"]
        members = [client] if client in clients else sorted(c for c in clients if c.startswith(f"{client}-"))
        keys.update(f"{member}:{entry['model']}" for member in members or [client])
    return keys


def validate(data: dict[str, Any], defaults: dict[str, Any]) -> None:
    known = catalogue_keys(defaults)
    for name, profile in data["profiles"].items():
        for kind in ("models", "ladder"):
            for key in profile.get(kind, []):
                if key not in known:
                    raise ProfilesError(f"profile {name}: {kind} names {key}, which is no model of the catalogue")
        for kind in ("pin", "ladder"):
            members = [profile[kind]] if kind == "pin" and kind in profile else profile.get(kind, [])
            for key in members:
                if key not in profile["models"]:
                    raise ProfilesError(f"profile {name}: {kind} names {key}, which is not one of its models")
        if profile.get("distinct_families", 1) > len({family_of(defaults, key) for key in profile["models"]}):
            raise ProfilesError(f"profile {name}: distinct_families is more than the families of its models")
        if profile.get("exclude_writer_family") and len({family_of(defaults, key) for key in profile["models"]}) < 2:
            raise ProfilesError(f"profile {name}: it leaves out the writer's family but its models span only one")


def family_of(defaults: dict[str, Any], key: str) -> str:
    client, model = key.split(":", 1)
    for entry in defaults["models"]:
        if entry["model"] == model and (entry["client"] == client or client.startswith(f"{entry['client']}-")):
            return str(entry["family"])
    raise ProfilesError(f"{key} is no model of the catalogue")


def _scalar(value: Any) -> str:
    return json.dumps(value, ensure_ascii=False)


def _value(value: Any, indent: int) -> str:
    pad = " " * indent
    if isinstance(value, dict):
        if all(isinstance(item, list) for item in value.values()):
            return "{" + ", ".join(f"{_scalar(key)}: {_value(item, 0)}" for key, item in value.items()) + "}"
        members = [f'{pad}  {_scalar(key)}: {_value(item, indent + 2)}' for key, item in value.items()]
        return "{\n" + ",\n".join(members) + f"\n{pad}}}"
    if isinstance(value, list):
        return "[" + ", ".join(_scalar(item) for item in value) + "]"
    return _scalar(value)


def rendered_profile(profile: dict[str, Any]) -> dict[str, Any]:
    """One profile in the gateway's field order, its tool scope written out (deny-all where left out)."""
    tools = profile.get("tools") or {}
    out: dict[str, Any] = {}
    for field in FIELD_ORDER:
        if field == "tools":
            out["tools"] = {kind: tools.get(kind, []) for kind in TOOL_KINDS}
        elif field == "description":
            out[field] = " ".join(str(profile[field]).split())
        elif field in profile:
            out[field] = profile[field]
    return out


def render_block(data: dict[str, Any]) -> str:
    body = {name: rendered_profile(profile) for name, profile in data["profiles"].items()}
    return f'  "profiles": {_value(body, 2)},\n'


def render_defaults(data: dict[str, Any], current: str) -> str:
    """``current`` (defaults.json) with its profiles section replaced by the rendered one."""
    block = render_block(data)
    if PROFILES_BLOCK.search(current):
        return PROFILES_BLOCK.sub(lambda _match: block, current, count=1)
    if current.count(INSERT_BEFORE) != 1:
        raise ProfilesError("defaults.json has no settings section to put the profiles before")
    return current.replace(INSERT_BEFORE, block + INSERT_BEFORE, 1)
