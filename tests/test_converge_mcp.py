"""Setup converges the MCP servers after it installs them, and no hand-written removal list remains.

The convergence itself is `agent-kit-merge converge mcp` (scripts/crates/agent-kit-merge/src/converge.rs, #810);
its behaviour is tested there.
"""

from __future__ import annotations

import sys
from pathlib import Path

KIT_ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(KIT_ROOT / "scripts"))

import render_registry  # noqa: E402
import yaml  # noqa: E402


def _registry() -> dict:
    return yaml.safe_load((KIT_ROOT / "registry" / "estate-tooling.yaml").read_text())


def test_setup_calls_the_rust_convergence() -> None:
    script = render_registry.render_setup_script(_registry())
    assert "out=$(agent_kit_merge_current converge mcp " in script
    assert "converge.py\" mcp" not in script


def test_setup_converges_after_the_servers_are_registered() -> None:
    script = render_registry.render_setup_script(_registry())
    mcp = script.index('log "MCP servers"')
    assert mcp < script.index("\n    converge_mcp\n", mcp) < script.index('log "MCP registration skipped')


def test_the_hand_written_retirement_lists_are_gone() -> None:
    script = render_registry.render_setup_script(_registry())
    for retired in render_registry.retired_servers(_registry()):
        assert f"claude mcp remove --scope user {retired} " not in script
        assert f"codex mcp remove {retired} " not in script
        vibe = [line for line in script.splitlines() if "vibe_remove" in line]
        assert not [line for line in vibe if f"--remove {retired} " in f"{line} "]
    assert "codex mcp remove context7 " not in script
    assert "hand-added" not in script
