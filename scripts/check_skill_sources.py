#!/usr/bin/env python3
"""Check every estate skill's recorded sources against GitHub.

Each estate skill (a `skills/workstation/estate-*` folder) carries a
`sources.yaml` beside its `SKILL.md`, listing the docs it was checked against:
a `file` entry pins the git blob sha of one path on the repository's default
branch, a `release` entry pins the repository's latest release tag. The
default run asks GitHub for each source's current blob or latest tag and lists
the sources that moved: the blob changed, the path is gone, or a newer release
exists. A request that fails (an auth error, a rate limit, a timeout after
bounded retries) is reported as a failure and never as a moved or gone source,
and the run exits non-zero.

Modes:
- `--validate` only checks that every `sources.yaml` is well formed: a
  non-empty `sources` list of entries with `kind` file or release, `repo`
  `<owner>/<name>`, a `path` and 40-hex `blob` for a file, a `tag` for a
  release, and an ISO `checked` date. It passes when there are no estate
  skills yet. CI runs it in the Python job.
- `--update <skill>` rewrites that skill's blobs, tags and dates from GitHub,
  after a person has rechecked the sources.
- `--sync-issue` keeps one open issue, found by its title, listing the moved
  sources: it opens the issue when drift appears, updates it while drift
  remains, and closes it only after a successful run finds none. A failed run
  leaves the issue untouched; the failed run itself is the signal.

Only repository paths, blob shas and tags travel through `sources.yaml` and
the drift issue. File content is never asked for beyond GitHub's metadata
reply and never written anywhere.
"""

from __future__ import annotations

import argparse
import datetime
import json
import re
import subprocess
import sys
from dataclasses import dataclass, field
from pathlib import Path
from urllib.parse import quote

import yaml

SOURCES_FILE = "sources.yaml"
BLOB = re.compile(r"[0-9a-f]{40}")
REPO = re.compile(r"[A-Za-z0-9][\w.-]*/[\w.-]+")
DATE = re.compile(r"\d{4}-\d{2}-\d{2}")
KINDS = {"file", "release"}
RETRIES = 3
TIMEOUT = 60  # seconds per gh call
DEFAULT_ISSUE_TITLE = "Skill sources drift"
# A reply that suggests the request may succeed later; anything else fails at once.
TRANSIENT = ("timeout", "timed out", "deadline exceeded", "connection", "http 5")


class CheckError(Exception):
    """A sources.yaml or a skill name cannot be used as it stands."""


class RequestFailure(Exception):
    """A GitHub request failed: auth, rate limit, or a timeout after retries."""


@dataclass
class Entry:
    """One well-formed sources.yaml entry, keeping its raw mapping for rewrites."""

    kind: str
    repo: str
    path: str
    blob: str
    tag: str
    checked: str
    raw: dict[str, object]


@dataclass
class Report:
    """What one drift check found: moved sources and failed requests, kept apart."""

    drift: list[str] = field(default_factory=list)
    failures: list[str] = field(default_factory=list)


def gh(args: list[str]) -> str:
    """Run `gh`, bounded-retrying transient failures; a failed request raises."""
    error = "gh made no attempt"
    for attempt in range(1, RETRIES + 1):
        try:
            result = subprocess.run(["gh", *args], capture_output=True, text=True, check=False, timeout=TIMEOUT)
        except subprocess.TimeoutExpired:
            error = "timeout waiting for gh"
            if attempt == RETRIES:
                break
            continue
        except OSError as failure:
            raise RequestFailure(str(failure)) from failure
        if result.returncode == 0:
            return result.stdout
        error = result.stderr.strip() or f"gh exited {result.returncode}"
        if not is_transient(error) or attempt == RETRIES:
            break
    raise RequestFailure(error)


def is_transient(error: str) -> bool:
    return any(word in error.lower() for word in TRANSIENT)


def _json(reply: str, label: str) -> object:
    try:
        return json.loads(reply)
    except ValueError as error:
        raise RequestFailure(f"{label}: GitHub's reply is not JSON") from error


def repo_reachable(repo: str) -> bool:
    """Whether the repository itself is visible: a 404 on a path means little when it is not."""
    try:
        gh(["api", f"repos/{repo}"])
    except RequestFailure as error:
        if "HTTP 404" in str(error):
            raise RequestFailure(f"{repo}: not found or not visible to this token (HTTP 404)") from error
        raise
    return True


def current_blob(repo: str, path: str) -> str | None:
    """The blob sha of one file on the default branch, or None when it is gone."""
    try:
        reply = gh(["api", "-H", "Accept: application/vnd.github.object+json",
                    f"repos/{repo}/contents/{quote(path, safe='/')}"])
    except RequestFailure as error:
        if "HTTP 404" in str(error) and repo_reachable(repo):
            return None
        raise
    data = _json(reply, f"{repo} {path}")
    sha = data.get("sha") if isinstance(data, dict) else None
    if not isinstance(sha, str):
        raise RequestFailure(f"{repo} {path}: GitHub's reply names no blob")
    return sha


def latest_tag(repo: str) -> str | None:
    """The repository's latest release tag, or None when it has no release."""
    try:
        reply = gh(["api", f"repos/{repo}/releases/latest"])
    except RequestFailure as error:
        if "HTTP 404" in str(error) and repo_reachable(repo):
            return None
        raise
    data = _json(reply, f"{repo} release")
    tag = data.get("tag_name") if isinstance(data, dict) else None
    if not isinstance(tag, str):
        raise RequestFailure(f"{repo}: GitHub's reply names no release tag")
    return tag


def is_date(text: str) -> bool:
    if not DATE.fullmatch(text):
        return False
    try:
        datetime.date.fromisoformat(text)
    except ValueError:
        return False
    return True


def parse_entry(item: object) -> tuple[Entry | None, list[str]]:
    """One entry and the problems that make it not well formed."""
    if not isinstance(item, dict):
        return None, ["entry is not a mapping"]
    raw: dict[str, object] = item
    problems: list[str] = []
    kind = raw.get("kind")
    if kind not in KINDS:
        problems.append("kind must be file or release")
    repo = raw.get("repo")
    if not isinstance(repo, str) or not REPO.fullmatch(repo):
        problems.append("repo must be <owner>/<name>")
    path = raw.get("path")
    blob = raw.get("blob")
    tag = raw.get("tag")
    if kind == "file":
        if not isinstance(path, str) or not path:
            problems.append("path must name a file in the repository")
        if not isinstance(blob, str) or not BLOB.fullmatch(blob):
            problems.append("blob must be a 40-hex git blob sha")
    if kind == "release" and (not isinstance(tag, str) or not tag):
        problems.append("tag must name a release")
    checked = raw.get("checked")
    if isinstance(checked, datetime.date):
        checked = checked.isoformat()  # YAML reads an unquoted ISO date as a date
    if not isinstance(checked, str) or not is_date(checked):
        problems.append("checked must be an ISO date")
    if problems:
        return None, problems
    return Entry(kind=str(kind), repo=str(repo), path=str(path or ""), blob=str(blob or ""),
                 tag=str(tag or ""), checked=str(checked), raw=raw), []


def parse(skill: Path) -> tuple[list[Entry], list[str]]:
    """One skill's sources and the problems that make its sources.yaml not well formed."""
    path = skill / SOURCES_FILE
    if not path.is_file():
        return [], [f"{skill.name}: has no {SOURCES_FILE}"]
    try:
        data = yaml.safe_load(path.read_text())
    except (OSError, UnicodeDecodeError, yaml.YAMLError) as error:
        return [], [f"{skill.name}: {SOURCES_FILE} cannot be read: {error}"]
    if not isinstance(data, dict) or not isinstance(data.get("sources"), list):
        return [], [f"{skill.name}: {SOURCES_FILE} does not hold a sources list"]
    sources = data["sources"]
    if not sources:
        return [], [f"{skill.name}: {SOURCES_FILE} lists no sources"]
    entries: list[Entry] = []
    problems: list[str] = []
    for number, item in enumerate(sources, start=1):
        entry, entry_problems = parse_entry(item)
        if entry is None:
            problems += [f"{skill.name}: {SOURCES_FILE} entry {number}: {problem}"
                         for problem in entry_problems]
        else:
            entries.append(entry)
    return entries, problems


def skills(root: Path) -> list[Path]:
    """The estate skill folders under the skills root."""
    return sorted(path for path in root.glob("estate-*") if path.is_dir())


def validate(root: Path) -> list[str]:
    """Every problem that keeps a sources.yaml from being well formed."""
    problems: list[str] = []
    for skill in skills(root):
        problems += parse(skill)[1]
    return problems


def check(root: Path) -> Report:
    """List the sources that moved, and the requests that failed, never mixed."""
    report = Report()
    for skill in skills(root):
        for entry in parse(skill)[0]:
            label = f"{skill.name}: {entry.repo}"
            if entry.kind == "file":
                try:
                    current = current_blob(entry.repo, entry.path)
                except RequestFailure as error:
                    report.failures.append(f"{label} {entry.path} check failed: {error}")
                    continue
                if current is None:
                    report.drift.append(f"{label} {entry.path} is gone")
                elif current != entry.blob:
                    report.drift.append(f"{label} {entry.path} moved (blob {entry.blob} -> {current})")
            else:
                try:
                    current = latest_tag(entry.repo)
                except RequestFailure as error:
                    report.failures.append(f"{label} release check failed: {error}")
                    continue
                if current is None:
                    report.drift.append(f"{label} release is gone (recorded {entry.tag})")
                elif current != entry.tag:
                    report.drift.append(f"{label} has a newer release {current} (recorded {entry.tag})")
    return report


def update(root: Path, name: str) -> list[str]:
    """Rewrite one skill's blobs, tags and dates from what GitHub holds now."""
    if "/" in name or not name.startswith("estate-"):
        raise CheckError(f"{name}: not an estate skill folder name")
    skill = root / name
    if not skill.is_dir():
        raise CheckError(f"{name}: no such skill under {root}")
    entries, problems = parse(skill)
    if problems:
        raise CheckError("\n".join(problems))
    today = datetime.date.today().isoformat()
    lines = [f"{name}: recording what GitHub holds today ({today})"]
    for entry in entries:
        if entry.kind == "file":
            blob = current_blob(entry.repo, entry.path)
            if blob is None:
                raise CheckError(f"{name}: {entry.repo} {entry.path} is gone; nothing to record")
            entry.raw["blob"] = blob
            lines.append(f"{name}: {entry.repo} {entry.path} blob {blob}")
        else:
            tag = latest_tag(entry.repo)
            if tag is None:
                raise CheckError(f"{name}: {entry.repo} has no release; nothing to record")
            entry.raw["tag"] = tag
            lines.append(f"{name}: {entry.repo} release {tag}")
        entry.raw["checked"] = today
    (skill / SOURCES_FILE).write_text(yaml.safe_dump({"sources": [entry.raw for entry in entries]}, sort_keys=False))
    return lines


def find_issue(title: str) -> int | None:
    """The number of the one open issue with this title, if there is one."""
    data = _json(gh(["issue", "list", "--state", "open", "--limit", "1000", "--json", "number,title"]), "gh issue list")
    for issue in data if isinstance(data, list) else []:
        if isinstance(issue, dict) and issue.get("title") == title:
            number = issue.get("number")
            if isinstance(number, int):
                return number
    return None


def issue_body(drift: list[str]) -> str:
    listing = "\n".join(f"- {line}" for line in drift)
    return (f"Sources that moved since {SOURCES_FILE} was last updated:\n\n{listing}\n\n"
            "After rechecking a source, run `scripts/check_skill_sources.py --update <skill>`.\n")


def sync_issue(title: str, drift: list[str]) -> str:
    """Keep one open issue in step with the drift report; say what was done."""
    number = find_issue(title)
    if drift:
        body = issue_body(drift)
        if number is None:
            gh(["issue", "create", "--title", title, "--body", body])
            return f"opened {title!r} listing {len(drift)} moved source(s)"
        gh(["issue", "edit", str(number), "--body", body])
        return f"updated {title!r} (#{number}) listing {len(drift)} moved source(s)"
    if number is not None:
        gh(["issue", "close", str(number)])
        return f"closed {title!r} (#{number})"
    return f"{title!r}: no drift and no issue open"


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Check each estate skill's recorded sources against GitHub.")
    parser.add_argument("--root", type=Path, default=Path("skills/workstation"),
                        help="folder holding the estate-* skill folders (default: skills/workstation)")
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument("--validate", action="store_true", help="only check that every sources.yaml is well formed")
    mode.add_argument("--update", metavar="SKILL", help="rewrite one skill's blobs, tags and dates from GitHub")
    mode.add_argument("--sync-issue", action="store_true", help="run the check and keep one open issue in step with it")
    parser.add_argument("--issue-title", default=DEFAULT_ISSUE_TITLE, help="the title --sync-issue finds its issue by")
    args = parser.parse_args(argv)
    try:
        if not args.root.is_dir():
            raise CheckError(f"{args.root}: no such skills folder")
        if args.validate:
            problems = validate(args.root)
            for problem in problems:
                print(problem)
            print(f"{len(problems)} problem(s)" if problems else "every sources.yaml is well formed")
            return 1 if problems else 0
        if args.update is not None:
            for line in update(args.root, args.update):
                print(line)
            return 0
        problems = validate(args.root)
        if problems:
            for problem in problems:
                print(problem)
            return 1
        report = check(args.root)
        for line in [*report.drift, *report.failures]:
            print(line)
        if report.drift:
            print(f"{len(report.drift)} source(s) moved")
        if report.failures:
            print(f"{len(report.failures)} check(s) failed")
        if not report.drift and not report.failures:
            print("no sources moved")
        if args.sync_issue:
            if report.failures:
                print("the drift issue is left untouched")
                return 1
            print(sync_issue(args.issue_title, report.drift))
            return 0
        return 1 if report.drift or report.failures else 0
    except (CheckError, RequestFailure) as error:
        print(f"check failed: {error}")
        return 1


if __name__ == "__main__":
    sys.exit(main())
