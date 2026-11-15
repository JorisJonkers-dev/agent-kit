"""scripts/install-workspace-tools.sh builds the registry's ``workspace_tools:`` from
the kit's pinned cargo workspace and installs each into ~/.local/bin.

Everything runs against temp directories with a stub ``cargo``: no real build and no
network. The script itself needs ``bash``, which the test starts through PATH.
"""

from __future__ import annotations

import os
import shutil
import subprocess
from pathlib import Path

import pytest
import yaml

KIT_ROOT = Path(__file__).resolve().parent.parent
INSTALLER = KIT_ROOT / "scripts" / "install-workspace-tools.sh"
REGISTRY = KIT_ROOT / "registry" / "estate-tooling.yaml"
TOOLS = ("bash", "sh", "env", "dirname", "cmp", "mkdir", "mktemp", "cp", "chmod", "mv", "rm", "cat", "printf", "grep")
NAMES = ("agent-launcher", "gold-band-gate", "ci-wait")


def _program(path: Path, body: str) -> Path:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(f"#!/usr/bin/env bash\n{body}\n")
    path.chmod(0o755)
    return path


class Kit:
    """A kit with a workspace manifest, and a stub cargo that 'builds' every -p package."""

    def __init__(self, tmp_path: Path) -> None:
        self.root = tmp_path / "kit"
        (self.root / "scripts").mkdir(parents=True)
        (self.root / "scripts" / "Cargo.toml").write_text("[workspace]\n")
        self.home = tmp_path / "home"
        self.home.mkdir()
        self.dest = self.home / ".local" / "bin"
        tools = tmp_path / "tools"
        tools.mkdir()
        for name in TOOLS:
            found = shutil.which(name)
            if found:
                (tools / name).symlink_to(found)
        self.tools = str(tools)
        self.bin = tmp_path / "cargo-bin"
        self.cargo()

    def cargo(self, code: int = 0, body: str = "echo built; exit 0", skip: str = "") -> None:
        """A cargo that records its arguments and writes one stub binary per ``-p`` package (but not ``skip``)."""
        _program(
            self.bin / "cargo",
            f'echo "$PWD $@" > {self.root}/cargo-args\n'
            f"mkdir -p {self.root}/scripts/target/release\n"
            'while [ "$#" -gt 0 ]; do\n'
            '  if [ "$1" = -p ]; then\n'
            f'    [ "$2" = "{skip}" ] || {{ printf \'#!/usr/bin/env bash\\n{body}\\n\' > '
            f'{self.root}/scripts/target/release/"$2"; chmod 755 {self.root}/scripts/target/release/"$2"; }}\n'
            "  fi\n"
            "  shift\n"
            "done\n"
            f"exit {code}",
        )

    def run(self, *args: str, path: bool = True, **env: str) -> subprocess.CompletedProcess[str]:
        full = {
            "HOME": str(self.home),
            "KIT_ROOT": str(self.root),
            "PATH": f"{self.bin}:{self.tools}" if path else self.tools,
            **env,
        }
        return subprocess.run(["bash", str(INSTALLER), *args], capture_output=True, text=True, check=False, env=full)


@pytest.fixture
def kit(tmp_path: Path) -> Kit:
    return Kit(tmp_path)


def test_the_installer_builds_every_name_in_one_locked_build_and_installs_each(kit: Kit) -> None:
    result = kit.run(*NAMES)
    assert result.returncode == 0, result.stderr
    for name in NAMES:
        assert f"installed {kit.dest / name}" in result.stdout
        assert os.access(kit.dest / name, os.X_OK)
    where, *args = (kit.root / "cargo-args").read_text().split()
    assert where == str(kit.root), "the build runs from the kit"
    assert args[:2] == ["build", "--quiet"] and "--release" in args and "--locked" in args
    assert [args[i + 1] for i, arg in enumerate(args) if arg == "-p"] == list(NAMES)
    assert args[args.index("--manifest-path") + 1] == f"{kit.root}/scripts/Cargo.toml"


def test_a_second_run_finds_every_tool_current(kit: Kit) -> None:
    kit.run(*NAMES)
    result = kit.run(*NAMES)
    assert result.returncode == 0
    assert result.stdout.count("already current") == len(NAMES)


def test_a_changed_binary_replaces_the_installed_one_atomically(kit: Kit) -> None:
    kit.run(*NAMES)
    kit.cargo(body="echo newer; exit 0")
    result = kit.run(*NAMES)
    assert result.stdout.count("installed ") == len(NAMES)
    assert "newer" in (kit.dest / "ci-wait").read_text()
    assert sorted(path.name for path in kit.dest.iterdir()) == sorted(NAMES), "no staged file is left behind"


def test_a_tool_that_prints_its_usage_and_exits_two_passes_the_smoke_check(kit: Kit) -> None:
    kit.cargo(body="echo usage: tool >&2; exit 2")
    assert kit.run(*NAMES).returncode == 0
    assert all((kit.dest / name).exists() for name in NAMES)


def test_a_failed_build_installs_nothing_and_leaves_what_is_there(kit: Kit) -> None:
    kit.run(*NAMES)
    before = (kit.dest / "ci-wait").read_bytes()
    kit.cargo(code=101, body="echo newer; exit 0")
    result = kit.run(*NAMES)
    assert result.returncode == 1 and "the installed tools are unchanged" in result.stdout
    assert (kit.dest / "ci-wait").read_bytes() == before


def test_one_binary_that_does_not_start_blocks_every_install(kit: Kit) -> None:
    kit.cargo(body="exit 127")
    result = kit.run(*NAMES)
    assert result.returncode == 1 and "does not start (exit 127)" in result.stdout
    assert not kit.dest.exists()


def test_a_binary_that_prints_nothing_does_not_start(kit: Kit) -> None:
    kit.cargo(body="exit 0")
    assert kit.run(*NAMES).returncode == 1


def test_a_build_that_leaves_a_binary_out_installs_nothing(kit: Kit) -> None:
    kit.cargo(skip="ci-wait")
    result = kit.run(*NAMES)
    assert result.returncode == 1 and "the build left no" in result.stdout
    assert not kit.dest.exists()


def test_an_install_that_cannot_land_is_reported(kit: Kit) -> None:
    kit.dest.parent.mkdir(parents=True)
    kit.dest.write_text("a file where the directory should be")
    result = kit.run(*NAMES)
    assert result.returncode == 1 and "could not create" in result.stdout


def test_an_unwritable_directory_fails_the_install_not_the_others(kit: Kit) -> None:
    kit.dest.mkdir(parents=True)
    kit.dest.chmod(0o500)
    try:
        result = kit.run(*NAMES)
    finally:
        kit.dest.chmod(0o700)
    assert result.returncode == 1 and "could not install" in result.stdout


def test_without_cargo_or_a_workspace_the_step_is_skipped(kit: Kit) -> None:
    assert "cargo is not on PATH" in kit.run(*NAMES, path=False).stdout
    (kit.root / "scripts" / "Cargo.toml").unlink()
    result = kit.run(*NAMES)
    assert result.returncode == 0 and "no cargo workspace" in result.stdout


def test_check_reports_each_tool_and_builds_nothing(kit: Kit) -> None:
    result = kit.run("--check", *NAMES)
    assert result.returncode == 0
    assert [line for line in result.stdout.splitlines() if "would build and install" in line] == [
        f"workspace tools: would build and install {kit.dest / name}" for name in NAMES
    ]
    assert not (kit.root / "cargo-args").exists() and not kit.dest.exists()


@pytest.mark.parametrize("args", [(), ("--check",), ("--chekc", "ci-wait"), ("Bad_Name",), ("../x",)])
def test_bad_usage_builds_nothing(kit: Kit, args: tuple[str, ...]) -> None:
    result = kit.run(*args)
    assert result.returncode == 2
    assert not (kit.root / "cargo-args").exists()


def test_the_binaries_go_where_bin_dir_says(kit: Kit, tmp_path: Path) -> None:
    other = tmp_path / "elsewhere"
    assert kit.run("ci-wait", WORKSPACE_TOOLS_BIN_DIR=str(other)).returncode == 0
    assert (other / "ci-wait").exists() and not kit.dest.exists()


# ---------------------------------------------------------------------------
# The registry declaration, the generated setup and the bundle
# ---------------------------------------------------------------------------

DECLARED = [
    "agent-launcher",
    "gold-band-gate",
    "gold-band-run-start",
    "gold-band-findings",
    "gold-band-proof",
    "research-layout",
    "ticket-publish",
    "workflow-lint",
    "coverage-gate",
    "ci-wait",
    "litellm-gate",
    "pr-composer",
]


def test_the_registry_declares_the_tools_and_each_is_a_workspace_crate_with_that_binary() -> None:
    declared = yaml.safe_load(REGISTRY.read_text())["workspace_tools"]
    assert [tool["name"] for tool in declared] == DECLARED
    members = (KIT_ROOT / "scripts" / "Cargo.toml").read_text()
    for name in DECLARED:
        assert f'"crates/{name}"' in members
        assert (KIT_ROOT / "scripts" / "crates" / name / "src" / "main.rs").is_file() or name == "gold-band-proof"


def test_setup_runs_the_installer_with_the_declared_names_and_the_bundle_ships_everything_it_reads() -> None:
    setup = (KIT_ROOT / "installer" / "setup-workstation.sh").read_text()
    assert "workspace_tools_setup" in setup
    call = 'scripts/install-workspace-tools.sh" ${mode[@]+"${mode[@]}"} \\\n    ' + " ".join(DECLARED) + " 2>&1"
    assert call in setup
    publish = (KIT_ROOT / "scripts" / "publish-installer-artifacts.sh").read_text()
    shipped = publish.split("KIT_PATHS=(")[1].split(")")[0].split()
    needed = ("scripts/install-workspace-tools.sh", "scripts/Cargo.toml", "scripts/Cargo.lock")
    for path in (*needed, "scripts/crates"):
        assert path in shipped, path


def test_a_registry_that_names_a_missing_crate_or_repeats_one_is_refused() -> None:
    import sys

    sys.path.insert(0, str(KIT_ROOT / "scripts"))
    import render_registry

    data = yaml.safe_load(REGISTRY.read_text())
    for tools, message in (
        ([{"name": "no-such-crate", "surfaces": ["workstation"]}], "not a crate"),
        ([{"name": "ci-wait", "surfaces": ["workstation"]}] * 2, "twice"),
        ([{"name": "Bad_Name", "surfaces": ["workstation"]}], "plain `name`"),
        ([{"name": "ci-wait"}], "must declare surfaces"),
        ([{"name": "ci-wait", "surfaces": ["mars"]}], "unknown surfaces"),
    ):
        data["workspace_tools"] = tools
        with pytest.raises(render_registry.RegistryError, match=message):
            render_registry.validate(data)
