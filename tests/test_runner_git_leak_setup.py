"""Setup removes the runner's git settings that leaked into a workstation's ~/.gitconfig.

The runner entrypoint once wrote its global git settings wherever it ran. On a
workstation they named the credential helper ``agent-gh-app``, which only the
runner image ships, so every push printed "git: 'credential-agent-gh-app' is not
a git command". Setup converges that machine: the leaked helper goes, and with
it the ``credential.useHttpPath`` and ``safe.directory /workspace`` the same run
wrote. Each run is the checked-in setup script against a temp HOME
(tests/setup_harness.py), with ``git config`` reaching a real git so the test
reads the file the run leaves behind.
"""

from __future__ import annotations

import shutil
import subprocess
import sys
from pathlib import Path

import pytest

KIT_ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(Path(__file__).resolve().parent))

from setup_harness import Run, setup_run  # noqa: E402

REAL_GIT = shutil.which("git")
pytestmark = pytest.mark.skipif(REAL_GIT is None, reason="needs a real git for git config")

LEAKED = """\
[user]
\tname = Owner
[credential]
\thelper = agent-gh-app
\tuseHttpPath = true
[safe]
\tdirectory = /workspace
\tdirectory = /Users/owner/code
"""


def _git_stub(calls: Path) -> str:
    # `git config` is real so the run edits the temp HOME's file; everything
    # else (clones of vendored skills) stays a recorded no-op.
    return f'if [ "$1" = config ]; then exec "{REAL_GIT}" "$@"; fi\necho "git $*" >> "{calls}"'


def _run(tmp_path: Path, gitconfig: str, *, args: tuple[str, ...] = (), helper_on_path: bool = False) -> Run:
    stubs = {"git": _git_stub(tmp_path / "calls.log")}
    if helper_on_path:
        stubs["git-credential-agent-gh-app"] = "exit 0"
    return setup_run(
        tmp_path,
        "Darwin",
        args=args,
        prepare_home=lambda home: (home / ".gitconfig").write_text(gitconfig),
        stubs=stubs,
    )


def _values(home: Path, key: str) -> list[str]:
    completed = subprocess.run(
        [str(REAL_GIT), "config", "--file", str(home / ".gitconfig"), "--get-all", key],
        capture_output=True,
        text=True,
        check=False,
    )
    return completed.stdout.splitlines()


def _fixed_lines(run: Run) -> list[str]:
    return [line for line in run.stdout.splitlines() if "fixed:" in line]


def test_setup_removes_the_leaked_runner_git_settings_and_says_so_once(tmp_path: Path) -> None:
    run = _run(tmp_path, LEAKED)

    assert _values(run.home, "credential.helper") == []
    assert _values(run.home, "credential.useHttpPath") == []
    assert _values(run.home, "safe.directory") == ["/Users/owner/code"]
    assert _values(run.home, "user.name") == ["Owner"]
    fixed = _fixed_lines(run)
    assert len(fixed) == 1, run.stdout
    assert fixed[0].startswith("setup:   ok    fixed: ")
    assert "agent-gh-app" in fixed[0]


def test_a_second_run_finds_nothing_left_to_fix(tmp_path: Path) -> None:
    first = _run(tmp_path / "first", LEAKED)
    second = _run(tmp_path / "second", (first.home / ".gitconfig").read_text())

    assert _fixed_lines(second) == []


def test_check_reports_the_leak_and_changes_nothing(tmp_path: Path) -> None:
    run = _run(tmp_path, LEAKED, args=("--check",))

    assert (run.home / ".gitconfig").read_text() == LEAKED
    assert _fixed_lines(run) == []
    assert "agent-gh-app" in run.stderr


def test_the_helper_on_path_means_the_settings_are_real_and_stay(tmp_path: Path) -> None:
    # Inside the runner image the helper exists, so its settings are not a leak.
    run = _run(tmp_path, LEAKED, helper_on_path=True)

    assert (run.home / ".gitconfig").read_text() == LEAKED
    assert _fixed_lines(run) == []


def test_without_the_agent_gh_app_helper_nothing_is_touched(tmp_path: Path) -> None:
    # useHttpPath and /workspace are only the runner's when its helper came with them.
    own = "[credential]\n\thelper = osxkeychain\n\tuseHttpPath = true\n[safe]\n\tdirectory = /workspace\n"
    run = _run(tmp_path, own)

    assert (run.home / ".gitconfig").read_text() == own
    assert _fixed_lines(run) == []
