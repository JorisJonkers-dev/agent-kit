"""The projects layout (docs/adr/0009-projects-layout-guard.md): the guard hook and the written rule.

The guard runs as the agents run it, `/bin/sh projects-layout-guard.sh` with the
payload on stdin, and HOME in a temp dir that holds ~/Projects.
"""

from __future__ import annotations

import json
import os
import shutil
import subprocess
import sys
from pathlib import Path

import pytest
from setup_harness import Run, setup_run

KIT_ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(KIT_ROOT / "scripts"))

import render_registry  # noqa: E402

GUARD = KIT_ROOT / "skills" / "workstation" / "projects-layout" / "hooks" / "projects-layout-guard.sh"
GIT = shutil.which("git")


def _guard(home: Path, payload: dict, **env: str) -> str:
    result = subprocess.run(
        ["/bin/sh", str(GUARD)],
        input=json.dumps(payload),
        capture_output=True,
        text=True,
        check=True,
        env={"HOME": str(home), "PATH": os.environ["PATH"], **env},
    )
    return result.stdout


def _bash(cwd: Path, command: str) -> dict:
    return {"hook_event_name": "PreToolUse", "tool_name": "Bash", "cwd": str(cwd), "tool_input": {"command": command}}


def _denied(out: str) -> str | None:
    if not out.strip():
        return None
    decision = json.loads(out)["hookSpecificOutput"]
    assert decision["permissionDecision"] == "deny"
    return str(decision["permissionDecisionReason"])


@pytest.fixture
def home(tmp_path: Path) -> Path:
    (tmp_path / "Projects").mkdir()
    return tmp_path


def test_a_clone_into_the_owner_folder_is_allowed(home: Path) -> None:
    command = "git clone git@github.com:JorisJonkers-dev/agent-kit.git"
    out = _guard(home, _bash(home / "Projects" / "JorisJonkers-dev", command))
    assert _denied(out) is None


def test_a_clone_into_tmp_is_denied_with_the_path_it_must_use(home: Path) -> None:
    reason = _denied(_guard(home, _bash(Path("/tmp"), "git clone https://github.com/JorisJonkers-dev/agent-kit")))
    assert reason is not None
    assert str(home / "Projects" / "JorisJonkers-dev" / "agent-kit") in reason


def test_a_gh_clone_after_cd_into_the_owner_folder_is_allowed(home: Path) -> None:
    command = "cd ~/Projects/ESA-Blueshell && gh repo clone ESA-Blueshell/website"
    assert _denied(_guard(home, _bash(Path("/tmp"), command))) is None


def test_a_gh_clone_to_the_projects_root_is_denied(home: Path) -> None:
    assert _denied(_guard(home, _bash(Path("/tmp"), "gh repo clone acme/widget ~/Projects/widget"))) is not None


def test_a_clone_from_another_host_is_left_alone(home: Path) -> None:
    assert _denied(_guard(home, _bash(Path("/tmp"), "git clone https://gitlab.com/a/b"))) is None


def test_a_command_that_only_mentions_clone_is_left_alone(home: Path) -> None:
    assert _denied(_guard(home, _bash(Path("/tmp"), "echo 'git clone x' | grep clone"))) is None


@pytest.mark.skipif(GIT is None, reason="needs git")
def test_a_worktree_next_to_the_repository_is_denied(home: Path) -> None:
    repo = home / "Projects" / "acme" / "widget"
    subprocess.run(["git", "init", "-q", str(repo)], check=True)
    reason = _denied(_guard(home, _bash(repo, "git worktree add -b feat/x ../widget-x")))
    assert reason is not None
    assert str(home / "Projects" / "acme" / "worktrees" / "widget") in reason


@pytest.mark.skipif(GIT is None, reason="needs git")
def test_a_worktree_in_the_owners_worktrees_folder_is_allowed(home: Path) -> None:
    repo = home / "Projects" / "acme" / "widget"
    subprocess.run(["git", "init", "-q", str(repo)], check=True)
    command = "git worktree add ~/Projects/acme/worktrees/widget/feat-x -b feat/x"
    assert _denied(_guard(home, _bash(repo, command))) is None


@pytest.mark.skipif(GIT is None, reason="needs git")
def test_a_codex_command_array_through_bash_c_is_checked(home: Path) -> None:
    repo = home / "Projects" / "acme" / "widget"
    subprocess.run(["git", "init", "-q", str(repo)], check=True)
    payload = _bash(Path("/tmp"), "")
    payload["tool_input"]["command"] = ["bash", "-lc", f"git -C {repo} worktree add .worktrees/z"]
    assert _denied(_guard(home, payload)) is not None


def test_enter_worktree_with_a_name_is_denied(home: Path) -> None:
    payload = {
        "hook_event_name": "PreToolUse",
        "tool_name": "EnterWorktree",
        "cwd": "/tmp",
        "tool_input": {"name": "x"},
    }
    assert _denied(_guard(home, payload)) is not None


def test_enter_worktree_with_a_path_is_allowed(home: Path) -> None:
    payload = {
        "hook_event_name": "PreToolUse",
        "tool_name": "EnterWorktree",
        "cwd": "/tmp",
        "tool_input": {"path": str(home / "Projects" / "acme" / "worktrees" / "widget" / "x")},
    }
    assert _denied(_guard(home, payload)) is None


def test_the_kill_switch_turns_the_guard_off(home: Path) -> None:
    payload = _bash(Path("/tmp"), "git clone https://github.com/acme/widget")
    assert _guard(home, payload, PROJECTS_LAYOUT_OFF="1") == ""


def test_a_machine_without_projects_is_left_alone(tmp_path: Path) -> None:
    assert _guard(tmp_path, _bash(Path("/tmp"), "git clone https://github.com/acme/widget")) == ""


def test_a_payload_that_is_not_json_is_let_through(home: Path) -> None:
    result = subprocess.run(
        ["/bin/sh", str(GUARD)], input="{not json", capture_output=True, text=True, check=False,
        env={"HOME": str(home), "PATH": os.environ["PATH"]},
    )
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
        script = fresh.home / agent / "skills" / "projects-layout" / "hooks" / "projects-layout-guard.py"
        assert script.read_bytes() == (GUARD.parent / "projects-layout-guard.py").read_bytes()


def test_the_rule_is_not_written_in_a_cloud_session() -> None:
    script = render_registry.render_setup_script(render_registry.load_registry())
    step = script[script.index("# The projects layout: its rule") :]
    assert step.index('if [ "${CLOUD}" != 1 ]; then') < step.index('layout_pointer "${CLAUDE_HOME}/CLAUDE.md"')
