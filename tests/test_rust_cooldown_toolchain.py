"""Setup makes sure cargo honours the 7-day crate cooldown (Rust 1.100 and later).

Each test runs the real setup script with stub rustup and rustc and asserts on
what setup reports and which rustup calls it made.
"""

from __future__ import annotations

from pathlib import Path

from setup_harness import Run
from setup_harness import setup_run as _setup_run

# NixOS keeps bash and the rest in the system profile, not /usr/bin.
SYSTEM_BIN = "/run/current-system/sw/bin" if Path("/run/current-system/sw/bin/bash").exists() else "/usr/bin"
STABLE = "stable-x86_64-unknown-linux-gnu"


def setup_run(tmp_path: Path, *args: str, stubs: dict[str, str]) -> Run:
    return _setup_run(
        tmp_path,
        "Linux",
        args=args,
        stubs=stubs,
        extra_env={"PATH": f"{tmp_path / 'bin'}:{SYSTEM_BIN}:/usr/bin:/bin"},
    )


def _rust(tmp_path: Path, version: str, default: str, *, after_update: str | None = None) -> dict[str, str]:
    """Stubs for rustc reporting ``version`` (``after_update`` once rustup update ran) and rustup."""
    log = tmp_path / "rustup.log"
    marker = tmp_path / "updated"
    later = after_update or version
    return {
        "rustc": (
            f'if [ -e "{marker}" ]; then echo "rustc {later} (x 2026-11-12)"; '
            f'else echo "rustc {version} (x 2026-10-13)"; fi'
        ),
        "rustup": (
            f'echo "rustup $*" >> "{log}"\n'
            'case "$1" in\n'
            f'  default) [ -n "{default}" ] && echo "{default} (default)" || exit 1 ;;\n'
            f'  update) touch "{marker}" ;;\n'
            "esac"
        ),
    }


def _calls(tmp_path: Path) -> list[str]:
    log = tmp_path / "rustup.log"
    return log.read_text().splitlines() if log.exists() else []


def test_a_toolchain_at_1_100_is_kept(tmp_path: Path) -> None:
    run = setup_run(tmp_path, stubs=_rust(tmp_path, "1.100.0", STABLE))
    assert "rust: rustc 1.100.0 honours the 7-day crate cooldown" in run.stdout
    assert not any(call.startswith("rustup update") for call in _calls(tmp_path))


def test_an_older_stable_toolchain_is_updated(tmp_path: Path) -> None:
    stubs = _rust(tmp_path, "1.99.1", STABLE, after_update="1.100.0")
    run = setup_run(tmp_path, stubs=stubs)
    assert "rustup update stable" in _calls(tmp_path)
    assert "rust: updated stable to rustc 1.100.0" in run.stdout


def test_check_mode_only_says_it_would_update(tmp_path: Path) -> None:
    run = setup_run(tmp_path, "--check", stubs=_rust(tmp_path, "1.99.1", STABLE))
    assert "would run: rustup update stable (rustc 1.99.1" in run.stdout
    assert not any(call.startswith("rustup update") for call in _calls(tmp_path))


def test_an_update_that_stays_old_fails(tmp_path: Path) -> None:
    run = setup_run(tmp_path, stubs=_rust(tmp_path, "1.99.1", STABLE))
    assert "rustup update stable left rustc 1.99.1" in run.stdout + run.stderr
    assert "cargo ignores the crate cooldown" in run.stdout + run.stderr


def test_a_default_pinned_to_an_older_release_is_reported_not_moved(tmp_path: Path) -> None:
    run = setup_run(tmp_path, stubs=_rust(tmp_path, "1.95.0", "1.95.0-x86_64-unknown-linux-gnu"))
    assert "default toolchain is 1.95.0-x86_64-unknown-linux-gnu, not stable" in run.stdout + run.stderr
    assert not any(call.startswith("rustup update") for call in _calls(tmp_path))
