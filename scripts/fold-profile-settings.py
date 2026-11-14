#!/usr/bin/env python3
"""Fold a secondary Claude profile's own settings.json back into the shared one.

A secondary profile (``~/.claude-personal``) shares the primary's
``settings.json`` through a symlink, because ``enabledPlugins`` and the
permission rules live there. When the profile gets a real file instead, for
example because Claude Code ran with that config root before setup linked it,
every plugin is off in that profile and later setup runs used to leave it so.

``fold-profile-settings.py SHARED COPY [--check]`` merges COPY into SHARED,
backs COPY up beside itself as ``settings.json.bak-<UTC time>`` and replaces it
with a symlink to SHARED. A key SHARED lacks is copied over, two lists are
joined without repeats, and two objects are merged the same way. A key both set
to different values is a conflict: then nothing is changed, and the conflicting
keys are printed, so the owner decides which value to keep.

Exit status: 0 folded and linked (or would be, with --check); 3 a conflict, or
a file that is not a JSON object, left alone; 2 bad arguments.
"""

from __future__ import annotations

import json
import os
import shutil
import sys
import time
from pathlib import Path
from typing import Any

LEFT_ALONE = 3


def fold(shared: Any, copy: Any, path: str = "") -> tuple[Any, list[str]]:
    """``shared`` with what ``copy`` adds, and the dotted keys where the two disagree."""
    if isinstance(shared, dict) and isinstance(copy, dict):
        merged = dict(shared)
        conflicts: list[str] = []
        for key, value in copy.items():
            where = f"{path}.{key}" if path else key
            if key not in merged:
                merged[key] = value
                continue
            merged[key], found = fold(merged[key], value, where)
            conflicts.extend(found)
        return merged, conflicts
    if isinstance(shared, list) and isinstance(copy, list):
        return shared + [item for item in copy if item not in shared], []
    return shared, [] if shared == copy else [path]


def read_object(path: Path) -> dict[str, Any] | None:
    try:
        value = json.loads(path.read_text() or "{}")
    except (OSError, ValueError):
        return None
    return value if isinstance(value, dict) else None


def main(argv: list[str]) -> int:
    check = "--check" in argv
    args = [arg for arg in argv if arg != "--check"]
    if len(args) != 2:
        print("usage: fold-profile-settings.py SHARED COPY [--check]", file=sys.stderr)
        return 2
    shared_path, copy_path = Path(args[0]), Path(args[1])
    if copy_path.is_symlink():
        print(f"{copy_path}: already a link; left alone")
        return LEFT_ALONE
    shared, copy = read_object(shared_path), read_object(copy_path)
    for path, value in ((shared_path, shared), (copy_path, copy)):
        if value is None:
            print(f"{path}: missing or not a JSON object; {copy_path} left alone")
            return LEFT_ALONE
    merged, conflicts = fold(shared, copy)
    if conflicts:
        print(f"{copy_path}: sets {', '.join(conflicts)} differently from {shared_path}; left alone")
        return LEFT_ALONE
    added = merged != shared
    if check:
        what = f"fold {copy_path} into {shared_path}" if added else f"back up {copy_path}"
        print(f"would {what} and link it to {shared_path}")
        return 0
    if added:
        write_atomically(shared_path, json.dumps(merged, indent=2) + "\n")
    backup = free_backup_path(copy_path)
    shutil.copy2(copy_path, backup)
    # The link replaces the copy in one rename, so the profile is never without a settings.json.
    link = copy_path.with_name(f".{copy_path.name}.link-{os.getpid()}")
    os.symlink(shared_path, link)
    os.replace(link, copy_path)
    print(f"folded {copy_path} into {shared_path} (backup {backup.name}) and linked it")
    return 0


def write_atomically(path: Path, text: str) -> None:
    """Writes ``path``'s real file by a rename beside it, so a shared file that is itself a link stays one."""
    target = path.resolve()
    tmp = target.with_name(f".{target.name}.fold-{os.getpid()}")
    tmp.write_text(text)
    os.replace(tmp, target)


def free_backup_path(path: Path) -> Path:
    """``<name>.bak-<UTC time>``, with a counter when that name is taken, so no earlier backup is overwritten."""
    stem = f"{path.name}.bak-{time.strftime('%Y%m%dT%H%M%SZ', time.gmtime())}"
    candidate = path.with_name(stem)
    counter = 1
    while candidate.exists() or candidate.is_symlink():
        counter += 1
        candidate = path.with_name(f"{stem}-{counter}")
    return candidate


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
