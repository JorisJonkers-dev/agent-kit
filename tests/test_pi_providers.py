"""Setup writes one Pi provider per LiteLLM route into Pi's models.json (agent-kit#737).

Each route becomes the provider ``litellm-<agent>``: the proxy URL, ``openai-completions``,
the models the route's access groups serve, and an ``apiKey`` that runs agent-kit-secret
at request time. ``scripts/render_registry.py`` renders the table and the setup step;
``scripts/pi-merge-providers.py`` merges it, keeping what the owner wrote, and removes a
provider whose route is gone.
"""

from __future__ import annotations

import importlib.util
import json
import sys
from pathlib import Path
from typing import Any

import pytest

KIT_ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(KIT_ROOT / "scripts"))

import litellm_catalogue  # noqa: E402
import render_registry  # noqa: E402

SCRIPT = KIT_ROOT / "scripts" / "pi-merge-providers.py"
HELPER = "/home/owner/.local/bin/agent-kit-secret"


def _helper():
    spec = importlib.util.spec_from_file_location("pi_merge_providers", SCRIPT)
    assert spec and spec.loader
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


merge = _helper()


def _table(*routes: tuple[str, str, list[str]]) -> dict[str, Any]:
    return {
        "base_url": "https://litellm.example/v1",
        "api": "openai-completions",
        "providers": [{"name": n, "key_env": k, "models": m} for n, k, m in routes],
    }


def _run(tmp_path: Path, table: dict[str, Any], *extra: str) -> tuple[int, str]:
    providers = tmp_path / "providers.json"
    providers.write_text(json.dumps(table))
    import contextlib
    import io

    out = io.StringIO()
    with contextlib.redirect_stdout(out):
        code = merge.main([
            "--providers", str(providers),
            "--models-json", str(tmp_path / "agent" / "models.json"),
            "--state", str(tmp_path / "state.json"),
            "--secret-helper", HELPER,
            *extra,
        ])
    return code, out.getvalue().strip()


def _models_json(tmp_path: Path) -> dict[str, Any]:
    return json.loads((tmp_path / "agent" / "models.json").read_text())


# --- rendering --------------------------------------------------------------


def _ids(models: list[dict[str, Any]]) -> list[str]:
    return [model["id"] for model in models]


def test_every_route_renders_one_provider_with_its_models() -> None:
    data = render_registry.load_registry()
    table = json.loads(render_registry.render_pi_providers(data))
    routes = data["litellm"]["routes"]
    assert table["base_url"] == data["litellm"]["base_url"]
    assert table["api"] == "openai-completions"
    # A route with no usable model (local, until a local/* model is probed) has no provider.
    listed = [r for r in routes if r["agent"] not in ("local", "free")]
    assert [p["name"] for p in table["providers"]] == [f"litellm-{r['agent']}" for r in listed]
    assert [p["key_env"] for p in table["providers"]] == [r["key_env"] for r in listed]
    for provider in table["providers"]:
        assert provider["models"], provider["name"]
        assert _ids(provider["models"]) == sorted(set(_ids(provider["models"])))


def _snapshot() -> Any:
    return litellm_catalogue.load_catalogue(live_path=KIT_ROOT / "no-such-live-copy.json")


def _usable_names(catalogue: Any) -> set[str]:
    return set(litellm_catalogue.usable_group_names(catalogue))


def test_route_models_are_the_canonical_chat_groups_with_their_context_window() -> None:
    catalogue = _snapshot()
    groups = catalogue.data["groups"]
    models = render_registry.litellm_route_models(["canonical"])
    withheld = litellm_catalogue.WITHHELD_ROUTES
    assert _ids(models) == sorted(
        n
        for n, g in groups.items()
        if g["modality"] == "chat"
        and n in _usable_names(catalogue)
        and not g["public"]
        and not {d["route"] for d in g["deployments"]} & withheld
    )
    by_id = {model["id"]: model for model in models}
    reached = litellm_catalogue.canonical_groups(catalogue, ["canonical"], [])
    expected = {g.name: g.context_length for g in reached if g.modality == "chat"}
    for name, length in expected.items():
        assert by_id[name].get("contextWindow") == length
    assert any("contextWindow" in model for model in models)
    assert all("cost" not in model for model in models), "a paid model carries no price: the catalogue has none"


def test_no_workstation_route_lists_a_claude_model() -> None:
    table = json.loads(render_registry.render_pi_providers(render_registry.load_registry()))
    listed = [m["id"] for p in table["providers"] for m in p["models"]]
    assert listed and not [i for i in listed if i.startswith("claude-")]


def test_the_openrouter_route_reaches_the_opencode_go_free_ids_at_zero_cost() -> None:
    catalogue = litellm_catalogue.Catalogue(
        {
            "updated_at": "2026-11-15T12:00:00Z",
            "providers": {
                "opencode-go": {
                    "models": [{"id": "opencode-go/beam-free", "metadata": {"status": "usable", "free": True}}]
                }
            },
            "groups": {
                "opencode-go-free/beam-1.0": {
                    "modality": "chat",
                    "public": True,
                    "deployments": [{"route": "opencode-go", "upstream_id": "beam-free", "order": 1}],
                }
            },
        },
        "live",
    )
    route = next(r for r in render_registry.load_registry()["litellm"]["routes"] if r["agent"] == "openrouter")
    assert "opencode-go-free-public" in route["models"]
    reached = render_registry.litellm_route_models(route["models"], catalogue)
    zero = {"input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0}
    assert [(m["id"], m.get("cost")) for m in reached] == [("opencode-go-free/beam-1.0", zero)]
    # The public key holds canonical and Google's free tier, never the OpenCode free group.
    assert render_registry.litellm_route_models(["google-free-public", "canonical"], catalogue) == []


def test_the_free_route_reaches_every_free_group_and_nothing_with_a_paid_fallback() -> None:
    def _usable(route: str, upstream: str) -> dict[str, Any]:
        return {"id": f"{route}/{upstream}", "metadata": {"status": "usable", "free": route == "openrouter"}}

    def _group(route: str, upstream: str, own: list[str] | None) -> dict[str, Any]:
        group: dict[str, Any] = {
            "modality": "chat",
            "public": True,
            "deployments": [{"route": route, "upstream_id": upstream, "order": 1}],
        }
        if own:
            group["access_groups"] = own
        return group

    catalogue = litellm_catalogue.Catalogue(
        {
            "updated_at": "2026-11-16T12:00:00Z",
            "providers": {
                "openrouter": {"models": [_usable("openrouter", "glm:free")]},
                "google": {"models": [_usable("google", "gemini-2.5-flash")]},
                "vibe": {"models": [_usable("vibe", "gemini-2.5-flash")]},
            },
            "groups": {
                "openrouter/glm": _group("openrouter", "glm:free", ["openrouter-free-public", "free"]),
                "google-free/gemini-2.5-flash": _group("google", "gemini-2.5-flash", ["free"]),
                # Google's free tier with the paid deployment behind it: never in the free group.
                "public/gemini-2.5-flash": {
                    **_group("google", "gemini-2.5-flash", None),
                    "deployments": [
                        {"route": "google", "upstream_id": "gemini-2.5-flash", "order": 1},
                        {"route": "vibe", "upstream_id": "gemini-2.5-flash", "order": 2},
                    ],
                },
            },
        },
        "live",
    )
    registry = render_registry.load_registry()
    route = next(r for r in registry["litellm"]["routes"] if r["agent"] == "free")
    assert route["models"] == ["free"] and route["data_scope"] == "public" and route["budget_eur"] is None
    reached = render_registry.litellm_route_models(route["models"], catalogue, route)
    assert [m["id"] for m in reached] == ["google-free/gemini-2.5-flash", "openrouter/glm"]


def test_free_groups_carry_zero_cost_and_only_the_matching_public_route_reaches_them() -> None:
    table = json.loads(render_registry.render_pi_providers(render_registry.load_registry()))
    providers = {p["name"]: p["models"] for p in table["providers"]}
    zero = {"input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0}
    free_openrouter = [m for m in providers["litellm-openrouter"] if m["id"].startswith("openrouter/")]
    assert free_openrouter and all(m["cost"] == zero for m in free_openrouter)
    assert not [m for m in providers["litellm-openrouter"] if m["id"].startswith("public/")]
    assert [m for m in providers["litellm-public"] if m["id"].startswith("public/")]
    assert not [m for m in providers["litellm-public"] if m["id"].startswith("openrouter/")]
    assert not [m for m in providers["litellm-vibe"] if "/" in m["id"]]


def test_rendering_ignores_the_live_cache_and_never_fetches(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    before = render_registry.render_pi_providers(render_registry.load_registry())
    live = tmp_path / "agent-kit" / "litellm-catalogue.json"
    live.parent.mkdir()
    live.write_text(json.dumps({"updated_at": "2099-01-01T00:00:00Z", "providers": {}, "groups": {}}))
    monkeypatch.setenv("XDG_CACHE_HOME", str(tmp_path))

    def refuse(*_args: Any, **_kwargs: Any) -> None:
        raise AssertionError("rendering must not run a process")

    monkeypatch.setattr(render_registry.litellm_catalogue.subprocess, "run", refuse)
    assert render_registry.render_pi_providers(render_registry.load_registry()) == before


def test_a_catalogue_that_cannot_be_read_stops_the_render(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(render_registry.litellm_catalogue, "SNAPSHOT", tmp_path / "missing.json")
    with pytest.raises(render_registry.RegistryError, match="snapshot cannot be read"):
        render_registry.litellm_route_models(["canonical"])


def test_an_unknown_access_group_is_refused() -> None:
    with pytest.raises(render_registry.RegistryError, match="not-a-group"):
        render_registry.litellm_route_models(["not-a-group"])


def test_setup_runs_the_merge_after_installing_pi() -> None:
    script = (KIT_ROOT / render_registry.SETUP_SCRIPT).read_text()
    install = script.index('ok "pi: installed at its pins"')
    step = script.index("scripts/pi-merge-providers.py")
    assert install < step < script.index("# Google Antigravity CLI")
    assert '--secret-helper "${HOME}/.local/bin/agent-kit-secret"' in script
    assert "pi_providers_args+=(--check)" in script


def test_the_rendered_table_is_checked_in() -> None:
    data = render_registry.load_registry()
    assert (KIT_ROOT / render_registry.PI_PROVIDERS).read_text() == render_registry.render_pi_providers(data)


# --- merging ------------------------------------------------------------------


def test_a_model_entry_keeps_its_context_window_and_cost(tmp_path: Path) -> None:
    zero = {"input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0}
    entry = {"id": "free-one", "contextWindow": 128000, "cost": zero}
    desired = merge.desired_providers(
        {"base_url": "https://x/v1", "api": "openai-completions", "providers": [
            {"name": "litellm-a", "key_env": "K", "models": [entry, "plain"]}]},
        HELPER,
    )
    assert desired["litellm-a"]["models"] == [entry, {"id": "plain"}]


def test_a_fresh_models_json_gets_every_route(tmp_path: Path) -> None:
    code, said = _run(tmp_path, _table(("litellm-vibe", "LITELLM_VIBE_KEY", ["glm-5.3", "mistral-small"])))
    assert code == 0 and said == "added litellm-vibe"
    assert _models_json(tmp_path)["providers"]["litellm-vibe"] == {
        "baseUrl": "https://litellm.example/v1",
        "api": "openai-completions",
        "apiKey": f"!{HELPER} get LITELLM_VIBE_KEY",
        "models": [{"id": "glm-5.3"}, {"id": "mistral-small"}],
    }


def test_the_owners_own_entries_are_kept(tmp_path: Path) -> None:
    path = tmp_path / "agent" / "models.json"
    path.parent.mkdir(parents=True)
    own = {"baseUrl": "http://localhost:11434/v1", "api": "openai-completions", "apiKey": "x", "models": [{"id": "q"}]}
    path.write_text(json.dumps({"providers": {"ollama": own}, "modelOverrides": {"a": {"name": "b"}}}))
    _run(tmp_path, _table(("litellm-vibe", "LITELLM_VIBE_KEY", ["glm-5.3"])))
    data = _models_json(tmp_path)
    assert data["providers"]["ollama"] == own
    assert data["modelOverrides"] == {"a": {"name": "b"}}
    assert "litellm-vibe" in data["providers"]


def test_a_second_run_converges(tmp_path: Path) -> None:
    table = _table(("litellm-vibe", "LITELLM_VIBE_KEY", ["glm-5.3"]))
    _run(tmp_path, table)
    before = (tmp_path / "agent" / "models.json").read_text()
    code, said = _run(tmp_path, table)
    assert code == 0 and said == "in place"
    assert (tmp_path / "agent" / "models.json").read_text() == before


def test_a_gone_route_is_removed_but_an_unrecorded_provider_is_not(tmp_path: Path) -> None:
    _run(tmp_path, _table(("litellm-vibe", "K1", ["a"]), ("litellm-old", "K2", ["b"])))
    path = tmp_path / "agent" / "models.json"
    data = json.loads(path.read_text())
    data["providers"]["litellm-mine"] = {"baseUrl": "https://x/v1", "api": "openai-completions", "models": []}
    path.write_text(json.dumps(data))
    code, said = _run(tmp_path, _table(("litellm-vibe", "K1", ["a", "c"])))
    assert code == 0 and said == "updated litellm-vibe; removed litellm-old"
    providers = _models_json(tmp_path)["providers"]
    assert set(providers) == {"litellm-vibe", "litellm-mine"}
    assert providers["litellm-vibe"]["models"] == [{"id": "a"}, {"id": "c"}]


def test_a_hand_edit_to_a_setup_provider_is_put_back(tmp_path: Path) -> None:
    table = _table(("litellm-vibe", "LITELLM_VIBE_KEY", ["glm-5.3"]))
    _run(tmp_path, table)
    path = tmp_path / "agent" / "models.json"
    data = json.loads(path.read_text())
    data["providers"]["litellm-vibe"]["baseUrl"] = "https://elsewhere/v1"
    path.write_text(json.dumps(data))
    _, said = _run(tmp_path, table)
    assert said == "updated litellm-vibe"
    assert _models_json(tmp_path)["providers"]["litellm-vibe"]["baseUrl"] == "https://litellm.example/v1"


def test_check_reports_and_writes_nothing(tmp_path: Path) -> None:
    code, said = _run(tmp_path, _table(("litellm-vibe", "K", ["a"])), "--check")
    assert code == 0 and said == "would be added litellm-vibe"
    assert not (tmp_path / "agent" / "models.json").exists()
    assert not (tmp_path / "state.json").exists()


def test_a_models_json_that_does_not_parse_is_left_alone(tmp_path: Path) -> None:
    path = tmp_path / "agent" / "models.json"
    path.parent.mkdir(parents=True)
    path.write_text("{ not json")
    code, _ = _run(tmp_path, _table(("litellm-vibe", "K", ["a"])))
    assert code == 2
    assert path.read_text() == "{ not json"


def test_a_models_json_with_odd_providers_is_left_alone(tmp_path: Path) -> None:
    path = tmp_path / "agent" / "models.json"
    path.parent.mkdir(parents=True)
    path.write_text('{"providers": []}')
    code, _ = _run(tmp_path, _table(("litellm-vibe", "K", ["a"])))
    assert code == 2
    assert path.read_text() == '{"providers": []}'


def test_the_helper_path_is_quoted_in_the_key_command() -> None:
    entry = merge.desired_providers(_table(("litellm-vibe", "K", ["a"])), "/home/a b/agent-kit-secret")
    assert entry["litellm-vibe"]["apiKey"] == "!'/home/a b/agent-kit-secret' get K"


def _probe(route: str, upstream: str, status: str) -> dict[str, Any]:
    return {"id": f"{route}/{upstream}", "metadata": {"status": status, "modality": "chat"}}


def _deployed(name: str, route: str, upstream: str, public: bool = False) -> dict[str, Any]:
    deployment = {"route": route, "upstream_id": upstream, "order": 1}
    return {"modality": "chat", "public": public, "deployments": [deployment]}


def test_a_group_is_listed_only_when_a_deployment_is_probed_usable() -> None:
    catalogue = litellm_catalogue.Catalogue(
        {
            "providers": {
                "vibe": {"models": [_probe("vibe", "ok", "usable"), _probe("vibe", "bad", "blocked"),
                                    _probe("vibe", "err", "error"), _probe("vibe", "what", "unknown")]},
                "opencode-go": {"models": [_probe("opencode-go", "fallback", "usable")]},
            },
            "groups": {
                "ok": _deployed("ok", "vibe", "ok"),
                "bad": _deployed("bad", "vibe", "bad"),
                "err": _deployed("err", "vibe", "err"),
                "what": _deployed("what", "vibe", "what"),
                "unprobed": _deployed("unprobed", "vibe", "nothing"),
                "second": {"modality": "chat", "public": False, "deployments": [
                    {"route": "vibe", "upstream_id": "bad", "order": 1},
                    {"route": "opencode-go", "upstream_id": "fallback", "order": 2}]},
            },
        },
        "live",
    )
    assert _ids(render_registry.litellm_route_models(["canonical"], catalogue)) == ["ok", "second"]


def _scoped_catalogue() -> Any:
    return litellm_catalogue.Catalogue(
        {
            "providers": {
                "vibe": {"models": [_probe("vibe", "paid", "usable")]},
                "local": {"models": [_probe("local", "qwen", "usable")]},
                "google": {"models": [_probe("google", "free", "usable")]},
            },
            "groups": {
                "paid": _deployed("paid", "vibe", "paid"),
                "qwen": _deployed("qwen", "local", "qwen"),
                "free": _deployed("free", "google", "free", public=True),
            },
        },
        "live",
    )


def test_the_local_route_lists_only_its_own_models_and_the_public_route_only_public_groups() -> None:
    catalogue = _scoped_catalogue()
    routes = {r["agent"]: r for r in render_registry.load_registry()["litellm"]["routes"]}
    local = render_registry.litellm_route_models(["canonical", "local/*"], catalogue, routes["local"])
    assert _ids(local) == ["qwen"]
    public = render_registry.litellm_route_models(["google-free-public", "canonical"], catalogue, routes["public"])
    assert _ids(public) == ["free"]
    assert _ids(render_registry.litellm_route_models(["canonical"], catalogue, routes["vibe"])) == ["paid", "qwen"]


def test_a_route_with_nothing_usable_has_no_pi_provider(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(render_registry, "litellm_route_models", lambda *_a, **_k: [])
    table = json.loads(render_registry.render_pi_providers(render_registry.load_registry()))
    assert table["providers"] == []


def test_the_rendered_table_lists_nothing_unusable_or_cloud_on_local_and_public() -> None:
    catalogue = _snapshot()
    usable = _usable_names(catalogue)
    table = json.loads(render_registry.render_pi_providers(render_registry.load_registry()))
    providers = {p["name"]: p["models"] for p in table["providers"]}
    assert "litellm-local" not in providers
    for name, models in providers.items():
        assert set(_ids(models)) <= usable, name
    assert all(catalogue.data["groups"][i]["public"] for i in _ids(providers["litellm-public"]))
