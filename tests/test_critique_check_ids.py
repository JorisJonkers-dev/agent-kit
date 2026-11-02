"""Check ids in the critique lenses and roles.

Every lens bullet and every role rule opens with a check id, a backticked
lower-case slug and a colon (``- `dependency-fails`: ...``), which a finding
names in its ``check`` field. Fan-out reads the ids with the same pattern and
gives a hint to a finding whose check its critic's prompt does not declare,
so an id must be unique within its file. In a lens every bullet is a check;
in a role the rules are the bullets of its first list, the later lists being
evidence kinds and severity levels.
"""

from __future__ import annotations

import re
from pathlib import Path

import pytest

CRITIQUE = Path(__file__).resolve().parent.parent / "skills" / "critique"
CHECK_BULLET = re.compile(r"^- `([a-z0-9]+(?:-[a-z0-9]+)*)`:")


def bullet_lists(text: str) -> list[list[str]]:
    """The top-level bullets of each list, a list being a blank-line-separated block that opens with a bullet."""
    blocks = [block.splitlines() for block in re.split(r"\n\s*\n", text)]
    return [[line for line in block if line.startswith("- ")] for block in blocks if block and block[0].startswith("- ")]


def check_id_problems(text: str, kind: str) -> list[str]:
    """What is wrong with a lens's or role's check ids: a check bullet without one, or an id used twice."""
    lists = bullet_lists(text)
    checks = [bullet for group in lists for bullet in group] if kind == "lens" else (lists[0] if lists else [])
    problems = [f"no check id: {bullet}" for bullet in checks if not CHECK_BULLET.match(bullet)]
    if not checks:
        problems.append("no checks")
    ids = re.findall(CHECK_BULLET.pattern, text, flags=re.MULTILINE)
    problems += [f"duplicate check id: {id_}" for id_ in sorted({i for i in ids if ids.count(i) > 1})]
    return problems


SHIPPED = sorted(
    [(path, "lens") for path in (CRITIQUE / "lenses").glob("*.md")]
    + [(path, "role") for path in (CRITIQUE / "roles").glob("*.md")]
)


def test_the_shipped_lenses_and_roles_are_all_checked() -> None:
    assert len([1 for _, kind in SHIPPED if kind == "lens"]) >= 8
    assert len([1 for _, kind in SHIPPED if kind == "role"]) >= 5


@pytest.mark.parametrize(("path", "kind"), SHIPPED, ids=lambda value: getattr(value, "name", value))
def test_every_lens_bullet_and_role_rule_opens_with_a_check_id_unique_in_its_file(path: Path, kind: str) -> None:
    assert check_id_problems(path.read_text(), kind) == []


LENS = """# Lens: sample

Check one thing.

- `first-check`: Look for the first thing,
  over two lines.
- `second-check`: Look for the second thing.

A closing note that is not a check.
"""


def test_a_lens_bullet_without_a_check_id_is_caught() -> None:
    text = LENS.replace("- `second-check`: Look", "- Look")
    assert check_id_problems(text, "lens") == ["no check id: - Look for the second thing."]


def test_a_check_id_used_twice_in_one_file_is_caught() -> None:
    text = LENS.replace("`second-check`", "`first-check`")
    assert check_id_problems(text, "lens") == ["duplicate check id: first-check"]


def test_a_check_id_must_be_a_backticked_slug_and_a_colon() -> None:
    for opening in ("- **first-check**:", "- `first-check` Look", "- `First_Check`:"):
        text = LENS.replace("- `first-check`:", opening)
        assert check_id_problems(text, "lens") != [], opening


def test_a_role_needs_ids_on_its_first_list_only() -> None:
    role = """# Role: sample

Look for:

- `bug`: a defect.
- `omission`: a missed criterion.

Rate severity:

- **critical**: the worst.
"""
    assert check_id_problems(role, "role") == []
    assert check_id_problems(role.replace("- `omission`: a", "- **Omission**: a"), "role") == [
        "no check id: - **Omission**: a missed criterion."
    ]
    assert check_id_problems("# Role: none\n\nNo list at all.\n", "role") == ["no checks"]
