"""Setup calls the workspace's Rust canary crate; converge reports the result.

converge.py opens every setup-run convergence with one call into the crate's
built binary for its version, so the Rust workspace path is proven end to end
from setup itself (docs/RUST.md). The unit cases fake a kit root (and the
toolchain) to cover each branch deterministically; the last case builds and
calls the real crate when the machine has a Rust toolchain, and skips with
the same reason setup itself prints when it does not.
"""

from __future__ import annotations

import re
import shutil
import subprocess
import sys
from pathlib import Path

import pytest

KIT_ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(KIT_ROOT / "scripts"))

import converge  # noqa: E402


def fake_kit(root: Path, binary: str, *, profile: str = "release") -> Path:
    """A kit root whose workspace crate has one built canary binary."""
    path = root / "scripts" / "target" / profile / converge.CRATE_NAME
    path.parent.mkdir(parents=True)
    path.write_text("#!/usr/bin/env bash\n" + binary + "\n")
    path.chmod(0o755)
    return root


def test_a_built_binary_reports_its_version(tmp_path: Path) -> None:
    kit = fake_kit(tmp_path, 'echo "agent-kit-setup 9.9.9"')
    line, code = converge.crate_report(kit, cargo=True)
    assert code == 0
    assert line.startswith("agent-kit-setup: agent-kit-setup 9.9.9 (")
    assert line.endswith(f"/scripts/target/release/{converge.CRATE_NAME})")


def test_a_debug_build_is_used_when_there_is_no_release_build(tmp_path: Path) -> None:
    kit = fake_kit(tmp_path, 'echo "agent-kit-setup 0.0.1"', profile="debug")
    line, code = converge.crate_report(kit, cargo=False)
    assert code == 0
    assert line.endswith(f"/scripts/target/debug/{converge.CRATE_NAME})")


def test_a_built_binary_that_cannot_answer_blocks_the_run(tmp_path: Path) -> None:
    kit = fake_kit(tmp_path, "echo noise >&2\nexit 3")
    line, code = converge.crate_report(kit, cargo=True)
    assert code == 1
    assert "answered exit 3, not a version" in line


def test_without_a_build_the_call_skips_with_the_build_command(tmp_path: Path) -> None:
    line, code = converge.crate_report(tmp_path, cargo=True)
    assert code == 0
    assert line == "agent-kit-setup: skipped: not built yet; run cargo build --manifest-path scripts/Cargo.toml"


def test_without_a_rust_toolchain_the_call_skips_with_that_reason(tmp_path: Path) -> None:
    line, code = converge.crate_report(tmp_path, cargo=False)
    assert code == 0
    assert line == "agent-kit-setup: skipped: no Rust toolchain (cargo) on PATH"


def test_main_prints_the_report_before_converging(tmp_path: Path, monkeypatch, capsys) -> None:
    monkeypatch.setattr(converge, "KIT_ROOT", tmp_path)
    monkeypatch.setattr(converge.shutil, "which", lambda _name: None)
    args = ["mcp", "--keep", str(tmp_path / "keep.yaml"), "--claude-home", str(tmp_path), "--check"]
    rc = converge.main(args)
    out = capsys.readouterr().out.splitlines()
    assert rc == 0
    assert out[0].startswith("agent-kit-setup: skipped: no Rust toolchain")


def test_the_built_crate_reaches_setup_end_to_end(tmp_path: Path) -> None:
    """The harness case: cargo builds the real crate, converge.py reports it."""
    if shutil.which("cargo") is None:
        pytest.skip("Rust toolchain (cargo) is absent: the canary crate cannot be built and run")
    build = subprocess.run(
        ["cargo", "build", "--release", "--manifest-path", str(KIT_ROOT / "scripts" / "Cargo.toml")],
        capture_output=True,
        text=True,
        check=False,
    )
    assert build.returncode == 0, build.stderr
    keep = tmp_path / "keep.yaml"
    keep.write_text("keep: []\n")
    result = subprocess.run(
        [sys.executable, str(KIT_ROOT / "scripts" / "converge.py"), "mcp", "--keep", str(keep),
         "--claude-home", str(tmp_path), "--check"],
        capture_output=True,
        text=True,
        check=False,
    )
    assert result.returncode == 0, result.stdout + result.stderr
    first = result.stdout.splitlines()[0]
    _, name, number, path = first.split()
    assert name == "agent-kit-setup"
    assert re.fullmatch(r"\d+\.\d+\.\d+", number)
    assert path == f"({KIT_ROOT}/scripts/target/release/agent-kit-setup)"


def test_a_binary_that_hangs_blocks_the_run_instead_of_hanging_it(tmp_path: Path, monkeypatch) -> None:
    monkeypatch.setattr(converge, "CRATE_TIMEOUT_S", 0.5)
    kit = fake_kit(tmp_path, "sleep 30")
    line, code = converge.crate_report(kit, cargo=True)
    assert code == 1
    assert "did not answer within" in line


def test_a_failing_canary_stops_the_run_before_any_surface_is_changed(tmp_path: Path, monkeypatch, capsys) -> None:
    kit = fake_kit(tmp_path / "kit", "exit 3")
    monkeypatch.setattr(converge, "KIT_ROOT", kit)
    monkeypatch.setattr(converge.shutil, "which", lambda _name: "/usr/bin/cargo")
    touched: list[str] = []
    monkeypatch.setattr(converge, "mcp", lambda _a: touched.append("mcp") or 0)
    monkeypatch.setattr(converge, "plugins", lambda _a: touched.append("plugins") or 0)
    rc = converge.main(["mcp", "--keep", str(tmp_path / "keep.yaml"), "--claude-home", str(tmp_path)])
    assert rc == 1
    assert touched == []
    assert "answered exit 3" in capsys.readouterr().out
