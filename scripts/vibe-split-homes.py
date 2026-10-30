#!/usr/bin/env python3
"""Split the one Vibe home of before into the shared home and vibe-private's.

Before vibe-private existed, every Vibe run shared one home, so its session
logs and history hold conversations that recalled Private Memory. That whole
home becomes vibe-private's, history and all. A fresh shared home (vibe,
vibe-study) gets only the settings and the login: never logs, sessions or history.
The fresh home is built beside the old one first, so a failure changes nothing
and the next run tries again.

Runs once, and only for a home that held Private Memory: when vibe-private's
home already exists, there is no old home, or it never had Private Memory,
it changes nothing. It never deletes anything. Stdlib only.

Usage::

    python3 scripts/vibe-split-homes.py --shared ~/.vibe --private ~/.vibe-private [--check]
"""

from __future__ import annotations

import argparse
import re
import shutil
import sys
from pathlib import Path

# What the fresh shared home is seeded with: settings and the login (``.env``
# holds Vibe's login where there is no keychain item), never state.
SETTINGS_FILES = ("config.toml", "hooks.toml", "AGENTS.md", ".env")
SETTINGS_DIRS = ("skills", "prompts", "agents")


def say(message: str) -> None:
    print(f"vibe-split-homes: {message}")


# The setup of before merged this server into the one home; only a home that
# had it can hold sessions that recalled Private Memory.
PRIVATE_SERVER = re.compile(r'^\s*name\s*=\s*["\']memory-private["\']\s*$', re.MULTILINE)


def held_private_memory(home: Path) -> bool:
    config = home / "config.toml"
    return config.is_file() and bool(PRIVATE_SERVER.search(config.read_text(errors="replace")))


def _seed(source: Path, staging: Path) -> list[str]:
    """Build the fresh shared home beside the old one, so a failure leaves the old one whole."""
    staging.mkdir(mode=0o700)
    seeded = []
    for name in SETTINGS_FILES:
        if (source / name).is_file():
            shutil.copy2(source / name, staging / name)
            seeded.append(name)
    for name in SETTINGS_DIRS:
        if (source / name).is_dir():
            shutil.copytree(source / name, staging / name, symlinks=True)
            seeded.append(f"{name}/")
    return seeded


def split(shared: Path, private: Path, check: bool = False) -> int:
    staging = shared.with_name(shared.name + ".split")
    if private.exists():
        if not shared.exists() and staging.is_dir():
            # Stopped between the two renames: finish it.
            if check:
                say(f"would finish the split: {staging} becomes {shared}")
            else:
                staging.rename(shared)
                say(f"finished an interrupted split: {staging} is now {shared}")
            return 0
        say(f"{private} exists; already split, nothing to do")
        return 0
    if not shared.is_dir():
        say(f"no {shared} to split; nothing to do")
        return 0
    if not held_private_memory(shared):
        say(f"{shared} never had Private Memory; it stays the shared home")
        return 0
    if check:
        say(f"would move {shared} to {private} and start a fresh shared Vibe home")
        return 0
    if staging.exists():
        shutil.rmtree(staging)  # a copy from a run that failed: settings only, rebuilt below
    try:
        seeded = _seed(shared, staging)
    except OSError:
        shutil.rmtree(staging, ignore_errors=True)
        raise
    shared.rename(private)
    staging.rename(shared)
    say(f"moved {shared} to {private}, with its history")
    say(f"seeded {shared} with {', '.join(seeded) or 'nothing'}; no history")
    return 0


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--shared", type=Path, required=True)
    parser.add_argument("--private", type=Path, required=True)
    parser.add_argument("--check", action="store_true", help="say what a run would do; change nothing")
    args = parser.parse_args(argv)
    try:
        return split(args.shared, args.private, args.check)
    except OSError as e:
        print(f"vibe-split-homes: stopped, {args.shared} left as it was: {e}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
