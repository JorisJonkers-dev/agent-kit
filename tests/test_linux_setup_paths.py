"""A setup run on a Linux host writes no macOS path and reports each macOS-only skip.

The run uses the setup harness: HOME is a temp dir, uname answers Linux, and
every external command is a stub. The macOS paths are the ones launchd, the
Keychain tooling, Homebrew and the application folder use.
"""

from __future__ import annotations

from pathlib import Path

from setup_harness import Run, setup_run

MAC_HOME_DIRS = ("Library", "Applications")
MAC_STRINGS = ("/Applications", "/opt/homebrew", "Library/LaunchAgents", "Library/Preferences")


def _linux_run(tmp_path: Path) -> Run:
    return setup_run(
        tmp_path,
        "Linux",
        stubs={
            "vibe": 'echo "vibe $*" >> "' + str(tmp_path / "calls.log") + '"',
            "kubectl": 'if [ "$1 $2" = "config current-context" ]; then echo ctx; fi',
        },
    )


def _mac_paths_written(home: Path) -> list[Path]:
    return [home / name for name in MAC_HOME_DIRS if (home / name).exists()]


def test_linux_run_writes_no_macos_path(tmp_path: Path) -> None:
    run = _linux_run(tmp_path)
    assert _mac_paths_written(run.home) == []
    for line in run.calls:
        assert not any(mac in line for mac in MAC_STRINGS), line
    # No Keychain, launchd or cask call is made on Linux.
    assert not [c for c in run.calls if c.startswith(("security ", "launchctl "))]
    assert not [c for c in run.calls if "--cask" in c]


def test_linux_run_reports_the_macos_only_skips(tmp_path: Path) -> None:
    run = _linux_run(tmp_path)
    output = run.stdout + run.stderr
    assert "antigravity-cli: macOS only; skipped on Linux" in output
    assert "antigravity: macOS only; skipped on Linux" in output


def test_linux_vibe_sign_in_check_is_skipped_and_reported(tmp_path: Path) -> None:
    run = _linux_run(tmp_path)
    output = run.stdout + run.stderr
    assert "vibe sign-in not checked: the check reads the macOS Keychain; skipped on Linux" in output
    assert not [c for c in run.calls if c.startswith("security ")]


def test_linux_secret_store_ignores_a_security_binary(tmp_path: Path) -> None:
    """A `security` on PATH does not make a Linux host use the Keychain."""
    run = _linux_run(tmp_path)
    assert not [c for c in run.calls if "find-generic-password" in c or "add-generic-password" in c]
