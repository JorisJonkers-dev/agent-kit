"""Setup converges the personal Gold Band instance's agent map on Linux.

A whole run of the checked-in setup script against a temp HOME with every
external command stubbed, so nothing real is touched. The uv stub records how
the converge command was called; the converge logic has its own tests in
test_gold_band_converge.py.
"""

from __future__ import annotations

import sys
from pathlib import Path

KIT_ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(Path(__file__).resolve().parent))

from setup_harness import setup_run  # noqa: E402


def _installed(home: Path) -> None:
    binary = home / ".local" / "bin" / "gold-band"
    binary.parent.mkdir(parents=True)
    binary.write_text("#!/bin/sh\n")
    binary.chmod(0o755)


def _converge_calls(calls: list[str]) -> list[str]:
    return [call for call in calls if "converge.py gold-band " in call]


def test_a_machine_without_gold_band_is_left_alone(tmp_path: Path) -> None:
    run = setup_run(tmp_path, "Linux")
    assert "gold band agents: Gold Band is not installed, its agent map is left alone" in run.stdout
    assert _converge_calls(run.calls) == []
    assert not (run.home / ".gold-band").exists()


def test_an_installed_gold_band_is_converged_against_its_settings(tmp_path: Path) -> None:
    run = setup_run(tmp_path, "Linux", prepare_home=_installed)
    (call,) = _converge_calls(run.calls)
    assert f"--keep {run.home}/.config/agent-kit/keep.yaml" in call
    assert f"--settings {run.home}/.gold-band/settings.json" in call
    assert "--registry" in call
    assert " --check " not in f"{call} "


def test_the_instance_data_directory_can_be_moved(tmp_path: Path) -> None:
    data = tmp_path / "instance"
    run = setup_run(tmp_path, "Linux", prepare_home=_installed, extra_env={"GOLD_BAND_HOME": str(data)})
    (call,) = _converge_calls(run.calls)
    assert f"--settings {data}/settings.json" in call


def test_check_mode_asks_for_a_report_only(tmp_path: Path) -> None:
    run = setup_run(tmp_path, "Linux", prepare_home=_installed, args=("--check",))
    (call,) = _converge_calls(run.calls)
    assert " --check " in f"{call} "


def test_a_failed_converge_fails_the_run(tmp_path: Path) -> None:
    run = setup_run(
        tmp_path,
        "Linux",
        prepare_home=_installed,
        stubs={"uv": 'echo "uv $*" >> "$0.calls"; echo "gold-band-agents: Gold Band is running"; exit 3'},
    )
    assert "the Gold Band agent map did not converge (see above)" in run.stdout + run.stderr
    assert run.returncode != 0


def test_the_mac_is_not_converged_here(tmp_path: Path) -> None:
    run = setup_run(tmp_path, "Darwin", prepare_home=_installed)
    assert "gold band agents: only the Linux personal instance is converged here, skipped" in run.stdout
    assert _converge_calls(run.calls) == []


def test_a_cloud_session_is_not_converged(tmp_path: Path) -> None:
    run = setup_run(tmp_path, "Linux", prepare_home=_installed, args=("--cloud",))
    assert _converge_calls(run.calls) == []


_FAILING_ROLES_UV = (
    'echo "uv $*" >> "$0.calls"; case "$*" in *gold-band-roles*) echo "gold-band-roles: running"; exit 3;; esac'
)


def _roles_calls(calls: list[str]) -> list[str]:
    return [call for call in calls if "converge.py gold-band-roles" in call]


def test_roles_are_left_alone_without_gold_band(tmp_path: Path) -> None:
    run = setup_run(tmp_path, "Linux")
    assert "gold band roles: Gold Band is not installed, its profiles are left alone" in run.stdout
    assert _roles_calls(run.calls) == []


def test_roles_are_converged_into_the_instance_profiles(tmp_path: Path) -> None:
    run = setup_run(tmp_path, "Linux", prepare_home=_installed)
    (call,) = _roles_calls(run.calls)
    assert f"--keep {run.home}/.config/agent-kit/keep.yaml" in call
    assert f"--profiles {run.home}/.gold-band/context/profiles" in call
    assert "--registry" in call
    assert " --check " not in f"{call} "


def test_the_roles_follow_the_moved_instance(tmp_path: Path) -> None:
    data = tmp_path / "instance"
    run = setup_run(tmp_path, "Linux", prepare_home=_installed, extra_env={"GOLD_BAND_HOME": str(data)})
    (call,) = _roles_calls(run.calls)
    assert f"--profiles {data}/context/profiles" in call


def test_check_mode_reports_the_roles_only(tmp_path: Path) -> None:
    run = setup_run(tmp_path, "Linux", prepare_home=_installed, args=("--check",))
    (call,) = _roles_calls(run.calls)
    assert " --check " in f"{call} "


def test_a_failed_roles_converge_fails_the_run(tmp_path: Path) -> None:
    run = setup_run(
        tmp_path,
        "Linux",
        prepare_home=_installed,
        stubs={"uv": _FAILING_ROLES_UV},
    )
    assert "the Gold Band roles did not converge (see above)" in run.stdout + run.stderr
    assert run.returncode != 0


def test_the_mac_and_a_cloud_session_do_not_converge_roles(tmp_path: Path) -> None:
    mac = setup_run(tmp_path / "mac", "Darwin", prepare_home=_installed)
    cloud = setup_run(tmp_path / "cloud", "Linux", prepare_home=_installed, args=("--cloud",))
    assert _roles_calls(mac.calls) == []
    assert _roles_calls(cloud.calls) == []
