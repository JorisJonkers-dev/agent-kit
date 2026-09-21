"""Drift guard: the banner-anchor regex must not duplicate on repeated syncs."""

from __future__ import annotations

import subprocess
import sys
from pathlib import Path

KIT_ROOT = Path(__file__).resolve().parent.parent
SYNC_SCRIPT = KIT_ROOT / "scripts" / "sync-hermes-registry.sh"

_SKILLS_CM = """apiVersion: v1
kind: ConfigMap
metadata:
  name: hermes-skills
data:
  sources.conf: |
    # stale body, replaced wholesale
"""

# Simulates a checkout that already accumulated TWO duplicate banners from
# the pre-fix script -- the exact shape found in fleet-infra.
_CONFIG_CM_CORRUPTED = """apiVersion: v1
kind: ConfigMap
metadata:
  name: hermes-config
data:
  config.yaml: |
    skills:
      external_dirs: []

    # GENERATED FROM registry/estate-tooling.yaml -- DO NOT EDIT.
    #
    # Copy this block under `mcp_servers:` in fleet-infra
    # cluster/flux/apps/agents/hermes/config-configmap.yaml.

    # GENERATED FROM registry/estate-tooling.yaml -- DO NOT EDIT.
    #
    # Copy this block under `mcp_servers:` in fleet-infra
    # cluster/flux/apps/agents/hermes/config-configmap.yaml.

    mcp_servers:
      # stale entry from before the last registry change
      old-server:
        url: "http://old.example.svc.cluster.local:8080/mcp"
"""


def _run_sync(fleet_root: Path, mode: str) -> subprocess.CompletedProcess:
    return subprocess.run(
        ["bash", str(SYNC_SCRIPT), str(fleet_root), mode],
        capture_output=True,
        text=True,
        check=False,
    )


def _make_fleet_root(tmp_path: Path) -> Path:
    fleet_root = tmp_path / "fleet-infra"
    hermes_dir = fleet_root / "cluster" / "flux" / "apps" / "agents" / "hermes"
    hermes_dir.mkdir(parents=True)
    (hermes_dir / "skills-configmap.yaml").write_text(_SKILLS_CM)
    (hermes_dir / "config-configmap.yaml").write_text(_CONFIG_CM_CORRUPTED)
    return fleet_root


def test_write_collapses_a_pre_existing_duplicated_banner_to_one(tmp_path: Path) -> None:
    fleet_root = _make_fleet_root(tmp_path)
    result = _run_sync(fleet_root, "--write")
    assert result.returncode == 0, result.stdout + result.stderr

    config = (fleet_root / "cluster/flux/apps/agents/hermes/config-configmap.yaml").read_text()
    assert config.count("GENERATED FROM registry/estate-tooling.yaml") == 1
    assert "old-server" not in config
    # A real server from the current registry landed.
    assert "  memory-api:" in config or "  kubernetes:" in config


def test_a_second_write_is_a_no_op(tmp_path: Path) -> None:
    """Idempotency: syncing twice must not grow the banner again."""
    fleet_root = _make_fleet_root(tmp_path)
    _run_sync(fleet_root, "--write")
    before = (fleet_root / "cluster/flux/apps/agents/hermes/config-configmap.yaml").read_text()

    check = _run_sync(fleet_root, "--check")
    assert check.returncode == 0, check.stdout + check.stderr
    assert "already current" in check.stdout

    _run_sync(fleet_root, "--write")
    after = (fleet_root / "cluster/flux/apps/agents/hermes/config-configmap.yaml").read_text()
    assert before == after
    assert after.count("GENERATED FROM registry/estate-tooling.yaml") == 1


def test_check_mode_reports_the_corrupted_file_as_stale(tmp_path: Path) -> None:
    fleet_root = _make_fleet_root(tmp_path)
    result = _run_sync(fleet_root, "--check")
    assert result.returncode != 0
    assert "OUT OF DATE" in result.stderr
    # The file on disk is untouched in --check mode.
    config = (fleet_root / "cluster/flux/apps/agents/hermes/config-configmap.yaml").read_text()
    assert config == _CONFIG_CM_CORRUPTED


if __name__ == "__main__":
    sys.exit(subprocess.call(["pytest", __file__, "-v"]))
