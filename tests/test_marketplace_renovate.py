"""Renovate moves every marketplace pin, and a skill source of the same upstream with it.

The registry refuses a marketplace pin that differs from a skill source of the
same upstream, so both must change in one pull request: Renovate puts updates
of one dependency in one branch, so the two matches share a depName, ref and
commit. The bump automerges only when every check passes, including Marketplace
Pin Review, and that check is not part of Pipeline Complete, so a person can
still merge a bump it flagged.
"""

from __future__ import annotations

import json
import re
from pathlib import Path

import yaml

KIT_ROOT = Path(__file__).resolve().parent.parent
REGISTRY = KIT_ROOT / "registry" / "estate-tooling.yaml"
RENOVATE = json.loads((KIT_ROOT / "renovate.json").read_text())
DEP_TYPE = "marketplace-pin"


def _managers() -> list[dict]:
    return [m for m in RENOVATE["customManagers"] if m.get("depTypeTemplate") == DEP_TYPE]


def _deps() -> list[tuple[str, str, str, str]]:
    text = REGISTRY.read_text()
    deps = []
    for manager in _managers():
        for pattern in manager["matchStrings"]:
            for match in re.finditer(pattern.replace("(?<", "(?P<"), text):
                deps.append((match["depName"], match["currentValue"], match["currentDigest"],
                             manager["datasourceTemplate"]))
    return deps


def _registry() -> dict:
    return yaml.safe_load(REGISTRY.read_text())


def _coupled() -> list[dict]:
    markets = {m["repo"].lower() for m in _registry()["marketplaces"]}
    return [s for s in _registry()["skill_sources"]
            if s["repo"].removeprefix("https://github.com/").lower() in markets]


def test_every_marketplace_pin_is_found_once_with_its_ref() -> None:
    deps = _deps()
    assert len(deps) == len(_registry()["marketplaces"]) + len(_coupled()), "nothing else matches"
    for market in _registry()["marketplaces"]:
        found = [d for d in deps if d[2] == market["commit"] and d[0] == market["repo"]]
        assert len(found) == 1 + sum(s["repo"].endswith(market["repo"]) for s in _coupled()), market["name"]
        assert {d[1] for d in found} == {market["ref"]}


def test_branches_use_git_refs_and_tags_use_github_tags() -> None:
    for name, ref, _, datasource in _deps():
        expected = "github-tags" if re.match(r"v?\d", ref) else "git-refs"
        assert datasource == expected, (name, ref)


def test_a_coupled_skill_source_moves_in_the_same_update() -> None:
    assert {s["name"] for s in _coupled()} == {"caveman", "drawio", "kubernetes-skill"}
    deps = _deps()
    for source in _coupled():
        repo = source["repo"].removeprefix("https://github.com/")
        assert (repo, source["ref"], source["commit"]) in {d[:3] for d in deps}, source["name"]


def test_no_other_skill_source_is_moved() -> None:
    coupled = {s["commit"] for s in _coupled()}
    markets = {m["commit"] for m in _registry()["marketplaces"]}
    for source in _registry()["skill_sources"]:
        if source["commit"] not in coupled | markets:
            assert source["commit"] not in {d[2] for d in _deps()}, source["name"]


def test_a_bump_automerges_only_through_renovate_once_every_check_passes() -> None:
    rule = next(r for r in RENOVATE["packageRules"] if r.get("matchDepTypes") == [DEP_TYPE])
    assert rule["automerge"] is True
    assert rule["platformAutomerge"] is False, "native automerge would wait only for required checks"


def test_a_branch_pin_has_no_release_age_to_wait_for() -> None:
    rule = next(r for r in RENOVATE["packageRules"]
                if r.get("matchDepTypes") == [DEP_TYPE] and r.get("matchDatasources") == ["git-refs"])
    assert rule["minimumReleaseAge"] is None


def test_the_render_commit_does_not_stop_the_automerge() -> None:
    workflow = (KIT_ROOT / ".github" / "workflows" / "renovate-render.yml").read_text()
    assert '"${APP_SLUG}[bot]@users.noreply.github.com"' in workflow
    assert "jorisjonkers-dev-agents[bot]@users.noreply.github.com" in RENOVATE["gitIgnoredAuthors"]


def test_the_review_runs_on_pull_requests_outside_pipeline_complete() -> None:
    jobs = yaml.safe_load((KIT_ROOT / ".github" / "workflows" / "ci.yml").read_text())["jobs"]
    review = jobs["marketplace-pins"]
    assert review["name"] == "Marketplace Pin Review"
    assert any("check_marketplace_bump.py --base-ref" in str(step.get("run", "")) for step in review["steps"])
    assert "marketplace-pins" not in jobs["pipeline-complete"]["needs"]


def test_a_bump_is_its_own_pull_request_and_cuts_a_release() -> None:
    rule = next(r for r in RENOVATE["packageRules"] if r.get("matchDepTypes") == [DEP_TYPE])
    assert "groupName" in rule and rule["groupName"] is None, "the shared minor-and-patch group would mix it in"
    assert rule["semanticCommitType"] == "fix", "release-please cuts no release for a chore"
