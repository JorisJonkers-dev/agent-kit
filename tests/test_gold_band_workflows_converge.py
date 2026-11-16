"""Setup converges the Gold Band workflow templates and fails --check when they are missing.

The convergence is `agent-kit-merge converge gold-band-workflows`
(scripts/crates/agent-kit-merge/src/gold_band_workflows.rs, #810); its behaviour is tested there.
"""

from __future__ import annotations

from pathlib import Path

KIT_ROOT = Path(__file__).resolve().parent.parent


def test_setup_runs_the_convergence_and_the_bundle_ships_the_workflows() -> None:
    setup = (KIT_ROOT / "installer" / "setup-workstation.sh").read_text()
    assert "gold_band_workflows() {" in setup and "\ngold_band_workflows\n" in setup
    assert "agent_kit_merge_current converge gold-band-workflows" in setup
    assert 'converge.py" gold-band-workflows' not in setup
    assert '--store "${band_home}/.gold-band/context/workflows.json"' in setup
    assert '--workflows "${KIT_ROOT}/installer/gold-band/workflows"' in setup
    publish = (KIT_ROOT / "scripts" / "publish-installer-artifacts.sh").read_text()
    assert "installer/gold-band/workflows" in publish.split("KIT_PATHS=(")[1].split(")")[0].split()


def test_setup_check_fails_when_templates_are_missing() -> None:
    setup = (KIT_ROOT / "installer" / "setup-workstation.sh").read_text()
    block = setup.split("gold_band_workflows() {")[1].split("\ngold_band_workflows\n")[0]
    assert 'grep -Eq "^would (add|change) "' in block and 'fail "Gold Band workflow templates are missing' in block
