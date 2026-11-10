"""The rustup default-toolchain step, run on its own under bash.

On NixOS, Nix installs rustup system-wide, but rustup keeps its default
toolchain per user in ~/.rustup: a fresh machine has rustup and no toolchain,
so `cargo` fails. The function is cut out of the rendered setup script and run
with stub log, ok, warn and run functions and a PATH that holds only the
binaries a test puts there. A test reads what the function did from a log file
and from the arguments the rustup stub recorded.
"""

from __future__ import annotations

import shutil
import subprocess
import sys
from pathlib import Path

KIT_ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(KIT_ROOT / "scripts"))

import render_registry  # noqa: E402

BASH = shutil.which("bash") or "/bin/bash"
# The comment line the renderer puts directly above the function, so the
# function can be cut out of the script without matching its body.
MARKER = "# --- rustup default toolchain"

PRELUDE = """
run() { echo "run $*" >> "$LOG"; "$@"; }
ok() { echo "ok $*" >> "$LOG"; }
warn() { echo "warn $*" >> "$LOG"; }
log() { echo "log $*" >> "$LOG"; }
"""

# Records every call, and answers `rustup default` the way a fresh machine (no
# default toolchain) or a set-up one does.
RUSTUP_STUB = """#!/bin/sh
echo "$*" >> "${RUSTUP_CALLS}"
if [ "$1" = default ] && [ "$#" = 1 ]; then
  if [ "${RUSTUP_HAS_DEFAULT:-0}" = 1 ]; then
    echo "stable-x86_64-unknown-linux-gnu (default)"
    exit 0
  fi
  echo "error: no default toolchain configured" >&2
  exit 1
fi
exit 0
"""


def _function() -> str:
    """The rendered function, from its marker comment to the closing brace at column 0."""
    script = render_registry.render_setup_script(render_registry.load_registry())
    start = script.index(MARKER)
    return script[start : script.index("\n}\n", start) + 3]


def _run(
    tmp_path: Path,
    *,
    nixos: bool,
    rustup: bool = True,
    has_default: bool = False,
    check_only: int = 0,
) -> tuple[list[str], list[str]]:
    """Run the function once; return what it logged and the rustup calls it made."""
    bin_dir = tmp_path / "bin"
    bin_dir.mkdir()
    if rustup:
        stub = bin_dir / "rustup"
        stub.write_text(RUSTUP_STUB)
        stub.chmod(0o755)
    marker = tmp_path / "NIXOS"
    if nixos:
        marker.write_text("")
    log = tmp_path / "log"
    calls = tmp_path / "rustup-calls"
    calls.write_text("")
    body = PRELUDE + _function() + "\nrustup_default_toolchain\n"
    subprocess.run(
        [BASH, "-c", body],
        check=True,
        env={
            "PATH": f"{bin_dir}:/usr/bin:/bin",
            "LOG": str(log),
            "RUSTUP_CALLS": str(calls),
            "RUSTUP_HAS_DEFAULT": "1" if has_default else "0",
            "AK_NIXOS_MARKER": str(marker),
            "CHECK_ONLY": str(check_only),
        },
    )
    return (
        log.read_text().splitlines() if log.exists() else [],
        calls.read_text().splitlines(),
    )


def test_off_nixos_it_does_nothing(tmp_path: Path) -> None:
    """A missing default toolchain is a NixOS problem; elsewhere the toolchain is the user's own."""
    lines, calls = _run(tmp_path, nixos=False)
    assert calls == []
    assert lines == []


def test_on_nixos_without_rustup_it_reports_where_rustup_comes_from(tmp_path: Path) -> None:
    lines, calls = _run(tmp_path, nixos=True, rustup=False)
    assert calls == []
    assert lines == [
        "warn rustup: not on PATH; on NixOS it comes from nixos-modules' devWorkstation",
    ]


def test_an_existing_default_is_kept(tmp_path: Path) -> None:
    """A repository's rust-toolchain file or a mise pin overrides the default on its own."""
    lines, calls = _run(tmp_path, nixos=True, has_default=True)
    assert "default stable" not in calls
    assert "component add clippy rustfmt rust-src" not in calls
    assert lines == [
        "ok rustup: default toolchain stable-x86_64-unknown-linux-gnu (default) kept",
    ]


def test_without_a_default_it_sets_stable_and_its_components(tmp_path: Path) -> None:
    lines, calls = _run(tmp_path, nixos=True)
    assert "default stable" in calls
    assert "component add clippy rustfmt rust-src" in calls
    assert lines == [
        "run rustup default stable",
        "run rustup component add clippy rustfmt rust-src",
        "ok rustup: default toolchain stable with clippy, rustfmt and rust-src",
    ]


def test_check_mode_reports_and_changes_nothing(tmp_path: Path) -> None:
    lines, calls = _run(tmp_path, nixos=True, check_only=1)
    # Only the probe that asks whether a default exists: nothing is set.
    assert calls == ["default"]
    assert lines == [
        "log would run: rustup default stable && rustup component add clippy rustfmt rust-src",
    ]
