"""Every model the gateway defaults name resolves through LiteLLM (agent-kit#684).

The gateway's vibe and opencode clients reach their models through the LiteLLM proxy, so a
default must be an id the catalogue lists as usable on its route (a Vibe default also by one of
its `names`). A default the catalogue does not list fails here, so a stale id is found before
a delegation is refused. The catalogue is private, so `registry/litellm-catalogue.json` is a
snapshot of its usable ids per route; its `comment` says how to refresh it.
"""

from __future__ import annotations

import json
import re
from pathlib import Path

KIT_ROOT = Path(__file__).resolve().parent.parent
DEFAULTS = KIT_ROOT / "skills" / "workstation" / "model-gateway" / "defaults.json"
CATALOGUE = KIT_ROOT / "registry" / "litellm-catalogue.json"
ROUTING = KIT_ROOT / "skills" / "workstation" / "critique" / "routing.json"

# Clients whose models are not served through the LiteLLM routes: Claude (anthropic key only,
# not probed by the catalogue yet), Codex (employer account) and agy (Google's own CLI).
NOT_VIA_LITELLM = {"claude", "codex", "agy", "hermes"}


def _routes() -> dict[str, set[str]]:
    return {route: set(ids) for route, ids in json.loads(CATALOGUE.read_text())["routes"].items()}


def unresolved(models: list[dict], routes: dict[str, set[str]]) -> list[str]:
    """The defaults no usable catalogue id matches, as `client:model`."""
    missing = []
    for entry in models:
        client, model = entry["client"], entry["model"]
        if client == "vibe":
            candidates = {model, *entry.get("names", [])}
            ok = bool(candidates & routes["vibe"])
        elif client == "opencode":
            route, _, bare = model.partition("/")
            ok = route == "opencode-go" and bare in routes["opencode-go"]
        else:
            assert client in NOT_VIA_LITELLM, f"{client} has no LiteLLM route check"
            continue
        if not ok:
            missing.append(f"{client}:{model}")
    return missing


def test_every_gateway_default_resolves_in_the_catalogue() -> None:
    models = json.loads(DEFAULTS.read_text())["models"]
    assert unresolved(models, _routes()) == []


def test_an_id_the_catalogue_does_not_list_is_reported() -> None:
    models = [
        {"client": "vibe", "model": "mistral-large-4-0-3"},
        {"client": "vibe", "model": "glm-5-3", "names": ["zai-glm-5-3"]},
        {"client": "opencode", "model": "opencode-go/kimi-k3.1-pro"},
        {"client": "opencode", "model": "opencode-go/qwen3.8-flash"},
        {"client": "claude", "model": "claude-opus-5-5"},
    ]
    assert unresolved(models, _routes()) == ["vibe:mistral-large-4-0-3", "opencode:opencode-go/kimi-k3.1-pro"]


def test_the_critique_gateway_models_resolve_too() -> None:
    routing = json.loads(ROUTING.read_text())
    shipped = {(m["client"], m["model"]): m for m in json.loads(DEFAULTS.read_text())["models"]}
    entries = [
        shipped[(c["gateway"]["client"], c["gateway"]["model"])] for c in routing["clients"].values() if "gateway" in c
    ]
    assert entries and unresolved(entries, _routes()) == []


def _norm(model: str) -> str:
    """One spelling for the same model on either route: `zai-glm-5.3-flash` and `glm-5-3-flash` agree."""
    bare = re.sub(r"^zai-", "", model.split("/")[-1].replace(".", "-"))
    return re.sub(r"-\d{4}$", "", bare)


def test_no_opencode_default_is_a_model_vibe_serves() -> None:
    """As long as Vibe serves a model, OpenCode Go is not used for it."""
    models = json.loads(DEFAULTS.read_text())["models"]
    vibe = {_norm(m) for m in _routes()["vibe"]}
    clash = [m["model"] for m in models if m["client"] == "opencode" and _norm(m["model"]) in vibe]
    assert clash == []


def test_the_deprecated_mistral_medium_default_is_gone() -> None:
    models = json.loads(DEFAULTS.read_text())["models"]
    assert "mistral-medium-3.5" not in {m["model"] for m in models}
    assert not any(m["model"] == "mistral-large-4-0-3" for m in models)


def test_no_critique_gateway_entry_sends_a_vibe_served_model_to_opencode() -> None:
    vibe = {_norm(m) for m in _routes()["vibe"]}
    clients = json.loads(ROUTING.read_text())["clients"].values()
    assert [
        c["gateway"]["model"]
        for c in clients
        if c.get("gateway", {}).get("client") == "opencode" and _norm(c["gateway"]["model"]) in vibe
    ] == []
