"""The delegation profiles registry: its schema, its checks against the gateway's catalogue, and its render
into the model gateway's defaults (#773, #774)."""

from __future__ import annotations

import copy
import json
import sys
from pathlib import Path

import pytest
import yaml

KIT_ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(KIT_ROOT / "scripts"))

import delegation_profiles  # noqa: E402
import render_registry  # noqa: E402

DEFAULTS = KIT_ROOT / delegation_profiles.DEFAULTS_PATH


def _defaults() -> dict:
    return json.loads(DEFAULTS.read_text())


def _registry() -> dict:
    return delegation_profiles.load_profiles()


def test_the_registry_states_the_four_seed_profiles_against_the_catalogue():
    data = _registry()
    assert sorted(data["profiles"]) == [
        "cheap-research",
        "critique",
        "cross-critic",
        "implement",
        "implement-cheap",
        "public-gemini",
        "red-team",
        "refute",
        "research",
        "triage",
    ]
    delegation_profiles.validate(data, _defaults())


def test_the_defaults_carry_exactly_what_the_registry_renders():
    rendered = delegation_profiles.render_defaults(_registry(), DEFAULTS.read_text(), render_registry._tier_profiles())
    assert rendered == DEFAULTS.read_text()
    section = json.loads(rendered)["profiles"]
    assert section["cheap-research"]["tools"] == {"mcp": [], "skills": [], "plugins": []}
    assert section["implement-cheap"]["ladder"][0] == "pi-vibe:deepseek-v4.1-flash"


def test_the_render_check_covers_the_profiles_section():
    stale = DEFAULTS.read_text().replace('"cheap-research"', '"cheap-lookups"')
    rendered = delegation_profiles.render_defaults(_registry(), stale)
    assert '"cheap-research"' in rendered
    rendered_files = render_registry.artifacts(render_registry.load_registry())
    assert delegation_profiles.DEFAULTS_PATH in rendered_files


def test_the_section_is_added_before_settings_when_the_defaults_have_none():
    without = delegation_profiles.PROFILES_BLOCK.sub("", DEFAULTS.read_text())
    assert '"profiles"' not in without
    tiers = render_registry._tier_profiles()
    assert delegation_profiles.render_defaults(_registry(), without, tiers) == DEFAULTS.read_text()
    with pytest.raises(delegation_profiles.ProfilesError, match="no settings section"):
        delegation_profiles.render_defaults(_registry(), "{}")


def _write(tmp_path: Path, data: dict) -> Path:
    path = tmp_path / "profiles.yaml"
    path.write_text(yaml.safe_dump(data))
    return path


@pytest.mark.parametrize(
    ("change", "message"),
    [
        (lambda d: d["profiles"]["cheap-research"].update(surprise=1), "surprise"),
        (lambda d: d["profiles"]["cheap-research"].pop("description"), "description"),
        (lambda d: d["profiles"]["cheap-research"].update(models=[]), "at least 1"),
        (lambda d: d["profiles"]["cheap-research"].update(max_spend_usd=0), "above 0"),
        (lambda d: d["profiles"]["cheap-research"].update(tools={"shell": []}), "shell"),
        (lambda d: d["profiles"]["cheap-research"].update(tools={"mcp": "all"}), "alternatives"),
        (lambda d: d["profiles"].update({"Bad Name": d["profiles"]["cheap-research"]}), "Bad Name"),
        (lambda d: d.update(version=2), "version"),
    ],
)
def test_the_schema_refuses_a_malformed_registry(tmp_path, change, message):
    data = copy.deepcopy(_registry())
    change(data)
    with pytest.raises(delegation_profiles.ProfilesError, match=message):
        delegation_profiles.load_profiles(_write(tmp_path, data))


def test_a_registry_that_is_not_a_mapping_is_refused(tmp_path):
    path = tmp_path / "profiles.yaml"
    path.write_text("- a\n")
    with pytest.raises(delegation_profiles.ProfilesError, match="must be a mapping"):
        delegation_profiles.load_profiles(path)


@pytest.mark.parametrize(
    ("change", "message"),
    [
        (lambda p: p["cheap-research"]["models"].append("pi-vibe:no-such-model"), "no model of the catalogue"),
        (lambda p: p["implement-cheap"].update(pin="pi-vibe:ministral-8b"), "pin names pi-vibe:ministral-8b"),
        (lambda p: p["implement-cheap"].update(ladder=["pi-vibe:ministral-8b"]), "ladder names pi-vibe:ministral-8b"),
        (lambda p: p["cheap-research"].update(distinct_families=9), "more than the families"),
        (
            lambda p: p["public-gemini"].update(
                exclude_writer_family=True, models=["pi-free:free/gemini-3.6-flash"]
            ),
            "only one",
        ),
    ],
)
def test_the_checks_refuse_a_profile_the_catalogue_cannot_serve(change, message):
    data = copy.deepcopy(_registry())
    change(data["profiles"])
    with pytest.raises(delegation_profiles.ProfilesError, match=message):
        delegation_profiles.validate(data, _defaults())


def test_a_model_the_catalogue_does_not_state_has_no_family():
    with pytest.raises(delegation_profiles.ProfilesError, match="no model of the catalogue"):
        delegation_profiles.family_of(_defaults(), "pi-vibe:nothing")


def test_every_pi_client_stands_for_its_group_entries():
    keys = delegation_profiles.catalogue_keys(_defaults())
    assert "pi-vibe:glm-5.3" in keys and "pi-opencode-go:glm-5.3" in keys and "claude:claude-opus-5-5" in keys


def test_the_description_is_folded_and_the_scope_written_out():
    out = delegation_profiles.rendered_profile({"description": "a\n  b", "models": ["x:y"], "tools": {"skills": "any"}})
    assert out == {"description": "a b", "models": ["x:y"], "tools": {"mcp": [], "skills": "any", "plugins": []}}


TIERS = ["tier-free", "tier-vibe", "tier-go-cheap", "tier-strong-open", "tier-review"]


def _routes() -> list[dict]:
    return render_registry.load_registry()["litellm"]["routes"]


def test_the_snapshot_states_the_five_tiers_and_splices_tier_steps():
    groups = delegation_profiles.tier_groups()
    assert list(groups) == TIERS
    assert groups["tier-free"]["ceiling"] == "free"
    assert delegation_profiles.flat_models(groups, "tier-go-cheap") == [
        "opencode-go-tier/deepseek-v4.1-flash",
        "opencode-go-tier/glm-5.3-flash",
        "opencode-go-tier/mimo-v2.6-flash",
    ]
    # tier-vibe ends in tier-go-cheap, spliced in with a model once; a tier that names itself does not loop.
    vibe = delegation_profiles.flat_models(groups, "tier-vibe")
    assert vibe[-3:] == delegation_profiles.flat_models(groups, "tier-go-cheap") and len(vibe) == len(set(vibe))
    looped = {"tier-a": {"steps": [{"tier": "tier-a"}, {"model": "m", "cost": "free"}], "key_models": ["tier-a"]}}
    assert delegation_profiles.flat_models(looped, "tier-a") == ["m"]


def test_a_missing_or_malformed_tier_snapshot_is_refused(tmp_path):
    with pytest.raises(delegation_profiles.ProfilesError, match="cannot be read"):
        delegation_profiles.tier_groups(tmp_path / "absent.json")
    for text, message in (
        ('{"tier_groups": {}}', "has no tiers"),
        ('{"tier_groups": {"tier-x": 3}}', "no key_models and steps"),
        ('{"tier_groups": {"tier-x": {"key_models": [], "steps": "x"}}}', "no key_models and steps"),
    ):
        path = tmp_path / "bad.json"
        path.write_text(text)
        with pytest.raises(delegation_profiles.ProfilesError, match=message):
            delegation_profiles.tier_groups(path)


def test_a_route_that_calls_a_tier_holds_exactly_its_key_models():
    groups = delegation_profiles.tier_groups()
    assert delegation_profiles.key_problems(groups, _routes()) == []
    wrong = [{"agent": "x", "models": ["tier-vibe", "canonical"]}]
    assert "calls tier-vibe but holds" in delegation_profiles.key_problems(groups, wrong)[0]


def test_each_tier_is_a_profile_on_the_route_that_holds_it():
    groups = delegation_profiles.tier_groups()
    profiles = delegation_profiles.tier_profiles(groups, _routes())
    assert {name: profile["models"] for name, profile in profiles.items()} == {
        "tier-free": ["pi-free:tier-free"],
        "tier-vibe": ["pi-tier-vibe:tier-vibe"],
        "tier-go-cheap": ["pi-tier-go-cheap:tier-go-cheap"],
        "tier-strong-open": ["pi-tier-strong-open:tier-strong-open"],
        "tier-review": ["pi-tier-review:tier-review"],
    }
    assert all(profile["tier"] == name for name, profile in profiles.items())
    # They are in the rendered defaults, deny-all on tools.
    rendered = json.loads((KIT_ROOT / delegation_profiles.DEFAULTS_PATH).read_text())["profiles"]
    assert rendered["tier-free"]["tier"] == "tier-free"
    assert rendered["tier-free"]["tools"] == {"mcp": [], "skills": [], "plugins": []}
    delegation_profiles.validate(
        delegation_profiles.load_profiles(),
        json.loads((KIT_ROOT / delegation_profiles.DEFAULTS_PATH).read_text()),
        profiles,
    )


def test_a_declared_profile_may_not_take_a_tiers_name_or_a_model_the_catalogue_lacks():
    defaults = json.loads((KIT_ROOT / delegation_profiles.DEFAULTS_PATH).read_text())
    data = delegation_profiles.load_profiles()
    profiles = delegation_profiles.tier_profiles(delegation_profiles.tier_groups(), _routes())
    clash = copy.deepcopy(data)
    clash["profiles"]["tier-free"] = clash["profiles"]["research"]
    with pytest.raises(delegation_profiles.ProfilesError, match="is a LiteLLM tier"):
        delegation_profiles.validate(clash, defaults, profiles)
    unknown = {"tier-new": {"models": ["pi-free:tier-new"], "tier": "tier-new", "description": "d"}}
    with pytest.raises(delegation_profiles.ProfilesError, match="no model of the catalogue"):
        delegation_profiles.validate(data, defaults, unknown)


def test_the_pi_providers_hold_each_tier_as_a_model_and_a_free_tier_costs_nothing():
    providers = {
        p["name"]: p
        for p in json.loads(render_registry.render_pi_providers(render_registry.load_registry()))["providers"]
    }
    # The free key also reaches every free group the catalogue deploys; the tier is one model among them.
    free = {m["id"]: m for m in providers["litellm-free"]["models"]}
    assert free["tier-free"]["cost"] == {
        "input": 0,
        "output": 0,
        "cacheRead": 0,
        "cacheWrite": 0,
    }
    assert [m["id"] for m in providers["litellm-tier-vibe"]["models"]] == ["tier-vibe"]
    assert "cost" not in providers["litellm-tier-vibe"]["models"][0]
    assert "litellm-tier-strong-open" not in providers


def test_a_route_that_calls_a_tier_with_other_keys_is_refused():
    data = render_registry.load_registry()
    broken = copy.deepcopy(data)
    next(r for r in broken["litellm"]["routes"] if r["agent"] == "tier-vibe")["models"] = ["tier-vibe", "canonical"]
    with pytest.raises(render_registry.RegistryError, match="calls tier-vibe"):
        render_registry._validate_litellm(broken)


def test_the_repository_renders_clean_with_the_tiers(monkeypatch):
    assert render_registry.stale_artifacts(render_registry.load_registry()) == []
