#!/usr/bin/env python3
"""Copy the skills of Claude Code plugins into OpenCode's global skills folder.

OpenCode reads ``~/.claude/skills`` but not Claude's plugin cache, so the
skills a plugin ships reach it only as copies. This asks Claude where each
plugin is installed (``claude plugin list --json``) and finds its skills the
way Claude does: every path the plugin's ``plugin.json`` lists under
``skills``, plus the default ``skills/`` folder, each either a skill folder or
a folder of them; a plugin with neither but a ``SKILL.md`` at its root is that
one skill. A skill takes the ``name:`` of its ``SKILL.md``. A plugin disabled in
Claude ships nothing, and a ``skills`` path that leads out of its plugin is
ignored.

Each copy carries a marker file (``.agent-kit-plugin-skill``) naming the plugin
it came from. A copy is built beside the skills, marked, and only then renamed
into place, so no interrupted run leaves an unmarked copy behind. A run
replaces each marked copy, removes the marked copies no plugin ships any more,
and never touches a folder without the marker: setup's own and the vendored
skills keep their name. Each copy is read back.

``--check`` prints what a run would do and changes nothing.

Usage::

    python scripts/opencode-plugin-skills.py --skills-dir DIR --plugins ID,... [--check]
"""

from __future__ import annotations

import argparse
import json
import re
import shutil
import subprocess
import sys
from pathlib import Path
from typing import Any

MARKER = ".agent-kit-plugin-skill"
# A copy being built; a leftover from an interrupted run is removed.
STAGING = ".agent-kit-staging-"
# OpenCode's rule for a skill name, which is also its folder name.
NAME = re.compile(r"^[a-z0-9]+(-[a-z0-9]+)*$")
# Never part of a skill: plugin metadata, dependencies and VCS state.
IGNORED_PARTS = {".claude-plugin", ".git", "node_modules"}
IGNORED = shutil.ignore_patterns(*IGNORED_PARTS)
# Seconds `claude plugin list` may take before the step gives up.
CLAUDE_TIMEOUT_S = 120


def installed_plugins() -> list[dict[str, Any]]:
    out = subprocess.run(["claude", "plugin", "list", "--json"], capture_output=True, text=True, check=True,
                         stdin=subprocess.DEVNULL, timeout=CLAUDE_TIMEOUT_S).stdout
    plugins: list[dict[str, Any]] = json.loads(out or "[]")
    return plugins


def skill_name(skill_md: Path, fallback: str) -> str:
    """The ``name:`` in the SKILL.md front matter, or the folder's name."""
    text = skill_md.read_text(errors="replace")
    match = re.match(r"^---\s*\n(.*?)\n---", text, re.DOTALL)
    if match:
        for line in match.group(1).splitlines():
            key, _, value = line.partition(":")
            if key.strip() == "name" and value.strip():
                return value.strip().strip("\"'")
    return fallback


def plugin_skills(root: Path) -> list[Path]:
    """The skill folders a plugin installed at ``root`` ships, in Claude's order."""
    manifest = root / ".claude-plugin" / "plugin.json"
    declared: Any = json.loads(manifest.read_text()).get("skills") if manifest.is_file() else None
    paths = [declared] if isinstance(declared, str) else list(declared or [])
    found: list[Path] = []
    top = root.resolve()
    for rel in [*paths, "skills"]:
        base = (root / str(rel)).resolve()
        # A path the plugin names may not lead out of the plugin.
        if not base.is_dir() or not base.is_relative_to(top):
            continue
        candidates = [base] if (base / "SKILL.md").is_file() else sorted(p.parent for p in base.glob("*/SKILL.md"))
        found += [c for c in candidates if c not in found]
    if not found and (root / "SKILL.md").is_file():
        found.append(root)
    return found


def wanted_skills(plugins: list[dict[str, Any]], ids: list[str]) -> tuple[dict[str, tuple[Path, str]], list[str]]:
    """Skill name to (source folder, plugin id) for each named plugin, and a note per one left out."""
    by_id: dict[str, dict[str, Any]] = {}
    for plugin in plugins:
        # A plugin can be listed once per scope; the user-scope entry is the shared install.
        if plugin.get("id") in ids and (plugin.get("scope") == "user" or plugin.get("id") not in by_id):
            by_id[str(plugin["id"])] = plugin
    wanted: dict[str, tuple[Path, str]] = {}
    notes: list[str] = []
    for plugin_id in ids:
        plugin = by_id.get(plugin_id)
        if plugin is None:
            notes.append(f"{plugin_id}: not installed in Claude Code; its skills are skipped")
            continue
        if plugin.get("enabled") is False:
            notes.append(f"{plugin_id}: disabled in Claude Code; its skills are skipped")
            continue
        root = Path(str(plugin.get("installPath") or ""))
        if not root.is_dir():
            notes.append(f"{plugin_id}: install path {root} is missing; its skills are skipped")
            continue
        try:
            folders = [(folder, skill_name(folder / "SKILL.md", folder.name)) for folder in plugin_skills(root)]
        except (json.JSONDecodeError, UnicodeDecodeError, OSError) as error:
            notes.append(f"{plugin_id}: cannot read its plugin.json or skills ({error}); its skills are skipped")
            continue
        for folder, name in folders:
            if not NAME.match(name):
                notes.append(f"{plugin_id}: skill {name!r} is not a valid OpenCode skill name; skipped")
            elif name in wanted:
                notes.append(f"{plugin_id}: skill {name} is already shipped by another plugin; skipped")
            else:
                wanted[name] = (folder, plugin_id)
    return wanted, notes


def ours(skills_dir: Path) -> set[str]:
    """The skill folders a run of this script made: the direct children with the marker."""
    if not skills_dir.is_dir():
        return set()
    return {p.name for p in skills_dir.iterdir() if p.is_dir() and not p.is_symlink() and (p / MARKER).is_file()}


def _install(source: Path, dest: Path, plugin_id: str) -> bool:
    """Build the copy beside ``dest``, mark it, check it, then swap it in."""
    staged = dest.with_name(STAGING + dest.name)
    shutil.rmtree(staged, ignore_errors=True)
    shutil.copytree(source, staged, ignore=IGNORED, symlinks=False)
    (staged / MARKER).write_text(plugin_id + "\n")
    expected = {p.relative_to(source) for p in source.rglob("*") if p.is_file()
                and not IGNORED_PARTS & set(p.relative_to(source).parts)}
    landed = {p.relative_to(staged) for p in staged.rglob("*") if p.is_file()}
    if not (staged / "SKILL.md").is_file() or not expected <= landed:
        shutil.rmtree(staged, ignore_errors=True)
        return False
    shutil.rmtree(dest, ignore_errors=True)
    staged.rename(dest)
    return (dest / "SKILL.md").is_file() and (dest / MARKER).is_file()


def sync(skills_dir: Path, wanted: dict[str, tuple[Path, str]], check: bool) -> tuple[list[str], bool]:
    mine = ours(skills_dir)
    lines: list[str] = []
    ok = True
    if not check and skills_dir.is_dir():
        for leftover in skills_dir.glob(STAGING + "*"):
            shutil.rmtree(leftover, ignore_errors=True)
    for name in sorted(mine - set(wanted)):
        dest = skills_dir / name
        if check:
            lines.append(f"would remove {dest} (no enabled plugin ships it)")
        else:
            shutil.rmtree(dest, ignore_errors=True)
            lines.append(f"removed {dest} (no enabled plugin ships it)")
    installed = 0
    for name, (source, plugin_id) in sorted(wanted.items()):
        dest = skills_dir / name
        if (dest.exists() or dest.is_symlink()) and name not in mine:
            lines.append(f"{name}: {dest} came from another source; left alone")
            continue
        if check:
            lines.append(f"would copy {name} -> {dest}")
            continue
        skills_dir.mkdir(parents=True, exist_ok=True)
        if _install(source, dest, plugin_id):
            installed += 1
        else:
            ok = False
            lines.append(f"{name}: the copy of {source} was incomplete; {dest} not replaced")
    if not check:
        lines.append(f"{installed} plugin skill(s) in {skills_dir}")
    return lines, ok


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Copy Claude Code plugin skills into OpenCode")
    parser.add_argument("--skills-dir", type=Path, required=True)
    parser.add_argument("--plugins", default="", metavar="ID,...")
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args(argv)
    ids = [i for i in args.plugins.split(",") if i]
    try:
        plugins = installed_plugins()
    except (subprocess.CalledProcessError, subprocess.TimeoutExpired, json.JSONDecodeError, OSError) as error:
        print(f"opencode-plugin-skills: claude plugin list failed ({error}); nothing changed", file=sys.stderr)
        return 1
    wanted, notes = wanted_skills(plugins, ids)
    lines, ok = sync(args.skills_dir, wanted, args.check)
    for line in [*notes, *lines]:
        print(line)
    return 0 if ok else 1


if __name__ == "__main__":
    raise SystemExit(main())
