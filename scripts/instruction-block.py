#!/usr/bin/env python3
"""Keep one marked block in a file and leave the rest of the file alone.

    instruction-block.py FILE NAME [--remove]

The block's body is read from stdin. It sits between
``<!-- agent-kit NAME begin -->`` and ``<!-- agent-kit NAME end -->``. A file
with no such block gets one appended; a file with one gets it replaced where
it stands; ``--remove`` takes it out. Every other line of the file is kept as
it is, and blocks named differently are never touched.

A file that opens the block and never closes it is left as it is, and the
script exits 2 so the owner can fix it. Prints ``changed`` or ``unchanged``.
"""

from __future__ import annotations

import argparse
import sys
from pathlib import Path


def _rewrite(lines: list[str], begin: str, end: str, block: list[str] | None) -> list[str]:
    """The lines with every block named by begin/end replaced by `block`, or dropped when it is None.

    The first block's place is where the new one goes; later ones are dropped.
    """
    out: list[str] = []
    placed = False
    index = 0
    while index < len(lines):
        if lines[index].strip() != begin:
            out.append(lines[index])
            index += 1
            continue
        close = next((i for i in range(index + 1, len(lines)) if lines[i].strip() == end), None)
        if close is None:
            raise ValueError(f"the block opened at line {index + 1} is never closed")
        if block is not None and not placed:
            out.extend(block)
            placed = True
        index = close + 1
    if block is not None and not placed:
        if out and out[-1].strip() != "":
            out.append("\n")
        out.extend(block)
    return out


def _read(path: Path) -> str:
    """The file's text with its own line endings, so CRLF lines stay CRLF."""
    with path.open(newline="") as handle:
        return handle.read()


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Keep one marked block in a file.")
    parser.add_argument("file", type=Path)
    parser.add_argument("name")
    parser.add_argument("--remove", action="store_true")
    args = parser.parse_args(argv)

    begin = f"<!-- agent-kit {args.name} begin -->"
    end = f"<!-- agent-kit {args.name} end -->"
    body = "" if args.remove else sys.stdin.read().rstrip("\n")
    original = _read(args.file) if args.file.exists() else ""
    newline = "\r\n" if "\r\n" in original else "\n"
    block = None
    if not args.remove:
        block = [f"{begin}{newline}", *(f"{line}{newline}" for line in body.split("\n")), f"{end}{newline}"]

    try:
        lines = original.splitlines(keepends=True)
        new_lines = _rewrite(lines, begin, end, block)
    except ValueError as error:
        print(f"{args.file}: the agent-kit {args.name} block: {error}; left alone", file=sys.stderr)
        return 2
    text = "".join(new_lines)
    if text == original:
        print("unchanged")
        return 0
    args.file.parent.mkdir(parents=True, exist_ok=True)
    with args.file.open("w", newline="") as handle:
        handle.write(text)
    print("changed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
