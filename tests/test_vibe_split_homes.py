"""The one-time split of the Vibe home.

Before the split, every Vibe run shared one home, so its history holds
sessions that recalled Private Memory. That home becomes vibe-private's, and
a fresh shared home gets only the settings, never the history.
"""

from __future__ import annotations

import subprocess
import sys
from pathlib import Path

KIT_ROOT = Path(__file__).resolve().parent.parent
SPLIT = KIT_ROOT / "scripts" / "vibe-split-homes.py"
sys.path.insert(0, str(KIT_ROOT / "scripts"))

import render_registry  # noqa: E402

# A Vibe home from before the split: Private Memory was one of its servers.
PRIVATE_CONFIG = 'active_model = "mine"\n\n[[mcp_servers]]\nname = "memory-private"\ntransport = "streamable-http"\n'


def _split(shared: Path, private: Path, *flags: str) -> subprocess.CompletedProcess:
    return subprocess.run([sys.executable, str(SPLIT), "--shared", str(shared), "--private", str(private), *flags],
                          capture_output=True, text=True, check=False)


def _old_home(home: Path) -> Path:
    old = home / ".vibe"
    (old / "logs" / "session").mkdir(parents=True)
    (old / "logs" / "session" / "s1.json").write_text("recalled: private note")
    (old / "vibehistory").write_text("private prompt\n")
    (old / ".env").write_text("MISTRAL_API_KEY=x\n")
    (old / "config.toml").write_text(PRIVATE_CONFIG)
    (old / "hooks.toml").write_text("[[hooks]]\nname = \"mine\"\n")
    (old / "AGENTS.md").write_text("my own instructions\n")
    (old / "skills" / "pr-composer").mkdir(parents=True)
    (old / "skills" / "pr-composer" / "SKILL.md").write_text("skill\n")
    (old / "prompts").mkdir()
    (old / "prompts" / "p.md").write_text("prompt\n")
    return old


def _tree(root: Path) -> dict[str, str]:
    return {str(p.relative_to(root)): p.read_text() for p in sorted(root.rglob("*")) if p.is_file()}


def test_the_old_home_with_its_history_becomes_vibe_privates_home(tmp_path: Path) -> None:
    before = _tree(_old_home(tmp_path))
    result = _split(tmp_path / ".vibe", tmp_path / ".vibe-private")
    assert result.returncode == 0, result.stderr
    assert _tree(tmp_path / ".vibe-private") == before


def test_the_fresh_shared_home_gets_the_settings_and_login_and_none_of_the_history(tmp_path: Path) -> None:
    _old_home(tmp_path)
    assert _split(tmp_path / ".vibe", tmp_path / ".vibe-private").returncode == 0
    assert _tree(tmp_path / ".vibe") == {
        "config.toml": PRIVATE_CONFIG,
        "hooks.toml": '[[hooks]]\nname = "mine"\n',
        ".env": "MISTRAL_API_KEY=x\n",
        "AGENTS.md": "my own instructions\n",
        "skills/pr-composer/SKILL.md": "skill\n",
        "prompts/p.md": "prompt\n",
    }


def test_a_second_run_changes_nothing(tmp_path: Path) -> None:
    _old_home(tmp_path)
    _split(tmp_path / ".vibe", tmp_path / ".vibe-private")
    (tmp_path / ".vibe" / "logs").mkdir()
    (tmp_path / ".vibe" / "logs" / "new.json").write_text("a study session")
    shared, private = _tree(tmp_path / ".vibe"), _tree(tmp_path / ".vibe-private")
    result = _split(tmp_path / ".vibe", tmp_path / ".vibe-private")
    assert result.returncode == 0, result.stderr
    assert (_tree(tmp_path / ".vibe"), _tree(tmp_path / ".vibe-private")) == (shared, private)


def test_an_existing_private_home_is_never_overwritten(tmp_path: Path) -> None:
    _old_home(tmp_path)
    (tmp_path / ".vibe-private").mkdir()
    (tmp_path / ".vibe-private" / "config.toml").write_text("already split\n")
    shared = _tree(tmp_path / ".vibe")
    assert _split(tmp_path / ".vibe", tmp_path / ".vibe-private").returncode == 0
    assert _tree(tmp_path / ".vibe-private") == {"config.toml": "already split\n"}
    assert _tree(tmp_path / ".vibe") == shared


def test_a_home_that_never_had_private_memory_stays_the_shared_home(tmp_path: Path) -> None:
    old = _old_home(tmp_path)
    (old / "config.toml").write_text('[[mcp_servers]]\nname = "memory-api"\n')
    before = _tree(old)
    result = _split(tmp_path / ".vibe", tmp_path / ".vibe-private")
    assert result.returncode == 0, result.stderr
    assert _tree(tmp_path / ".vibe") == before
    assert not (tmp_path / ".vibe-private").exists()


def test_without_an_old_home_there_is_nothing_to_split(tmp_path: Path) -> None:
    result = _split(tmp_path / ".vibe", tmp_path / ".vibe-private")
    assert result.returncode == 0, result.stderr
    assert not (tmp_path / ".vibe").exists()
    assert not (tmp_path / ".vibe-private").exists()


def test_a_split_that_fails_halfway_leaves_the_old_home_whole_and_is_retried(tmp_path: Path) -> None:
    old = _old_home(tmp_path)
    unreadable = old / "skills" / "pr-composer" / "SKILL.md"
    unreadable.chmod(0)
    try:
        assert _split(tmp_path / ".vibe", tmp_path / ".vibe-private").returncode != 0
    finally:
        unreadable.chmod(0o644)
    assert not (tmp_path / ".vibe-private").exists()
    assert (old / "logs" / "session" / "s1.json").read_text() == "recalled: private note"
    assert [p.name for p in tmp_path.iterdir()] == [".vibe"], "nothing half-built is left behind"
    assert _split(tmp_path / ".vibe", tmp_path / ".vibe-private").returncode == 0
    assert (tmp_path / ".vibe-private" / "logs" / "session" / "s1.json").exists()
    assert not (tmp_path / ".vibe" / "logs").exists()


def test_check_says_what_a_real_run_would_do_and_changes_nothing(tmp_path: Path) -> None:
    old = _old_home(tmp_path)
    before = _tree(old)
    result = _split(tmp_path / ".vibe", tmp_path / ".vibe-private", "--check")
    assert result.returncode == 0, result.stderr
    assert "would move" in result.stdout
    assert _tree(old) == before and not (tmp_path / ".vibe-private").exists()
    (old / "config.toml").write_text('[[mcp_servers]]\nname = "memory-api"\n')
    result = _split(tmp_path / ".vibe", tmp_path / ".vibe-private", "--check")
    assert "would move" not in result.stdout
    assert "stays the shared home" in result.stdout


def test_setup_splits_the_home_before_merging_into_either_and_never_in_a_cloud_run() -> None:
    script = render_registry.render_setup_script(render_registry.load_registry())
    lines = script.splitlines()
    # Top level: it runs whether or not vibe is on PATH, before converge or a skill copy can touch a home.
    split = lines.index('[ "${CLOUD}" = 1 ] || _vibe_split_homes')
    shared_merge = next(i for i, line in enumerate(lines) if "generated/vibe/mcp-servers.toml\" --config" in line)
    converge = lines.index("    converge_mcp")
    private_skill = next(i for i, line in enumerate(lines) if 'dest="${VIBE_PRIVATE_HOME}/skills/' in line)
    assert split < min(shared_merge, converge, private_skill)
    fn = script[script.index("_vibe_split_homes() {"):]
    fn = fn[: fn.index("\n}\n")]
    assert 'scripts/vibe-split-homes.py" --shared "${VIBE_HOME}" --private "${VIBE_PRIVATE_HOME}"' in fn


def test_the_split_runs_in_check_mode_without_moving_anything() -> None:
    script = render_registry.render_setup_script(render_registry.load_registry())
    fn = script[script.index("_vibe_split_homes() {"):]
    fn = fn[: fn.index("\n}\n")]
    assert '[ "${CHECK_ONLY}" = 1 ] && args+=(--check)' in fn
