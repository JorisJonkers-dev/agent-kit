"""Every review and critic route defaults to a Flash-class, small or free model.

Owner rule (2026-11-26): routing goes with Flash generally, and reviews use Flash
or free models when possible. A full-size cheap model (GLM-5.3, Mistral Large,
Kimi) must never be picked for a review automatically; it stays reachable only
by naming it in a request. The two paid frontier clients are the orchestrator
and deep tier, not a cheap review slot, and are the only listed exceptions.
"""

from __future__ import annotations

import json
import re
from pathlib import Path
from typing import Any

import pytest
import yaml

KIT_ROOT = Path(__file__).resolve().parent.parent
ROUTING = KIT_ROOT / "skills/workstation/critique/routing.json"
DEFAULTS = KIT_ROOT / "skills/workstation/model-gateway/defaults.json"
REGISTRY = KIT_ROOT / "registry/estate-tooling.yaml"
TIERS = KIT_ROOT / "registry/litellm-tiers.json"

#: A model id that is Flash-class, small or free.
CHEAP = re.compile(r"flash|free|-fast|small|mini|lite", re.IGNORECASE)

#: Routing clients allowed although their model is not in the cheap class: the
#: orchestrator/deep tier, paid frontier models that a roster lists as its own `work` pool.
ROUTING_EXCEPTIONS = {
    "codex-work": "paid frontier orchestrator tier (the `work` pool), never a cheap review slot",
    "claude-work": "paid frontier orchestrator tier (the `work` pool), never a cheap review slot",
}

#: Gateway `<client>:<model>` keys allowed in a review profile for the same reason.
PROFILE_EXCEPTIONS = {
    "claude:claude-sonnet-5-5": "paid frontier orchestrator/deep tier",
    "claude:claude-haiku-5-5": "Haiku is Anthropic's small tier",
    "codex:gpt-5.5": "paid frontier orchestrator/deep tier",
}

REVIEW_PROFILES = ("cross-critic", "critique", "red-team", "refute")


def _routing() -> dict[str, Any]:
    return json.loads(ROUTING.read_text())


def _slot_clients(node: Any) -> set[str]:
    """Every client named by a stage slot, the triage and cold-read models and the fallbacks."""
    found: set[str] = set()
    if isinstance(node, dict):
        if isinstance(node.get("client"), str):
            found.add(node["client"])
        for value in node.values():
            found |= _slot_clients(value)
    elif isinstance(node, list):
        for value in node:
            found |= _slot_clients(value)
    return found


def _reviewer_clients(routing: dict[str, Any]) -> set[str]:
    names = _slot_clients(routing["stages"]) | _slot_clients(routing["triage"]) | _slot_clients(routing["cold_read"])
    for source, targets in routing["fallbacks"].items():
        names.add(source)
        names.update(targets)
    return names


def _cheap_enough(client: str, entry: dict[str, Any]) -> bool:
    return bool(entry.get("free")) or bool(CHEAP.search(entry["model"]))


def test_every_reviewer_client_in_routing_is_flash_small_or_free() -> None:
    routing = _routing()
    offenders = []
    for name in sorted(_reviewer_clients(routing)):
        entry = routing["clients"][name]
        if name in ROUTING_EXCEPTIONS or _cheap_enough(name, entry):
            continue
        offenders.append(f"{name} -> {entry['model']}")
    assert not offenders, f"review routes must name a Flash-class, small or free model: {offenders}"


def test_the_routing_exceptions_are_real_clients_with_a_reason() -> None:
    clients = _routing()["clients"]
    for name, reason in ROUTING_EXCEPTIONS.items():
        assert name in clients and reason
        assert not _cheap_enough(name, clients[name]), f"{name} is cheap now: drop its exception"


def test_the_full_model_clients_are_gone_from_routing() -> None:
    routing = _routing()
    used = _reviewer_clients(routing)
    for name in ("vibe-glm", "vibe-mistral-large", "pi-vibe-glm", "pi-vibe-mistral-large", "pi-opencode-go-glm"):
        assert name not in routing["clients"], f"{name} is the full model: renamed to its flash or small client"
        assert name not in used


def test_the_vibe_critic_agents_pin_flash_or_small_models() -> None:
    registry = yaml.safe_load(REGISTRY.read_text())
    active = [a for a in registry["vibe_critic_agents"] if a["surfaces"]]
    assert active
    for agent in active:
        assert CHEAP.search(agent["model"]), f"{agent['name']} pins {agent['model']}"
    routing = _routing()
    agents = {a["name"]: a for a in active}
    for name, entry in routing["clients"].items():
        if entry["kind"] == "vibe":
            assert agents[entry["agent"]]["model"] == entry["model"], name


def test_the_review_profiles_of_the_gateway_pick_flash_small_or_free_models() -> None:
    profiles = json.loads(DEFAULTS.read_text())["profiles"]
    offenders = []
    for profile in REVIEW_PROFILES:
        for key in profiles[profile]["models"]:
            if key in PROFILE_EXCEPTIONS:
                continue
            if not CHEAP.search(key.split(":", 1)[1]):
                offenders.append(f"{profile}: {key}")
    assert not offenders, offenders


def test_the_profile_exceptions_are_listed_in_a_review_profile() -> None:
    profiles = json.loads(DEFAULTS.read_text())["profiles"]
    listed = {key for profile in REVIEW_PROFILES for key in profiles[profile]["models"]}
    assert set(PROFILE_EXCEPTIONS) <= listed
    assert all(PROFILE_EXCEPTIONS.values())


def test_the_review_tier_runs_only_flash_small_or_free_models() -> None:
    steps = json.loads(TIERS.read_text())["tier_groups"]["tier-review"]["steps"]
    for step in steps:
        assert step["cost"] == "free" or CHEAP.search(step["model"]), step
