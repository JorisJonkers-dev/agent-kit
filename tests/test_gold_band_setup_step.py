"""Setup converges the personal Gold Band instance's agent map on Linux.

A whole run of the checked-in setup script against a temp HOME with every
external command stubbed, so nothing real is touched. The uv stub records how
the converge command was called; the converge logic has its own tests in
test_gold_band_converge.py.
"""

from __future__ import annotations

import shutil
import sys
from pathlib import Path

import pytest

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
    assert f"--settings {data}/.gold-band/settings.json" in call


def test_a_blank_data_directory_is_the_default_one(tmp_path: Path) -> None:
    run = setup_run(tmp_path, "Linux", prepare_home=_installed, extra_env={"GOLD_BAND_HOME": "  "})
    (call,) = _converge_calls(run.calls)
    assert f"--settings {run.home}/.gold-band/settings.json" in call


def test_check_mode_asks_for_a_report_only(tmp_path: Path) -> None:
    run = setup_run(tmp_path, "Linux", prepare_home=_installed, args=("--check",))
    (call,) = _converge_calls(run.calls)
    assert " --check " in f"{call} "


def test_a_failed_converge_fails_the_run(tmp_path: Path) -> None:
    run = setup_run(
        tmp_path,
        "Linux",
        prepare_home=_installed,
        stubs={"uv": 'echo "uv $*" >> "$0.calls"; echo "gold-band-agents: cannot converge"; exit 3'},
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
    assert f"--profiles {data}/.gold-band/context/profiles" in call


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


def test_a_running_instance_stops_setup_before_other_write_groups(tmp_path: Path) -> None:
    run = setup_run(
        tmp_path,
        "Linux",
        prepare_home=_installed,
        stubs={"uv": 'echo "uv $*" >> "$0.calls"; echo "gold-band-agents: Gold Band instance running"; exit 4'},
    )
    assert "Gold Band is running: setup stopped" in run.stdout + run.stderr
    assert run.returncode != 0
    assert "Setup summary" not in run.stdout + run.stderr


def _overlay(home: Path) -> None:
    path = home / ".config" / "agent-kit" / "overlay.yaml"
    path.parent.mkdir(parents=True)
    path.write_text("instances:\n  gold-band:\n    employer_roots: [~/employer]\n")


def _launcher(tmp_path: Path, body: str) -> dict[str, str]:
    binary = tmp_path / "agent-launcher-stub"
    binary.write_text(f"#!/bin/sh\n{body}\n")
    binary.chmod(0o755)
    return {"AGENT_LAUNCHER_BIN": str(binary)}


def test_the_read_limit_is_checked_on_the_work_mac(tmp_path: Path) -> None:
    env = _launcher(tmp_path, 'echo "args: $*"; echo "read limit: ok, 1 employer root(s)"')
    run = setup_run(tmp_path / "run", "Darwin", prepare_home=_overlay, extra_env=env)
    assert "args: read-limit-check" in run.stdout
    assert "read limit: ok, 1 employer root(s)" in run.stdout
    assert "gold band read limit" not in run.stderr


def test_setup_refuses_the_personal_instance_when_the_read_limit_cannot_be_applied(tmp_path: Path) -> None:
    env = _launcher(tmp_path, 'echo "agent-launcher: sandbox-exec is not available" >&2; exit 1')
    run = setup_run(tmp_path / "run", "Darwin", prepare_home=_overlay, extra_env=env)
    out = run.stdout + run.stderr
    assert "sandbox-exec is not available" in out
    assert "gold band read limit: the personal instance must not be started on this machine" in out
    assert run.returncode != 0


def test_setup_refuses_when_the_launcher_cannot_be_asked(tmp_path: Path) -> None:
    if shutil.which("agent-launcher", path="/usr/bin:/bin"):
        pytest.skip("agent-launcher is on the harness PATH here")
    run = setup_run(tmp_path, "Darwin", prepare_home=_overlay)
    assert "agent-launcher is not installed, so the launcher cannot be asked" in run.stdout + run.stderr
    assert run.returncode != 0


def test_a_mac_without_an_overlay_needs_no_read_limit(tmp_path: Path) -> None:
    env = _launcher(tmp_path, "exit 1")
    run = setup_run(tmp_path / "run", "Darwin", extra_env=env)
    assert "gold band read limit: no overlay, not the work Mac" in run.stdout
    assert "gold band read limit:" not in run.stderr


def test_linux_has_no_employer_clones_and_no_read_limit_check(tmp_path: Path) -> None:
    env = _launcher(tmp_path, 'echo "ran" ; exit 1')
    run = setup_run(tmp_path / "run", "Linux", prepare_home=_overlay, extra_env=env)
    assert "gold band read limit" not in run.stdout + run.stderr


def test_a_cloud_session_skips_the_read_limit_check(tmp_path: Path) -> None:
    env = _launcher(tmp_path, "exit 1")
    run = setup_run(tmp_path / "run", "Darwin", prepare_home=_overlay, extra_env=env, args=("--cloud",))
    assert "gold band read limit" not in run.stdout + run.stderr


def test_setup_asks_the_launcher_on_the_path_when_no_binary_is_named(tmp_path: Path) -> None:
    refusal = "agent-launcher: the read limit cannot be applied"
    stubs = {"agent-launcher": f'echo "agent-launcher $*" >> "$0.calls"; echo "{refusal}" >&2; exit 1'}
    run = setup_run(tmp_path, "Darwin", prepare_home=_overlay, stubs=stubs)
    out = run.stdout + run.stderr
    assert "the read limit cannot be applied" in out
    assert "the personal instance must not be started on this machine" in out
    assert run.returncode != 0
