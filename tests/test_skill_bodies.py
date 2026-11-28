"""Every skill body stays short, and what a skill links from its folder exists.

A skill's SKILL.md is read whole every time the skill loads, so it holds what it is,
when to use it and the core commands, and links part files in the same folder for the
rest. Setup copies the whole skill folder, so the parts ship with it.
"""

from __future__ import annotations

import re
from pathlib import Path

import pytest

KIT_ROOT = Path(__file__).resolve().parent.parent
SKILL_FILES = sorted((KIT_ROOT / "skills").rglob("SKILL.md"))
MAX_BODY_BYTES = 10_000


@pytest.mark.parametrize("skill", SKILL_FILES, ids=lambda p: p.parent.name)
def test_the_skill_body_is_under_ten_kilobytes(skill: Path) -> None:
    assert len(skill.read_bytes()) < MAX_BODY_BYTES, f"{skill.relative_to(KIT_ROOT)}: split into part files"


@pytest.mark.parametrize("skill", SKILL_FILES, ids=lambda p: p.parent.name)
def test_every_part_file_a_skill_links_exists_and_every_part_is_linked(skill: Path) -> None:
    text = skill.read_text()
    linked = {m for m in re.findall(r"\]\(([A-Za-z0-9_.-]+\.md)\)", text)}
    for name in linked:
        assert (skill.parent / name).is_file(), f"{skill.parent.name} links {name}, which is missing"
    parts = {p.name for p in skill.parent.glob("*.md")} - {"SKILL.md", "INSTALL.md", "README.md"}
    assert parts <= linked, f"{skill.parent.name}: part files not linked from SKILL.md: {sorted(parts - linked)}"
