"""memory-private: keep a directory's Claude Code sessions away from the shared Hindsight bank.

The hindsight-memory plugin skips both retain paths (Stop and SessionEnd) when
HINDSIGHT_DISABLED is true (Hindsight Coding Agents), and Claude Code hands a project's
.claude/settings.local.json `env` block to hook processes. So marking a
directory private is one key in that file.
"""

from __future__ import annotations

import json
import os
import subprocess
import sys
from pathlib import Path

KIT_ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(KIT_ROOT / "scripts"))

import render_registry  # noqa: E402
import yaml  # noqa: E402

CMD = KIT_ROOT / "installer" / "memory-private"


# No global or system git config: a developer's own excludes must not make
# the git-ignore tests pass on their machine and fail on CI.
_EMPTY_HOME = Path(os.environ.get("TMPDIR", "/tmp")) / "memory-private-test-home"
_EMPTY_HOME.mkdir(exist_ok=True)
HERMETIC_GIT = {
    **os.environ,
    "GIT_CONFIG_GLOBAL": "/dev/null",
    "GIT_CONFIG_NOSYSTEM": "1",
    "HOME": str(_EMPTY_HOME),
    "XDG_CONFIG_HOME": str(_EMPTY_HOME / ".config"),
}


def _run(*args: str, cwd: Path | None = None) -> subprocess.CompletedProcess:
    return subprocess.run(
        ["bash", str(CMD), *args], capture_output=True, text=True, check=False, cwd=cwd, env=HERMETIC_GIT,
    )


def _settings(project: Path) -> dict:
    return json.loads((project / ".claude" / "settings.local.json").read_text())


def test_on_marks_the_directory_private(tmp_path: Path) -> None:
    result = _run("on", str(tmp_path))
    assert result.returncode == 0, result.stderr
    assert _settings(tmp_path)["env"]["HINDSIGHT_DISABLED"] == "true"


def test_on_keeps_the_existing_local_settings(tmp_path: Path) -> None:
    (tmp_path / ".claude").mkdir()
    (tmp_path / ".claude" / "settings.local.json").write_text(
        json.dumps({"permissions": {"allow": ["Bash(ls)"]}, "env": {"FOO": "1"}}),
    )
    assert _run("on", str(tmp_path)).returncode == 0
    data = _settings(tmp_path)
    assert data["permissions"] == {"allow": ["Bash(ls)"]}
    assert data["env"] == {"FOO": "1", "HINDSIGHT_DISABLED": "true"}


def test_off_restores_auto_retain_and_keeps_the_rest(tmp_path: Path) -> None:
    (tmp_path / ".claude").mkdir()
    (tmp_path / ".claude" / "settings.local.json").write_text(json.dumps({"env": {"FOO": "1"}}))
    assert _run("on", str(tmp_path)).returncode == 0
    assert _run("off", str(tmp_path)).returncode == 0
    assert _settings(tmp_path) == {"env": {"FOO": "1"}}


def test_off_drops_an_env_block_it_emptied(tmp_path: Path) -> None:
    assert _run("on", str(tmp_path)).returncode == 0
    assert _run("off", str(tmp_path)).returncode == 0
    assert "env" not in _settings(tmp_path)


def test_status_reports_both_states(tmp_path: Path) -> None:
    assert "not private" in _run("status", str(tmp_path)).stdout
    _run("on", str(tmp_path))
    out = _run("status", str(tmp_path)).stdout
    assert "private" in out and "not private" not in out


def test_directory_defaults_to_the_current_one(tmp_path: Path) -> None:
    assert _run("on", cwd=tmp_path).returncode == 0
    assert _settings(tmp_path)["env"]["HINDSIGHT_DISABLED"] == "true"


def test_unknown_command_fails_with_usage(tmp_path: Path) -> None:
    result = _run("maybe", str(tmp_path))
    assert result.returncode != 0
    assert "usage" in result.stderr.lower()


def test_setup_installs_the_command() -> None:
    data = yaml.safe_load((KIT_ROOT / "registry" / "estate-tooling.yaml").read_text())
    script = render_registry.render_setup_script(data)
    assert 'install_kit_command memory-private' in script


def test_the_bundle_ships_the_command() -> None:
    publish = (KIT_ROOT / "scripts" / "publish-installer-artifacts.sh").read_text()
    assert "installer/memory-private" in publish.split("KIT_PATHS=(")[1].split(")")[0].split()


def _install(tmp_path: Path) -> subprocess.CompletedProcess:
    data = yaml.safe_load((KIT_ROOT / "registry" / "estate-tooling.yaml").read_text())
    script = render_registry.render_setup_script(data)
    start = script.index("install_kit_command() {")
    fn = script[start : script.index("\n}\n", start) + 3]
    stubs = 'ok() { echo "ok $*"; }\nlog() { echo "log $*"; }\nwarn() { echo "warn $*"; }\nfail() { echo "FAIL $*"; }\n'
    body = f'CHECK_ONLY=0\nKIT_ROOT="{KIT_ROOT}"\nHOME="{tmp_path}"\n{stubs}{fn}\ninstall_kit_command memory-private\n'
    return subprocess.run(["bash", "-c", body], capture_output=True, text=True, check=False)


def test_install_puts_the_command_on_path(tmp_path: Path) -> None:
    result = _install(tmp_path)
    assert "FAIL" not in result.stdout, result.stdout
    dest = tmp_path / ".local" / "bin" / "memory-private"
    assert dest.read_bytes() == CMD.read_bytes()
    assert dest.stat().st_mode & 0o111


def test_install_leaves_a_foreign_file_alone(tmp_path: Path) -> None:
    dest = tmp_path / ".local" / "bin" / "memory-private"
    dest.parent.mkdir(parents=True)
    dest.write_text("#!/bin/sh\necho mine\n")
    result = _install(tmp_path)
    assert "left alone" in result.stdout
    assert dest.read_text() == "#!/bin/sh\necho mine\n"


def _git(repo: Path, *args: str) -> str:
    return subprocess.run(
        ["git", "-C", str(repo), *args], capture_output=True, text=True, check=True, env=HERMETIC_GIT,
    ).stdout


def test_a_settings_file_it_creates_is_not_committable(tmp_path: Path) -> None:
    """Claude Code git-ignores settings.local.json only when it creates it itself."""
    _git(tmp_path, "init", "-q")
    assert _run("on", str(tmp_path)).returncode == 0
    assert _git(tmp_path, "status", "--porcelain", "--untracked-files=all") == ""


def test_exclude_entry_is_added_once(tmp_path: Path) -> None:
    _git(tmp_path, "init", "-q")
    _run("on", str(tmp_path))
    _run("off", str(tmp_path))
    _run("on", str(tmp_path))
    exclude = (tmp_path / ".git" / "info" / "exclude").read_text()
    assert exclude.count(".claude/settings.local.json") == 1


def test_outside_git_it_still_works(tmp_path: Path) -> None:
    assert _run("on", str(tmp_path)).returncode == 0
    assert not (tmp_path / ".git").exists()
