"""The per-repository setup file: the setup command writes it, the schema
validates it, and a repository without a valid one is refused (SetupFileError).

Two fixture repositories stand in for the estate's two instances: a personal
one (JorisJonkers-dev, the prefixed label taxonomy, no flaky job) and a work
one (an org repository with its own label names, a remapped triage vocabulary
and two reviewed flaky jobs). Each is a real git repository whose origin names
the tracker; labels are given on the command line so no gh is needed, except
in the one test that stubs it.
"""

from __future__ import annotations

import datetime
import importlib.util
import subprocess
import sys
from pathlib import Path

import pytest
import yaml

_spec = importlib.util.spec_from_file_location("repo_setup", Path(__file__).parent.parent / "scripts" / "repo_setup.py")
assert _spec and _spec.loader
rs = importlib.util.module_from_spec(_spec)
sys.modules["repo_setup"] = rs
_spec.loader.exec_module(rs)

PERSONAL_LABELS = ["type: bug", "type: feature", "area: agents", "priority: P1", "ready-for-agent", "status: triage"]
PERSONAL_TRIAGE = {
    "needs-triage": "status: triage",
    "needs-info": "status: triage",
    "ready-for-agent": "ready-for-agent",
    "ready-for-human": "type: feature",
    "wontfix": "type: bug",
}
WORK_LABELS = ["bug", "feature", "triage", "info-needed", "agent-ready", "human-ready", "wont-fix"]
WORK_TRIAGE = [
    "needs-triage=triage",
    "needs-info=info-needed",
    "ready-for-agent=agent-ready",
    "ready-for-human=human-ready",
    "wontfix=wont-fix",
]


def make_repo(tmp_path: Path, origin: str) -> Path:
    repo = tmp_path / "repo"
    repo.mkdir()
    subprocess.run(["git", "init", "-q", str(repo)], check=True)
    subprocess.run(["git", "-C", str(repo), "remote", "add", "origin", origin], check=True)
    return repo


def personal_args(repo: Path) -> list[str]:
    args = ["write", "--repo", str(repo)]
    for label in PERSONAL_LABELS:
        args += ["--label", label]
    for role, label in PERSONAL_TRIAGE.items():
        args += ["--triage", f"{role}={label}"]
    return args


def work_args(repo: Path) -> list[str]:
    args = ["write", "--repo", str(repo), "--reviewed-by", "a.dev", "--reviewed-on", "2026-11-01"]
    for label in WORK_LABELS:
        args += ["--label", label]
    for item in WORK_TRIAGE:
        args += ["--triage", item]
    args += ["--flaky", "e2e-browser", "Chromium download times out on the shared runner"]
    return [*args, "--flaky", "integration-db", "Container start races the health probe"]


def test_personal_repo_gets_a_valid_file(tmp_path: Path, capsys: pytest.CaptureFixture[str]) -> None:
    repo = make_repo(tmp_path, "git@github.com:JorisJonkers-dev/some-tool.git")
    assert rs.main(personal_args(repo)) == 0
    assert capsys.readouterr().out.strip() == "changed"
    data = rs.load(repo)
    assert data["tracker"] == {"kind": "github", "repo": "JorisJonkers-dev/some-tool"}
    assert data["labels"] == PERSONAL_LABELS
    assert data["triage_labels"]["ready-for-agent"] == "ready-for-agent"
    assert data["triage_labels"]["needs-info"] == "status: triage"
    assert data["flaky_jobs"] == []
    assert rs.main(["check", "--repo", str(repo)]) == 0


def test_work_repo_records_each_flaky_job_with_its_reviewed_reason(tmp_path: Path) -> None:
    repo = make_repo(tmp_path, "https://github.com/acme-corp/billing")
    assert rs.main([*work_args(repo), "--tracker-repo", "acme-corp/billing"]) == 0
    data = rs.load(repo)
    assert data["tracker"]["repo"] == "acme-corp/billing"
    assert data["triage_labels"]["wontfix"] == "wont-fix"
    assert data["flaky_jobs"] == [
        {
            "name": "e2e-browser",
            "reason": "Chromium download times out on the shared runner",
            "reviewed_by": "a.dev",
            "reviewed_on": "2026-11-01",
        },
        {
            "name": "integration-db",
            "reason": "Container start races the health probe",
            "reviewed_by": "a.dev",
            "reviewed_on": "2026-11-01",
        },
    ]


def test_rerun_is_byte_identical(tmp_path: Path, capsys: pytest.CaptureFixture[str]) -> None:
    repo = make_repo(tmp_path, "https://github.com/acme-corp/billing.git")
    assert rs.main(work_args(repo)) == 0
    first = (repo / rs.SETUP_FILE).read_bytes()
    capsys.readouterr()
    assert rs.main(work_args(repo)) == 0
    assert capsys.readouterr().out.strip() == "unchanged"
    assert (repo / rs.SETUP_FILE).read_bytes() == first


def test_labels_come_from_gh_when_not_given(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    repo = make_repo(tmp_path, "git@github.com:acme-corp/billing.git")
    stub = tmp_path / "bin"
    stub.mkdir()
    gh = stub / "gh"
    gh.write_text("#!/bin/sh\necho \"$@\" > " + str(tmp_path / "gh.args") + "\nprintf 'needs-triage\\nneeds-info\\n"
                  "ready-for-agent\\nready-for-human\\nwontfix\\n'\n")  # fmt: skip
    gh.chmod(0o755)
    monkeypatch.setenv("PATH", f"{stub}:/usr/bin:/bin:/run/current-system/sw/bin")
    assert rs.main(["write", "--repo", str(repo)]) == 0
    assert "acme-corp/billing" in (tmp_path / "gh.args").read_text()
    assert rs.load(repo)["labels"][0] == "needs-triage"


def test_repository_without_a_file_is_refused(tmp_path: Path, capsys: pytest.CaptureFixture[str]) -> None:
    repo = make_repo(tmp_path, "https://github.com/acme-corp/billing")
    with pytest.raises(rs.SetupFileError, match="missing"):
        rs.load(repo)
    assert rs.main(["check", "--repo", str(repo)]) == 3
    assert "missing" in capsys.readouterr().err


def test_flaky_job_without_reviewer_is_not_written(tmp_path: Path, capsys: pytest.CaptureFixture[str]) -> None:
    repo = make_repo(tmp_path, "https://github.com/acme-corp/billing")
    args = [a for a in work_args(repo) if a not in ("--reviewed-by", "a.dev")]
    assert rs.main(args) == 3
    assert "--reviewed-by" in capsys.readouterr().err
    assert not (repo / rs.SETUP_FILE).exists()


def test_invalid_input_writes_nothing(tmp_path: Path, capsys: pytest.CaptureFixture[str]) -> None:
    repo = make_repo(tmp_path, "https://gitlab.com/acme-corp/billing")
    assert rs.main(["write", "--repo", str(repo), "--label", "bug", "--label", "bug"]) == 3
    err = capsys.readouterr().err
    assert "tracker.repo must be OWNER/NAME" in err
    assert "is not in labels" in err
    assert not (repo / rs.SETUP_FILE).exists()


def good() -> dict[str, object]:
    return rs.build("o/r", ["a", *rs.TRIAGE_ROLES], {}, [("job", "why")], "me", "2026-11-01")


def test_good_document_is_valid() -> None:
    assert rs.validate(good()) == []


@pytest.mark.parametrize(
    ("mutate", "expected"),
    [
        (lambda d: d.update(version=2), "version must be 1"),
        (lambda d: d.update(version=True), "version must be 1"),
        (lambda d: d["tracker"].update(repo="../.."), "tracker.repo must be OWNER/NAME"),
        (lambda d: d["flaky_jobs"][0].update(reviewed_on=datetime.datetime(2026, 11, 1, 10)), "reviewed_on must be"),
        (lambda d: d.update(extra=1), "unknown key: extra"),
        (lambda d: d.update(tracker="x"), "tracker must be a mapping"),
        (lambda d: d["tracker"].update(kind="jira", other=1), "tracker.kind must be one of"),
        (lambda d: d["tracker"].update(other=1), "tracker: unknown key: other"),
        (lambda d: d.update(labels=[]), "labels must be a non-empty list"),
        (lambda d: d.update(labels=["a", "a", *rs.TRIAGE_ROLES]), "labels must not repeat"),
        (lambda d: d.update(labels=[1]), "labels must be a non-empty list"),
        (lambda d: d.update(triage_labels=[]), "triage_labels must be a mapping"),
        (lambda d: d["triage_labels"].update(bogus="a"), "unknown role: bogus"),
        (lambda d: d["triage_labels"].pop("wontfix"), "triage_labels.wontfix is missing"),
        (lambda d: d.update(flaky_jobs="x"), "flaky_jobs must be a list"),
        (lambda d: d.update(flaky_jobs=["x"]), "flaky_jobs[0] must be a mapping"),
        (lambda d: d["flaky_jobs"][0].update(reason=" "), "flaky_jobs[0].reason is required"),
        (lambda d: d["flaky_jobs"][0].update(reviewed_by=""), "flaky_jobs[0].reviewed_by is required"),
        (lambda d: d["flaky_jobs"][0].update(reviewed_on="2026-13-40"), "reviewed_on must be a YYYY-MM-DD"),
        (lambda d: d["flaky_jobs"][0].update(reviewed_on="yesterday"), "reviewed_on must be a YYYY-MM-DD"),
        (lambda d: d["flaky_jobs"][0].update(note="x"), "unknown key: note"),
        (lambda d: d["flaky_jobs"].append(dict(d["flaky_jobs"][0])), "repeats 'job'"),
    ],
)
def test_schema_rejects(mutate: object, expected: str) -> None:
    data = good()
    mutate(data)  # type: ignore[operator]
    assert any(expected in p for p in rs.validate(data)), rs.validate(data)


def test_non_mapping_document_is_invalid() -> None:
    assert rs.validate([]) == ["the file must be a mapping"]


def test_unquoted_yaml_date_is_accepted(tmp_path: Path) -> None:
    repo = make_repo(tmp_path, "https://github.com/acme-corp/billing")
    data = good()
    data["flaky_jobs"][0]["reviewed_on"] = "2026-11-01"  # type: ignore[index]
    path = repo / rs.SETUP_FILE
    path.parent.mkdir(parents=True)
    path.write_text(yaml.safe_dump(data).replace("'2026-11-01'", "2026-11-01"))
    assert rs.load(repo)["flaky_jobs"][0]["reviewed_on"].isoformat() == "2026-11-01"


def test_unparseable_and_invalid_files_are_refused(tmp_path: Path) -> None:
    repo = make_repo(tmp_path, "https://github.com/acme-corp/billing")
    path = repo / rs.SETUP_FILE
    path.parent.mkdir(parents=True)
    path.write_text("a: [unclosed\n")
    with pytest.raises(rs.SetupFileError, match="not valid YAML"):
        rs.load(repo)
    path.write_text("version: 1\n")
    with pytest.raises(rs.SetupFileError, match="tracker must be a mapping"):
        rs.load(repo)


def test_cli_entry_point(tmp_path: Path) -> None:
    repo = make_repo(tmp_path, "https://github.com/acme-corp/billing")
    script = Path(rs.__file__)
    run = subprocess.run([sys.executable, str(script), "check", "--repo", str(repo)], capture_output=True, text=True)
    assert run.returncode == 3
    assert "missing" in run.stderr


def test_rerun_without_flaky_keeps_reviewed_jobs(tmp_path: Path) -> None:
    repo = make_repo(tmp_path, "https://github.com/acme-corp/billing")
    assert rs.main(work_args(repo)) == 0
    args = [a for a in work_args(repo)[:-6]]
    assert rs.main(args) == 0
    assert len(rs.load(repo)["flaky_jobs"]) == 2


@pytest.mark.parametrize("bad", ["needs-tirage=x", "needs-triage"])
def test_bad_triage_argument_is_refused(tmp_path: Path, bad: str) -> None:
    repo = make_repo(tmp_path, "https://github.com/acme-corp/billing")
    assert rs.main([*personal_args(repo), "--triage", bad]) == 3


def test_missing_repo_directory_is_not_created(tmp_path: Path) -> None:
    assert rs.main(["write", "--repo", str(tmp_path / "typo")]) == 3
    assert not (tmp_path / "typo").exists()


def test_ssh_alias_origin_names_the_tracker(tmp_path: Path) -> None:
    assert rs._git_tracker_repo(make_repo(tmp_path, "git@github.com-work:acme/billing.git")) == "acme/billing"


def test_lookalike_host_is_not_github(tmp_path: Path) -> None:
    assert rs._git_tracker_repo(make_repo(tmp_path, "https://notgithub.com/a/b")) == ""


def test_gh_failure_is_reported(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    stub = tmp_path / "bin"
    stub.mkdir()
    (stub / "gh").write_text("#!/bin/sh\necho 'not logged in' >&2\nexit 1\n")
    (stub / "gh").chmod(0o755)
    monkeypatch.setenv("PATH", f"{stub}:/usr/bin:/bin:/run/current-system/sw/bin")
    assert rs._gh_labels("a/b") == []
    assert "not logged in" in capsys.readouterr().err


def test_non_utf8_file_is_refused(tmp_path: Path) -> None:
    repo = make_repo(tmp_path, "https://github.com/acme-corp/billing")
    path = repo / rs.SETUP_FILE
    path.parent.mkdir(parents=True)
    path.write_bytes(b"\xff\xfe")
    with pytest.raises(rs.SetupFileError):
        rs.load(repo)
