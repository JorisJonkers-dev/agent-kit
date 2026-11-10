"""The five critique skills renamed to follow the order they run in are
retired entries: setup removes their old directories from every agent home,
and leaves the new ones and any other skill alone."""

from __future__ import annotations

import sys
from pathlib import Path

import pytest
import yaml

KIT_ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(KIT_ROOT / "scripts"))

import render_registry  # noqa: E402
from setup_harness import setup_run  # noqa: E402

RENAMED = {
    "critique-decisions": "review-grill",
    "critique-spec": "review-spec",
    "check-tickets": "review-tickets",
    "review-ticket": "review-implementation",
    "review-feature": "review-pr",
}
HOMES = (".claude", ".codex", ".vibe", ".vibe-private", ".gemini/config", ".gemini/antigravity-cli")


def _seed(home: Path) -> None:
    for h in HOMES:
        for old in (*RENAMED, "someone-elses-skill"):
            d = home / h / "skills" / old
            d.mkdir(parents=True)
            (d / "SKILL.md").write_text("old\n")


def test_registry_retires_each_old_name_and_names_its_replacement() -> None:
    data = yaml.safe_load((KIT_ROOT / "registry" / "estate-tooling.yaml").read_text())
    assert {r["name"]: r["renamed_to"] for r in data["retired_skills"]} == RENAMED
    shipped = {s["name"] for s in data["local_skills"]}
    assert shipped.isdisjoint(RENAMED)
    assert set(RENAMED.values()) <= shipped


def test_setup_removes_the_old_directories_and_installs_the_new(tmp_path: Path) -> None:
    run = setup_run(tmp_path, "Darwin", prepare_home=_seed)
    for h in HOMES:
        skills = run.home / h / "skills"
        for old, new in RENAMED.items():
            assert not (skills / old).exists(), (h, old)
            assert (skills / new / "SKILL.md").is_file(), (h, new)
        assert (skills / "someone-elses-skill" / "SKILL.md").is_file(), h


def test_check_mode_removes_nothing(tmp_path: Path) -> None:
    run = setup_run(tmp_path, "Darwin", args=("--check",), prepare_home=_seed)
    for h in HOMES:
        for old in RENAMED:
            assert (run.home / h / "skills" / old / "SKILL.md").is_file(), (h, old)
    assert "would remove retired skill" in run.stdout


def _registry_with(entry: object) -> dict:

    data = yaml.safe_load((KIT_ROOT / "registry" / "estate-tooling.yaml").read_text())
    data["retired_skills"] = [entry]
    return data


@pytest.mark.parametrize(
    "entry",
    [
        {"name": 'a"b'},
        {"name": "$(touch x)"},
        {"name": "a b"},
        {"name": "../x"},
        {"name": ""},
        {"name": ".x"},
        {"renamed_to": "x"},
        {"name": 3},
        "critique-spec",
        ["a"],
    ],
)
def test_a_malformed_or_unsafe_retired_skill_is_refused(entry: object) -> None:

    with pytest.raises(render_registry.RegistryError):
        render_registry.validate(_registry_with(entry))
