"""The swap of the Rust critique runner in behind the critique skill's launcher.

`skills/workstation/critique/critique` prefers the installed Rust binary, with
the skill directory as its assets, and falls back to the node runner beside it,
so a machine that has not built the binary keeps working and the rollback needs
no setup run; the hooks and every skill call the launcher. Everything here runs
against temp directories with stub programs: no real cargo build, no real node,
no real runner.
"""

from __future__ import annotations

import os
import shutil
import subprocess
from pathlib import Path

import pytest

KIT_ROOT = Path(__file__).resolve().parent.parent
SKILL = KIT_ROOT / "skills" / "workstation" / "critique"
LAUNCHER = SKILL / "critique"
INSTALLER = KIT_ROOT / "scripts" / "install-critique-rust.sh"


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
    skill = root / ".claude" / "skills" / "critique"
    skill.mkdir(parents=True)
    shutil.copy(LAUNCHER, skill / "critique")
    (skill / "critique.mjs").write_text("// bundle\n")
    return root


def _critique(home: Path, *args: str, **env: str) -> subprocess.CompletedProcess[str]:
    full = {"HOME": str(home), "PATH": f"{home.parent / 'bin'}:{_tools(home.parent / 'tools')}", **env}
    launcher = home / ".claude" / "skills" / "critique" / "critique"
    return subprocess.run(["sh", str(launcher), *args], capture_output=True, text=True, check=False, env=full)


def _install_rust(home: Path) -> Path:
    binary = home / ".local" / "share" / "agent-kit" / "critique" / "critique"
    return _program(binary, 'echo "rust $* assets=$AGENT_KIT_CRITIQUE_ASSETS"')


def test_without_the_rust_binary_the_launcher_runs_the_node_runner_beside_it(home: Path) -> None:
    result = _critique(home, "stats", "--json", "{}")
    assert result.returncode == 0, result.stderr
    assert result.stdout == f"node {home}/.claude/skills/critique/critique.mjs stats --json {{}}\n"


def test_with_the_rust_binary_installed_the_launcher_runs_it_with_the_skill_as_its_assets(home: Path) -> None:
    _install_rust(home)
    result = _critique(home, "stats", "--json", "{}")
    assert result.stdout == f"rust stats --json {{}} assets={home}/.claude/skills/critique\n"


def test_an_unexecutable_rust_binary_falls_back_to_node(home: Path) -> None:
    _install_rust(home).chmod(0o644)
    assert _critique(home, "stats").stdout.startswith("node ")


def test_the_environment_rolls_one_call_back_to_node(home: Path) -> None:
    _install_rust(home)
    assert _critique(home, "stats", AGENT_KIT_CRITIQUE="node").stdout.startswith("node ")
    assert _critique(home, "stats", AGENT_KIT_CRITIQUE="rust").stdout.startswith("rust stats")


def test_the_pin_file_rolls_every_call_back_until_it_is_removed(home: Path) -> None:
    _install_rust(home)
    pin = home / ".config" / "agent-kit" / "critique-node"
    pin.parent.mkdir(parents=True)
    pin.write_text("")
    assert _critique(home, "stats").stdout.startswith("node ")
    pin.unlink()
    assert _critique(home, "stats").stdout.startswith("rust stats")


def test_another_rust_binary_can_be_named(home: Path, tmp_path: Path) -> None:
    other = _program(tmp_path / "other" / "critique", 'echo "other $*"')
    assert _critique(home, "stats", AGENT_KIT_CRITIQUE_BIN=str(other)).stdout == "other stats\n"


def test_the_hooks_and_the_skills_run_the_launcher() -> None:
    for hook in sorted((SKILL / "hooks").glob("*.sh")):
        assert f'exec /bin/sh "${{0%/*}}/../critique" hook {hook.stem}' in hook.read_text(), hook
    for skill in sorted((KIT_ROOT / "skills" / "workstation").glob("*/SKILL.md")):
        assert "/critique.mjs" not in skill.read_text().replace("`critique.mjs`", ""), skill


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
        self.dest = home / ".local" / "share" / "agent-kit" / "critique" / "critique"
        self.cargo(0, "echo built")

    def cargo(self, code: int, binary_body: str | None, smoke: int = 0) -> None:
        build = f"mkdir -p {self.root}/scripts/target/release\n"
        if binary_body is not None:
            build += (
                f"printf '#!/usr/bin/env bash\\n[ \"$1\" = --help ] && exit {smoke}\\n{binary_body}\\n' "
                f"> {self.root}/scripts/target/release/critique\n"
                f"chmod 755 {self.root}/scripts/target/release/critique\n"
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
    assert args[args.index("-p") + 1] == "critique"
    assert args[args.index("--manifest-path") + 1] == f"{kit.root}/scripts/Cargo.toml"
    assert os.access(kit.dest, os.X_OK)
    assert kit.run().stdout.count("already current") == 1


def test_a_failed_build_leaves_the_installed_binary_alone(kit: Kit) -> None:
    kit.run()
    before = kit.dest.read_bytes()
    kit.cargo(101, "echo new")
    result = kit.run()
    assert result.returncode == 1
    assert "the installed runner is unchanged" in result.stdout
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


def test_a_directory_where_the_binary_should_be_is_not_executed(home: Path) -> None:
    (home / ".local" / "share" / "agent-kit" / "critique" / "critique").mkdir(parents=True)
    assert _critique(home, "stats").stdout.startswith("node ")


def test_an_unknown_argument_builds_nothing(kit: Kit) -> None:
    result = kit.run("--chekc")
    assert result.returncode == 2
    assert not (kit.root / "cargo-args").exists()


def test_without_cargo_a_workspace_or_with_the_pin_the_step_is_skipped(kit: Kit, home: Path) -> None:
    assert "cargo is not on PATH" in kit.run(path=False).stdout
    (kit.root / "scripts" / "Cargo.toml").unlink()
    assert "no cargo workspace" in kit.run().stdout
    (kit.root / "scripts" / "Cargo.toml").write_text("[workspace]\n")
    pin = home / ".config" / "agent-kit" / "critique-node"
    pin.parent.mkdir(parents=True)
    pin.write_text("")
    result = kit.run()
    assert result.returncode == 0 and "the node runner is pinned" in result.stdout
    assert not kit.dest.exists()


def test_check_builds_nothing(kit: Kit) -> None:
    result = kit.run("--check")
    assert result.returncode == 0 and "would build and install" in result.stdout
    assert not kit.dest.exists()
    assert not (kit.root / "scripts" / "target").exists()


def test_setup_runs_the_installer_and_the_bundle_ships_it() -> None:
    setup = (KIT_ROOT / "installer" / "setup-workstation.sh").read_text()
    assert "critique_rust_setup" in setup
    assert "scripts/install-critique-rust.sh" in setup
    assert "scripts/install-critique-rust.sh" in (KIT_ROOT / "scripts" / "publish-installer-artifacts.sh").read_text()
