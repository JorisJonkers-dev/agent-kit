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
# The helpers resolve a path with readlink -f, which a stub PATH does not hold
# and macOS's own readlink does not implement.
READLINK = shutil.which("greadlink") or shutil.which("readlink") or "/usr/bin/readlink"

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


def _nix_helpers() -> str:
    """The two Nix helpers, cut out of the rendered script the way _block cuts a server."""
    script = SETUP.read_text()
    start = script.index("nix_provides() {")
    return script[start : script.index("\n}\n", script.index("non_nix_copy() {")) + 3]


def _run(
    tmp_path: Path,
    plugin: str,
    *,
    on_path: tuple[str, ...] = (),
    nixos: bool = False,
    in_store: tuple[str, ...] = (),
) -> list[str]:
    """Run one server's block with stub binaries.

    `on_path` are plain copies, first on PATH. `in_store` are copies Nix
    provides: the stub sits in a fake store and PATH holds a symlink to it,
    after the plain copies, so a test can put one in front of the other.
    """
    # Resolved, so the store prefix matches what `readlink -f` prints even
    # where the temporary directory is itself reached through a symlink.
    root = tmp_path.resolve()
    bin_dir = root / "bin"
    bin_dir.mkdir()
    nix_bin = root / "nix-bin"
    nix_bin.mkdir()
    store = root / "store"
    store.mkdir()
    for name in on_path:
        (bin_dir / name).write_text("#!/bin/sh\n")
        (bin_dir / name).chmod(0o755)
    for name in in_store:
        (store / name).write_text("#!/bin/sh\n")
        (store / name).chmod(0o755)
        (nix_bin / name).symlink_to(store / name)
    # A shim keeps the PATH hermetic: no real copy of the binary under test
    # can leak in from the machine running the tests.
    (bin_dir / "readlink").write_text(f'#!/bin/sh\nexec "{READLINK}" "$@"\n')
    (bin_dir / "readlink").chmod(0o755)
    marker = tmp_path / "NIXOS"
    if nixos:
        marker.write_text("")
    log = tmp_path / "log"
    body = (
        PRELUDE
        + _nix_helpers()
        + _block(plugin)
        + '\nprintf "missing %s\\n" "${missing_lsp_binaries[@]}" >> "$LOG"\n'
    )
    subprocess.run(
        [BASH, "-c", body],
        check=True,
        env={
            "PATH": f"{bin_dir}:{nix_bin}",
            "LOG": str(log),
            "AK_NIXOS_MARKER": str(marker),
            "AK_NIX_STORE": str(store),
        },
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


def test_on_nixos_a_binary_from_the_store_is_left_to_nix(tmp_path: Path) -> None:
    """Nix provides typescript-language-server, so setup installs nothing and warns about nothing."""
    lines = _run(tmp_path, "typescript-lsp", nixos=True, in_store=("typescript-language-server",))
    assert "run claude plugin enable typescript-lsp@agent-kit-official" in lines
    assert not any(line.startswith("run_sh") for line in lines)
    assert not any("not from Nix" in line for line in lines)


def test_on_nixos_a_binary_outside_the_store_is_reported_as_not_from_nix(tmp_path: Path) -> None:
    """A copy agent-kit installed itself is not what Nix provides, and setup says so."""
    lines = _run(tmp_path, "typescript-lsp", nixos=True, on_path=("typescript-language-server",))
    assert "run claude plugin enable typescript-lsp@agent-kit-official" in lines
    assert any(line.startswith("warn") and "is not from Nix" in line for line in lines)


def test_on_nixos_a_copy_in_front_of_the_store_copy_is_reported_as_shadowing(tmp_path: Path) -> None:
    lines = _run(
        tmp_path,
        "typescript-lsp",
        nixos=True,
        on_path=("typescript-language-server",),
        in_store=("typescript-language-server",),
    )
    assert any(line.startswith("warn") and "can shadow" in line for line in lines)


def test_off_nixos_a_binary_outside_the_store_is_not_reported(tmp_path: Path) -> None:
    """Off NixOS nothing changes: the binary is simply on PATH."""
    lines = _run(tmp_path, "typescript-lsp", on_path=("typescript-language-server",))
    assert "run claude plugin enable typescript-lsp@agent-kit-official" in lines
    assert not any("not from Nix" in line for line in lines)


def test_the_uv_step_is_guarded_by_nix_provides() -> None:
    """NixOS gets uv from Nix, so setup reports it instead of installing or updating it."""
    script = render_registry.render_setup_script(render_registry.load_registry())
    start = script.index("# Python tool/venv manager")
    step = script[start : script.index("\nreport_antigravity_signin", start)]
    assert "if nix_provides uv; then" in step
    assert 'ok "uv: provided by Nix; setup does not install or update it"' in step
    # The install and update commands are still there, for every other machine.
    assert "uv self update" in step
    assert "astral.sh/uv/install.sh" in step


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
