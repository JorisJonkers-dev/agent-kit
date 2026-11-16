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
    assert sorted(data["profiles"]) == ["cheap-research", "cross-critic", "implement-cheap", "public-gemini"]
    delegation_profiles.validate(data, _defaults())


def test_the_defaults_carry_exactly_what_the_registry_renders():
    rendered = delegation_profiles.render_defaults(_registry(), DEFAULTS.read_text())
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
    assert delegation_profiles.render_defaults(_registry(), without) == DEFAULTS.read_text()
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
        (lambda p: p["public-gemini"].update(exclude_writer_family=True, models=["pi-public:public/gemini-3.8-flash"]),
         "only one"),
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
