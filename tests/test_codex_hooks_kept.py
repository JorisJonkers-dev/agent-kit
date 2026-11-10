"""The retired-hook purge strips old knowledge hooks from Codex's hooks.json, never the file.

Hindsight Coding Agents keeps its Codex hooks in ~/.codex/hooks.json, the file
the retired knowledge hooks once used. Deleting it whole (as the purge did)
silently unwired Codex's memory on every setup run.
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

CODING_AGENTS = {"hooks": [{"type": "command", "command": 'node "/h/.hindsight/coding-agents/dist/codex-hook.js"'}]}
RETIRED = {"hooks": [{"type": "command", "command": "/h/.codex/hooks/kb-user-prompt-recall.sh"}]}


def _fn(script: str, name: str) -> str:
    # Its body embeds Python whose own `}` lines would end a naive split early.
    start = script.index(f"\n{name}() {{") + 1
    end = script.index("\n}\n", script.index('rmdir "${CLAUDE_HOOKS_DIR}"', start))
    return script[start : end + 3]


def _purge(tmp_path: Path, check_only: bool = False) -> Path:
    script = render_registry.render_setup_script(
        yaml.safe_load((KIT_ROOT / "registry" / "estate-tooling.yaml").read_text()),
    )
    codex = tmp_path / ".codex"
    codex.mkdir()
    hooks = codex / "hooks.json"
    hooks.write_text(json.dumps({"hooks": {"UserPromptSubmit": [CODING_AGENTS, RETIRED], "Stop": [RETIRED]}}))
    (tmp_path / ".claude").mkdir()
    body = (
        f"CHECK_ONLY={int(check_only)}\nCLAUDE_HOME={tmp_path}/.claude\nCODEX_HOME={codex}\n"
        f"CLAUDE_HOOKS_DIR={tmp_path}/.claude/hooks\nCODEX_HOOKS_DIR={codex}/hooks\n"
        "log() { :; }\nok() { :; }\nwarn() { echo \"warn $*\" >&2; }\nfail() { echo \"fail $*\" >&2; }\n"
        + _fn(script, "purge_retired_hooks") + "purge_retired_hooks\n"
    )
    result = subprocess.run(["/bin/bash", "-c", body], capture_output=True, text=True, check=False,
                            env={**os.environ})
    assert result.returncode == 0 and "fail" not in result.stderr, result.stderr
    return hooks


def test_codex_keeps_its_hooks_file_and_only_the_retired_hooks_go(tmp_path: Path) -> None:
    hooks = _purge(tmp_path)
    assert hooks.exists(), "the Coding Agents hooks file must survive the purge"
    assert json.loads(hooks.read_text())["hooks"] == {"UserPromptSubmit": [CODING_AGENTS]}


def test_check_mode_leaves_it_untouched(tmp_path: Path) -> None:
    hooks = _purge(tmp_path, check_only=True)
    assert len(json.loads(hooks.read_text())["hooks"]["UserPromptSubmit"]) == 2
