"""The AGENTS.md lint, driven through its CLI against tiny fake repositories.

`check_agents_md.py` reads each repository root it is given and reports what is
wrong with that repository's `AGENTS.md`: a missing or empty file, a markdown
link whose target does not exist, an inline code span that names a repository
path that does not exist, and a `<path>::<symbol>` reference whose symbol is not
in the file. Fenced code blocks are skipped, external and anchor links are
ignored, a fragment or query is stripped before a link target is resolved, and
only spans that look like repository paths are checked at all.

Every test builds a throwaway repository under `tmp_path` and runs the script as
a subprocess, so what is asserted is the exit code, stdout and stderr a person
sees: problems on stderr as `<REPO_ROOT>/AGENTS.md:<line>: <message>`, the
repository count on stdout when the run is clean, and exit 2 for a root that is
not a directory.
"""

from __future__ import annotations

import subprocess
import sys
from pathlib import Path

import pytest

SCRIPT = Path(__file__).resolve().parent.parent / "scripts" / "check_agents_md.py"

CLEAN_POINTER = """\
# Agent contract

The estate conventions live in one place:
https://github.com/JorisJonkers-dev/workspace/blob/main/CLAUDE.md

Read [the contract](https://example.com/contract) before a non-trivial change,
and see [this section](#agent-contract) for the rest.

Run `scripts/run.py` and read `docs/guide.md`.
"""


def run(*args: str, cwd: Path | None = None) -> subprocess.CompletedProcess[str]:
    """Run the lint with `args`, from `cwd` when one is given."""
    return subprocess.run(
        [sys.executable, str(SCRIPT), *args],
        capture_output=True,
        text=True,
        cwd=cwd,
        timeout=60,
    )


def make_repo(
    base: Path,
    agents_md: str | None = None,
    files: dict[str, str] | None = None,
    name: str = "repo",
) -> Path:
    """Build a repository under `base` and return its root.

    `agents_md` is written to `AGENTS.md` unless it is None, which leaves the
    file out. `files` maps a path relative to the root to its content.
    """
    root = base / name
    root.mkdir(parents=True)
    for rel, content in (files or {}).items():
        path = root / rel
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content, encoding="utf-8")
    if agents_md is not None:
        (root / "AGENTS.md").write_text(agents_md, encoding="utf-8")
    return root


def problems(result: subprocess.CompletedProcess[str]) -> list[str]:
    """The problem lines the run printed to stderr."""
    return result.stderr.splitlines()


def test_missing_agents_md(tmp_path: Path) -> None:
    root = make_repo(tmp_path)

    result = run(str(root))

    assert result.returncode == 1
    assert problems(result) == [f"{root}/AGENTS.md:1: AGENTS.md is missing"]
    assert result.stdout == ""


def test_empty_agents_md(tmp_path: Path) -> None:
    root = make_repo(tmp_path, agents_md="")

    result = run(str(root))

    assert result.returncode == 1
    assert problems(result) == [f"{root}/AGENTS.md:1: AGENTS.md is empty"]


def test_whitespace_only_agents_md_is_empty(tmp_path: Path) -> None:
    root = make_repo(tmp_path, agents_md="  \n\t\n\n   \n")

    result = run(str(root))

    assert result.returncode == 1
    assert problems(result) == [f"{root}/AGENTS.md:1: AGENTS.md is empty"]


def test_clean_pointer_file(tmp_path: Path) -> None:
    root = make_repo(
        tmp_path,
        agents_md=CLEAN_POINTER,
        files={"scripts/run.py": "print('run')\n", "docs/guide.md": "# Guide\n"},
    )

    result = run(str(root))

    assert result.returncode == 0
    assert result.stderr == ""
    assert result.stdout == "agents-md: 1 repositories checked\n"


def test_dead_link(tmp_path: Path) -> None:
    root = make_repo(tmp_path, agents_md="# Agent contract\n\nRead [the guide](docs/guide.md).\n")

    result = run(str(root))

    assert result.returncode == 1
    assert problems(result) == [f"{root}/AGENTS.md:3: dead path: docs/guide.md"]


def test_valid_link(tmp_path: Path) -> None:
    root = make_repo(
        tmp_path,
        agents_md="# Agent contract\n\nRead [the guide](docs/guide.md).\n",
        files={"docs/guide.md": "# Guide\n"},
    )

    result = run(str(root))

    assert result.returncode == 0
    assert result.stdout == "agents-md: 1 repositories checked\n"


def test_link_to_a_directory(tmp_path: Path) -> None:
    root = make_repo(
        tmp_path,
        agents_md="# Agent contract\n\nRead [the docs](docs).\n",
        files={"docs/guide.md": "# Guide\n"},
    )

    result = run(str(root))

    assert result.returncode == 0


@pytest.mark.parametrize(
    "target",
    [
        "https://example.com/contract",
        "http://example.com/contract",
        "mailto:someone@example.com",
        "ftp://example.com/contract",
        "#agent-contract",
        "#",
    ],
)
def test_external_and_anchor_links_are_skipped(tmp_path: Path, target: str) -> None:
    root = make_repo(tmp_path, agents_md=f"# Agent contract\n\nRead [the contract]({target}).\n")

    result = run(str(root))

    assert result.returncode == 0
    assert result.stdout == "agents-md: 1 repositories checked\n"


@pytest.mark.parametrize("target", ["docs/guide.md#install", "docs/guide.md?plain=1", "docs/guide.md#a?b"])
def test_fragment_and_query_are_stripped(tmp_path: Path, target: str) -> None:
    root = make_repo(
        tmp_path,
        agents_md=f"# Agent contract\n\nRead [the guide]({target}).\n",
        files={"docs/guide.md": "# Guide\n"},
    )

    result = run(str(root))

    assert result.returncode == 0


def test_dead_link_with_fragment(tmp_path: Path) -> None:
    root = make_repo(tmp_path, agents_md="# Agent contract\n\nRead [the guide](docs/missing.md#install).\n")

    result = run(str(root))

    assert result.returncode == 1
    assert len(problems(result)) == 1
    assert "dead path: docs/missing.md" in result.stderr


def test_dead_inline_path(tmp_path: Path) -> None:
    root = make_repo(tmp_path, agents_md="# Agent contract\n\nRun `scripts/missing.py` first.\n")

    result = run(str(root))

    assert result.returncode == 1
    assert problems(result) == [f"{root}/AGENTS.md:3: dead path: scripts/missing.py"]


def test_valid_inline_path(tmp_path: Path) -> None:
    root = make_repo(
        tmp_path,
        agents_md="# Agent contract\n\nRun `scripts/run.py` first.\n",
        files={"scripts/run.py": "print('run')\n"},
    )

    result = run(str(root))

    assert result.returncode == 0


def test_inline_directory_path(tmp_path: Path) -> None:
    root = make_repo(
        tmp_path,
        agents_md="# Agent contract\n\nEverything lives in `scripts/`.\n",
        files={"scripts/run.py": "print('run')\n"},
    )

    result = run(str(root))

    assert result.returncode == 0


def test_dead_inline_directory_path(tmp_path: Path) -> None:
    root = make_repo(tmp_path, agents_md="# Agent contract\n\nEverything lives in `missing/`.\n")

    result = run(str(root))

    assert result.returncode == 1
    assert len(problems(result)) == 1
    assert "dead path: missing" in result.stderr


def test_dot_slash_prefix_is_stripped(tmp_path: Path) -> None:
    root = make_repo(
        tmp_path,
        agents_md="# Agent contract\n\nRun `./scripts/run.py` first.\n",
        files={"scripts/run.py": "print('run')\n"},
    )

    result = run(str(root))

    assert result.returncode == 0


@pytest.mark.parametrize(
    "span",
    [
        "*.py",
        "scripts/*.py",
        "a?b",
        "<path>",
        "scripts/<name>.py",
        "${HOME}/notes.md",
        "{a,b}.md",
        "a|b",
        "~/notes.md",
        "-rf",
        "https://example.com/x",
        "foo://bar",
        "/etc/passwd",
        "two words",
        "scripts/run.py --verbose",
    ],
)
def test_spans_that_are_not_repository_paths_are_ignored(tmp_path: Path, span: str) -> None:
    root = make_repo(tmp_path, agents_md=f"# Agent contract\n\nRun `{span}` first.\n")

    result = run(str(root))

    assert result.returncode == 0
    assert result.stdout == "agents-md: 1 repositories checked\n"


def test_fenced_blocks_are_skipped(tmp_path: Path) -> None:
    agents_md = (
        "# Agent contract\n"
        "\n"
        "```sh\n"
        "cat `missing.py`\n"
        "see [the guide](docs/missing.md)\n"
        "call `scripts/gone.py::main`\n"
        "```\n"
        "\n"
        "~~~\n"
        "`also/missing.py`\n"
        "~~~\n"
    )
    root = make_repo(tmp_path, agents_md=agents_md)

    result = run(str(root))

    assert result.returncode == 0
    assert result.stdout == "agents-md: 1 repositories checked\n"


@pytest.mark.parametrize(
    ("span", "expected"),
    [
        ("README.md", []),
        ("notes.txt", []),
        ("Makefile", []),
        ("notes.rst", []),
        ("missing.md", ["dead path: missing.md"]),
        ("missing.py", ["dead path: missing.py"]),
    ],
)
def test_no_slash_spans_use_the_extension_allowlist(tmp_path: Path, span: str, expected: list[str]) -> None:
    root = make_repo(
        tmp_path,
        agents_md=f"# Agent contract\n\nRead `{span}` first.\n",
        files={"README.md": "# Readme\n", "notes.txt": "notes\n"},
    )

    result = run(str(root))

    if expected:
        assert result.returncode == 1
        assert problems(result) == [f"{root}/AGENTS.md:3: {message}" for message in expected]
    else:
        assert result.returncode == 0


def test_dead_symbol(tmp_path: Path) -> None:
    root = make_repo(
        tmp_path,
        agents_md="# Agent contract\n\nCall `scripts/run.py::main` first.\n",
        files={"scripts/run.py": "def other() -> None:\n    pass\n"},
    )

    result = run(str(root))

    assert result.returncode == 1
    assert problems(result) == [f"{root}/AGENTS.md:3: dead symbol: scripts/run.py::main"]


def test_live_symbol_with_parens(tmp_path: Path) -> None:
    root = make_repo(
        tmp_path,
        agents_md="# Agent contract\n\nCall `scripts/run.py::main()` first.\n",
        files={"scripts/run.py": "def main() -> int:\n    return 0\n"},
    )

    result = run(str(root))

    assert result.returncode == 0


def test_symbol_must_be_a_whole_word(tmp_path: Path) -> None:
    root = make_repo(
        tmp_path,
        agents_md="# Agent contract\n\nCall `scripts/run.py::main` first.\n",
        files={"scripts/run.py": "def main_helper() -> None:\n    pass\n"},
    )

    result = run(str(root))

    assert result.returncode == 1
    assert problems(result) == [f"{root}/AGENTS.md:3: dead symbol: scripts/run.py::main"]


def test_symbol_on_a_directory_path(tmp_path: Path) -> None:
    root = make_repo(
        tmp_path,
        agents_md="# Agent contract\n\nCall `lib/::helper` first.\n",
        files={"lib/deep/helper.py": "def helper() -> None:\n    pass\n"},
    )

    result = run(str(root))

    assert result.returncode == 0


def test_symbol_on_a_directory_path_that_lacks_it(tmp_path: Path) -> None:
    root = make_repo(
        tmp_path,
        agents_md="# Agent contract\n\nCall `lib/::helper` first.\n",
        files={"lib/deep/other.py": "def other() -> None:\n    pass\n"},
    )

    result = run(str(root))

    assert result.returncode == 1
    assert problems(result) == [f"{root}/AGENTS.md:3: dead symbol: lib/::helper"]


def test_symbol_path_missing(tmp_path: Path) -> None:
    root = make_repo(tmp_path, agents_md="# Agent contract\n\nCall `scripts/gone.py::main` first.\n")

    result = run(str(root))

    assert result.returncode == 1
    assert problems(result) == [f"{root}/AGENTS.md:3: dead path: scripts/gone.py"]


def test_invalid_utf8(tmp_path: Path) -> None:
    root = make_repo(tmp_path)
    (root / "AGENTS.md").write_bytes(b"# Agent contract\n\xff\xfe not utf-8\n")

    result = run(str(root))

    assert result.returncode == 1
    assert problems(result) == [f"{root}/AGENTS.md:1: AGENTS.md is not valid utf-8"]


def test_line_numbers_are_one_based(tmp_path: Path) -> None:
    agents_md = "# Agent contract\n\nOne.\n\nTwo.\n\nRun `scripts/missing.py`.\n"
    root = make_repo(tmp_path, agents_md=agents_md)

    result = run(str(root))

    assert result.returncode == 1
    assert problems(result) == [f"{root}/AGENTS.md:7: dead path: scripts/missing.py"]


def test_each_problem_is_reported_once_per_line(tmp_path: Path) -> None:
    root = make_repo(tmp_path, agents_md="# Agent contract\n\nRun `a/missing.py` and `a/missing.py`.\n")

    result = run(str(root))

    assert result.returncode == 1
    assert problems(result) == [f"{root}/AGENTS.md:3: dead path: a/missing.py"]


def test_the_same_problem_on_two_lines_is_reported_twice(tmp_path: Path) -> None:
    root = make_repo(tmp_path, agents_md="# Agent contract\n\nRun `a/missing.py`.\n\nRun `a/missing.py`.\n")

    result = run(str(root))

    assert result.returncode == 1
    assert problems(result) == [
        f"{root}/AGENTS.md:3: dead path: a/missing.py",
        f"{root}/AGENTS.md:5: dead path: a/missing.py",
    ]


def test_multiple_repos_report_only_the_dirty_one(tmp_path: Path) -> None:
    clean = make_repo(tmp_path, agents_md="# Agent contract\n\nNothing to see.\n", name="clean")
    dirty = make_repo(tmp_path, agents_md="# Agent contract\n\nRun `missing.py`.\n", name="dirty")

    result = run(str(clean), str(dirty))

    assert result.returncode == 1
    assert problems(result) == [f"{dirty}/AGENTS.md:3: dead path: missing.py"]
    assert result.stdout == ""


def test_multiple_clean_repos_are_counted(tmp_path: Path) -> None:
    first = make_repo(tmp_path, agents_md="# Agent contract\n\nNothing to see.\n", name="first")
    second = make_repo(tmp_path, agents_md="# Agent contract\n\nNothing to see.\n", name="second")

    result = run(str(first), str(second))

    assert result.returncode == 0
    assert result.stdout == "agents-md: 2 repositories checked\n"


def test_default_root_is_the_current_directory(tmp_path: Path) -> None:
    root = make_repo(tmp_path, agents_md="# Agent contract\n\nNothing to see.\n")

    result = run(cwd=root)

    assert result.returncode == 0
    assert result.stdout == "agents-md: 1 repositories checked\n"


def test_default_root_reports_a_missing_file(tmp_path: Path) -> None:
    root = make_repo(tmp_path)

    result = run(cwd=root)

    assert result.returncode == 1
    assert len(problems(result)) == 1
    assert problems(result)[0].endswith("AGENTS.md:1: AGENTS.md is missing")


def test_root_is_reported_as_given(tmp_path: Path) -> None:
    make_repo(tmp_path)

    result = run("repo", cwd=tmp_path)

    assert result.returncode == 1
    assert problems(result) == ["repo/AGENTS.md:1: AGENTS.md is missing"]


def test_nonexistent_root_is_a_usage_error(tmp_path: Path) -> None:
    result = run(str(tmp_path / "nope"))

    assert result.returncode == 2
    assert result.stderr != ""
    assert result.stdout == ""


def test_root_that_is_a_file_is_a_usage_error(tmp_path: Path) -> None:
    path = tmp_path / "AGENTS.md"
    path.write_text("# Agent contract\n", encoding="utf-8")

    result = run(str(path))

    assert result.returncode == 2
    assert result.stderr != ""


def test_usage_error_wins_over_a_clean_repo(tmp_path: Path) -> None:
    clean = make_repo(tmp_path, agents_md="# Agent contract\n\nNothing to see.\n")

    result = run(str(clean), str(tmp_path / "nope"))

    assert result.returncode == 2


def test_require_flag_is_accepted(tmp_path: Path) -> None:
    root = make_repo(tmp_path, agents_md="# Agent contract\n\nNothing to see.\n")

    result = run("--require", str(root))

    assert result.returncode == 0
    assert result.stdout == "agents-md: 1 repositories checked\n"


def test_require_flag_still_requires_the_file(tmp_path: Path) -> None:
    root = make_repo(tmp_path)

    result = run("--require", str(root))

    assert result.returncode == 1
    assert problems(result) == [f"{root}/AGENTS.md:1: AGENTS.md is missing"]


def test_query_only_link_is_skipped(tmp_path: Path) -> None:
    repo = make_repo(tmp_path, agents_md="See [search](?q=1).\n")
    result = run(str(repo))
    assert result.returncode == 0, result.stderr


def test_symbol_reference_on_a_non_path_is_ignored(tmp_path: Path) -> None:
    repo = make_repo(tmp_path, agents_md="Call `foo::bar` or `Class::method()`.\n")
    result = run(str(repo))
    assert result.returncode == 0, result.stderr
