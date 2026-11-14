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


def test_every_route_renders_one_provider_with_its_models() -> None:
    data = render_registry.load_registry()
    table = json.loads(render_registry.render_pi_providers(data))
    routes = data["litellm"]["routes"]
    assert table["base_url"] == data["litellm"]["base_url"]
    assert table["api"] == "openai-completions"
    assert [p["name"] for p in table["providers"]] == [f"litellm-{r['agent']}" for r in routes]
    assert [p["key_env"] for p in table["providers"]] == [r["key_env"] for r in routes]
    for provider in table["providers"]:
        assert provider["models"], provider["name"]
        assert provider["models"] == sorted(set(provider["models"]))


def test_route_models_are_the_union_of_its_access_groups() -> None:
    groups = json.loads((KIT_ROOT / render_registry.LITELLM_ACCESS_GROUPS).read_text())["groups"]
    public = render_registry.litellm_route_models(["google-free-public", "canonical"])
    assert public == sorted(set(groups["google-free-public"]) | set(groups["canonical"]))
    assert render_registry.litellm_route_models(["canonical"]) == sorted(
        groups["canonical"]
    )


def test_only_the_public_route_reaches_the_free_tier_models() -> None:
    table = json.loads(render_registry.render_pi_providers(render_registry.load_registry()))
    groups = json.loads((KIT_ROOT / render_registry.LITELLM_ACCESS_GROUPS).read_text())["groups"]
    free = set(groups["google-free-public"])
    for provider in table["providers"]:
        reaches_free = bool(free & set(provider["models"]))
        assert reaches_free == (provider["name"] == "litellm-public"), provider["name"]


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
