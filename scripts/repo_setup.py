#!/usr/bin/env python3
"""The per-repository setup file, its validator and its setup command.

    repo_setup.py write  [--repo DIR] [--tracker-repo OWNER/NAME]
                         [--label NAME ...] [--triage ROLE=LABEL ...]
                         [--flaky JOB REASON ...] [--reviewed-by WHO]
                         [--reviewed-on YYYY-MM-DD]
    repo_setup.py check  [--repo DIR]

The file is ``docs/agents/setup.yaml``, next to the ``docs/agents/`` files the
issue-tracker and triage skills keep, and it is the one machine-readable
record a repository holds of how agents work in it:

    version: 1
    tracker: {kind: github, repo: OWNER/NAME}
    labels: [NAME, ...]                  # the label vocabulary
    triage_labels: {ROLE: LABEL, ...}    # the five roles, each a label above
    flaky_jobs:                          # CI jobs a rerun may retry once
      - {name: JOB, reason: WHY, reviewed_by: WHO, reviewed_on: YYYY-MM-DD}

The five triage roles are the triage skill's own canonical ones, so a
repository that already maps them keeps its mapping. A flaky job without a
reason or without who reviewed it is invalid: the CI-wait gate only reruns a
job that is listed here with a reviewed reason.

``write`` builds the file, validates it and writes it only when it is valid
(exit 3 and nothing written otherwise); with no ``--label`` it reads the
vocabulary from ``gh label list``. Re-running with the same input leaves the
file byte-identical (give --reviewed-on to pin the date of new flaky
jobs); a rerun without --flaky keeps the flaky jobs already listed.
``check`` exits 0 for a valid file and 3 for a missing or invalid one, printing each
problem to stderr; consumers call ``load`` and
refuse a repository whose file raises ``SetupFileError``.
"""

from __future__ import annotations

import argparse
import contextlib
import datetime
import re
import subprocess
import sys
from pathlib import Path
from typing import Any

import yaml

SETUP_FILE = Path("docs") / "agents" / "setup.yaml"
VERSION = 1
TRACKER_KINDS = ("github",)
TRIAGE_ROLES = ("needs-triage", "needs-info", "ready-for-agent", "ready-for-human", "wontfix")
_REPO_RE = re.compile(r"^(?!\.+/)[A-Za-z0-9_.-]+/(?!\.+$)[A-Za-z0-9_.-]+$")
_DATE_RE = re.compile(r"^\d{4}-\d{2}-\d{2}$")
_TOP_KEYS = {"version", "tracker", "labels", "triage_labels", "flaky_jobs"}
_FLAKY_KEYS = {"name", "reason", "reviewed_by", "reviewed_on"}


class SetupFileError(Exception):
    """The setup file is missing or invalid; ``problems`` says each way."""

    def __init__(self, problems: list[str]) -> None:
        super().__init__("; ".join(problems))
        self.problems = problems


def _text(value: object) -> bool:
    return isinstance(value, str) and bool(value.strip())


def validate(data: object) -> list[str]:
    """Every way ``data`` fails the setup-file schema; empty when it is valid."""
    if not isinstance(data, dict):
        return ["the file must be a mapping"]
    problems = [f"unknown key: {k}" for k in sorted(set(data) - _TOP_KEYS, key=str)]
    if data.get("version") != VERSION or isinstance(data.get("version"), bool):
        problems.append(f"version must be {VERSION}")
    problems += _validate_tracker(data.get("tracker"))
    labels = data.get("labels")
    label_set: set[str] = set()
    if not isinstance(labels, list) or not labels or not all(_text(x) for x in labels):
        problems.append("labels must be a non-empty list of names")
    else:
        label_set = set(labels)
        if len(label_set) != len(labels):
            problems.append("labels must not repeat")
    problems += _validate_triage(data.get("triage_labels"), label_set)
    problems += _validate_flaky(data.get("flaky_jobs"))
    return problems


def _validate_tracker(tracker: object) -> list[str]:
    if not isinstance(tracker, dict):
        return ["tracker must be a mapping with kind and repo"]
    problems = [f"tracker: unknown key: {k}" for k in sorted(set(tracker) - {"kind", "repo"}, key=str)]
    if tracker.get("kind") not in TRACKER_KINDS:
        problems.append(f"tracker.kind must be one of {', '.join(TRACKER_KINDS)}")
    repo = tracker.get("repo")
    if not isinstance(repo, str) or not _REPO_RE.match(repo):
        problems.append("tracker.repo must be OWNER/NAME")
    return problems


def _validate_triage(triage: object, labels: set[str]) -> list[str]:
    if not isinstance(triage, dict):
        return ["triage_labels must be a mapping"]
    problems = [f"triage_labels: unknown role: {r}" for r in sorted(set(triage) - set(TRIAGE_ROLES), key=str)]
    for role in TRIAGE_ROLES:
        label = triage.get(role)
        if not _text(label):
            problems.append(f"triage_labels.{role} is missing")
        elif labels and label not in labels:
            problems.append(f"triage_labels.{role}: {label!r} is not in labels")
    return problems


def _validate_flaky(jobs: object) -> list[str]:
    if not isinstance(jobs, list):
        return ["flaky_jobs must be a list (empty when no job is flaky)"]
    problems: list[str] = []
    seen: set[str] = set()
    for i, job in enumerate(jobs):
        where = f"flaky_jobs[{i}]"
        if not isinstance(job, dict):
            problems.append(f"{where} must be a mapping")
            continue
        problems += [f"{where}: unknown key: {k}" for k in sorted(set(job) - _FLAKY_KEYS, key=str)]
        for key in ("name", "reason", "reviewed_by"):
            if not _text(job.get(key)):
                problems.append(f"{where}.{key} is required")
        if _text(job.get("name")):
            if job["name"] in seen:
                problems.append(f"{where}.name repeats {job['name']!r}")
            seen.add(job["name"])
        if not _valid_date(job.get("reviewed_on")):
            problems.append(f"{where}.reviewed_on must be a YYYY-MM-DD date")
    return problems


def _valid_date(value: object) -> bool:
    if isinstance(value, datetime.datetime):
        return False
    if isinstance(value, datetime.date):  # YAML reads an unquoted date as a date
        return True
    if not isinstance(value, str) or not _DATE_RE.match(value):
        return False
    try:
        datetime.date.fromisoformat(value)
    except ValueError:
        return False
    return True


def load(repo: Path) -> dict[str, Any]:
    """The repository's validated setup file; raises ``SetupFileError`` otherwise."""
    path = repo / SETUP_FILE
    if not path.is_file():
        raise SetupFileError([f"{SETUP_FILE} is missing; run repo_setup.py write"])
    try:
        data = yaml.safe_load(path.read_text(encoding="utf-8"))
    except (yaml.YAMLError, UnicodeDecodeError) as exc:
        raise SetupFileError([f"{SETUP_FILE} is not valid YAML: {exc}"]) from exc
    problems = validate(data)
    if problems:
        raise SetupFileError(problems)
    assert isinstance(data, dict)
    return data


def build(
    tracker_repo: str,
    labels: list[str],
    triage: dict[str, str],
    flaky: list[tuple[str, str]],
    reviewed_by: str,
    reviewed_on: str,
) -> dict[str, Any]:
    """The setup file's content; triage roles not given map to themselves."""
    return {
        "version": VERSION,
        "tracker": {"kind": "github", "repo": tracker_repo},
        "labels": labels,
        "triage_labels": {role: triage.get(role, role) for role in TRIAGE_ROLES},
        "flaky_jobs": [
            {"name": name, "reason": reason, "reviewed_by": reviewed_by, "reviewed_on": reviewed_on}
            for name, reason in flaky
        ],
    }


def render(data: dict[str, Any]) -> str:
    return yaml.safe_dump(data, sort_keys=False, default_flow_style=False, allow_unicode=True)


def _git_tracker_repo(repo: Path) -> str:
    url = subprocess.run(
        ["git", "-C", str(repo), "remote", "get-url", "origin"], capture_output=True, text=True, check=False
    ).stdout.strip()
    match = re.search(r"(?:^|[@/])github\.com(?:-[\w.-]+)?[:/]([^/]+/[^/]+?)(?:\.git)?/?$", url)
    return match.group(1) if match else ""


def _gh_labels(tracker_repo: str) -> list[str]:
    out = subprocess.run(
        ["gh", "label", "list", "--repo", tracker_repo, "--limit", "1000", "--json", "name", "--jq", ".[].name"],
        capture_output=True,
        text=True,
        check=False,
    )
    if out.returncode != 0:
        print(f"repo-setup: gh label list failed: {out.stderr.strip()}", file=sys.stderr)
    return [line for line in out.stdout.splitlines() if line.strip()]


def _fail(problems: list[str]) -> int:
    for problem in problems:
        print(f"repo-setup: {problem}", file=sys.stderr)
    return 3


def cmd_write(args: argparse.Namespace) -> int:
    repo = Path(args.repo)
    tracker_repo = args.tracker_repo or _git_tracker_repo(repo)
    labels = list(dict.fromkeys(args.label)) or _gh_labels(tracker_repo)
    triage: dict[str, str] = {}
    for item in args.triage:
        role, sep, label = item.partition("=")
        if not sep or role not in TRIAGE_ROLES:
            return _fail([f"--triage {item!r} must be ROLE=LABEL with ROLE one of {', '.join(TRIAGE_ROLES)}"])
        triage[role] = label
    flaky = [(name, reason) for name, reason in args.flaky]
    if not repo.is_dir():
        return _fail([f"--repo {repo} is not a directory"])
    if flaky and not args.reviewed_by:
        return _fail(["--reviewed-by is required when a flaky job is listed"])
    data = build(
        tracker_repo,
        labels,
        triage,
        flaky,
        args.reviewed_by or "",
        args.reviewed_on or datetime.date.today().isoformat(),
    )
    if not flaky:  # a rerun without --flaky keeps the reviewed jobs already listed
        with contextlib.suppress(SetupFileError):
            data["flaky_jobs"] = load(repo)["flaky_jobs"]
    problems = validate(data)
    if problems:
        return _fail(problems)
    path = repo / SETUP_FILE
    text = render(data)
    if path.is_file() and path.read_text() == text:
        print("unchanged")
        return 0
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text)
    print("changed")
    return 0


def cmd_check(args: argparse.Namespace) -> int:
    try:
        load(Path(args.repo))
    except SetupFileError as exc:
        return _fail(exc.problems)
    print("ok")
    return 0


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest="command", required=True)
    write = sub.add_parser("write", help="write the setup file")
    write.add_argument("--repo", default=".")
    write.add_argument("--tracker-repo", default="")
    write.add_argument("--label", action="append", default=[])
    write.add_argument("--triage", action="append", default=[], metavar="ROLE=LABEL")
    write.add_argument("--flaky", nargs=2, action="append", default=[], metavar=("JOB", "REASON"))
    write.add_argument("--reviewed-by", default="")
    write.add_argument("--reviewed-on", default="")
    write.set_defaults(func=cmd_write)
    check = sub.add_parser("check", help="validate the setup file")
    check.add_argument("--repo", default=".")
    check.set_defaults(func=cmd_check)
    args = parser.parse_args(argv)
    code: int = args.func(args)
    return code


if __name__ == "__main__":
    sys.exit(main())
