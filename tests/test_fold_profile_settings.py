"""A secondary Claude profile's own settings.json is folded back into the shared one and linked.

Without the link the profile has no enabledPlugins, so every plugin is off in it; a
copy Claude Code wrote before setup linked the profile used to stay that way.
"""

from __future__ import annotations

import json
import subprocess
import sys
from pathlib import Path

SCRIPT = Path(__file__).resolve().parent.parent / "scripts" / "fold-profile-settings.py"


def _run(*args: str) -> subprocess.CompletedProcess[str]:
    return subprocess.run([sys.executable, str(SCRIPT), *args], capture_output=True, text=True, check=False)


def _write(path: Path, value: object) -> Path:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value))
    return path


SHARED = {
    "enabledPlugins": {"mattpocock-skills@agent-kit-official": True},
    "permissions": {"deny": ["Read(~/.vibe/**)"], "allow": []},
    "theme": "dark",
}


def test_a_copy_that_adds_nothing_is_backed_up_and_replaced_by_the_link(tmp_path: Path) -> None:
    shared = _write(tmp_path / "work" / "settings.json", SHARED)
    copy = _write(
        tmp_path / "personal" / "settings.json", {"theme": "dark", "permissions": {"deny": ["Read(~/.vibe/**)"]}}
    )
    before = shared.read_text()
    result = _run(str(shared), str(copy))
    assert result.returncode == 0, result.stdout + result.stderr
    assert copy.is_symlink() and copy.resolve() == shared.resolve()
    assert shared.read_text() == before
    backups = list(copy.parent.glob("settings.json.bak-*"))
    assert len(backups) == 1 and json.loads(backups[0].read_text())["theme"] == "dark"


def test_what_only_the_copy_holds_is_folded_into_the_shared_file(tmp_path: Path) -> None:
    shared = _write(tmp_path / "work" / "settings.json", SHARED)
    copy = _write(
        tmp_path / "personal" / "settings.json",
        {
            "permissions": {"deny": ["Read(~/.vibe/**)", "Bash(rm -rf *)"], "allow": ["mcp__overleaf__*"]},
            "agentPushNotifEnabled": True,
        },
    )
    assert _run(str(shared), str(copy)).returncode == 0
    merged = json.loads(shared.read_text())
    assert merged["permissions"]["deny"] == ["Read(~/.vibe/**)", "Bash(rm -rf *)"]
    assert merged["permissions"]["allow"] == ["mcp__overleaf__*"]
    assert merged["agentPushNotifEnabled"] is True
    assert merged["enabledPlugins"] == SHARED["enabledPlugins"]


def test_a_copy_that_disagrees_with_the_shared_file_is_left_alone(tmp_path: Path) -> None:
    shared = _write(tmp_path / "work" / "settings.json", SHARED)
    copy = _write(
        tmp_path / "personal" / "settings.json",
        {"theme": "light", "enabledPlugins": {"mattpocock-skills@agent-kit-official": False}},
    )
    before = shared.read_text()
    result = _run(str(shared), str(copy))
    assert result.returncode == 3
    assert "theme" in result.stdout and "enabledPlugins.mattpocock-skills@agent-kit-official" in result.stdout
    assert not copy.is_symlink() and shared.read_text() == before
    assert not list(copy.parent.glob("settings.json.bak-*"))


def test_check_changes_nothing(tmp_path: Path) -> None:
    shared = _write(tmp_path / "work" / "settings.json", SHARED)
    copy = _write(tmp_path / "personal" / "settings.json", {"agentPushNotifEnabled": True})
    result = _run(str(shared), str(copy), "--check")
    assert result.returncode == 0 and result.stdout.startswith("would fold")
    assert not copy.is_symlink() and "agentPushNotifEnabled" not in shared.read_text()


def test_a_file_that_is_not_a_json_object_is_left_alone(tmp_path: Path) -> None:
    shared = _write(tmp_path / "work" / "settings.json", SHARED)
    copy = tmp_path / "personal" / "settings.json"
    copy.parent.mkdir(parents=True)
    copy.write_text("not json")
    assert _run(str(shared), str(copy)).returncode == 3
    assert copy.read_text() == "not json"


def test_a_shared_file_that_is_a_link_stays_a_link(tmp_path: Path) -> None:
    real = _write(tmp_path / "store" / "settings.json", SHARED)
    shared = tmp_path / "work" / "settings.json"
    shared.parent.mkdir(parents=True)
    shared.symlink_to(real)
    copy = _write(tmp_path / "personal" / "settings.json", {"agentPushNotifEnabled": True})
    assert _run(str(shared), str(copy)).returncode == 0
    assert shared.is_symlink() and json.loads(real.read_text())["agentPushNotifEnabled"] is True


def test_an_earlier_backup_is_never_overwritten(tmp_path: Path) -> None:
    shared = _write(tmp_path / "work" / "settings.json", SHARED)
    copy = _write(tmp_path / "personal" / "settings.json", {"theme": "dark"})
    stamp_taken = copy.with_name("settings.json.bak-taken")
    stamp_taken.write_text("earlier")
    import importlib.util

    spec = importlib.util.spec_from_file_location("fold", SCRIPT)
    assert spec is not None and spec.loader is not None
    fold = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(fold)
    first = fold.free_backup_path(copy)
    first.write_text("x")
    second = fold.free_backup_path(copy)
    assert second != first and second.name.startswith(first.name)
    assert _run(str(shared), str(copy)).returncode == 0
    assert stamp_taken.read_text() == "earlier"


def test_a_missing_shared_file_is_named_and_nothing_changes(tmp_path: Path) -> None:
    copy = _write(tmp_path / "personal" / "settings.json", {"theme": "dark"})
    result = _run(str(tmp_path / "work" / "settings.json"), str(copy))
    assert result.returncode == 3 and "work/settings.json" in result.stdout
    assert not copy.is_symlink()


def test_bad_arguments_exit_2() -> None:
    assert _run("only-one").returncode == 2
