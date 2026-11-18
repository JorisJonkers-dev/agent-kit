#!/usr/bin/env python3
"""The LiteLLM tiers: a snapshot of the tier policy, and what is derived from it.

The tier policy has one source: ``catalog/routing.json`` key ``tier_groups`` in
JorisJonkers-dev/litellm. A tier is a callable model name on the proxy
(``tier-free``, ``tier-vibe``, ``tier-go-cheap``, ``tier-strong-open``,
``tier-review``) with a ``ceiling`` (the dearest cost class it may hold),
``key_models`` (what a key must hold to call it) and ordered ``steps``. agent-kit
defines no tier of its own: ``registry/litellm-tiers.json`` is that key copied
verbatim (with where it came from), and the delegation profiles, the key lists
and the Pi providers are rendered from it.

Usage::

    python3 scripts/litellm_tiers.py --write [--ref REF]    # re-snapshot from litellm
    python3 scripts/litellm_tiers.py --check [--upstream REF]

``--check`` exits 1 when the snapshot is malformed. With ``--upstream REF`` it also
fetches that ref of the litellm repository (through ``gh api``) and exits 1 when
its ``tier_groups`` differ from the snapshot, which is how a drift is found.
Stdlib only.
"""

from __future__ import annotations

import argparse
import base64
import json
import subprocess
import sys
from collections.abc import Callable, Mapping, Sequence
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parent.parent
SNAPSHOT = ROOT / "registry" / "litellm-tiers.json"
UPSTREAM_REPO = "JorisJonkers-dev/litellm"
UPSTREAM_PATH = "catalog/routing.json"
COST_CLASSES = ("free", "vibe", "go")

Runner = Callable[[Sequence[str]], "subprocess.CompletedProcess[str]"]


class TiersError(RuntimeError):
    """The tier snapshot is malformed, or cannot be refreshed."""


def validate(data: object) -> list[str]:
    """What is wrong with a snapshot; empty when it is one."""
    problems: list[str] = []
    if not isinstance(data, dict):
        return ["not a JSON object"]
    if not isinstance(data.get("source"), dict):
        problems.append("no source object")
    groups = data.get("tier_groups")
    if not isinstance(groups, dict) or not groups:
        return [*problems, "no tier_groups object"]
    for name, tier in groups.items():
        if not isinstance(name, str) or not name.startswith("tier-"):
            problems.append(f"tier {name!r}: a tier is named tier-<name>")
            continue
        if not isinstance(tier, dict):
            problems.append(f"tier {name}: not an object")
            continue
        if tier.get("ceiling") not in COST_CLASSES:
            problems.append(f"tier {name}: ceiling must be one of {', '.join(COST_CLASSES)}")
        keys = tier.get("key_models")
        if not isinstance(keys, list) or name not in keys or not all(isinstance(k, str) for k in keys):
            problems.append(f"tier {name}: key_models must be a list of names that includes {name}")
        steps = tier.get("steps")
        if not isinstance(steps, list) or not steps:
            problems.append(f"tier {name}: no steps")
            continue
        for step in steps:
            ok = isinstance(step, dict) and (
                (set(step) == {"tier"} and step["tier"] in groups)
                or (set(step) == {"model", "cost"} and isinstance(step["model"], str) and step["cost"] in COST_CLASSES)
            )
            if not ok:
                problems.append(f"tier {name}: step {step!r} is neither a model with a cost class nor a known tier")
    return problems


def load(path: Path = SNAPSHOT) -> dict[str, Any]:
    """The snapshot, validated; raises TiersError when it is missing or malformed."""
    try:
        data = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, ValueError) as error:
        raise TiersError(f"the tier snapshot {path} cannot be read: {error}") from error
    problems = validate(data)
    if problems:
        raise TiersError(f"the tier snapshot {path} is malformed: " + "; ".join(problems))
    return data


def tier_groups(path: Path = SNAPSHOT) -> dict[str, Any]:
    return dict(load(path)["tier_groups"])


def flat_models(groups: Mapping[str, Any], name: str, seen: frozenset[str] = frozenset()) -> list[str]:
    """The model names of a tier's chain in order, tier steps spliced in, a model once."""
    out: list[str] = []
    for step in groups[name]["steps"]:
        names = (
            flat_models(groups, step["tier"], seen | {name})
            if "tier" in step and step["tier"] not in seen
            else ([] if "tier" in step else [step["model"]])
        )
        out.extend(model for model in names if model not in out)
    return out


def route_agent(groups: Mapping[str, Any], name: str, routes: Sequence[Mapping[str, Any]]) -> str:
    """The litellm route (agent id) whose key calls a tier: the one that holds the tier's name.

    A tier no route holds is one no workstation key calls (an escalation key); it is still
    named ``<tier>`` so its profile can be rendered and says why it cannot run.
    """
    for route in routes:
        if name in route.get("models", []):
            return str(route["agent"])
    return name


def key_problems(groups: Mapping[str, Any], routes: Sequence[Mapping[str, Any]]) -> list[str]:
    """Where a route that calls a tier does not hold exactly the keys the tier's ``key_models`` list."""
    problems = []
    for name, tier in groups.items():
        for route in routes:
            if not isinstance(route, dict):
                continue  # the registry check says a route is a mapping
            if name in route.get("models", []) and sorted(route["models"]) != sorted(tier["key_models"]):
                problems.append(
                    f"litellm route {route['agent']} calls {name} but holds {sorted(route['models'])}, "
                    f"not the tier's key_models {sorted(tier['key_models'])}"
                )
    return problems


def _gh(argv: Sequence[str]) -> subprocess.CompletedProcess[str]:
    return subprocess.run(list(argv), capture_output=True, text=True, timeout=60, check=False)


def fetch_upstream(ref: str, runner: Runner = _gh) -> dict[str, Any]:
    """The litellm repository's ``tier_groups`` at ``ref``, as a snapshot; raises TiersError when it cannot be read."""
    argv = ["gh", "api", f"repos/{UPSTREAM_REPO}/contents/{UPSTREAM_PATH}?ref={ref}"]
    try:
        result = runner(argv)
    except (OSError, subprocess.SubprocessError) as error:
        raise TiersError(f"cannot run gh to fetch the tier policy: {error}") from error
    if result.returncode != 0:
        detail = (result.stderr or "").strip().splitlines()[-1:] or ["no output"]
        raise TiersError(f"gh could not fetch {UPSTREAM_REPO}/{UPSTREAM_PATH} at {ref}: {detail[0]}")
    try:
        listing = json.loads(result.stdout)
        routing = json.loads(base64.b64decode(listing["content"]))
        snapshot = {
            "source": {
                "repo": UPSTREAM_REPO,
                "path": UPSTREAM_PATH,
                "key": "tier_groups",
                "ref": ref,
                "blob": listing["sha"],
            },
            "tier_groups": routing["tier_groups"],
        }
    except (ValueError, KeyError, TypeError) as error:
        raise TiersError(f"{UPSTREAM_REPO}/{UPSTREAM_PATH} at {ref} has no readable tier_groups: {error}") from error
    problems = validate(snapshot)
    if problems:
        raise TiersError(f"{UPSTREAM_REPO}/{UPSTREAM_PATH} at {ref} has malformed tier_groups: " + "; ".join(problems))
    return snapshot


def render(snapshot: Mapping[str, Any]) -> str:
    return json.dumps(snapshot, indent=2) + "\n"


def drift(snapshot: Mapping[str, Any], upstream: Mapping[str, Any]) -> list[str]:
    """The tiers that differ between the snapshot and what upstream says now."""
    ours, theirs = snapshot["tier_groups"], upstream["tier_groups"]
    return sorted(name for name in {*ours, *theirs} if ours.get(name) != theirs.get(name))


def main(argv: Sequence[str] | None = None, runner: Runner = _gh) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n", 1)[0])
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--write", action="store_true", help="re-snapshot the tier policy from litellm")
    mode.add_argument(
        "--check", action="store_true", help="fail when the snapshot is malformed (or, with --upstream, stale)"
    )
    parser.add_argument("--ref", default="main", help="the litellm ref --write snapshots (default main)")
    parser.add_argument("--upstream", metavar="REF", help="with --check, compare against this litellm ref")
    parser.add_argument("--path", type=Path, default=SNAPSHOT, help=argparse.SUPPRESS)
    args = parser.parse_args(argv)
    try:
        if args.write:
            args.path.write_text(render(fetch_upstream(args.ref, runner)), encoding="utf-8")
            print(f"tier snapshot written to {args.path}")
            return 0
        snapshot = load(args.path)
        if args.upstream:
            stale = drift(snapshot, fetch_upstream(args.upstream, runner))
            if stale:
                print(
                    f"tier snapshot differs from litellm {args.upstream}: {', '.join(stale)}; "
                    f"run --write --ref {args.upstream}",
                    file=sys.stderr,
                )
                return 1
        print("tier snapshot ok")
        return 0
    except TiersError as error:
        print(str(error), file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
