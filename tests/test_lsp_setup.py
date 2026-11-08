"""Each rendered language-server block, run on its own under bash.

The block is cut out of the checked-in setup script and run with stub
functions for run, ok, warn, log and run_sh, and a PATH that holds only the
binaries a test puts there. A test reads what the block did from a log.
"""

from __future__ import annotations

import copy
import shutil
import subprocess
import sys
from pathlib import Path

import pytest

KIT_ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(KIT_ROOT / "scripts"))

import render_registry  # noqa: E402

SETUP = KIT_ROOT / "installer" / "setup-workstation.sh"
BASH = shutil.which("bash") or "/bin/bash"

PRELUDE = """
run() { echo "run $*" >> "$LOG"; }
run_sh() { echo "run_sh $1" >> "$LOG"; }
ok() { echo "ok $*" >> "$LOG"; }
warn() { echo "warn $*" >> "$LOG"; }
log() { echo "log $*" >> "$LOG"; }
CLOUD=0
unpinned_markets=()
installed_plugins=''
missing_lsp_binaries=()
"""


def _block(plugin: str) -> str:
    script = SETUP.read_text()
    start = script.index(f"    # {plugin} -> ")
    ends = [i for i in (script.find("\n    # ", start + 1), script.find("\n  else\n", start)) if i != -1]
    return script[start : min(ends)]


def _run(tmp_path: Path, plugin: str, *, on_path: tuple[str, ...] = (), nixos: bool = False) -> list[str]:
    bin_dir = tmp_path / "bin"
    bin_dir.mkdir()
    for name in on_path:
        (bin_dir / name).write_text("#!/bin/sh\n")
        (bin_dir / name).chmod(0o755)
    marker = tmp_path / "NIXOS"
    if nixos:
        marker.write_text("")
    log = tmp_path / "log"
    body = PRELUDE + _block(plugin) + '\nprintf "missing %s\\n" "${missing_lsp_binaries[@]}" >> "$LOG"\n'
    subprocess.run(
        [BASH, "-c", body],
        check=True,
        env={"PATH": str(bin_dir), "LOG": str(log), "AK_NIXOS_MARKER": str(marker)},
    )
    return log.read_text().splitlines()


def test_jdtls_without_java_stays_disabled_and_is_not_reinstalled(tmp_path: Path) -> None:
    lines = _run(tmp_path, "jdtls-lsp", on_path=("jdtls",))
    assert "run claude plugin disable jdtls-lsp@agent-kit-official" in lines
    assert "missing jdtls-lsp: install java, which jdtls runs" in lines
    assert not any(line.startswith("run_sh") for line in lines)


def test_jdtls_with_java_is_enabled(tmp_path: Path) -> None:
    lines = _run(tmp_path, "jdtls-lsp", on_path=("jdtls", "java"))
    assert "run claude plugin enable jdtls-lsp@agent-kit-official" in lines


def test_on_nixos_a_missing_server_is_reported_as_a_nix_config_package_not_installed(tmp_path: Path) -> None:
    lines = _run(tmp_path, "gopls-lsp", nixos=True)
    assert not any(line.startswith("run_sh") for line in lines)
    assert "missing gopls-lsp: declare pkgs.gopls in nix-config" in lines


def test_off_nixos_a_missing_server_is_installed(tmp_path: Path) -> None:
    lines = _run(tmp_path, "gopls-lsp")
    assert "run_sh go install golang.org/x/tools/gopls@latest" in lines


def test_on_nixos_a_server_without_a_nixos_package_still_installs(tmp_path: Path) -> None:
    lines = _run(tmp_path, "typescript-lsp", nixos=True)
    assert any(line.startswith("run_sh npm install -g typescript-language-server") for line in lines)


def test_a_server_that_is_off_by_default_is_never_installed(tmp_path: Path) -> None:
    lines = _run(tmp_path, "liquid-lsp")
    assert not any(line.startswith("run_sh") for line in lines)
    assert "run claude plugin disable liquid-lsp@agent-kit-official" in lines


def test_a_server_that_is_off_by_default_turns_on_once_its_binary_is_installed(tmp_path: Path) -> None:
    lines = _run(tmp_path, "liquid-lsp", on_path=("shopify",))
    assert "run claude plugin enable liquid-lsp@agent-kit-official" in lines


def test_kotlin_lsp_downloads_from_the_host_that_signs_the_cdn_link() -> None:
    assert 'curl -fsSL -o /tmp/kotlin-lsp.tgz "https://download.jetbrains.com/' in _block("kotlin-lsp")


@pytest.mark.parametrize("field", ["runtime", "nixos"])
def test_a_language_server_field_naming_a_command_or_package_must_be_a_string(field: str) -> None:
    data = copy.deepcopy(render_registry.load_registry())
    data["language_servers"][0][field] = ["java"]
    with pytest.raises(render_registry.RegistryError, match=field):
        render_registry.validate(data)
