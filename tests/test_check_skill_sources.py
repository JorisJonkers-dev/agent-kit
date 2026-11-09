"""The skill sources drift check, driven against a fake `gh` CLI.

`check_skill_sources.py` reads each estate skill's `sources.yaml` and asks
GitHub for the current blob of every `file` entry and the latest tag of every
`release` entry. It lists what moved, reports a failed request as a failure and
never as drift, validates the files, rewrites one skill with `--update`, and
keeps one drift issue in step with `--sync-issue`. Nothing here touches
GitHub: a stub `gh` on PATH serves blob shas, release tags and issues from a
state directory.
"""

from __future__ import annotations

import datetime
import json
import stat
import subprocess
import sys
from pathlib import Path

import pytest
import yaml

KIT_ROOT = Path(__file__).resolve().parent.parent
SCRIPT = KIT_ROOT / "scripts" / "check_skill_sources.py"
SKILL_SOURCES_WORKFLOW = KIT_ROOT / ".github" / "workflows" / "skill-sources.yml"
CI_WORKFLOW = KIT_ROOT / ".github" / "workflows" / "ci.yml"

ISSUE_TITLE = "Skill sources drift"
BLOB_OLD = "a" * 40
BLOB_NEW = "b" * 40
CHECKED = "2026-11-03"

FILE_KEY = "repos/JorisJonkers-dev/deploy-kit/contents/spec/v1/10-project-intent.md"
RELEASE_KEY = "repos/JorisJonkers-dev/github-workflows/releases/latest"

FILE_SOURCES = f"""sources:
  - kind: file
    repo: JorisJonkers-dev/deploy-kit
    path: spec/v1/10-project-intent.md
    blob: {BLOB_OLD}
    checked: {CHECKED}
"""

RELEASE_SOURCES = """sources:
  - kind: release
    repo: JorisJonkers-dev/github-workflows
    tag: v0.43.0
    checked: 2026-11-03
"""

BOTH_SOURCES = f"""sources:
  - kind: file
    repo: JorisJonkers-dev/deploy-kit
    path: spec/v1/10-project-intent.md
    blob: {BLOB_OLD}
    checked: {CHECKED}
  - kind: release
    repo: JorisJonkers-dev/github-workflows
    tag: v0.43.0
    checked: {CHECKED}
"""

MOVED_LINE = (f"estate-bootstrap: JorisJonkers-dev/deploy-kit spec/v1/10-project-intent.md "
              f"moved (blob {BLOB_OLD} -> {BLOB_NEW})")

FAKE_GH = '''#!/usr/bin/env python3
# Fake `gh` for the skill sources tests. `gh api <path>` serves the JSON held
# under that path in $FAKE_GH_DIR/api.json: an entry with an "error" key fails
# with that stderr, a missing key fails with HTTP 404. `gh issue ...` keeps its
# issues in $FAKE_GH_DIR/issues.json. Every call is logged to calls.log.
import json
import os
import sys
from pathlib import Path

STATE = Path(os.environ["FAKE_GH_DIR"])


def main() -> int:
    args = sys.argv[1:]
    with (STATE / "calls.log").open("a") as handle:
        handle.write(" ".join(args) + "\\n")
    if not args:
        print("fake gh: no invocation", file=sys.stderr)
        return 1
    if args[0] == "api":
        path = next((arg for arg in args[1:] if arg.startswith("repos/")), None)
        table = json.loads((STATE / "api.json").read_text())
        entry = table.get(path)
        if entry is None:
            print("gh: Not Found (HTTP 404)", file=sys.stderr)
            return 1
        if "error" in entry:
            print(entry["error"], file=sys.stderr)
            return 1
        print(json.dumps(entry))
        return 0
    if args[0] == "issue":
        store = STATE / "issues.json"
        issues = json.loads(store.read_text())
        subcommand = args[1]
        if subcommand == "list":
            print(json.dumps([{"number": issue["number"], "title": issue["title"]}
                              for issue in issues if issue["state"] == "open"]))
            return 0
        if subcommand == "create":
            options = dict(zip(args[2::2], args[3::2]))
            number = max((issue["number"] for issue in issues), default=0) + 1
            issues.append({"number": number, "title": options["--title"],
                           "body": options["--body"], "state": "open"})
        elif subcommand == "edit":
            options = dict(zip(args[3::2], args[4::2]))
            for issue in issues:
                if issue["number"] == int(args[2]):
                    issue["body"] = options["--body"]
        elif subcommand == "close":
            for issue in issues:
                if issue["number"] == int(args[2]):
                    issue["state"] = "closed"
        else:
            print(f"fake gh: unsupported invocation: {args}", file=sys.stderr)
            return 1
        store.write_text(json.dumps(issues))
        return 0
    print(f"fake gh: unsupported invocation: {args}", file=sys.stderr)
    return 1

if __name__ == "__main__":
    sys.exit(main())
'''


@pytest.fixture
def fake_gh_bin(tmp_path: Path) -> Path:
    bin_dir = tmp_path / "fake-bin"
    bin_dir.mkdir()
    gh_path = bin_dir / "gh"
    gh_path.write_text(FAKE_GH)
    gh_path.chmod(gh_path.stat().st_mode | stat.S_IEXEC)
    return bin_dir


def state(tmp_path: Path, api: dict | None = None, issues: list | None = None) -> Path:
    """The state directory the fake `gh` serves answers from."""
    state_dir = tmp_path / "gh-state"
    state_dir.mkdir(exist_ok=True)
    (state_dir / "api.json").write_text(json.dumps(api if api is not None else {}))
    (state_dir / "issues.json").write_text(json.dumps(issues if issues is not None else []))
    (state_dir / "calls.log").write_text("")
    return state_dir


def skill(root: Path, name: str, sources: str | None) -> Path:
    """One estate skill folder, with `sources.yaml` unless `sources` is None."""
    skill_dir = root / name
    skill_dir.mkdir(parents=True, exist_ok=True)
    if sources is not None:
        (skill_dir / "sources.yaml").write_text(sources)
    return skill_dir


def run(fake_gh_bin: Path, state_dir: Path, tmp_path: Path, *args: str) -> subprocess.CompletedProcess:
    return subprocess.run(
        [sys.executable, str(SCRIPT), *args],
        capture_output=True, text=True, check=False,
        env={"PATH": f"{fake_gh_bin}:/usr/bin:/bin:/usr/local/bin:/opt/homebrew/bin:/sbin",
             "FAKE_GH_DIR": str(state_dir), "GH_TOKEN": "test-token", "HOME": str(tmp_path)},
    )


def calls(state_dir: Path) -> str:
    return (state_dir / "calls.log").read_text()


def issues(state_dir: Path) -> list:
    return json.loads((state_dir / "issues.json").read_text())


# --- the drift check: unchanged, moved, gone, and a newer release ---


def test_an_unchanged_source_reports_no_drift(tmp_path: Path, fake_gh_bin: Path) -> None:
    root = tmp_path / "workstation"
    skill(root, "estate-bootstrap", BOTH_SOURCES)
    state_dir = state(tmp_path, {FILE_KEY: {"sha": BLOB_OLD}, RELEASE_KEY: {"tag_name": "v0.43.0"}})
    result = run(fake_gh_bin, state_dir, tmp_path, "--root", str(root))
    assert result.returncode == 0, result.stdout + result.stderr
    assert "no sources moved" in result.stdout


def test_a_moved_blob_is_listed(tmp_path: Path, fake_gh_bin: Path) -> None:
    root = tmp_path / "workstation"
    skill(root, "estate-bootstrap", FILE_SOURCES)
    state_dir = state(tmp_path, {FILE_KEY: {"sha": BLOB_NEW}})
    result = run(fake_gh_bin, state_dir, tmp_path, "--root", str(root))
    assert result.returncode == 1, result.stdout + result.stderr
    assert MOVED_LINE in result.stdout
    assert "1 source(s) moved" in result.stdout


def test_a_missing_path_is_listed(tmp_path: Path, fake_gh_bin: Path) -> None:
    root = tmp_path / "workstation"
    skill(root, "estate-bootstrap", BOTH_SOURCES)
    state_dir = state(tmp_path, {RELEASE_KEY: {"tag_name": "v0.43.0"}})
    result = run(fake_gh_bin, state_dir, tmp_path, "--root", str(root))
    assert result.returncode == 1, result.stdout + result.stderr
    assert "estate-bootstrap: JorisJonkers-dev/deploy-kit spec/v1/10-project-intent.md is gone" in result.stdout


def test_a_newer_release_tag_is_listed(tmp_path: Path, fake_gh_bin: Path) -> None:
    root = tmp_path / "workstation"
    skill(root, "estate-bootstrap", RELEASE_SOURCES)
    state_dir = state(tmp_path, {RELEASE_KEY: {"tag_name": "v0.44.0"}})
    result = run(fake_gh_bin, state_dir, tmp_path, "--root", str(root))
    assert result.returncode == 1, result.stdout + result.stderr
    assert "estate-bootstrap: JorisJonkers-dev/github-workflows has a newer release v0.44.0 (recorded v0.43.0)" \
        in result.stdout


# --- a failed request is a failure, never drift ---


def test_a_failed_request_is_a_failure_and_never_drift(tmp_path: Path, fake_gh_bin: Path) -> None:
    root = tmp_path / "workstation"
    skill(root, "estate-bootstrap", BOTH_SOURCES)
    state_dir = state(tmp_path, {FILE_KEY: {"error": "gh: HTTP 401: Bad credentials"},
                                 RELEASE_KEY: {"tag_name": "v0.43.0"}})
    result = run(fake_gh_bin, state_dir, tmp_path, "--root", str(root))
    assert result.returncode == 1, result.stdout + result.stderr
    assert "check failed: gh: HTTP 401: Bad credentials" in result.stdout
    assert "1 check(s) failed" in result.stdout
    assert "moved" not in result.stdout
    assert "gone" not in result.stdout
    assert "newer" not in result.stdout


def test_a_timeout_is_retried_a_bounded_number_of_times(tmp_path: Path, fake_gh_bin: Path) -> None:
    root = tmp_path / "workstation"
    skill(root, "estate-bootstrap", FILE_SOURCES)
    state_dir = state(tmp_path, {FILE_KEY: {"error": "gh: timeout awaiting response"}})
    result = run(fake_gh_bin, state_dir, tmp_path, "--root", str(root))
    assert result.returncode == 1, result.stdout + result.stderr
    assert "check failed: gh: timeout awaiting response" in result.stdout
    assert "moved" not in result.stdout
    attempts = [line for line in calls(state_dir).splitlines() if "contents/" in line]
    assert len(attempts) == 3, f"expected three bounded retries, saw {attempts}"


# --- --validate: missing, empty, malformed, well formed, and no skills ---


def test_validate_passes_when_there_are_no_estate_skills(tmp_path: Path, fake_gh_bin: Path) -> None:
    root = tmp_path / "workstation"
    root.mkdir()
    (root / "pr-composer").mkdir()  # a workstation skill that is not an estate skill
    result = run(fake_gh_bin, state(tmp_path), tmp_path, "--root", str(root), "--validate")
    assert result.returncode == 0, result.stdout + result.stderr


def test_validate_fails_on_a_missing_sources_yaml(tmp_path: Path, fake_gh_bin: Path) -> None:
    root = tmp_path / "workstation"
    skill(root, "estate-bootstrap", None)
    result = run(fake_gh_bin, state(tmp_path), tmp_path, "--root", str(root), "--validate")
    assert result.returncode == 1
    assert "estate-bootstrap: has no sources.yaml" in result.stdout


def test_validate_fails_on_an_empty_sources_list(tmp_path: Path, fake_gh_bin: Path) -> None:
    root = tmp_path / "workstation"
    skill(root, "estate-bootstrap", "sources: []\n")
    result = run(fake_gh_bin, state(tmp_path), tmp_path, "--root", str(root), "--validate")
    assert result.returncode == 1
    assert "estate-bootstrap: sources.yaml lists no sources" in result.stdout


def test_validate_fails_on_a_malformed_entry(tmp_path: Path, fake_gh_bin: Path) -> None:
    root = tmp_path / "workstation"
    malformed = [
        f"sources:\n  - kind: file\n    repo: JorisJonkers-dev/deploy-kit\n    path: spec/v1/a.md\n"
        f"    blob: not-a-sha\n    checked: {CHECKED}\n",
        f"sources:\n  - kind: branch\n    repo: JorisJonkers-dev/deploy-kit\n    checked: {CHECKED}\n",
        f"sources:\n  - kind: file\n    repo: deploy-kit\n    path: spec/v1/a.md\n    blob: {BLOB_OLD}\n"
        f"    checked: {CHECKED}\n",
        f"sources:\n  - kind: file\n    repo: JorisJonkers-dev/deploy-kit\n    path: spec/v1/a.md\n"
        f"    blob: {BLOB_OLD}\n    checked: 03/11/2026\n",
        f"sources:\n  - kind: release\n    repo: JorisJonkers-dev/github-workflows\n    checked: {CHECKED}\n",
        "sources:\n  - just text\n",
    ]
    for body in malformed:
        root = tmp_path / "workstation"
        skill(root, "estate-bootstrap", body)
        result = run(fake_gh_bin, state(tmp_path), tmp_path, "--root", str(root), "--validate")
        assert result.returncode == 1, body
        assert "estate-bootstrap: sources.yaml entry 1" in result.stdout, body
        assert "well formed" not in result.stdout, body
        (root / "estate-bootstrap" / "sources.yaml").unlink()


def test_validate_passes_on_a_well_formed_file(tmp_path: Path, fake_gh_bin: Path) -> None:
    root = tmp_path / "workstation"
    skill(root, "estate-bootstrap", BOTH_SOURCES)
    result = run(fake_gh_bin, state(tmp_path), tmp_path, "--root", str(root), "--validate")
    assert result.returncode == 0, result.stdout + result.stderr


# --- --update rewrites one skill's blobs, tags and dates ---


def test_update_rewrites_blobs_tags_and_dates(tmp_path: Path, fake_gh_bin: Path) -> None:
    root = tmp_path / "workstation"
    skill(root, "estate-bootstrap", BOTH_SOURCES)
    state_dir = state(tmp_path, {FILE_KEY: {"sha": BLOB_NEW}, RELEASE_KEY: {"tag_name": "v0.44.0"}})
    result = run(fake_gh_bin, state_dir, tmp_path, "--root", str(root), "--update", "estate-bootstrap")
    assert result.returncode == 0, result.stdout + result.stderr
    data = yaml.safe_load((root / "estate-bootstrap" / "sources.yaml").read_text())
    entries = data["sources"]
    today = datetime.date.today().isoformat()
    assert entries[0] == {"kind": "file", "repo": "JorisJonkers-dev/deploy-kit",
                          "path": "spec/v1/10-project-intent.md", "blob": BLOB_NEW, "checked": today}
    assert entries[1] == {"kind": "release", "repo": "JorisJonkers-dev/github-workflows",
                          "tag": "v0.44.0", "checked": today}


def test_update_leaves_the_file_alone_when_a_request_fails(tmp_path: Path, fake_gh_bin: Path) -> None:
    root = tmp_path / "workstation"
    skill(root, "estate-bootstrap", FILE_SOURCES)
    state_dir = state(tmp_path, {FILE_KEY: {"error": "gh: HTTP 403: API rate limit exceeded"}})
    result = run(fake_gh_bin, state_dir, tmp_path, "--root", str(root), "--update", "estate-bootstrap")
    assert result.returncode == 1
    assert "rate limit" in result.stdout
    assert (root / "estate-bootstrap" / "sources.yaml").read_text() == FILE_SOURCES


# --- --sync-issue keeps one issue, found by its title, in step ---


def test_no_drift_and_no_issue_opens_nothing(tmp_path: Path, fake_gh_bin: Path) -> None:
    root = tmp_path / "workstation"
    skill(root, "estate-bootstrap", FILE_SOURCES)
    state_dir = state(tmp_path, {FILE_KEY: {"sha": BLOB_OLD}})
    result = run(fake_gh_bin, state_dir, tmp_path, "--root", str(root), "--sync-issue")
    assert result.returncode == 0, result.stdout + result.stderr
    assert issues(state_dir) == []
    assert "issue create" not in calls(state_dir)


def test_drift_and_no_issue_opens_one_issue(tmp_path: Path, fake_gh_bin: Path) -> None:
    root = tmp_path / "workstation"
    skill(root, "estate-bootstrap", FILE_SOURCES)
    state_dir = state(tmp_path, {FILE_KEY: {"sha": BLOB_NEW}})
    result = run(fake_gh_bin, state_dir, tmp_path, "--root", str(root), "--sync-issue")
    assert result.returncode == 0, result.stdout + result.stderr
    listed = issues(state_dir)
    assert len(listed) == 1
    assert listed[0]["title"] == ISSUE_TITLE
    assert listed[0]["state"] == "open"
    assert MOVED_LINE in listed[0]["body"]


def test_drift_and_an_existing_issue_updates_that_issue(tmp_path: Path, fake_gh_bin: Path) -> None:
    root = tmp_path / "workstation"
    skill(root, "estate-bootstrap", FILE_SOURCES)
    state_dir = state(tmp_path, {FILE_KEY: {"sha": BLOB_NEW}},
                      [{"number": 7, "title": ISSUE_TITLE, "body": "old body", "state": "open"}])
    result = run(fake_gh_bin, state_dir, tmp_path, "--root", str(root), "--sync-issue")
    assert result.returncode == 0, result.stdout + result.stderr
    listed = issues(state_dir)
    assert len(listed) == 1
    assert listed[0]["number"] == 7
    assert "old body" not in listed[0]["body"]
    assert MOVED_LINE in listed[0]["body"]
    assert "issue create" not in calls(state_dir)
    assert "issue edit 7" in calls(state_dir)


def test_no_drift_and_an_existing_issue_closes_it(tmp_path: Path, fake_gh_bin: Path) -> None:
    root = tmp_path / "workstation"
    skill(root, "estate-bootstrap", FILE_SOURCES)
    state_dir = state(tmp_path, {FILE_KEY: {"sha": BLOB_OLD}},
                      [{"number": 7, "title": ISSUE_TITLE, "body": "old body", "state": "open"}])
    result = run(fake_gh_bin, state_dir, tmp_path, "--root", str(root), "--sync-issue")
    assert result.returncode == 0, result.stdout + result.stderr
    listed = issues(state_dir)
    assert listed[0]["state"] == "closed"
    assert "issue close 7" in calls(state_dir)


def test_a_failed_run_leaves_the_issue_untouched(tmp_path: Path, fake_gh_bin: Path) -> None:
    root = tmp_path / "workstation"
    skill(root, "estate-bootstrap", FILE_SOURCES)
    seeded = {"number": 7, "title": ISSUE_TITLE, "body": "old body", "state": "open"}
    state_dir = state(tmp_path, {FILE_KEY: {"error": "gh: HTTP 401: Bad credentials"}}, [seeded])
    result = run(fake_gh_bin, state_dir, tmp_path, "--root", str(root), "--sync-issue")
    assert result.returncode == 1
    assert "check failed: gh: HTTP 401: Bad credentials" in result.stdout
    assert "issue is left untouched" in result.stdout
    assert issues(state_dir) == [seeded]
    for verb in ("issue create", "issue edit", "issue close"):
        assert verb not in calls(state_dir)


# --- the workflow wiring these tests keep honest ---


def test_the_workflow_runs_weekly_and_on_dispatch_in_one_concurrency_group() -> None:
    doc = yaml.safe_load(SKILL_SOURCES_WORKFLOW.read_text())
    trigger = doc["on"]
    assert "workflow_dispatch" in trigger
    assert trigger["schedule"], "a weekly cron"
    concurrency = doc["concurrency"]
    assert concurrency["cancel-in-progress"] in (False, "false"), "runs never overlap"
    steps = [step for job in doc["jobs"].values() for step in job["steps"]]
    runs = [step.get("run", "") for step in steps]
    assert "uv run python scripts/check_skill_sources.py --sync-issue" in runs
    uses = [step.get("uses", "") for step in steps]
    assert any(use.startswith("actions/create-github-app-token@") for use in uses), "an App token"


def test_the_python_job_validates_sources() -> None:
    doc = yaml.safe_load(CI_WORKFLOW.read_text())
    runs = [step.get("run", "") for step in doc["jobs"]["python"]["steps"]]
    assert "uv run python scripts/check_skill_sources.py --validate" in runs
