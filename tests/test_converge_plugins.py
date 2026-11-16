"""Setup converges the Claude plugins after it installs them, and no hand-written removal list remains.

The convergence itself is `agent-kit-merge converge plugins` (scripts/crates/agent-kit-merge/src/converge.rs, #810);
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
    assert "out=$(agent_kit_merge converge plugins " in script
    assert "converge.py\" plugins" not in script


def test_setup_converges_plugins_after_installing_them() -> None:
    script = render_registry.render_setup_script(_registry())
    lsp = script.index('log "language servers skipped (--no-lsp)"')
    assert lsp < script.index("\n  converge_plugins\n", lsp) < script.index('log "MCP servers"')


def test_the_hand_written_retired_plugin_removal_is_gone() -> None:
    script = render_registry.render_setup_script(_registry())
    for retired in _registry()["retired_plugins"]:
        assert f"claude plugin uninstall {retired['ref']}" not in script
        assert f"claude plugin marketplace remove {retired['marketplace']}" not in script
