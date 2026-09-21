#!/usr/bin/env python3
"""Pure helpers for stamping and checking the release version token.

Split out of scripts/publish-installer-artifacts.sh so the comparison logic
that actually decides pass/fail is unit-testable without a network call.
"""

from __future__ import annotations

import argparse
import hashlib
import sys
from pathlib import Path

VERSION_PREFIX = "# agent-kit "


def stamp_version(script_text: str, tag: str) -> str:
    """Insert the version token as line 2, right after the shebang."""
    lines = script_text.splitlines(keepends=True)
    if not lines or not lines[0].startswith("#!"):
        raise ValueError("script has no shebang on line 1")
    stamp = f"{VERSION_PREFIX}{tag}\n"
    return lines[0] + stamp + "".join(lines[1:])


def extract_version(script_text: str) -> str | None:
    """Return the stamped version token, or None if line 2 does not carry one."""
    lines = script_text.splitlines()
    if len(lines) < 2 or not lines[1].startswith(VERSION_PREFIX):
        return None
    return lines[1][len(VERSION_PREFIX) :]


def check_version(script_text: str, expected_tag: str) -> None:
    actual = extract_version(script_text)
    if actual != expected_tag:
        raise SystemExit(f"version token mismatch: expected {expected_tag!r}, got {actual!r}")


def sha256_of(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def parse_sha256sum_line(line: str) -> tuple[str, str]:
    """Parse one `sha256sum` output line: '<hex>  <name>' (two spaces, binary mode)."""
    stripped = line.strip()
    parts = stripped.split(maxsplit=1)
    if len(parts) != 2:
        raise ValueError(f"not a sha256sum line: {line!r}")
    digest, name = parts
    return digest, name.lstrip("*")


def check_checksum_file(sha256_file_text: str, tarball_path: Path) -> None:
    first_line = sha256_file_text.strip().splitlines()[0]
    digest, name = parse_sha256sum_line(first_line)
    if name != tarball_path.name:
        raise SystemExit(f"checksum file names {name!r}, expected {tarball_path.name!r}")
    actual = sha256_of(tarball_path)
    if actual != digest:
        raise SystemExit(f"checksum mismatch: file claims {digest}, tarball hashes to {actual}")


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)

    stamp_parser = sub.add_parser("stamp-version", help="stdin: script; stdout: script with the token inserted")
    stamp_parser.add_argument("tag")

    check_parser = sub.add_parser("check-version", help="stdin: script; fails unless its token matches TAG")
    check_parser.add_argument("tag")

    checksum_parser = sub.add_parser("check-checksum", help="fails unless SHA256_FILE names and matches TARBALL")
    checksum_parser.add_argument("tarball", type=Path)
    checksum_parser.add_argument("sha256_file", type=Path)

    args = parser.parse_args(argv)

    if args.command == "stamp-version":
        sys.stdout.write(stamp_version(sys.stdin.read(), args.tag))
    elif args.command == "check-version":
        check_version(sys.stdin.read(), args.tag)
    elif args.command == "check-checksum":
        check_checksum_file(args.sha256_file.read_text(), args.tarball)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
