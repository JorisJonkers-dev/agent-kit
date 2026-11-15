"""The projects layout (docs/adr/0009-projects-layout-guard.md): the guard shim and the written rule.

The guard's decisions are tested in the Rust crate (scripts/crates/projects-layout-guard);
here is the shim the agents run, `/bin/sh projects-layout-guard.sh`, and the install.
"""

from __future__ import annotations

import os
import subprocess
import sys
from pathlib import Path

import pytest
from setup_harness import Run, setup_run

KIT_ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(KIT_ROOT / "scripts"))

import render_registry  # noqa: E402

GUARD = KIT_ROOT / "skills" / "workstation" / "projects-layout" / "hooks" / "projects-layout-guard.sh"


def _shim(env: dict[str, str], payload: str = "{}") -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        ["/bin/sh", str(GUARD)], input=payload, capture_output=True, text=True, check=False,
        env={"PATH": os.environ["PATH"], **env},
    )


def test_the_shim_runs_the_installed_guard_with_the_payload_on_stdin(tmp_path: Path) -> None:
    guard = tmp_path / "projects-layout-guard"
    guard.write_text("#!/bin/sh\necho \"guard saw: $(cat)\"\n")
    guard.chmod(0o755)
    result = _shim({"HOME": str(tmp_path / "home"), "WORKSPACE_TOOLS_BIN_DIR": str(tmp_path)}, "payload")
    assert (result.returncode, result.stdout) == (0, "guard saw: payload\n")


def test_the_shim_defaults_to_the_local_bin_of_home(tmp_path: Path) -> None:
    bin_dir = tmp_path / ".local" / "bin"
    bin_dir.mkdir(parents=True)
    guard = bin_dir / "projects-layout-guard"
    guard.write_text("#!/bin/sh\necho from-home\n")
    guard.chmod(0o755)
    assert _shim({"HOME": str(tmp_path)}).stdout == "from-home\n"


def test_the_shim_allows_the_call_when_the_guard_is_not_installed(tmp_path: Path) -> None:
    result = _shim({"HOME": str(tmp_path)})
    assert (result.returncode, result.stdout, result.stderr) == (0, "", "")
    plain = tmp_path / "projects-layout-guard"
    plain.write_text("#!/bin/sh\necho nope\n")  # not executable
    result = _shim({"HOME": str(tmp_path), "WORKSPACE_TOOLS_BIN_DIR": str(tmp_path)})
    assert (result.returncode, result.stdout) == (0, "")


# ---------------------------------------------------------------------------
# The written rule, in each client's global instruction file
# ---------------------------------------------------------------------------

BEGIN = "<!-- agent-kit projects-layout begin -->"


@pytest.fixture(scope="module")
def fresh(tmp_path_factory: pytest.TempPathFactory) -> Run:
    return setup_run(tmp_path_factory.mktemp("layout-fresh"), "Darwin")


@pytest.mark.parametrize("file", [".claude/CLAUDE.md", ".codex/AGENTS.md"])
def test_setup_writes_the_rule_into_each_global_instruction_file(fresh: Run, file: str) -> None:
    text = (fresh.home / file).read_text()
    assert text.count(BEGIN) == 1
    assert "`~/Projects/<owner>/worktrees/<repo>/<name>`" in text


def test_setup_installs_the_skill_with_its_guard(fresh: Run) -> None:
    for agent in (".claude", ".codex"):
        script = fresh.home / agent / "skills" / "projects-layout" / "hooks" / "projects-layout-guard.sh"
        assert script.read_bytes() == GUARD.read_bytes()


def test_the_rule_is_not_written_in_a_cloud_session() -> None:
    script = render_registry.render_setup_script(render_registry.load_registry())
    step = script[script.index("# The projects layout: its rule") :]
    assert step.index('if [ "${CLOUD}" != 1 ]; then') < step.index('layout_pointer "${CLAUDE_HOME}/CLAUDE.md"')
