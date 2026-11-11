#!/usr/bin/env python3
"""Check that each repository's AGENTS.md names only things that exist.

AGENTS.md is the file a coding agent reads first, so a path or a symbol it
names that has since moved is worse than no pointer at all: the agent follows
it and fails. This lint reads each repository root it is given and reports:

- a missing or empty `AGENTS.md` (whitespace-only counts as empty);
- a markdown link whose target does not exist, resolved against the repository
  root, after a `#fragment` or `?query` is stripped;
- an inline code span that looks like a repository path and does not exist;
- a `<path>::<symbol>` span whose path is gone, or whose symbol no longer
  appears as a whole word in that file (or, for a directory, in any file under
  it).

Fenced code blocks are skipped entirely, and so are links with a scheme
(`https:`, `mailto:`, ...) and pure `#anchor` links. Only spans that look like
repository paths are checked: no whitespace, no glob or shell characters, not
absolute, not a URL, and either containing a `/` or ending in a known file
extension.

Problems go to stderr, one per line, as `<REPO_ROOT>/AGENTS.md:<line>: <message>`
with the root spelled as it was given. A clean run prints
`agents-md: <n> repositories checked` to stdout. The exit code is 0 when every
repository is clean, 1 when any problem was found, and 2 when a REPO_ROOT is
not a directory.
"""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path
from urllib.parse import unquote

AGENTS_MD = "AGENTS.md"
FENCE = re.compile(r"^\s{0,3}(`{3,}|~{3,})")
LINK = re.compile(r"\[[^\]]*\]\(<?([^)\s>]+)>?(?:\s+\"[^\"]*\")?\)")
SPAN = re.compile(r"`([^`\n]+)`")
SCHEME = re.compile(r"^[A-Za-z][A-Za-z0-9+.-]*:")
SYMBOL = re.compile(r"^(?P<path>.+?)::(?P<symbol>\w+(?:(?:\.|::)\w+)*)(?:\(\))?$")
# Characters that make a span a pattern, a shell fragment or a placeholder rather than a path.
NOT_A_PATH = frozenset("*?<>${}|~")
EXTENSIONS = frozenset(
    [
        "md",
        "py",
        "rs",
        "ts",
        "tsx",
        "js",
        "mjs",
        "json",
        "yaml",
        "yml",
        "toml",
        "sh",
        "nix",
        "kt",
        "kts",
        "go",
        "lock",
        "txt",
        "cfg",
    ]
)


def _unfenced_lines(text: str) -> list[tuple[int, str]]:
    """Return the (1-based line number, text) pairs outside fenced code blocks."""
    kept: list[tuple[int, str]] = []
    fence = ""
    for number, line in enumerate(text.splitlines(), start=1):
        match = FENCE.match(line)
        if fence:
            if match and match.group(1)[0] == fence[0] and len(match.group(1)) >= len(fence):
                fence = ""
            continue
        if match:
            fence = match.group(1)
            continue
        kept.append((number, line))
    return kept


def _clean(span: str) -> str:
    """Strip the trailing slash and the leading `./` a span may carry."""
    candidate = re.sub(r"(?::\d+)?(?:\(\))?$", "", span).rstrip("/")
    return candidate[2:] if candidate.startswith("./") else candidate


def _looks_like_path(span: str) -> bool:
    """Say whether an inline code span is meant as a repository path."""
    if not span or any(character.isspace() for character in span):
        return False
    if any(character in NOT_A_PATH for character in span):
        return False
    if span.startswith(("-", "/", "~")) or "://" in span:
        return False
    candidate = _clean(span)
    if span.endswith("/") or "/" in candidate:
        return True
    stem, dot, extension = candidate.rpartition(".")
    return bool(dot and stem and extension in EXTENSIONS)


def _resolve(root: Path, span: str) -> Path | None:
    """Resolve a span against the repository root, or None when it is not there."""
    path = root / _clean(span)
    return path if path.exists() else None


def _mentions(path: Path, symbol: str) -> bool:
    """Say whether `symbol` appears as a whole word in a file, or under a directory."""
    pattern = re.compile(rf"\b{re.escape(symbol)}\b")
    files = [path] if path.is_file() else [entry for entry in path.rglob("*") if entry.is_file()]
    return any(pattern.search(file.read_text(encoding="utf-8", errors="ignore")) for file in files)


def _link_problem(root: Path, target: str) -> str:
    """The problem a markdown link target causes, or an empty string."""
    if not target or target.startswith("#") or SCHEME.match(target):
        return ""
    path = target.split("#", 1)[0].split("?", 1)[0]
    if not path:
        return ""
    if not (root / unquote(path).lstrip("/")).exists():
        return f"dead path: {path}"
    return ""


def _span_problem(root: Path, span: str) -> str:
    """The problem an inline code span causes, or an empty string."""
    reference = SYMBOL.match(span)
    if reference:
        path = reference.group("path")
        if not _looks_like_path(path):
            return ""
        resolved = _resolve(root, path)
        if resolved is None:
            return f"dead path: {path}"
        names = re.split(r"\.|::", reference.group("symbol"))
        return "" if all(_mentions(resolved, name) for name in names) else f"dead symbol: {span}"
    if not _looks_like_path(span):
        return ""
    return "" if _resolve(root, span) is not None else f"dead path: {_clean(span)}"


def _line_problems(root: Path, line: str) -> list[str]:
    """The problems one line causes, in the order they appear on it."""
    found: list[tuple[int, str]] = []
    for match in LINK.finditer(line):
        message = _link_problem(root, match.group(1))
        if message:
            found.append((match.start(), message))
    for match in SPAN.finditer(line):
        message = _span_problem(root, match.group(1))
        if message:
            found.append((match.start(), message))
    return [message for _, message in sorted(found)]


def check_repo(root: Path) -> list[str]:
    """Return the problem lines for one repository, each prefixed with its AGENTS.md path."""
    label = f"{root}/{AGENTS_MD}"
    path = root / AGENTS_MD
    if not path.is_file():
        return [f"{label}:1: {AGENTS_MD} is missing"]
    try:
        text = path.read_text(encoding="utf-8")
    except UnicodeDecodeError:
        return [f"{label}:1: {AGENTS_MD} is not valid utf-8"]
    if not text.strip():
        return [f"{label}:1: {AGENTS_MD} is empty"]
    problems: list[str] = []
    for number, line in _unfenced_lines(text):
        seen: set[str] = set()
        for message in _line_problems(root, line):
            if message not in seen:
                seen.add(message)
                problems.append(f"{label}:{number}: {message}")
    return problems


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        prog="check_agents_md.py",
        description="Check each repository's AGENTS.md for dead paths and symbols.",
    )
    parser.add_argument(
        "--require",
        action="store_true",
        help="accepted for compatibility; AGENTS.md is always required",
    )
    parser.add_argument(
        "roots",
        nargs="*",
        metavar="REPO_ROOT",
        help="repository root to check (default: the current directory)",
    )
    args = parser.parse_args(argv)
    roots = [Path(root) for root in args.roots] or [Path()]
    for root in roots:
        if not root.is_dir():
            print(f"{root}: not a directory", file=sys.stderr)
            return 2
    problems = [line for root in roots for line in check_repo(root)]
    for line in problems:
        print(line, file=sys.stderr)
    if problems:
        return 1
    print(f"agents-md: {len(roots)} repositories checked")
    return 0


if __name__ == "__main__":
    sys.exit(main())
