"""Setup installs the work Claude profile into ~/.claude whatever CLAUDE_CONFIG_DIR says.

A personal Claude session exports its own CLAUDE_CONFIG_DIR. Setup run from it
must still fill the default directory the work instance reads, and leave the
personal root as a separate profile. Full runs of the checked-in setup script
against a temp HOME (tests/setup_harness.py).
"""

from __future__ import annotations

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from setup_harness import setup_run


def _surfaces(home: Path) -> set[str]:
    skills = home / ".claude" / "skills"
    return {p.name for p in skills.iterdir()} if skills.is_dir() else set()


def test_ambient_config_dir_does_not_redirect_the_work_profile(tmp_path: Path) -> None:
    other = tmp_path / "elsewhere"
    run = setup_run(tmp_path / "ambient", "Linux", extra_env={"CLAUDE_CONFIG_DIR": str(other)})
    assert _surfaces(run.home), "work profile skills were not installed into ~/.claude"
    assert not other.exists(), "setup wrote into the ambient CLAUDE_CONFIG_DIR"
    assert "ignoring CLAUDE_CONFIG_DIR" in run.stdout


def test_personal_session_leaves_personal_profile_separate(tmp_path: Path) -> None:
    def personal(home: Path) -> None:
        (home / ".claude-personal").mkdir()

    run = setup_run(
        tmp_path / "personal",
        "Linux",
        prepare_home=personal,
        extra_env={"CLAUDE_CONFIG_DIR": str(tmp_path / "personal" / "home" / ".claude-personal")},
    )
    assert _surfaces(run.home)
    personal_skills = run.home / ".claude-personal" / "skills"
    assert personal_skills.is_symlink()
    assert personal_skills.resolve() == (run.home / ".claude" / "skills").resolve()


def test_without_an_override_nothing_is_reported(tmp_path: Path) -> None:
    run = setup_run(tmp_path / "plain", "Linux")
    assert "ignoring CLAUDE_CONFIG_DIR" not in run.stdout
    assert _surfaces(run.home)


def test_claude_calls_for_the_work_profile_run_without_a_config_root(tmp_path: Path) -> None:
    """The work profile keeps its .claude.json in HOME, so its claude calls carry no CLAUDE_CONFIG_DIR."""
    other = tmp_path / "elsewhere"
    log = tmp_path / "claude-env.log"
    run = setup_run(
        tmp_path / "env",
        "Linux",
        stubs={
            "claude": (
                f'echo "$* @ ${{CLAUDE_CONFIG_DIR:-unset}}" >> "{log}"\n'
                'echo "{}" > "${CLAUDE_CONFIG_DIR:-$HOME}/.claude.json"'
            ),
        },
        extra_env={"CLAUDE_CONFIG_DIR": str(other)},
    )
    seen = log.read_text().splitlines() if log.exists() else []
    assert run.home.exists()
    assert not any(str(other) in line for line in seen), seen
    work_calls = [line for line in seen if line.endswith("@ unset")]
    assert work_calls, seen
    assert (run.home / ".claude.json").exists()
    assert not (run.home / ".claude" / ".claude.json").exists()
