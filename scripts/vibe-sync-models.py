#!/usr/bin/env python3
"""Give Mistral Vibe every Mistral chat model the key can use, and keep the list current.

Vibe lists only the models under ``[[models]]`` in its config (its built-in
default is Mistral Medium 3.5). This reads Mistral's ``/v1/models`` with
``MISTRAL_API_KEY`` and writes one entry per underlying model: Mistral lists
the same model under several ids, so they are grouped by ``name`` and the
``-latest`` id is preferred, which carries upgrades without a change here.
Left out: Vibe's own default, models without tool calling (Vibe needs them),
transcription models and deprecated ones.

A model you defined yourself wins over a synced one of the same alias, and
everything else in the config is kept. The aliases this wrote are recorded in
~/.config/agent-kit/vibe-models.json, so a model gone upstream is removed on
the next run. When the API cannot be read, the config is left as it was.

Usage::

    uv run --no-project --with tomlkit python scripts/vibe-sync-models.py --config <config.toml>
"""

from __future__ import annotations

import argparse
import json
import os
import ssl
import sys
import urllib.error
import urllib.request
from pathlib import Path
from typing import Any

import tomlkit

URL = os.environ.get("AGENT_KIT_MISTRAL_MODELS_URL", "https://api.mistral.ai/v1/models")
VIBE_DEFAULT = "mistral-vibe-cli-latest"
SYSTEM_CA = ("/etc/ssl/cert.pem", "/etc/ssl/certs/ca-certificates.crt", "/etc/pki/tls/certs/ca-bundle.crt")


def note(message: str) -> None:
    print(f"vibe-sync-models: {message}", file=sys.stderr)


def tls_context() -> ssl.SSLContext:
    context = ssl.create_default_context()
    if not context.cert_store_stats()["x509_ca"]:
        for path in SYSTEM_CA:
            if os.path.isfile(path):
                context.load_verify_locations(cafile=path)
                break
    return context


def fetch(key: str) -> list[dict[str, Any]]:
    request = urllib.request.Request(URL, headers={
        "Authorization": f"Bearer {key}",
        # Cloudflare-fronted APIs refuse the default Python-urllib agent.
        "User-Agent": "agent-kit-vibe-models/1",
    })
    with urllib.request.urlopen(request, timeout=20, context=tls_context()) as response:
        return list(json.loads(response.read())["data"])


def wanted(models: list[dict[str, Any]]) -> dict[str, dict[str, Any]]:
    """alias -> Vibe model entry, one per underlying model."""
    groups: dict[str, list[dict[str, Any]]] = {}
    for m in models:
        caps = m.get("capabilities") or {}
        if not caps.get("completion_chat") or not caps.get("function_calling"):
            continue
        if caps.get("audio_transcription") or m.get("deprecation"):
            continue
        groups.setdefault(str(m.get("name") or m["id"]), []).append(m)
    entries: dict[str, dict[str, Any]] = {}
    for group in groups.values():
        ids = {m["id"] for m in group} | {a for m in group for a in (m.get("aliases") or [])}
        if VIBE_DEFAULT in ids:
            continue
        latest = sorted((m for m in group if m["id"].endswith("-latest")), key=lambda m: (len(m["id"]), m["id"]))
        pick = (latest or sorted(group, key=lambda m: (len(m["id"]), m["id"])))[0]
        caps = pick.get("capabilities") or {}
        reasoning = bool(caps.get("reasoning"))
        entry: dict[str, Any] = {
            "name": pick["id"],
            "provider": "mistral",
            "alias": pick["id"],
            "display_name": pick["id"],
            "supports_images": bool(caps.get("vision")),
            "thinking": "high" if reasoning else "off",
            "thinking_levels": ["off", "high"] if reasoning else ["off"],
        }
        if pick.get("max_context_length"):
            entry["max_context_length"] = int(pick["max_context_length"])
        entries[pick["id"]] = entry
    return entries


def state_path() -> Path:
    return Path.home() / ".config" / "agent-kit" / "vibe-models.json"


def sync(config: Path, entries: dict[str, dict[str, Any]]) -> int:
    try:
        managed = set(json.loads(state_path().read_text()))
    except (OSError, ValueError):
        managed = set()
    doc = tomlkit.parse(config.read_text()) if config.is_file() else tomlkit.document()
    current = [m.unwrap() for m in doc.get("models") or []]
    kept = [m for m in current if str(m.get("alias") or m.get("name")) not in managed]
    yours = {str(m.get("alias") or m.get("name")) for m in kept}
    ours = {alias: e for alias, e in entries.items() if alias not in yours}
    models = tomlkit.aot()
    for entry in [*kept, *(ours[a] for a in sorted(ours))]:
        models.append(tomlkit.item(entry))
    if "models" in doc:
        del doc["models"]
    if len(models):
        doc["models"] = models
    config.parent.mkdir(parents=True, exist_ok=True)
    config.write_text(tomlkit.dumps(doc).lstrip("\n"))
    state_path().parent.mkdir(parents=True, exist_ok=True)
    state_path().write_text(json.dumps(sorted(ours)) + "\n")
    back = {str(m.get("alias")) for m in tomlkit.parse(config.read_text()).get("models") or []}
    missing = sorted(set(ours) - back)
    if missing:
        note(f"FAILED, not written: {missing}")
        return 1
    print(f"vibe-sync-models: {len(ours)} Mistral model(s) offered in {config}")
    return 0


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--config", type=Path, required=True)
    args = parser.parse_args(argv)
    key = os.environ.get("MISTRAL_API_KEY", "")
    if not key:
        note("MISTRAL_API_KEY is not set; models not synced")
        return 0
    try:
        models = fetch(key)
    except (urllib.error.URLError, OSError, ValueError, KeyError) as e:
        note(f"could not read the Mistral model list ({type(e).__name__}); models not synced")
        return 0
    return sync(args.config, wanted(models))


if __name__ == "__main__":
    raise SystemExit(main())
