"""The cask step moves a stray <binary> in Homebrew's bin directory aside.

`brew install --cask antigravity-cli` refuses to run while a file named
`agy` already sits in `$(brew --prefix)/bin` -- one an earlier manual
install put there, which is exactly the machine the cask is for. The
step moves that file aside to a dated `.bak` beside it (it is never
deleted), installs the cask, and moves the file back when the install
still fails; through `run`, so `--check` only prints the moves.
"""

from __future__ import annotations

from pathlib import Path

from setup_harness import setup_run

STRAY = "the stray agy, left by an earlier manual install"


def _stray_agy(root: Path) -> Path:
    """A stray agy in the stub Homebrew's bin directory."""
    agy = root / "brew" / "bin" / "agy"
    agy.parent.mkdir(parents=True)
    agy.write_text(STRAY)
    return agy


def _brew(root: Path, *, fail_install: bool = False) -> str:
    """A stub brew: --prefix names a temp dir, and a cask lists only once installed."""
    prefix = root / "brew"
    lines = [
        f'echo "brew $*" >> "{root / "calls.log"}"',
        'case "$1" in',
        f'  --prefix) printf \'%s\\n\' "{prefix}" ;;',
        "esac",
        'case "$1 $2" in',
        f'  "list --cask") [ -f "{root}/casks/$3" ] ;;',
        '  "install --cask")',
    ]
    if fail_install:
        lines.append("    exit 1 ;;")
    lines += [
        f'    mkdir -p "{root}/casks" && : > "{root}/casks/$3" ;;',
        "esac",
    ]
    return "\n".join(lines)


def test_a_stray_binary_is_moved_aside_and_the_cask_installs(tmp_path: Path) -> None:
    root = tmp_path / "run"
    agy = _stray_agy(root)
    run = setup_run(root, "Darwin", stubs={"brew": _brew(root)})

    backups = sorted(agy.parent.glob("agy.bak-*"))
    assert len(backups) == 1, run.stdout + run.stderr
    # The old file arrived there unharmed, and the cask owns the step again.
    assert backups[0].read_text() == STRAY
    assert not agy.exists()
    assert "brew install --cask antigravity-cli" in run.calls
    assert "antigravity-cli installed" in run.stdout
    assert f"moved {agy} aside to" in run.stdout
    assert f"aside to {backups[0]}" in run.stdout


def test_a_failed_install_restores_the_stray_binary(tmp_path: Path) -> None:
    root = tmp_path / "run"
    agy = _stray_agy(root)
    run = setup_run(root, "Darwin", stubs={"brew": _brew(root, fail_install=True)})

    assert agy.read_text() == STRAY
    assert not list(agy.parent.glob("agy.bak-*"))
    assert f"restored {agy} from" in run.stdout
    assert "brew install --cask antigravity-cli did not install it" in run.stderr


def test_check_mode_prints_the_move_and_moves_nothing(tmp_path: Path) -> None:
    root = tmp_path / "run"
    agy = _stray_agy(root)
    run = setup_run(root, "Darwin", args=("--check",), stubs={"brew": _brew(root)})

    assert agy.read_text() == STRAY
    assert not list(agy.parent.glob("agy.bak-*"))
    assert f"would run: mv {agy}" in run.stdout
    assert "antigravity-cli would be installed" in run.stdout
