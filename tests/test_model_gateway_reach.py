"""The model gateway's reach: the `model-gateway` command on PATH, and the
marked pointer block each client's global instruction file carries.

The setup assertions run the checked-in setup script against a temp HOME with
every external command stubbed (tests/setup_harness.py). The block helper is
run directly on temp files.
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
HELPER = KIT_ROOT / "scripts" / "instruction-block.py"
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


def _helper(
    path: Path, name: str = "model-gateway", *, remove: bool = False, body: str = ""
) -> subprocess.CompletedProcess:
    args = [sys.executable, str(HELPER), str(path), name]
    if remove:
        args.append("--remove")
    return subprocess.run(args, input=body, capture_output=True, text=True, check=False)


def _block(body: str) -> str:
    return f"{BEGIN}\n{body}\n{END}\n"


@pytest.fixture(scope="module")
def macos_run(tmp_path_factory: pytest.TempPathFactory) -> Run:
    return setup_run(tmp_path_factory.mktemp("model-gateway-reach-macos"), "Darwin")


# ---------------------------------------------------------------------------
# The command
# ---------------------------------------------------------------------------


def test_the_command_is_a_kit_command_that_runs_the_gateway_with_node() -> None:
    text = COMMAND.read_text()
    assert COMMAND.stat().st_mode & 0o777 == 0o755
    assert text.splitlines()[0] == "#!/usr/bin/env bash"
    assert "# managed by agent-kit setup-workstation.sh" in text.splitlines()
    assert re.search(r"^exec node \"[^\"]*skills/model-gateway/model-gateway\.mjs\" \"\$@\"$", text, re.M)


def test_setup_puts_the_command_on_path(macos_run: Run) -> None:
    dest = macos_run.home / ".local" / "bin" / "model-gateway"
    assert dest.read_bytes() == COMMAND.read_bytes()
    assert dest.stat().st_mode & 0o777 == 0o755
    assert f"ok    model-gateway -> {dest}" in macos_run.stdout


def test_the_installed_command_runs_the_claude_copy_of_the_gateway_with_node(macos_run: Run, tmp_path: Path) -> None:
    bin_dir = tmp_path / "bin"
    bin_dir.mkdir()
    node = bin_dir / "node"
    node.write_text('#!/usr/bin/env bash\necho "node $*"\n')
    node.chmod(0o755)

    result = subprocess.run(
        [str(macos_run.home / ".local" / "bin" / "model-gateway"), "models", "--json", "{}"],
        capture_output=True,
        text=True,
        check=False,
        env={"HOME": str(macos_run.home), "PATH": f"{bin_dir}:/usr/bin:/bin"},
    )

    assert result.returncode == 0, result.stderr
    assert result.stdout == f"node {macos_run.home}/.claude/skills/model-gateway/model-gateway.mjs models --json {{}}\n"


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


def test_the_bundle_ships_the_command_and_the_block_helper_it_needs() -> None:
    listed = re.search(r"KIT_PATHS=\((.*?)\n\)", PUBLISH.read_text(), re.S)
    assert listed is not None
    paths = listed.group(1).split()
    assert "installer/model-gateway" in paths
    assert "scripts/instruction-block.py" in paths


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
    assert text.endswith("\nTail I wrote.\n")
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


def test_the_helper_appends_its_block_to_a_missing_file(tmp_path: Path) -> None:
    file = tmp_path / "AGENTS.md"

    result = _helper(file, body="Pointer line.")

    assert result.returncode == 0, result.stderr
    assert result.stdout.strip() == "changed"
    assert file.read_text() == _block("Pointer line.")


def test_the_helper_is_unchanged_on_a_second_run_with_the_same_body(tmp_path: Path) -> None:
    file = tmp_path / "AGENTS.md"
    _helper(file, body="Pointer line.")

    result = _helper(file, body="Pointer line.")

    assert result.stdout.strip() == "unchanged"


def test_the_helper_replaces_its_block_in_place_and_keeps_the_text_around_it(tmp_path: Path) -> None:
    file = tmp_path / "AGENTS.md"
    file.write_text("Before.\n\n" + _block("old") + "After.\n")

    result = _helper(file, body="new")

    assert result.returncode == 0, result.stderr
    assert file.read_text() == "Before.\n\n" + _block("new") + "After.\n"


def test_the_helper_leaves_a_file_with_an_unclosed_block_alone(tmp_path: Path) -> None:
    file = tmp_path / "AGENTS.md"
    original = f"Mine.\n{BEGIN}\nhalf a block\n"
    file.write_text(original)

    result = _helper(file, body="new")

    assert result.returncode == 2
    assert "model-gateway" in result.stderr
    assert file.read_text() == original


def test_the_helper_removes_only_its_block(tmp_path: Path) -> None:
    file = tmp_path / "AGENTS.md"
    file.write_text("Before.\n\n" + _block("pointer") + "After.\n")

    result = _helper(file, remove=True)

    assert result.returncode == 0, result.stderr
    assert file.read_text() == "Before.\n\nAfter.\n"


def test_the_helper_removing_from_a_file_without_its_block_changes_nothing(tmp_path: Path) -> None:
    file = tmp_path / "AGENTS.md"
    file.write_text("Mine.\n")

    result = _helper(file, remove=True)

    assert result.stdout.strip() == "unchanged"
    assert file.read_text() == "Mine.\n"


def test_the_helper_keeps_other_named_blocks_in_the_same_file(tmp_path: Path) -> None:
    file = tmp_path / "AGENTS.md"
    other = "<!-- agent-kit other begin -->\nother\n<!-- agent-kit other end -->\n"
    file.write_text(other)

    _helper(file, body="pointer")

    assert file.read_text().startswith(other)


def test_the_helper_keeps_a_crlf_file_byte_for_byte_around_its_block(tmp_path: Path) -> None:
    file = tmp_path / "AGENTS.md"
    file.write_bytes(b"Mine.\r\n\r\n" + _block("old").replace("\n", "\r\n").encode() + b"Tail.\r\n")

    result = _helper(file, body="new")

    assert result.returncode == 0, result.stderr
    assert file.read_bytes() == b"Mine.\r\n\r\n" + _block("new").replace("\n", "\r\n").encode() + b"Tail.\r\n"
