"""The swap of the Rust model gateway in behind the `model-gateway` command.

The command prefers the installed Rust binary and falls back to the node
runner, so a machine that has not built the binary yet keeps working, and the
rollback needs no setup run. Everything here runs against temp directories with
stub programs: no real cargo build, no real node, no real gateway.
"""

from __future__ import annotations

import os
import shutil
import subprocess
from pathlib import Path

import pytest

KIT_ROOT = Path(__file__).resolve().parent.parent
COMMAND = KIT_ROOT / "installer" / "model-gateway"
INSTALLER = KIT_ROOT / "scripts" / "install-model-gateway-rust.sh"


TOOLS = ("bash", "env", "dirname", "cmp", "mkdir", "mktemp", "cp", "chmod", "mv", "rm", "cat", "printf", "sleep")


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
    return root


def _gateway(home: Path, *args: str, **env: str) -> subprocess.CompletedProcess[str]:
    full = {"HOME": str(home), "PATH": f"{home.parent / 'bin'}:{_tools(home.parent / 'tools')}", **env}
    return subprocess.run([str(COMMAND), *args], capture_output=True, text=True, check=False, env=full)


def _install_rust(home: Path) -> Path:
    return _program(home / ".local" / "share" / "agent-kit" / "model-gateway" / "model-gateway", 'echo "rust $*"')


def test_without_the_rust_binary_the_command_runs_the_node_runner(home: Path) -> None:
    result = _gateway(home, "models", "--json", "{}")
    assert result.returncode == 0, result.stderr
    assert result.stdout == f"node {home}/.claude/skills/model-gateway/model-gateway.mjs models --json {{}}\n"


def test_with_the_rust_binary_installed_the_command_runs_it(home: Path) -> None:
    _install_rust(home)
    result = _gateway(home, "models", "--json", "{}")
    assert result.stdout == "rust models --json {}\n"


def test_an_unexecutable_rust_binary_falls_back_to_node(home: Path) -> None:
    binary = _install_rust(home)
    binary.chmod(0o644)
    assert _gateway(home, "usage").stdout.startswith("node ")


def test_the_environment_rolls_one_call_back_to_node(home: Path) -> None:
    _install_rust(home)
    assert _gateway(home, "usage", AGENT_KIT_MODEL_GATEWAY="node").stdout.startswith("node ")
    assert _gateway(home, "usage", AGENT_KIT_MODEL_GATEWAY="rust").stdout == "rust usage\n"


def test_the_pin_file_rolls_every_call_back_until_it_is_removed(home: Path) -> None:
    _install_rust(home)
    pin = home / ".config" / "agent-kit" / "model-gateway-node"
    pin.parent.mkdir(parents=True)
    pin.write_text("")
    assert _gateway(home, "usage").stdout.startswith("node ")
    pin.unlink()
    assert _gateway(home, "usage").stdout == "rust usage\n"


def test_another_rust_binary_can_be_named(home: Path, tmp_path: Path) -> None:
    other = _program(tmp_path / "other" / "gateway", 'echo "other $*"')
    assert _gateway(home, "usage", AGENT_KIT_MODEL_GATEWAY_BIN=str(other)).stdout == "other usage\n"


def test_the_command_keeps_the_node_exec_line_the_setup_tests_pin() -> None:
    lines = COMMAND.read_text().splitlines()
    assert lines[0] == "#!/usr/bin/env bash"
    assert "# managed by agent-kit setup-workstation.sh" in lines
    assert 'exec node "${CLAUDE_CONFIG_DIR:-${HOME}/.claude}/skills/model-gateway/model-gateway.mjs" "$@"' in lines


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
        self.dest = home / ".local" / "share" / "agent-kit" / "model-gateway" / "model-gateway"
        self.cargo(0, "echo built")

    def cargo(self, code: int, binary_body: str | None, smoke: int = 0) -> None:
        build = f"mkdir -p {self.root}/scripts/target/release\n"
        if binary_body is not None:
            build += (
                f"printf '#!/usr/bin/env bash\\n[ \"$1\" = --help ] && exit {smoke}\\n{binary_body}\\n' "
                f"> {self.root}/scripts/target/release/model-gateway\n"
                f"chmod 755 {self.root}/scripts/target/release/model-gateway\n"
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
    assert args[args.index("-p") + 1] == "model-gateway"
    assert args[args.index("--manifest-path") + 1] == f"{kit.root}/scripts/Cargo.toml"
    assert os.access(kit.dest, os.X_OK)
    assert kit.run().stdout.count("already current") == 1


def test_a_failed_build_leaves_the_installed_binary_alone(kit: Kit) -> None:
    kit.run()
    before = kit.dest.read_bytes()
    kit.cargo(101, "echo new")
    result = kit.run()
    assert result.returncode == 1
    assert "the installed gateway is unchanged" in result.stdout
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
    (home / ".local" / "share" / "agent-kit" / "model-gateway" / "model-gateway").mkdir(parents=True)
    assert _gateway(home, "usage").stdout.startswith("node ")


def test_an_unknown_argument_builds_nothing(kit: Kit) -> None:
    result = kit.run("--chekc")
    assert result.returncode == 2
    assert not (kit.root / "cargo-args").exists()


def test_without_cargo_a_workspace_or_with_the_pin_the_step_is_skipped(kit: Kit, home: Path) -> None:
    assert "cargo is not on PATH" in kit.run(path=False).stdout
    (kit.root / "scripts" / "Cargo.toml").unlink()
    assert "no cargo workspace" in kit.run().stdout
    (kit.root / "scripts" / "Cargo.toml").write_text("[workspace]\n")
    pin = home / ".config" / "agent-kit" / "model-gateway-node"
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
    assert "model_gateway_rust_setup" in setup
    assert "scripts/install-model-gateway-rust.sh" in setup
    assert (
        "scripts/install-model-gateway-rust.sh" in (KIT_ROOT / "scripts" / "publish-installer-artifacts.sh").read_text()
    )
