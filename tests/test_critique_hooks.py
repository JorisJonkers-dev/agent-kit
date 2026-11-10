"""The three critique hooks of ADR-0002, installed by setup (agent-kit#198).

Setup merges them by name into Claude Code's settings.json and Codex's
hooks.json, keeps the operator's own hooks and the retired-hook purge beside
them, and removes them on --uninstall. Every run here uses a temp HOME with
every external command stubbed (tests/setup_harness.py), so no real agent home
is touched. The last tests run the hook scripts themselves against a real git
repository, with the bundled runner.
"""

from __future__ import annotations

import importlib.util
import json
import os
import shutil
import subprocess
import sys
import time
from pathlib import Path

import pytest

KIT_ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(KIT_ROOT / "scripts"))

import render_registry  # noqa: E402
from setup_harness import Run, setup_run  # noqa: E402

GUARD = "projects-layout-guard"
HOOKS = ("critique-commit-gate", "critique-pr-gate", "critique-session-start", GUARD)
EVENTS = {
    "critique-commit-gate": "PreToolUse",
    "critique-pr-gate": "PreToolUse",
    "critique-session-start": "SessionStart",
    GUARD: "PreToolUse",
}
# The skill each hook's script lives in (ADR-0002, ADR-0009).
SKILLS = {name: "critique" for name in HOOKS} | {GUARD: "projects-layout"}

OWN_HOOK = {"matcher": "Bash", "hooks": [{"type": "command", "command": "node /opt/own/audit-hook.js"}]}
CODING_AGENTS = {"hooks": [{"type": "command", "command": 'node "/h/.hindsight/coding-agents/dist/codex-hook.js"'}]}


def _retired(home: Path) -> dict:
    return {"hooks": [{"type": "command", "command": f"{home}/.claude/hooks/pre-tool-use-edit-recall.sh"}]}


def _group(agent_home: Path, name: str) -> dict:
    """The one matcher group setup wires per hook: sh running the script in the agent home's skill for it."""
    script = agent_home / "skills" / SKILLS[name] / "hooks" / f"{name}.sh"
    matcher = "Bash|EnterWorktree" if name == GUARD and agent_home.name == ".claude" else "Bash"
    group: dict = {"matcher": matcher} if EVENTS[name] == "PreToolUse" else {}
    group["hooks"] = [{"type": "command", "command": f'/bin/sh "{script}"', "timeout": 5}]
    return group


def _hooks(path: Path) -> dict:
    return json.loads(path.read_text()).get("hooks", {})


def _named(hooks: dict, name: str) -> list[tuple[str, int, dict]]:
    return [
        (event, index, group)
        for event, groups in hooks.items()
        for index, group in enumerate(groups)
        if any(f"{name}.sh" in handler.get("command", "") for handler in group.get("hooks", []))
    ]


def _write(path: Path, data: dict) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(data, indent=2) + "\n")


@pytest.fixture(scope="module")
def fresh(tmp_path_factory: pytest.TempPathFactory) -> Run:
    return setup_run(tmp_path_factory.mktemp("hooks-fresh"), "Darwin")


def _lived_in(home: Path) -> None:
    """A machine with the operator's own hooks, a retired knowledge hook, and Coding Agents' Codex hook."""
    _write(
        home / ".claude" / "settings.json",
        {
            "model": "opus",
            "hooks": {
                "PreToolUse": [OWN_HOOK, _retired(home)],
                "Stop": [{"hooks": [{"type": "command", "command": "say done"}]}],
            },
        },
    )
    _write(home / ".codex" / "hooks.json", {"hooks": {"PreToolUse": [OWN_HOOK], "UserPromptSubmit": [CODING_AGENTS]}})


@pytest.fixture(scope="module")
def lived_in(tmp_path_factory: pytest.TempPathFactory) -> Run:
    return setup_run(tmp_path_factory.mktemp("hooks-lived-in"), "Darwin", prepare_home=_lived_in)


def test_the_renderer_installs_exactly_the_hooks_the_validator_allows() -> None:
    spec = importlib.util.spec_from_file_location("validate_manifest", KIT_ROOT / "scripts" / "validate_manifest.py")
    assert spec is not None and spec.loader is not None
    validator = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(validator)
    hooks = render_registry.critique_hooks()
    assert {h["name"]: h["event"] for h in hooks} == validator.ALLOWED_HOOK_EVENTS
    assert all(set(h["agents"]) == {"claude", "codex"} for h in hooks)


def test_a_fresh_machine_gets_the_four_hooks_in_claude_code_and_codex(fresh: Run) -> None:
    assert "FAIL  critique hooks" not in fresh.stderr, fresh.stderr
    for agent, file in ((".claude", "settings.json"), (".codex", "hooks.json")):
        home = fresh.home / agent
        hooks = _hooks(home / file)
        assert hooks == {
            "PreToolUse": [
                _group(home, "critique-commit-gate"),
                _group(home, "critique-pr-gate"),
                _group(home, GUARD),
            ],
            "SessionStart": [_group(home, "critique-session-start")],
        }, agent
        for name in HOOKS:
            script = home / "skills" / SKILLS[name] / "hooks" / f"{name}.sh"
            source = KIT_ROOT / "skills" / "workstation" / SKILLS[name] / "hooks" / f"{name}.sh"
            assert script.read_bytes() == source.read_bytes()
    assert "critique hooks installed in Claude Code" in fresh.stdout
    assert "critique hooks installed in Codex" in fresh.stdout


def test_the_operators_own_hooks_survive_and_the_retired_purge_still_runs_beside_the_new_hooks(lived_in: Run) -> None:
    assert "FAIL  critique hooks" not in lived_in.stderr, lived_in.stderr
    claude = lived_in.home / ".claude"
    settings = json.loads((claude / "settings.json").read_text())
    assert settings["model"] == "opus"
    assert settings["hooks"] == {
        "PreToolUse": [
            OWN_HOOK,
            _group(claude, "critique-commit-gate"),
            _group(claude, "critique-pr-gate"),
            _group(claude, GUARD),
        ],
        "Stop": [{"hooks": [{"type": "command", "command": "say done"}]}],
        "SessionStart": [_group(claude, "critique-session-start")],
    }
    codex = lived_in.home / ".codex"
    assert _hooks(codex / "hooks.json") == {
        "PreToolUse": [
            OWN_HOOK,
            _group(codex, "critique-commit-gate"),
            _group(codex, "critique-pr-gate"),
            _group(codex, GUARD),
        ],
        "UserPromptSubmit": [CODING_AGENTS],
        "SessionStart": [_group(codex, "critique-session-start")],
    }


def test_a_second_run_leaves_the_hook_files_byte_for_byte_as_they_were(tmp_path: Path) -> None:
    """Codex trusts a hook by its definition, so the definition must not move between runs."""
    first = setup_run(tmp_path / "first", "Darwin", prepare_home=_lived_in)
    before = {f: (first.home / f).read_bytes() for f in (".claude/settings.json", ".codex/hooks.json")}

    def keep(home: Path) -> None:
        for name, data in before.items():
            (home / name).parent.mkdir(parents=True, exist_ok=True)
            (home / name).write_bytes(data.replace(str(first.home).encode(), str(home).encode()))

    second = setup_run(tmp_path / "second", "Darwin", prepare_home=keep)

    for name, data in before.items():
        assert (second.home / name).read_bytes() == data.replace(str(first.home).encode(), str(second.home).encode()), (
            name
        )
    assert "critique hooks already installed in Codex" in second.stdout


def test_a_hook_that_moved_is_replaced_in_place_so_codex_keeps_its_position(tmp_path: Path) -> None:
    """Codex keys a hook's trust by its position: an existing critique group is rewritten where it stands."""

    def stale(home: Path) -> None:
        old = {
            "matcher": "Bash",
            "hooks": [{"type": "command", "command": "/old/kit/hooks/critique-pr-gate.sh", "timeout": 30}],
        }
        _write(home / ".codex" / "hooks.json", {"hooks": {"PreToolUse": [old, OWN_HOOK]}})

    run = setup_run(tmp_path, "Darwin", prepare_home=stale)

    codex = run.home / ".codex"
    assert _hooks(codex / "hooks.json")["PreToolUse"] == [
        _group(codex, "critique-pr-gate"),
        OWN_HOOK,
        _group(codex, "critique-commit-gate"),
        _group(codex, GUARD),
    ]


def test_a_critique_handler_sharing_a_group_with_the_operators_is_split_out(tmp_path: Path) -> None:
    def shared(home: Path) -> None:
        mixed = {
            "matcher": "Bash",
            "hooks": [OWN_HOOK["hooks"][0], {"type": "command", "command": "sh critique-commit-gate.sh"}],
        }
        _write(home / ".claude" / "settings.json", {"hooks": {"PreToolUse": [mixed]}})

    run = setup_run(tmp_path, "Darwin", prepare_home=shared)

    claude = run.home / ".claude"
    assert _hooks(claude / "settings.json")["PreToolUse"] == [
        OWN_HOOK,
        _group(claude, "critique-commit-gate"),
        _group(claude, "critique-pr-gate"),
        _group(claude, GUARD),
    ]


def test_check_mode_reports_and_changes_nothing(tmp_path: Path) -> None:
    run = setup_run(tmp_path, "Darwin", args=("--check",), prepare_home=_lived_in)

    assert _hooks(run.home / ".codex" / "hooks.json") == {"PreToolUse": [OWN_HOOK], "UserPromptSubmit": [CODING_AGENTS]}
    assert "would install the critique hooks in Codex" in run.stdout


def test_a_hook_file_that_does_not_parse_is_left_untouched_with_a_warning(tmp_path: Path) -> None:
    def broken(home: Path) -> None:
        (home / ".codex").mkdir(parents=True)
        (home / ".codex" / "hooks.json").write_text("{not json")

    run = setup_run(tmp_path, "Darwin", prepare_home=broken)

    assert (run.home / ".codex" / "hooks.json").read_text() == "{not json"
    assert "critique hooks: " + str(run.home / ".codex" / "hooks.json") + " left untouched" in run.stderr


def test_a_profile_settings_symlink_stays_a_symlink(lived_in: Run) -> None:
    """The personal profile shares settings.json by symlink; the merge writes through it."""
    personal = lived_in.home / ".claude-personal" / "settings.json"
    assert personal.is_symlink()
    assert _named(_hooks(personal), "critique-commit-gate") == _named(
        _hooks(lived_in.home / ".claude" / "settings.json"), "critique-commit-gate"
    )
    assert len(_named(_hooks(personal), "critique-commit-gate")) == 1


def test_uninstall_removes_the_hooks_and_keeps_the_operators(tmp_path: Path) -> None:
    installed = setup_run(tmp_path / "installed", "Darwin", prepare_home=_lived_in)
    claude_before = (installed.home / ".claude" / "settings.json").read_text()
    codex_before = (installed.home / ".codex" / "hooks.json").read_text()

    def from_installed(home: Path) -> None:
        for name, text in ((".claude/settings.json", claude_before), (".codex/hooks.json", codex_before)):
            (home / name).parent.mkdir(parents=True, exist_ok=True)
            (home / name).write_text(text.replace(str(installed.home), str(home)))
        # A retired hook a machine still has, which the purge beside the removal takes out.
        data = json.loads((home / ".claude/settings.json").read_text())
        data["hooks"]["PreToolUse"].append(_retired(home))
        (home / ".claude/settings.json").write_text(json.dumps(data))

    run = setup_run(tmp_path / "uninstalled", "Darwin", args=("--uninstall",), prepare_home=from_installed)

    assert "FAIL  critique hooks" not in run.stderr, run.stderr
    assert _hooks(run.home / ".claude" / "settings.json") == {
        "PreToolUse": [OWN_HOOK],
        "Stop": [{"hooks": [{"type": "command", "command": "say done"}]}],
    }
    assert _hooks(run.home / ".codex" / "hooks.json") == {"PreToolUse": [OWN_HOOK], "UserPromptSubmit": [CODING_AGENTS]}
    assert "critique hooks removed from Codex" in run.stdout


def test_uninstall_drops_a_hooks_section_left_empty(tmp_path: Path) -> None:
    def only_critique(home: Path) -> None:
        codex = home / ".codex"
        _write(codex / "hooks.json", {"hooks": {"SessionStart": [_group(codex, "critique-session-start")]}})

    run = setup_run(tmp_path, "Darwin", args=("--uninstall",), prepare_home=only_critique)

    assert json.loads((run.home / ".codex" / "hooks.json").read_text()) == {}


def test_setup_fails_rather_than_wiring_a_hook_to_a_missing_script() -> None:
    script = render_registry.render_setup_script(render_registry.load_registry())
    assert 'fail "critique hooks: ${home}/skills/${name}.sh is missing; not wired into ${path}"' in script
    # The scripts are copied in step 7, the hooks merged in step 9.
    assert script.index('cp -R "${src}" "${dest}"') < script.index("\ncritique_hooks install\n")


# ---------------------------------------------------------------------------
# The hook scripts themselves, against a real repository and the bundled runner
# ---------------------------------------------------------------------------

SCRIPTS = KIT_ROOT / "skills" / "workstation" / "critique" / "hooks"
NODE = shutil.which("node")


def _git(repo: Path, *args: str) -> None:
    env = {
        **os.environ,
        "GIT_AUTHOR_NAME": "t",
        "GIT_AUTHOR_EMAIL": "t@example.com",
        "GIT_COMMITTER_NAME": "t",
        "GIT_COMMITTER_EMAIL": "t@example.com",
        "GIT_CONFIG_GLOBAL": "/dev/null",
    }
    subprocess.run(["git", "-C", str(repo), *args], check=True, capture_output=True, env=env)


@pytest.fixture
def repo(tmp_path: Path) -> tuple[Path, Path]:
    """An opted-in repository (code allowed) with a risky change staged, and the HOME whose allowlist opts it in."""
    home = tmp_path / "home"
    allowlist = home / ".config" / "agent-kit" / "critique-allowlist.json"
    allowlist.parent.mkdir(parents=True)
    entry = {"families": ["mistral"], "code": True, "updated_at": "2026-11-01T00:00:00Z"}
    allowlist.write_text(json.dumps({"version": 1, "repositories": {"github.com/example/widgets": entry}}))
    repo = tmp_path / "widgets"
    repo.mkdir()
    _git(repo, "init", "-q", "-b", "main")
    _git(repo, "remote", "add", "origin", "git@github.com:example/widgets.git")
    (repo / "README.md").write_text("# Widgets\n")
    _git(repo, "add", "-A")
    _git(repo, "commit", "-q", "-m", "base")
    (repo / "src" / "auth").mkdir(parents=True)
    (repo / "src" / "auth" / "session.ts").write_text("export const ttl = 60\n")
    _git(repo, "add", "-A")
    flows = repo / ".git" / "critique" / "flows"
    flows.mkdir(parents=True)
    (flows / "widgets.json").write_text(
        json.dumps({"phase": "implementation", "repository": "github.com/example/widgets"}),
    )
    return repo, home


def _hook(name: str, payload: dict, home: Path, **env: str) -> tuple[subprocess.CompletedProcess[str], float]:
    started = time.monotonic()
    result = subprocess.run(
        ["/bin/sh", str(SCRIPTS / f"{name}.sh")],
        input=json.dumps(payload),
        capture_output=True,
        text=True,
        check=False,
        env={"HOME": str(home), "PATH": os.environ["PATH"], "GIT_CONFIG_GLOBAL": "/dev/null", **env},
    )
    return result, time.monotonic() - started


def _bash(repo: Path, command: str) -> dict:
    return {"cwd": str(repo), "hook_event_name": "PreToolUse", "tool_name": "Bash", "tool_input": {"command": command}}


@pytest.mark.skipif(NODE is None, reason="node is not on PATH")
def test_the_commit_gate_script_blocks_an_unreviewed_risky_commit_with_the_deny_and_exit_0(
    repo: tuple[Path, Path],
) -> None:
    path, home = repo
    result, took = _hook("critique-commit-gate", _bash(path, 'git commit -m "Expire sessions"'), home)

    assert result.returncode == 0, result.stderr
    assert json.loads(result.stdout) == {
        "hookSpecificOutput": {
            "hookEventName": "PreToolUse",
            "permissionDecision": "deny",
            "permissionDecisionReason": "Commit blocked: the staged change is risky "
            "(src/auth/session.ts is a risk path) and no passing /review-implementation run matches it. "
            "Run /review-implementation, then commit.",
        },
    }
    assert took < 1.0, f"the commit gate took {took:.2f}s"


@pytest.mark.skipif(NODE is None, reason="node is not on PATH")
def test_the_gate_scripts_print_nothing_when_they_allow(repo: tuple[Path, Path]) -> None:
    path, home = repo
    commit_on_pr, _ = _hook("critique-commit-gate", _bash(path, "gh pr create"), home)
    pr_on_commit, _ = _hook("critique-pr-gate", _bash(path, "git commit -m x"), home)
    killed, _ = _hook("critique-commit-gate", _bash(path, "git commit -m x"), home, CRITIQUE_OFF="1")

    assert [(r.returncode, r.stdout) for r in (commit_on_pr, pr_on_commit, killed)] == [(0, ""), (0, ""), (0, "")]


@pytest.mark.skipif(NODE is None, reason="node is not on PATH")
def test_the_gate_scripts_allow_everything_when_the_repository_has_no_active_flow(repo: tuple[Path, Path]) -> None:
    path, home = repo
    (path / ".git" / "critique" / "flows" / "widgets.json").unlink()
    commit, _ = _hook("critique-commit-gate", _bash(path, "git commit -m x"), home)
    pr, _ = _hook("critique-pr-gate", _bash(path, "gh pr create --fill"), home)

    assert [(r.returncode, r.stdout) for r in (commit, pr)] == [(0, ""), (0, "")]


@pytest.mark.skipif(NODE is None, reason="node is not on PATH")
def test_the_pr_gate_script_blocks_a_pr_with_no_feature_review(repo: tuple[Path, Path]) -> None:
    path, home = repo
    result, _ = _hook("critique-pr-gate", _bash(path, "gh pr create --fill"), home)

    reason = json.loads(result.stdout)["hookSpecificOutput"]["permissionDecisionReason"]
    assert result.returncode == 0 and reason.startswith("PR blocked: no /review-pr run has recorded a gate for ")


@pytest.mark.skipif(NODE is None, reason="node is not on PATH")
def test_the_session_start_script_suggests_flow_setup_in_an_opted_in_repository_only(
    repo: tuple[Path, Path], tmp_path: Path
) -> None:
    path, home = repo
    start = {"hook_event_name": "SessionStart", "source": "startup"}
    opted, _ = _hook("critique-session-start", {**start, "cwd": str(path)}, home)
    elsewhere, _ = _hook("critique-session-start", {**start, "cwd": str(tmp_path)}, home)

    assert opted.returncode == 0
    assert opted.stdout.startswith("Critique: /flow setup has not checked this machine yet (")
    assert opted.stdout.endswith(" Run /flow setup.\n") and opted.stdout.count("\n") == 1
    assert (elsewhere.returncode, elsewhere.stdout) == (0, "")


def test_a_hook_script_with_no_node_fails_without_exit_2_so_neither_agent_blocks(tmp_path: Path) -> None:
    result = subprocess.run(
        ["/bin/sh", str(SCRIPTS / "critique-commit-gate.sh")],
        input="{}",
        capture_output=True,
        text=True,
        check=False,
        env={"HOME": str(tmp_path), "PATH": str(tmp_path)},
    )
    assert result.returncode not in (0, 2) and result.stdout == ""
