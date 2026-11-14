#!/usr/bin/env python3
"""The live LiteLLM model catalogue: fetch it, read it, and say what a route reaches.

The litellm repository's sync probes every provider route's models hourly and
publishes the result as ``model-catalog.json`` in the private repository
JorisJonkers-dev/litellm-model-catalog: per provider route, each model's status
(``usable``, ``blocked`` with the provider's own reason, ``unknown``,
``budget-exhausted``, ``error``, ``not-chat``, ``unprobed``, ``pending``) and
whether it is free, plus the canonical groups the proxy serves.

Setup fetches it (``fetch``) into the live copy,
``$XDG_CACHE_HOME/agent-kit/litellm-catalogue.json`` (else
``~/.cache/agent-kit/litellm-catalogue.json``): the published file verbatim, so
its mtime is when it was fetched and its own ``updated_at`` is when it was
probed. The repository is private, so the fetch goes through ``gh api`` with
the owner's GitHub login. ``registry/litellm-catalogue.json`` is a trimmed
snapshot in the same shape, the fallback when there is no live copy.

``load_catalogue`` never fails: a missing, unreadable or stale live copy is a
warning, and the snapshot stands in. The model gateway (node and Rust) reads
the same live copy itself; this module is the reader for setup's own writers
(the Pi providers per route).

Usage::

    python3 scripts/litellm_catalogue.py fetch [--dest PATH]
    python3 scripts/litellm_catalogue.py show [--provider vibe]

Exit status: ``fetch`` exits 0 when the live copy was written and 1 when it
was not (setup reports that as a warning and goes on); ``show`` exits 0. Stdlib only.
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import tempfile
from collections.abc import Callable, Iterable, Mapping, Sequence
from dataclasses import dataclass, field
from datetime import UTC, datetime, timedelta
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parent.parent
SNAPSHOT = ROOT / "registry" / "litellm-catalogue.json"
ROUTES = ROOT / "registry" / "generated" / "litellm" / "routes.json"
CATALOGUE_REPO = "JorisJonkers-dev/litellm-model-catalog"
CATALOGUE_FILE = "model-catalog.json"
# The sync runs hourly; a catalogue two days old means the sync or the fetch has stopped.
STALE_AFTER = timedelta(hours=48)
# The access group every key holds; a group that is not public is in it.
CANONICAL = "canonical"

Runner = Callable[[Sequence[str]], "subprocess.CompletedProcess[str]"]


@dataclass(frozen=True)
class CatalogueModel:
    """One model a provider route lists, by its id without the route prefix."""

    id: str
    status: str
    reason: str
    free: bool
    modality: str

    @property
    def usable(self) -> bool:
        return self.status == "usable"


@dataclass(frozen=True)
class CanonicalGroup:
    """One canonical model name the proxy serves, and the provider routes behind it."""

    name: str
    modality: str
    free: bool
    public: bool
    routes: tuple[str, ...]


@dataclass(frozen=True)
class Catalogue:
    """The catalogue read, where it came from, and what was wrong on the way."""

    data: Mapping[str, Any]
    source: str
    warnings: tuple[str, ...] = field(default_factory=tuple)


def live_catalogue_path(env: Mapping[str, str] | None = None) -> Path:
    """``$XDG_CACHE_HOME/agent-kit/litellm-catalogue.json``, else under ``~/.cache``; a relative XDG is ignored."""
    env = os.environ if env is None else env
    xdg = env.get("XDG_CACHE_HOME", "")
    root = Path(xdg) if xdg.startswith("/") else Path(env.get("HOME", str(Path.home()))) / ".cache"
    return root / "agent-kit" / "litellm-catalogue.json"


def validate(data: object) -> str | None:
    """Why ``data`` is not a catalogue, or None when it is one: an object with ``updated_at`` and ``providers``."""
    if not isinstance(data, dict):
        return "not a JSON object"
    if not isinstance(data.get("updated_at"), str):
        return "no updated_at"
    providers = data.get("providers")
    if not isinstance(providers, dict):
        return "no providers object"
    for name, provider in providers.items():
        if not isinstance(provider, dict) or not isinstance(provider.get("models"), list):
            return f"provider {name} has no models list"
    return None


def _read(path: Path) -> tuple[dict[str, Any] | None, str | None]:
    try:
        data = json.loads(path.read_text(encoding="utf-8"))
    except FileNotFoundError:
        return None, f"no live LiteLLM catalogue at {path}: run setup to fetch it"
    except (OSError, ValueError) as error:
        return None, f"the LiteLLM catalogue at {path} cannot be read: {error}"
    problem = validate(data)
    if problem is not None:
        return None, f"the LiteLLM catalogue at {path} is not a catalogue: {problem}"
    return data, None


def updated_at(data: Mapping[str, Any]) -> datetime | None:
    """When the catalogue was probed, or None when its ``updated_at`` is not an instant."""
    try:
        moment = datetime.fromisoformat(str(data.get("updated_at", "")).replace("Z", "+00:00"))
    except ValueError:
        return None
    return moment if moment.tzinfo is not None else moment.replace(tzinfo=UTC)


def staleness(data: Mapping[str, Any], now: datetime, where: str) -> str | None:
    """A warning when the catalogue is older than STALE_AFTER or names no instant; None when it is fresh."""
    moment = updated_at(data)
    if moment is None:
        return f"the LiteLLM catalogue {where} names no updated_at instant, so its age is unknown"
    if now - moment > STALE_AFTER:
        hours = int((now - moment).total_seconds() // 3600)
        return f"the LiteLLM catalogue {where} is stale: updated {data['updated_at']}, {hours} hours ago"
    return None


def load_catalogue(
    live_path: Path | None = None,
    snapshot_path: Path | None = None,
    now: datetime | None = None,
) -> Catalogue:
    """The live copy, else the snapshot, with every problem as a warning; it never raises.

    A stale live copy is still used (it is the newest there is) and warned about. With
    neither file readable the catalogue is empty (``source`` ``none``), so a caller lists
    nothing from it rather than failing.
    """
    live_path = live_catalogue_path() if live_path is None else live_path
    snapshot_path = SNAPSHOT if snapshot_path is None else snapshot_path
    now = datetime.now(UTC) if now is None else now
    data, problem = _read(live_path)
    if data is not None:
        stale = staleness(data, now, f"at {live_path}")
        return Catalogue(data, "live", () if stale is None else (stale,))
    warnings = [problem] if problem is not None else []
    snapshot, snapshot_problem = _read(snapshot_path)
    if snapshot is None:
        warnings.append(snapshot_problem or f"no LiteLLM catalogue snapshot at {snapshot_path}")
        return Catalogue({"updated_at": "", "providers": {}, "groups": {}}, "none", tuple(warnings))
    warnings.append(f"using the snapshot {snapshot_path} (updated {snapshot['updated_at']}) instead")
    return Catalogue(snapshot, "snapshot", tuple(warnings))


def provider_models(catalogue: Catalogue, provider: str) -> list[CatalogueModel]:
    """Every model one provider route lists, in the catalogue's order; an unknown provider lists none."""
    entry = catalogue.data.get("providers", {}).get(provider)
    if not isinstance(entry, dict):
        return []
    models = []
    for model in entry.get("models", []):
        if not isinstance(model, dict) or not isinstance(model.get("id"), str):
            continue
        raw = model.get("metadata")
        meta: dict[str, Any] = raw if isinstance(raw, dict) else {}
        bare = model["id"].removeprefix(f"{provider}/")
        models.append(
            CatalogueModel(
                id=bare,
                status=str(meta.get("status", "unknown")),
                reason=str(meta.get("reason", "")),
                free=meta.get("free") is True,
                modality=str(meta.get("modality", "")),
            )
        )
    return models


def usable_ids(catalogue: Catalogue, provider: str) -> list[str]:
    """The ids one provider route lists as usable, without the route prefix."""
    return [model.id for model in provider_models(catalogue, provider) if model.usable]


def public_access_groups(routes_path: Path = ROUTES) -> frozenset[str]:
    """The access groups only ``data_scope: public`` routes hold, from routes.json: the ones that gate public groups."""
    try:
        routes = json.loads(routes_path.read_text(encoding="utf-8"))["routes"]
    except (OSError, ValueError, KeyError, TypeError):
        return frozenset()
    public: set[str] = set()
    other: set[str] = set()
    for route in routes:
        groups = set(route.get("models", []))
        (public if route.get("data_scope") == "public" else other).update(groups)
    return frozenset(public - other - {CANONICAL})


def _group_free(catalogue: Catalogue, deployments: Sequence[Mapping[str, Any]]) -> bool:
    """Whether every first-tried deployment of a group is a model its provider marks free."""
    first = [d for d in deployments if d.get("order") == min((x.get("order", 0) for x in deployments), default=0)]
    if not first:
        return False
    for deployment in first:
        route = str(deployment.get("route", ""))
        upstream = str(deployment.get("upstream_id", ""))
        known = {model.id: model for model in provider_models(catalogue, route)}
        model = known.get(upstream.removeprefix(f"{route}/"))
        if model is None or not model.free:
            return False
    return True


def canonical_groups(
    catalogue: Catalogue,
    access_groups: Iterable[str],
    public_groups: Iterable[str] | None = None,
) -> list[CanonicalGroup]:
    """The canonical model names a key holding ``access_groups`` reaches, sorted by name.

    A group that names its own ``access_groups`` in the catalogue is reached when the key
    holds one of them. Otherwise a group that is not public is in ``canonical``, and a
    public group is reached only when the key holds one of the public-only access groups
    (``public_groups``; by default the ones only ``data_scope: public`` routes hold in
    routes.json), so no group name is fixed here. A public group (a free tier) counts as
    free; another group is free when every deployment tried first is a free model.
    """
    held = set(access_groups)
    gates = public_access_groups() if public_groups is None else frozenset(public_groups)
    reached = []
    for name, group in sorted(catalogue.data.get("groups", {}).items()):
        if not isinstance(group, dict):
            continue
        public = group.get("public") is True
        own = group.get("access_groups")
        if isinstance(own, list):
            ok = bool(held & set(own))
        elif public:
            ok = bool(held & gates)
        else:
            ok = CANONICAL in held
        if not ok:
            continue
        deployments = [d for d in group.get("deployments", []) if isinstance(d, dict)]
        reached.append(
            CanonicalGroup(
                name=name,
                modality=str(group.get("modality", "")),
                free=public or _group_free(catalogue, deployments),
                public=public,
                routes=tuple(dict.fromkeys(str(d.get("route", "")) for d in deployments)),
            )
        )
    return reached


def _gh(argv: Sequence[str]) -> subprocess.CompletedProcess[str]:
    return subprocess.run(list(argv), capture_output=True, text=True, timeout=60, check=False)


def fetch(dest: Path | None = None, runner: Runner = _gh) -> str | None:
    """Writes the published catalogue to the live copy; returns why it did not, or None when it did.

    The file is checked before it replaces the old copy, and is written whole (a
    temporary file renamed over it), so a failed fetch leaves the old copy in place.
    """
    dest = live_catalogue_path() if dest is None else dest
    argv = [
        "gh",
        "api",
        "-H",
        "Accept: application/vnd.github.raw",
        f"repos/{CATALOGUE_REPO}/contents/{CATALOGUE_FILE}",
    ]
    try:
        result = runner(argv)
    except (OSError, subprocess.SubprocessError) as error:
        return f"cannot run gh to fetch the LiteLLM catalogue: {error}"
    if result.returncode != 0:
        detail = (result.stderr or "").strip().splitlines()[-1:] or ["no output"]
        return f"gh could not fetch {CATALOGUE_REPO}/{CATALOGUE_FILE}: {detail[0]}"
    try:
        data = json.loads(result.stdout)
    except ValueError as error:
        return f"{CATALOGUE_REPO}/{CATALOGUE_FILE} is not JSON: {error}"
    problem = validate(data)
    if problem is not None:
        return f"{CATALOGUE_REPO}/{CATALOGUE_FILE} is not a catalogue: {problem}"
    dest.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.NamedTemporaryFile("w", dir=dest.parent, prefix=".litellm-catalogue.", delete=False) as out:
        out.write(result.stdout)
    os.replace(out.name, dest)
    return None


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n", 1)[0])
    sub = parser.add_subparsers(dest="command", required=True)
    fetch_parser = sub.add_parser("fetch", help="fetch the published catalogue into the live copy")
    fetch_parser.add_argument("--dest", type=Path)
    show = sub.add_parser("show", help="print what the catalogue says, live copy else snapshot")
    show.add_argument("--provider")
    args = parser.parse_args(argv)
    if args.command == "fetch":
        problem = fetch(args.dest)
        if problem is not None:
            print(problem, file=sys.stderr)
            return 1
        print(f"LiteLLM catalogue written to {args.dest or live_catalogue_path()}")
        return 0
    catalogue = load_catalogue()
    for warning in catalogue.warnings:
        print(f"warning: {warning}", file=sys.stderr)
    providers = [args.provider] if args.provider else sorted(catalogue.data.get("providers", {}))
    for provider in providers:
        for model in provider_models(catalogue, provider):
            mark = " (free)" if model.free else ""
            reason = f": {model.reason}" if model.reason else ""
            print(f"{provider}/{model.id}\t{model.status}{mark}{reason}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
