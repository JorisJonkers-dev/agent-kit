"""The model gateway's reach: the `model-gateway` command on PATH, and the
marked pointer block each client's global instruction file carries.

The setup assertions run the checked-in setup script against a temp HOME with
every external command stubbed (tests/setup_harness.py).
"""

from __future__ import annotations

import importlib.util
import re
import subprocess
import sys
from pathlib import Path

import pytest
import yaml

KIT_ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(KIT_ROOT / "scripts"))
sys.path.insert(0, str(KIT_ROOT / "tests"))

from setup_harness import Run, setup_run  # noqa: E402

COMMAND = KIT_ROOT / "installer" / "model-gateway"
PUBLISH = KIT_ROOT / "scripts" / "publish-installer-artifacts.sh"
BEGIN = "<!-- agent-kit model-gateway begin -->"
END = "<!-- agent-kit model-gateway end -->"
# Every global instruction file that carries the pointer, relative to HOME.
POINTERS = (
    ".claude/CLAUDE.md",
    ".codex/AGENTS.md",
    ".vibe/AGENTS.md",
    ".vibe-private/AGENTS.md",
    ".gemini/config/AGENTS.md",
    ".hermes/SOUL.md",
    ".config/opencode/AGENTS.md",
)


def _registry() -> dict:
    return yaml.safe_load((KIT_ROOT / "registry" / "estate-tooling.yaml").read_text())


def _render_registry():
    spec = importlib.util.spec_from_file_location("render_registry", KIT_ROOT / "scripts" / "render_registry.py")
    assert spec and spec.loader
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def _block(body: str) -> str:
    return f"{BEGIN}\n{body}\n{END}\n"


@pytest.fixture(scope="module")
def macos_run(tmp_path_factory: pytest.TempPathFactory) -> Run:
    return setup_run(tmp_path_factory.mktemp("model-gateway-reach-macos"), "Darwin")


# ---------------------------------------------------------------------------
# The command
# ---------------------------------------------------------------------------


def test_the_command_is_a_kit_command_that_runs_the_rust_gateway() -> None:
    text = COMMAND.read_text()
    assert COMMAND.stat().st_mode & 0o777 == 0o755
    assert text.splitlines()[0] == "#!/usr/bin/env bash"
    assert "# managed by agent-kit setup-workstation.sh" in text.splitlines()
    assert "node" not in text.replace("# managed by", "")


def test_setup_puts_the_command_on_path(macos_run: Run) -> None:
    dest = macos_run.home / ".local" / "bin" / "model-gateway"
    assert dest.read_bytes() == COMMAND.read_bytes()
    assert dest.stat().st_mode & 0o777 == 0o755
    assert f"ok    model-gateway -> {dest}" in macos_run.stdout


def test_the_installed_command_runs_the_rust_gateway_binary(macos_run: Run, tmp_path: Path) -> None:
    binary = tmp_path / "gateway"
    binary.write_text('#!/usr/bin/env bash\necho "rust $*"\n')
    binary.chmod(0o755)

    result = subprocess.run(
        [str(macos_run.home / ".local" / "bin" / "model-gateway"), "models", "--json", "{}"],
        capture_output=True,
        text=True,
        check=False,
        env={"HOME": str(macos_run.home), "PATH": "/usr/bin:/bin", "AGENT_KIT_MODEL_GATEWAY_BIN": str(binary)},
    )

    assert result.returncode == 0, result.stderr
    assert result.stdout == "rust models --json {}\n"


def test_retiring_the_gateway_removes_the_command_and_its_pointers() -> None:
    render = _render_registry()
    data = _registry()
    data["local_skills"] = [s for s in data["local_skills"] if s["name"] != "model-gateway"]

    retired = render.render_setup_script(data)
    declared = render.render_setup_script(_registry())

    assert "install_kit_command model-gateway" in declared
    assert "install_kit_command model-gateway" not in retired
    assert "retire_kit_command model-gateway" in retired
    assert 'gateway_pointer "${CLAUDE_HOME}/CLAUDE.md" remove' in retired


def test_the_bundle_ships_the_command_and_the_crate_that_writes_its_block() -> None:
    listed = re.search(r"KIT_PATHS=\((.*?)\n\)", PUBLISH.read_text(), re.S)
    assert listed is not None
    paths = listed.group(1).split()
    assert "installer/model-gateway" in paths
    assert "scripts/crates" in paths


# ---------------------------------------------------------------------------
# The pointer in each client's global instruction file
# ---------------------------------------------------------------------------


def test_every_client_global_instruction_file_carries_one_pointer_block(macos_run: Run) -> None:
    for relative in POINTERS:
        text = (macos_run.home / relative).read_text()
        assert text.count(BEGIN) == 1, relative
        assert text.count(END) == 1, relative


def test_the_pointer_names_the_command_its_models_subcommand_and_the_delegate_skill(macos_run: Run) -> None:
    text = (macos_run.home / ".codex" / "AGENTS.md").read_text()
    body = text.split(BEGIN, 1)[1].split(END, 1)[0]
    assert "`model-gateway`" in body
    assert "`model-gateway models`" in body
    assert "`delegate` skill" in body
    assert 2 <= len([line for line in body.splitlines() if line.strip()]) <= 4


def test_the_pointer_for_opencode_honours_xdg_config_home(tmp_path: Path) -> None:
    xdg = tmp_path / "xdg"
    run = setup_run(tmp_path / "run", "Linux", extra_env={"XDG_CONFIG_HOME": str(xdg)})

    assert (xdg / "opencode" / "AGENTS.md").read_text().count(BEGIN) == 1
    assert not (run.home / ".config" / "opencode" / "AGENTS.md").exists()


def test_a_second_run_replaces_setups_block_and_keeps_the_text_around_it(tmp_path: Path) -> None:
    def prepare(home: Path) -> None:
        file = home / ".codex" / "AGENTS.md"
        file.parent.mkdir(parents=True)
        file.write_text("# My notes\n\nKeep this.\n\n" + _block("an old pointer") + "\nTail I wrote.\n")

    run = setup_run(tmp_path, "Darwin", prepare_home=prepare)

    text = (run.home / ".codex" / "AGENTS.md").read_text()
    assert text.startswith("# My notes\n\nKeep this.\n\n")
    # The gateway block is replaced where it stood; the projects-layout block
    # is the only text setup adds after the user's tail.
    assert text.split("<!-- agent-kit projects-layout begin -->")[0].endswith("\nTail I wrote.\n\n")
    assert text.count(BEGIN) == 1
    assert "an old pointer" not in text
    assert "`model-gateway models`" in text


def test_a_users_own_claude_instructions_keep_every_line_and_gain_only_the_block(tmp_path: Path) -> None:
    def prepare(home: Path) -> None:
        file = home / ".claude" / "CLAUDE.md"
        file.parent.mkdir(parents=True)
        file.write_text("Be terse.\n")

    run = setup_run(tmp_path, "Darwin", prepare_home=prepare)

    text = (run.home / ".claude" / "CLAUDE.md").read_text()
    assert text.startswith("Be terse.\n\n" + BEGIN)
    assert text.count(BEGIN) == 1


def test_check_only_writes_no_command_and_no_pointer(tmp_path: Path) -> None:
    run = setup_run(tmp_path, "Darwin", args=("--check",))

    assert not (run.home / ".local" / "bin" / "model-gateway").exists()
    assert not (run.home / ".claude" / "CLAUDE.md").exists()


# ---------------------------------------------------------------------------
# The block helper
# ---------------------------------------------------------------------------