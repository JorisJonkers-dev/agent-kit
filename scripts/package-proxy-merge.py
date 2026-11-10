#!/usr/bin/env python3
"""Set up one marked block in a config file, keeping the rest of the file.

    package-proxy-merge.py FILE NAME [--remove] [--validate {toml,ini}]
                            [--refuse REGEX] [--mode MODE]

The block's body is read from stdin. It sits between
``# agent-kit NAME begin`` and ``# agent-kit NAME end`` -- `#` comments,
which npmrc, java .properties, TOML and pip.conf all take. A file with
no such block gets one appended; a file with one gets that block
replaced where it stands; ``--remove`` takes the block out. Every other
line of the file is kept as it is.

``--validate`` parses the merged result before anything is written
(``toml`` through tomllib, ``ini`` through configparser) and refuses
the merge when it does not parse. ``--refuse`` refuses the merge when a
line of the file outside the block matches REGEX -- the shell uses it
to leave a file alone rather than mangle a user setting the block would
clash with. ``--mode`` gives the file's mode (0600 for a file that
holds a key); without it, an existing file keeps its own mode and a new
file takes the umask's default.

A file that opens the block and never closes it is left as it is, and
the script exits 2 so the owner can fix it. On a refused merge it
prints one line naming the conflict to stderr and exits 3. Prints
``changed`` or ``unchanged``.
"""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path


def _markers(name: str) -> tuple[str, str]:
    """The begin and end lines of setup's block in one file."""
    return f"# agent-kit {name} begin", f"# agent-kit {name} end"


def _block_bounds(lines: list[str], begin: str, end: str) -> tuple[int | None, int | None]:
    """Where the first block of this name stands, or (None, None) when none does.

    A block that opens and never closes raises: merging under a half-written
    block would corrupt the file, so the caller leaves the file alone instead.
    """
    start = next((i for i, line in enumerate(lines) if line.strip() == begin), None)
    if start is None:
        return None, None
    stop = next((i for i in range(start + 1, len(lines)) if lines[i].strip() == end), None)
    if stop is None:
        raise ValueError(f"the block opened at line {start + 1} is never closed")
    return start, stop


def _without_block(lines: list[str], begin: str, end: str) -> list[str]:
    """The lines with every block of this name taken out."""
    out: list[str] = []
    skipping = False
    for line in lines:
        stripped = line.strip()
        if not skipping and stripped == begin:
            skipping = True
            continue
        if skipping and stripped == end:
            skipping = False
            if out and not out[-1].endswith("\n"):
                out[-1] += "\n"
            continue
        if not skipping:
            out.append(line)
    return out


def _append_block(lines: list[str], block: list[str]) -> list[str]:
    """The lines with one block appended, kept apart by a blank line."""
    out = list(lines)
    if out and not out[-1].endswith("\n"):
        out[-1] += "\n"
    if out and out[-1].strip() != "":
        out.append("\n")
    out.extend(block)
    return out


def _parse(kind: str, text: str) -> str | None:
    """Why the text does not parse, or None when it does."""
    if kind == "toml":
        import tomllib

        try:
            tomllib.loads(text)
        except (tomllib.TOMLDecodeError, ValueError) as exc:
            return str(exc)
        return None
    if kind == "ini":
        import configparser

        parser = configparser.ConfigParser(interpolation=None, strict=True)
        try:
            parser.read_string(text)
        except configparser.Error as exc:
            return str(exc)
        return None
    raise SystemExit(f"package-proxy-merge: {kind!r} is not a validation kind")


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Set up one marked block in a config file.")
    parser.add_argument("file", type=Path)
    parser.add_argument("name")
    parser.add_argument("--remove", action="store_true")
    parser.add_argument("--validate", choices=["toml", "ini"], default=None)
    parser.add_argument("--refuse", metavar="REGEX", default=None)
    parser.add_argument("--mode", metavar="MODE", default=None)
    args = parser.parse_args(argv)

    begin, end = _markers(args.name)
    try:
        refuse = re.compile(args.refuse) if args.refuse is not None else None
    except re.error as exc:
        print(f"package-proxy-merge: {args.refuse!r} is not a regex: {exc}", file=sys.stderr)
        return 2

    existing = args.file.read_text() if args.file.is_file() else ""
    lines = existing.splitlines(keepends=True)
    try:
        start, stop = _block_bounds(lines, begin, end)
    except ValueError as exc:
        print(f"package-proxy-merge: {args.file}: {exc}", file=sys.stderr)
        return 2

    # The conflict check reads the file WITHOUT this setup's block, so a
    # re-run does not trip over the block it wrote the time before.
    if refuse is not None:
        for number, line in enumerate(_without_block(lines, begin, end), start=1):
            if refuse.search(line):
                # Name the property only, never its value: a user's own
                # password or token must not reach the output.
                key, sep, _ = line.partition("=")
                named = f" ({key.strip()!r})" if sep else ""
                print(
                    f"package-proxy-merge: refusing to merge into {args.file}: "
                    f"line {number}{named} is outside the block "
                    "and must not be overridden",
                    file=sys.stderr,
                )
                return 3

    if args.remove:
        if start is None:
            print("unchanged")
            return 0
        merged = _without_block(lines, begin, end)
    else:
        # The block's body arrives on stdin; the written block is that
        # body inside the marker lines.
        body = [line + "\n" for line in sys.stdin.read().splitlines()]
        if not body:
            print("package-proxy-merge: the block body is empty", file=sys.stderr)
            return 2
        block = [begin + "\n", *body, end + "\n"]
        if start is not None and stop is not None:
            merged = lines[:start] + block + lines[stop + 1 :]
        else:
            merged = _append_block(lines, block)

    merged_text = "".join(merged)
    if merged_text == existing:
        print("unchanged")
        return 0

    # Validate the merged result BEFORE anything is written, so a bad
    # merge never lands on disk in the first place.
    if args.validate is not None:
        bad = _parse(args.validate, merged_text)
        if bad is not None:
            print(
                f"package-proxy-merge: refusing to write {args.file}: "
                f"the merged result does not parse: {bad}",
                file=sys.stderr,
            )
            return 3

    args.file.parent.mkdir(parents=True, exist_ok=True)
    if args.file.is_file():
        backup = args.file.with_name(args.file.name + ".bak-agent-kit")
        if not backup.is_file():
            backup.write_text(existing)
    if args.mode is not None:
        mode = int(args.mode, 8)
    elif args.file.is_file():
        # Keep an existing file's own bits; only the content changes.
        mode = args.file.stat().st_mode & 0o777
    else:
        # 0o666, passed to open() so the umask still applies.
        mode = 0o666

    import os

    fd = os.open(args.file, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, mode)
    with os.fdopen(fd, "w") as handle:
        handle.write(merged_text)
    if args.mode is not None:
        # os.open's mode applies to new files only; chmod pins an
        # existing one to the asked-for mode too.
        os.chmod(args.file, mode)
    print("changed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
