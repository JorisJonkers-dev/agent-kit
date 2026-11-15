"""The swap of the Rust build tracker in behind the build-tracker skill's launcher.

`skills/workstation/build-tracker/build-tracker` prefers the installed Rust
binary (~/.local/bin/build-tracker) and falls back to the node bundle beside
it, so a machine that has not built the binary keeps working and the rollback
needs no setup run. Everything here runs against temp directories with stub
programs: no real cargo build, no real node, no real tracker, and nothing under
the real home.
"""

from __future__ import annotations

import os
import shutil
import subprocess
from pathlib import Path

import pytest

KIT_ROOT = Path(__file__).resolve().parent.parent
SKILL = KIT_ROOT / "skills" / "workstation" / "build-tracker"
LAUNCHER = SKILL / "build-tracker"
INSTALLER = KIT_ROOT / "scripts" / "install-build-tracker-rust.sh"

TOOLS = ("bash", "sh", "env", "dirname", "cmp", "mkdir", "mktemp", "cp", "chmod", "mv", "rm", "cat", "printf", "sleep")


def _tools(directory: Path) -> str:
    """A directory holding only the basic tools (links to the host's), so a test's PATH is exactly what it names."""
    directory.mkdir(parents=True, exist_ok=True)
    for name in TOOLS:
        found = shutil.which(name)
        if found and not (directory / name).exists():
            (directory / name).symlink_to(found)
    return str(directory)


def _program(path: Path, body: str) -> Path:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(f"#!/usr/bin/env bash\n{body}\n")
    path.chmod(0o755)
    return path


@pytest.fixture
def home(tmp_path: Path) -> Path:
    root = tmp_path / "home"
    root.mkdir()
    _program(tmp_path / "bin" / "node", 'echo "node $*"')
    skill = root / ".claude" / "skills" / "build-tracker"
    skill.mkdir(parents=True)
    shutil.copy(LAUNCHER, skill / "build-tracker")
    (skill / "build-tracker.mjs").write_text("// bundle\n")
    return root


def _tracker(home: Path, *args: str, **env: str) -> subprocess.CompletedProcess[str]:
    full = {"HOME": str(home), "PATH": f"{home.parent / 'bin'}:{_tools(home.parent / 'tools')}", **env}
    launcher = home / ".claude" / "skills" / "build-tracker" / "build-tracker"
    return subprocess.run(["sh", str(launcher), *args], capture_output=True, text=True, check=False, env=full)


def _install_rust(home: Path) -> Path:
    return _program(home / ".local" / "bin" / "build-tracker", 'echo "rust $*"')


def test_without_the_rust_binary_the_launcher_runs_the_node_bundle_beside_it(home: Path) -> None:
    result = _tracker(home, "render", "--tickets", "t.yaml")
    assert result.returncode == 0, result.stderr
    assert result.stdout == f"node {home}/.claude/skills/build-tracker/build-tracker.mjs render --tickets t.yaml\n"


def test_with_the_rust_binary_installed_the_launcher_runs_it(home: Path) -> None:
    _install_rust(home)
    assert _tracker(home, "render", "--tickets", "t.yaml").stdout == "rust render --tickets t.yaml\n"


def test_an_unexecutable_rust_binary_falls_back_to_node(home: Path) -> None:
    _install_rust(home).chmod(0o644)
    assert _tracker(home, "path").stdout.startswith("node ")


def test_a_directory_where_the_binary_should_be_is_not_executed(home: Path) -> None:
    (home / ".local" / "bin" / "build-tracker").mkdir(parents=True)
    assert _tracker(home, "path").stdout.startswith("node ")


def test_the_environment_rolls_one_call_back_to_node(home: Path) -> None:
    _install_rust(home)
    assert _tracker(home, "path", AGENT_KIT_BUILD_TRACKER="node").stdout.startswith("node ")
    assert _tracker(home, "path", AGENT_KIT_BUILD_TRACKER="rust").stdout.startswith("rust path")


def test_the_pin_file_rolls_every_call_back_until_it_is_removed(home: Path) -> None:
    _install_rust(home)
    pin = home / ".config" / "agent-kit" / "build-tracker-node"
    pin.parent.mkdir(parents=True)
    pin.write_text("")
    assert _tracker(home, "path").stdout.startswith("node ")
    pin.unlink()
    assert _tracker(home, "path").stdout.startswith("rust path")


def test_another_rust_binary_can_be_named(home: Path, tmp_path: Path) -> None:
    other = _program(tmp_path / "other" / "build-tracker", 'echo "other $*"')
    assert _tracker(home, "path", AGENT_KIT_BUILD_TRACKER_BIN=str(other)).stdout == "other path\n"


def test_the_skills_run_the_launcher() -> None:
    for name in ("build-tracker", "flow"):
        text = (KIT_ROOT / "skills" / "workstation" / name / "SKILL.md").read_text()
        assert "node <this folder>/build-tracker.mjs" not in text
        assert "node <build-tracker dir>/build-tracker.mjs" not in text
    assert "sh <this folder>/build-tracker render --tickets" in (SKILL / "SKILL.md").read_text()


# ---------------------------------------------------------------------------
# The installer
# ---------------------------------------------------------------------------


class Kit:
    """A kit directory with a workspace manifest, and a stub cargo that 'builds' the binary."""

    def __init__(self, tmp_path: Path, home: Path) -> None:
        self.root = tmp_path / "kit"
        (self.root / "scripts").mkdir(parents=True)
        (self.root / "scripts" / "Cargo.toml").write_text("[workspace]\n")
        self.home = home
        self.tools = _tools(tmp_path / "tools")
        self.bin = tmp_path / "cargo-bin"
        self.dest = home / ".local" / "bin" / "build-tracker"
        self.cargo(0, "echo built")

    def cargo(self, code: int, binary_body: str | None, smoke: int = 0) -> None:
        build = f"mkdir -p {self.root}/scripts/target/release\n"
        if binary_body is not None:
            build += (
                f"printf '#!/usr/bin/env bash\\n[ \"$1\" = --help ] && exit {smoke}\\n{binary_body}\\n' "
                f"> {self.root}/scripts/target/release/build-tracker\n"
                f"chmod 755 {self.root}/scripts/target/release/build-tracker\n"
            )
        _program(self.bin / "cargo", f'echo "$@" > {self.root}/cargo-args\n{build}exit {code}')

    def run(self, *args: str, path: bool = True, **env: str) -> subprocess.CompletedProcess[str]:
        full = {
            "HOME": str(self.home),
            "KIT_ROOT": str(self.root),
            "PATH": f"{self.bin}:{self.tools}" if path else self.tools,
            **env,
        }
        return subprocess.run(["bash", str(INSTALLER), *args], capture_output=True, text=True, check=False, env=full)


@pytest.fixture
def kit(tmp_path: Path, home: Path) -> Kit:
    return Kit(tmp_path, home)


def test_the_installer_builds_and_installs_the_binary(kit: Kit) -> None:
    result = kit.run()
    assert result.returncode == 0, result.stderr
    assert f"installed {kit.dest}" in result.stdout
    args = (kit.root / "cargo-args").read_text().split()
    assert args[:2] == ["build", "--release"] and "--locked" in args
    assert args[args.index("-p") + 1] == "build-tracker"
    assert args[args.index("--manifest-path") + 1] == f"{kit.root}/scripts/Cargo.toml"
    assert os.access(kit.dest, os.X_OK)
    assert kit.run().stdout.count("already current") == 1


def test_the_installer_honours_xdg_bin_home(kit: Kit, tmp_path: Path) -> None:
    elsewhere = tmp_path / "elsewhere"
    assert kit.run(XDG_BIN_HOME=str(elsewhere)).returncode == 0
    assert os.access(elsewhere / "build-tracker", os.X_OK) and not kit.dest.exists()


def test_a_failed_build_leaves_the_installed_binary_alone(kit: Kit) -> None:
    kit.run()
    before = kit.dest.read_bytes()
    kit.cargo(101, "echo new")
    result = kit.run()
    assert result.returncode == 1
    assert "the installed build tracker is unchanged" in result.stdout
    assert kit.dest.read_bytes() == before


def test_a_binary_that_does_not_answer_help_is_not_installed(kit: Kit) -> None:
    kit.cargo(0, "echo new", smoke=1)
    result = kit.run()
    assert result.returncode == 1
    assert not kit.dest.exists()


def test_a_build_that_leaves_no_binary_is_a_failure(kit: Kit) -> None:
    kit.cargo(0, None)
    assert kit.run().returncode == 1


def test_an_install_that_cannot_land_is_reported(kit: Kit) -> None:
    kit.dest.parent.parent.mkdir(parents=True)
    kit.dest.parent.write_text("a file where the directory should be")
    result = kit.run()
    assert result.returncode == 1
    assert "could not create" in result.stdout


def test_an_unknown_argument_builds_nothing(kit: Kit) -> None:
    result = kit.run("--chekc")
    assert result.returncode == 2
    assert not (kit.root / "cargo-args").exists()


def test_without_cargo_a_workspace_or_with_the_pin_the_step_is_skipped(kit: Kit, home: Path) -> None:
    assert "cargo is not on PATH" in kit.run(path=False).stdout
    (kit.root / "scripts" / "Cargo.toml").unlink()
    assert "no cargo workspace" in kit.run().stdout
    (kit.root / "scripts" / "Cargo.toml").write_text("[workspace]\n")
    pin = home / ".config" / "agent-kit" / "build-tracker-node"
    pin.parent.mkdir(parents=True)
    pin.write_text("")
    result = kit.run()
    assert result.returncode == 0 and "the node bundle is pinned" in result.stdout
    assert not kit.dest.exists()


def test_check_builds_nothing(kit: Kit) -> None:
    result = kit.run("--check")
    assert result.returncode == 0 and "would build and install" in result.stdout
    assert not kit.dest.exists()
    assert not (kit.root / "scripts" / "target").exists()


def test_setup_runs_the_installer_and_the_bundle_ships_it() -> None:
    setup = (KIT_ROOT / "installer" / "setup-workstation.sh").read_text()
    assert "build_tracker_rust_setup" in setup
    assert "scripts/install-build-tracker-rust.sh" in setup
    assert (
        "scripts/install-build-tracker-rust.sh" in (KIT_ROOT / "scripts" / "publish-installer-artifacts.sh").read_text()
    )
