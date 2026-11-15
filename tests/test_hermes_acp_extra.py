"""hermes-agent is installed with its `acp` extra, and setup repairs and checks it (#769).

`hermes-acp` imports `agent-client-protocol` at start; an install without the
extra dies with ModuleNotFoundError, and an upgrade never adds an extra.
"""

from __future__ import annotations

import re
from pathlib import Path

import yaml

KIT_ROOT = Path(__file__).resolve().parent.parent


def _hermes() -> dict:
    data = yaml.safe_load((KIT_ROOT / "registry" / "estate-tooling.yaml").read_text())
    return next(c for c in data["clis"] if c["name"] == "hermes-agent")


def test_every_hermes_install_carries_the_acp_extra_under_the_cooldown() -> None:
    cli = _hermes()
    for cmd in (cli["install"], cli["update"], cli["repair"], cli["container"]["install"]):
        assert "hermes-agent[mcp,acp]" in cmd
        assert re.search(r'--exclude-newer \\?"7 days\\?"', cmd)
    assert "--reinstall" in cli["repair"] and "--exclude-newer" in cli["repair"]
    assert "import acp" in cli["health"]


def test_setup_probes_acp_and_reinstalls_in_place() -> None:
    setup = (KIT_ROOT / "installer" / "setup-workstation.sh").read_text()
    block = setup.split("# Nous Research Hermes Agent.")[1].split("# Mistral Vibe")[0]
    assert "import acp" in block and "--reinstall" in block
    assert 'if [ "${CHECK_ONLY}" = 1 ]; then\n      fail "hermes:' in block
    container = (KIT_ROOT / "installer" / "setup-container.sh").read_text()
    assert "hermes-agent[mcp,acp]==${VERSION}" in container
