"""The LiteLLM tier snapshot (registry/litellm-tiers.json), its check, and what is rendered from it (#905)."""

from __future__ import annotations

import base64
import copy
import json
import subprocess
import sys
from pathlib import Path

import pytest

KIT_ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(KIT_ROOT / "scripts"))

import delegation_profiles  # noqa: E402
import litellm_tiers  # noqa: E402
import render_registry  # noqa: E402

TIERS = ["tier-free", "tier-vibe", "tier-go-cheap", "tier-strong-open", "tier-review"]


def _routes() -> list[dict]:
    return render_registry.load_registry()["litellm"]["routes"]


def _runner(routing: dict, sha: str = "abc", code: int = 0):
    def run(argv):
        body = json.dumps({"sha": sha, "content": base64.b64encode(json.dumps(routing).encode()).decode()})
        return subprocess.CompletedProcess(
            list(argv), code, body if code == 0 else "", "" if code == 0 else "HTTP 404\nnot found"
        )

    return run


def test_the_snapshot_states_the_five_tiers_and_is_valid():
    groups = litellm_tiers.tier_groups()
    assert list(groups) == TIERS
    assert litellm_tiers.validate(json.loads(litellm_tiers.SNAPSHOT.read_text())) == []
    assert groups["tier-free"]["ceiling"] == "free"
    assert litellm_tiers.flat_models(groups, "tier-go-cheap") == [
        "opencode-go-tier/deepseek-v4.1-flash",
        "opencode-go-tier/glm-5.3-flash",
        "opencode-go-tier/mimo-v2.6-flash",
    ]
    # tier-vibe ends in tier-go-cheap, spliced in with a model once.
    vibe = litellm_tiers.flat_models(groups, "tier-vibe")
    assert vibe[-3:] == litellm_tiers.flat_models(groups, "tier-go-cheap") and len(vibe) == len(set(vibe))


def test_a_malformed_snapshot_says_what_is_wrong():
    good = json.loads(litellm_tiers.SNAPSHOT.read_text())
    assert litellm_tiers.validate([]) == ["not a JSON object"]
    assert "no tier_groups object" in litellm_tiers.validate({"source": {}})[-1]
    broken = copy.deepcopy(good)
    broken["tier_groups"]["tier-free"]["ceiling"] = "platinum"
    broken["tier_groups"]["tier-vibe"]["key_models"] = ["vibe-tier"]
    broken["tier_groups"]["tier-go-cheap"]["steps"] = []
    broken["tier_groups"]["tier-review"]["steps"] = [{"tier": "tier-nowhere"}]
    broken["tier_groups"]["nope"] = {}
    broken["tier_groups"]["tier-x"] = 3
    broken["source"] = 1
    problems = "\n".join(litellm_tiers.validate(broken))
    for fragment in (
        "ceiling must be",
        "key_models must be a list",
        "no steps",
        "neither a model",
        "is named tier-",
        "not an object",
        "no source",
    ):
        assert fragment in problems, fragment


def test_load_refuses_a_missing_or_malformed_snapshot(tmp_path):
    with pytest.raises(litellm_tiers.TiersError, match="cannot be read"):
        litellm_tiers.load(tmp_path / "absent.json")
    bad = tmp_path / "bad.json"
    bad.write_text("{}")
    with pytest.raises(litellm_tiers.TiersError, match="malformed"):
        litellm_tiers.load(bad)


def test_write_snapshots_upstream_and_check_finds_drift(tmp_path, capsys):
    routing = {"tier_groups": litellm_tiers.tier_groups(), "other": 1}
    path = tmp_path / "tiers.json"
    assert litellm_tiers.main(["--write", "--ref", "r1", "--path", str(path)], _runner(routing, "blob1")) == 0
    written = json.loads(path.read_text())
    assert written["source"]["ref"] == "r1" and written["source"]["blob"] == "blob1"
    assert litellm_tiers.main(["--check", "--path", str(path)]) == 0
    assert litellm_tiers.main(["--check", "--upstream", "r1", "--path", str(path)], _runner(routing)) == 0
    drifted = copy.deepcopy(routing)
    drifted["tier_groups"]["tier-free"]["steps"] = drifted["tier_groups"]["tier-free"]["steps"][:1]
    assert litellm_tiers.main(["--check", "--upstream", "r1", "--path", str(path)], _runner(drifted)) == 1
    assert "differs from litellm r1: tier-free" in capsys.readouterr().err


@pytest.mark.parametrize(
    ("runner", "message"),
    [
        (lambda argv: subprocess.CompletedProcess(list(argv), 1, "", "HTTP 404\nnot found"), "could not fetch"),
        (lambda argv: subprocess.CompletedProcess(list(argv), 0, "not json", ""), "no readable tier_groups"),
        (_runner({"tier_groups": {"tier-x": {}}}), "malformed tier_groups"),
        (_runner({"nothing": 1}), "no readable tier_groups"),
    ],
)
def test_a_fetch_that_fails_is_reported(runner, message, capsys, tmp_path):
    assert litellm_tiers.main(["--write", "--path", str(tmp_path / "t.json")], runner) == 1
    assert message in capsys.readouterr().err


def test_a_gh_that_cannot_run_is_reported(capsys, tmp_path):
    def broken(argv):
        raise FileNotFoundError("gh")

    assert litellm_tiers.main(["--write", "--path", str(tmp_path / "t.json")], broken) == 1
    assert "cannot run gh" in capsys.readouterr().err


def test_a_route_that_calls_a_tier_holds_exactly_its_key_models():
    groups = litellm_tiers.tier_groups()
    assert litellm_tiers.key_problems(groups, _routes()) == []
    wrong = [{"agent": "x", "models": ["tier-vibe", "canonical"]}]
    assert "calls tier-vibe but holds" in litellm_tiers.key_problems(groups, wrong)[0]


def test_each_tier_is_a_profile_on_the_route_that_holds_it():
    groups = litellm_tiers.tier_groups()
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
    profiles = delegation_profiles.tier_profiles(litellm_tiers.tier_groups(), _routes())
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
    free = providers["litellm-free"]["models"]
    assert [m["id"] for m in free] == ["tier-free"] and free[0]["cost"] == {
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
