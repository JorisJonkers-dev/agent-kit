#!/usr/bin/env python3
"""Offline mechanical checks of one estate repository (or template) directory.

Usage:
    audit.py REPO_DIR [--latest-sha SHA] [--json]

Prints one line per gap as "<file>: <gap> (source: <doc>)", or "no gaps".
Exit 0 when the audit ran, gaps or not. Exit 2 when REPO_DIR is not a
directory or --latest-sha is not a 40-hex SHA.

The source names are labels for live documents named in SKILL.md. This
script does not read them and copies nothing from them. Stdlib only.
"""

import argparse
import json
import os
import re
import sys
from pathlib import Path

PIN_RE = re.compile(r"JorisJonkers-dev/github-workflows/([^@\s'\"]+)@([^\s'\"#]+)")
SHA_RE = re.compile(r"^[0-9a-f]{40}$")
NAME_RE = re.compile(r"^name:\s*['\"]?Pipeline Complete['\"]?\s*$")
COMMENT_RE = re.compile(r"(^|\s)#.*$")
JSON_TOKEN_RE = re.compile(r'"(?:\\.|[^"\\])*"|//[^\n]*|/\*.*?\*/', re.DOTALL)
MAX_TEXT_BYTES = 1_000_000
RENOVATE_PRESET = "github>JorisJonkers-dev/renovate-config"
PRUNE = {".git", "node_modules", "vendor", ".venv", "target", "build"}

SRC_PIN = "github-workflows README.md"
SRC_RELEASE = "github-workflows latest release"
SRC_CI = "workspace docs/decisions/ADR-0018-repository-templates.md"
SRC_RENOVATE = "renovate-config README.md"
SRC_DEPLOY = "workspace CLAUDE.md (Flux reads deploy/production)"
SRC_NONE = "none (unverified)"
SRC_PROXY = "github-workflows README.md (packages-proxy-token is a deprecated alias)"
SRC_ORG = "workspace CLAUDE.md (old-org references are guarded)"


def rel(path, root):
    return path.relative_to(root).as_posix()


def workflow_files(root):
    wf = root / ".github" / "workflows"
    if not wf.is_dir():
        return []
    return sorted(p for p in wf.iterdir() if p.suffix in (".yml", ".yaml"))


def pin_files(root):
    """Workflows, composite actions, and the .tmpl workflow templates."""
    found = list(workflow_files(root))
    for base, suffixes in ((root / ".github" / "actions", (".yml", ".yaml")), (root / "templates", (".tmpl",))):
        for dirpath, dirnames, filenames in os.walk(base):
            dirnames[:] = [d for d in dirnames if d not in PRUNE]
            found += [Path(dirpath) / f for f in sorted(filenames) if f.endswith(suffixes)]
    return found


def strip_comment(line):
    return COMMENT_RE.sub("", line)


def check_pins(root, latest, gaps):
    for path in pin_files(root):
        for lineno, raw in enumerate(path.read_text(errors="replace").splitlines(), 1):
            line = strip_comment(raw)
            for m in PIN_RE.finditer(line):
                ref = m.group(2)
                where = f"line {lineno}: github-workflows {m.group(1)}@{ref}"
                if ref == "main":
                    gaps.append((rel(path, root), f"{where} pinned to main", SRC_PIN))
                elif not SHA_RE.match(ref):
                    gaps.append((rel(path, root), f"{where} not pinned by 40-hex SHA", SRC_PIN))
                elif latest and ref != latest:
                    gaps.append((rel(path, root), f"{where} is not the latest release SHA {latest}", SRC_RELEASE))


def count_pipeline_jobs(text):
    """Count job-level `name: Pipeline Complete` lines under the top-level jobs: key."""
    hits = 0
    in_jobs = False
    job_indent = None
    key_indent = None
    for raw in text.splitlines():
        line = strip_comment(raw).rstrip()
        if not line.strip():
            continue
        indent = len(line) - len(line.lstrip())
        if indent == 0:
            in_jobs = line.strip() == "jobs:"
            job_indent = key_indent = None
        elif in_jobs:
            if job_indent is None:
                job_indent = indent
            if indent == job_indent:
                key_indent = None
            elif key_indent is None:
                key_indent = indent
            if indent == key_indent and NAME_RE.match(line.strip()):
                hits += 1
    return hits


def check_pipeline(root, gaps):
    hits = sum(count_pipeline_jobs(p.read_text(errors="replace")) for p in workflow_files(root))
    if hits == 0:
        gaps.append((".github/workflows", "no job named Pipeline Complete (the one required check)", SRC_CI))
    elif hits > 1:
        gaps.append((".github/workflows", f"{hits} jobs named Pipeline Complete; expected one", SRC_CI))


def check_renovate(root, gaps):
    candidates = [root / "renovate.json", root / ".github" / "renovate.json"]
    present = [p for p in candidates if p.is_file()]
    if not present:
        gaps.append(("renovate.json", "no renovate.json at the root or in .github", SRC_RENOVATE))
        return
    path = present[0]
    text = JSON_TOKEN_RE.sub(
        lambda m: m.group(0) if m.group(0).startswith('"') else "", path.read_text(errors="replace")
    )
    try:
        data = json.loads(text)
    except ValueError:
        gaps.append((rel(path, root), "could not confirm the preset (renovate.json could not be parsed)", SRC_RENOVATE))
        return
    extends = data.get("extends", []) if isinstance(data, dict) else []
    if isinstance(extends, str):
        extends = [extends]
    if not isinstance(extends, list):
        extends = []
    if not any(isinstance(e, str) and (e == RENOVATE_PRESET or e.startswith(RENOVATE_PRESET + "#")) for e in extends):
        gaps.append((rel(path, root), f'does not extend "{RENOVATE_PRESET}"', SRC_RENOVATE))


PROXY_TOKEN = ("packages-proxy-token", SRC_PROXY)
SCOPED_TEXT = [
    ("deploy-config-schema", "deploy-kit README.md (deploy-config-schema is archived)"),
    ("ExtraToast/", SRC_ORG),
]
SCOPED_ROOTS = (".github", "deploy", "Taskfile.yml", "renovate.json")
SCOPED_SUFFIXES = (".yml", ".yaml", ".json", ".tf", ".sh", ".mjs")


def read_text_file(path):
    """The text of a small regular text file, or None for binary, large or non-regular files."""
    if not path.is_file() or path.stat().st_size > MAX_TEXT_BYTES:
        return None
    data = path.read_bytes()
    if b"\0" in data[:8192]:
        return None
    return data.decode(errors="replace")


def check_retired(root, gaps):
    if (root / "platform").is_dir():
        gaps.append(("platform/", "top-level platform/ directory; deploy files belong in deploy/", SRC_DEPLOY))
    for dirpath, dirnames, filenames in os.walk(root):
        dirnames[:] = [d for d in dirnames if d not in PRUNE]
        for name in sorted(filenames):
            path = Path(dirpath) / name
            relpath = rel(path, root)
            if name.lower() in ("fleet.yaml", "fleet.yml"):
                gaps.append(
                    (
                        relpath,
                        "fleet.yaml is flagged as retired (unverified: no live doc says so); projects are <name>.project.yml",
                        SRC_NONE,
                    )
                )
            text = read_text_file(path)
            if text is None:
                continue
            tokens = [PROXY_TOKEN]
            if relpath.split("/")[0] in SCOPED_ROOTS and name.endswith(SCOPED_SUFFIXES):
                tokens += SCOPED_TEXT
            for lineno, line in enumerate(text.splitlines(), 1):
                for token, source in tokens:
                    if token in line:
                        gaps.append((relpath, f"line {lineno}: retired reference {token!r}", source))


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[1])
    ap.add_argument("repo_dir")
    ap.add_argument("--latest-sha", default=None, help="40-hex SHA of the latest github-workflows release")
    ap.add_argument("--json", action="store_true", help="print a JSON list instead of lines")
    args = ap.parse_args(argv)

    root = Path(args.repo_dir).expanduser()
    if not root.is_dir():
        print(f"audit.py: not a directory: {args.repo_dir}", file=sys.stderr)
        return 2
    if args.latest_sha is not None and not SHA_RE.match(args.latest_sha):
        print("audit.py: --latest-sha must be 40 lowercase hex characters", file=sys.stderr)
        return 2

    gaps = []
    check_pins(root, args.latest_sha, gaps)
    check_pipeline(root, gaps)
    check_renovate(root, gaps)
    check_retired(root, gaps)

    if args.json:
        print(json.dumps([{"file": f, "gap": g, "source": s} for f, g, s in gaps], indent=2))
    elif not gaps:
        print("no gaps")
    else:
        for f, g, s in gaps:
            print(f"{f}: {g} (source: {s})")
    return 0


if __name__ == "__main__":
    sys.exit(main())
