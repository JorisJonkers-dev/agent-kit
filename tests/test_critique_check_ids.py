"""Check ids in the critique lenses and roles.

Every lens bullet and every role rule opens with a check id, a backticked
lower-case slug and a colon (``- `dependency-fails`: ...``), which a finding
names in its ``check`` field. Fan-out reads the ids with the same pattern and
gives a hint to a finding whose check its critic's prompt does not declare,
so an id must be unique within its file. In a lens every bullet is a check;
in a role the rules are the bullets of its first list, the later lists being
evidence kinds and severity levels.

A check may also fix its findings' severity, in brackets before the colon
(``- `no-rollback` (critical): ...``); the runner then sets the severity and
only the orchestrator's verdict can change it, with a reason. Only the files
on ``DECLARES_SEVERITY`` may do so.
"""

from __future__ import annotations

import re
from pathlib import Path

import pytest

CRITIQUE = Path(__file__).resolve().parent.parent / "skills" / "critique"
CHECK_BULLET = re.compile(r"^- `([a-z0-9]+(?:-[a-z0-9]+)*)`(?: \((critical|major|minor)\))?:")

# The lens and role files, by path under skills/critique, whose checks may fix
# a severity. The tickets that add `adr-record` and `plan-critic` add them here.
DECLARES_SEVERITY: frozenset[str] = frozenset()


def bullet_lists(text: str) -> list[list[str]]:
    """The top-level bullets of each list, a list being a blank-line-separated block that opens with a bullet."""
    blocks = [block.splitlines() for block in re.split(r"\n\s*\n", text)]
    return [
        [line for line in block if line.startswith("- ")]
        for block in blocks
        if block and block[0].startswith("- ")
    ]


def check_id_problems(text: str, kind: str, may_declare_severity: bool = False) -> list[str]:
    """What is wrong with a lens's or role's check ids.

    A check bullet without one, an id used twice, or a severity it may not declare.
    """
    lists = bullet_lists(text)
    checks = [bullet for group in lists for bullet in group] if kind == "lens" else (lists[0] if lists else [])
    problems = [f"no check id: {bullet}" for bullet in checks if not CHECK_BULLET.match(bullet)]
    if not checks:
        problems.append("no checks")
    declared = re.findall(CHECK_BULLET.pattern, text, flags=re.MULTILINE)
    ids = [id_ for id_, _ in declared]
    problems += [f"duplicate check id: {id_}" for id_ in sorted({i for i in ids if ids.count(i) > 1})]
    if not may_declare_severity:
        problems += [f"declares a severity but is not on the list: {id_}" for id_, severity in declared if severity]
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
    allowed = path.relative_to(CRITIQUE).as_posix() in DECLARES_SEVERITY
    assert check_id_problems(path.read_text(), kind, may_declare_severity=allowed) == []


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


def test_a_severity_is_declared_only_by_a_file_on_the_list() -> None:
    fixed = LENS.replace("- `first-check`:", "- `first-check` (critical):")
    assert check_id_problems(fixed, "lens") == ["declares a severity but is not on the list: first-check"]
    assert check_id_problems(fixed, "lens", may_declare_severity=True) == []


def test_a_declared_severity_must_be_one_of_the_three_levels() -> None:
    text = LENS.replace("- `first-check`:", "- `first-check` (blocker):")
    assert check_id_problems(text, "lens", may_declare_severity=True) == [
        "no check id: - `first-check` (blocker): Look for the first thing,"
    ]


def test_the_files_on_the_severity_list_exist() -> None:
    assert all((CRITIQUE / name).is_file() for name in DECLARES_SEVERITY)


@pytest.mark.parametrize(
    ("path", "kind"),
    [(path, kind) for path, kind in SHIPPED if path.relative_to(CRITIQUE).as_posix() not in DECLARES_SEVERITY],
    ids=lambda value: getattr(value, "name", value),
)
def test_a_severity_in_a_shipped_file_off_the_list_fails(path: Path, kind: str) -> None:
    text = path.read_text()
    first = re.search(CHECK_BULLET.pattern, text, flags=re.MULTILINE)
    assert first is not None
    fixed = text.replace(first.group(0), f"- `{first.group(1)}` (critical):", 1)
    assert f"declares a severity but is not on the list: {first.group(1)}" in check_id_problems(fixed, kind)
