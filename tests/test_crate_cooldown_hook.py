"""The 7-day crate cooldown pre-commit hook (.githooks/pre-commit).

Each test commits to a scratch git repository whose core.hooksPath points at
the checked-in hook, with a fake crate-age that exits with the code under test.
What is asserted is what git does: whether the commit exists afterwards. crate-age's
own exit codes (1 too young, 2 age unproven) come from the standards repo; the
hook's job is to refuse on both, and on a missing checker.
"""

from __future__ import annotations

import os
import shutil
import subprocess
from pathlib import Path

import pytest

HOOK = Path(__file__).resolve().parent.parent / ".githooks" / "pre-commit"


def git(repo: Path, *args: str, env: dict[str, str] | None = None) -> subprocess.CompletedProcess[str]:
    # The machine's own git config (global hooks, signing) must not take part.
    isolated = {**(env or os.environ), "GIT_CONFIG_GLOBAL": os.devnull, "GIT_CONFIG_SYSTEM": os.devnull}
    return subprocess.run(["git", "-C", str(repo), *args], capture_output=True, text=True, env=isolated, check=False)


@pytest.fixture
def repo(tmp_path: Path) -> Path:
    path = tmp_path / "repo"
    path.mkdir()
    git(path, "init", "-q")
    git(path, "config", "user.email", "owner@example.invalid")
    git(path, "config", "user.name", "Owner")
    git(path, "config", "commit.gpgsign", "false")
    git(path, "config", "core.hooksPath", str(HOOK.parent))
    return path


def fake_crate_age(tmp_path: Path, status: int) -> Path:
    script = tmp_path / "crate-age"
    script.write_text(f'#!/bin/sh\necho "called with $*" >&2\nexit {status}\n')
    script.chmod(0o755)
    return script


def git_only_path(tmp_path: Path) -> str:
    """A PATH holding git and nothing else, so no crate-age on the machine can be found."""
    bin_dir = tmp_path / "bin"
    bin_dir.mkdir(exist_ok=True)
    link = bin_dir / "git"
    if not link.exists():
        link.symlink_to(shutil.which("git") or "git")
    return str(bin_dir)


def commit(repo: Path, files: dict[str, str], crate_age: Path | None) -> subprocess.CompletedProcess[str]:
    for name, body in files.items():
        (repo / name).write_text(body)
    git(repo, "add", "-A")
    env = {k: v for k, v in os.environ.items() if k != "CRATE_AGE"}
    env["PATH"] = git_only_path(repo.parent)
    if crate_age is not None:
        env["CRATE_AGE"] = str(crate_age)
    return git(repo, "commit", "-q", "-m", "chore: change", env=env)


def committed(repo: Path) -> bool:
    return git(repo, "rev-parse", "--verify", "-q", "HEAD").returncode == 0


def test_a_crate_that_proves_old_enough_commits(repo: Path, tmp_path: Path) -> None:
    result = commit(repo, {"Cargo.lock": "version = 4\n"}, fake_crate_age(tmp_path, 0))
    assert result.returncode == 0, result.stderr
    assert committed(repo)


def test_a_crate_younger_than_seven_days_is_refused(repo: Path, tmp_path: Path) -> None:
    result = commit(repo, {"Cargo.lock": "version = 4\n"}, fake_crate_age(tmp_path, 1))
    assert result.returncode != 0
    assert not committed(repo)
    assert "blocked (exit 1)" in result.stderr


def test_a_crate_whose_age_cannot_be_proven_is_refused(repo: Path, tmp_path: Path) -> None:
    result = commit(repo, {"Cargo.toml": "[workspace]\n"}, fake_crate_age(tmp_path, 2))
    assert result.returncode != 0
    assert not committed(repo)
    assert "blocked (exit 2)" in result.stderr


def test_a_missing_checker_fails_closed(repo: Path) -> None:
    result = commit(repo, {"Cargo.lock": "version = 4\n"}, None)
    assert result.returncode != 0
    assert not committed(repo)
    assert "crate-age is not available" in result.stderr


def test_a_commit_that_touches_no_cargo_file_never_calls_the_checker(repo: Path, tmp_path: Path) -> None:
    result = commit(repo, {"README.md": "hello\n"}, fake_crate_age(tmp_path, 1))
    assert result.returncode == 0, result.stderr
    assert committed(repo)
