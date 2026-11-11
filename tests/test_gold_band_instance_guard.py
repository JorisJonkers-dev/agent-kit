"""The running-instance guard: detection by process and data directory, and the mid-run stop.

Process listings are stubbed or built in a temporary directory; no test reads
the machine's real processes.
"""

from __future__ import annotations

import os
import sys
from pathlib import Path

import pytest
import yaml

KIT_ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(KIT_ROOT / "scripts"))
sys.path.insert(0, str(KIT_ROOT / "tests"))

import converge  # noqa: E402
import test_gold_band_converge as agents  # noqa: E402
import test_gold_band_roles as roles  # noqa: E402

Process = converge._Process


@pytest.fixture(autouse=True)
def _detect(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.delenv("AGENT_KIT_GOLD_BAND_RUNNING", raising=False)


def _listing(monkeypatch: pytest.MonkeyPatch, *procs: Process) -> None:
    monkeypatch.setattr(converge, "_gold_band_processes", lambda: list(procs))


def test_a_process_named_like_the_app_holding_our_data_dir_is_an_instance(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    # Any install path counts: detection is by name, never by location.
    _listing(monkeypatch, Process(7, "/opt/anywhere/gold-band-desktop", str(tmp_path)))
    found = converge._gold_band_running(tmp_path)
    assert found is not None
    assert "gold-band-personal" in found
    assert "process 7" in found


def test_the_macos_bundle_executable_name_counts(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    _listing(monkeypatch, Process(9, "/Applications/Gold Band.app/Contents/MacOS/Gold Band", None))
    assert converge._gold_band_running(tmp_path) is not None


def test_an_instance_on_another_data_dir_does_not_block(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    _listing(monkeypatch, Process(7, "gold-band-desktop", str(tmp_path / "other")))
    assert converge._gold_band_running(tmp_path / "mine") is None


def test_other_processes_do_not_block(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    _listing(monkeypatch, Process(1, "/usr/bin/vim", str(tmp_path)))
    assert converge._gold_band_running(tmp_path) is None


def test_the_override_variable_wins(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    _listing(monkeypatch, Process(7, "gold-band-desktop", str(tmp_path)))
    monkeypatch.setenv("AGENT_KIT_GOLD_BAND_RUNNING", "0")
    assert converge._gold_band_running(tmp_path) is None
    monkeypatch.setenv("AGENT_KIT_GOLD_BAND_RUNNING", "1")
    assert converge._gold_band_running(tmp_path) is not None


def _proc_entry(root: Path, pid: str, argv: list[str], env: dict[str, str] | None) -> None:
    entry = root / pid
    entry.mkdir()
    (entry / "cmdline").write_bytes(b"\0".join(a.encode() for a in argv) + b"\0")
    if env is not None:
        (entry / "environ").write_bytes(b"\0".join(f"{k}={v}".encode() for k, v in env.items()) + b"\0")


def test_proc_listing_reads_executable_and_data_dir(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("HOME", str(tmp_path / "home"))
    root = tmp_path / "proc"
    root.mkdir()
    (root / "self").mkdir()
    (root / "notapid").mkdir()
    _proc_entry(root, "11", ["/x/gold-band-desktop", "--flag"], {"GOLD_BAND_HOME": "/data/gb", "A": "b"})
    _proc_entry(root, "12", ["/x/gold-band-desktop"], {"PATH": "/bin"})
    _proc_entry(root, "13", ["/x/gold-band-desktop"], None)  # environ hidden
    _proc_entry(root, "14", ["/x/gold-band-desktop"], {"HOME": "/home/other"})
    (root / "15").mkdir()  # cmdline unreadable: gone mid-scan
    got = {p.pid: p for p in converge._processes_proc(root)}
    assert got[11] == Process(11, "/x/gold-band-desktop", "/data/gb")
    assert got[12].home == str(tmp_path / "home" / ".gold-band")
    assert got[13].home is None  # hidden data dir counts as ours
    assert got[14].home == "/home/other/.gold-band"
    assert set(got) == {11, 12, 13, 14}


def test_proc_listing_skips_other_users(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    root = tmp_path / "proc"
    root.mkdir()
    _proc_entry(root, "21", ["gold-band-desktop"], {})
    monkeypatch.setattr(os, "getuid", lambda: os.stat(root / "21").st_uid + 1)
    assert converge._processes_proc(root) == []


def test_ps_listing_is_limited_to_the_current_user(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    bin_dir = tmp_path / "bin"
    bin_dir.mkdir()
    fake = bin_dir / "ps"
    fake.write_text(
        '#!/bin/sh\necho "$@" > "$(dirname "$0")/args"\n'
        'echo "  31 /Applications/Gold Band.app/Contents/MacOS/Gold Band"\necho "junk"\n'
    )
    fake.chmod(0o755)
    monkeypatch.setenv("PATH", f"{bin_dir}:{os.environ['PATH']}")
    assert converge._processes_ps() == [Process(31, "/Applications/Gold Band.app/Contents/MacOS/Gold Band", None)]
    assert f"-U {os.getuid()}" in (bin_dir / "args").read_text()


def test_the_listing_uses_ps_without_proc_and_survives_failure(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(Path, "exists", lambda self: False)
    monkeypatch.setattr(converge, "_processes_ps", lambda: [Process(1, "x", None)])
    assert converge._gold_band_processes() == [Process(1, "x", None)]

    def boom() -> list:
        raise FileNotFoundError("ps")

    monkeypatch.setattr(converge, "_processes_ps", boom)
    assert converge._gold_band_processes() == []


def test_the_listing_uses_proc_when_present(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(Path, "exists", lambda self: True)
    monkeypatch.setattr(converge, "_processes_proc", lambda root: [Process(2, "y", None)])
    assert converge._gold_band_processes() == [Process(2, "y", None)]


def test_a_running_instance_stops_agents_setup_with_nothing_written(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    machine = agents.Machine(tmp_path, monkeypatch, settings={"agents": {}})
    _listing(monkeypatch, Process(7, "gold-band-desktop", str(machine.settings.parent)))
    before = machine.settings.read_bytes()
    assert machine.run() == converge.GOLD_BAND_RUNNING_EXIT
    out = capsys.readouterr().out
    assert "gold-band-personal" in out and "process 7" in out
    assert machine.settings.read_bytes() == before
    assert not machine.backup.exists()


def test_an_instance_starting_before_the_agents_write_stops_it(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    machine = agents.Machine(tmp_path, monkeypatch, settings={"agents": {}})
    answers = iter([None, "gold-band-personal (process 7)"])
    monkeypatch.setattr(converge, "_gold_band_running", lambda _dir: next(answers))
    before = machine.settings.read_bytes()
    assert machine.run() == converge.GOLD_BAND_RUNNING_EXIT
    out = capsys.readouterr().out
    assert "started during setup" in out
    assert "written: nothing" in out
    assert "not yet written: settings.json" in out
    assert machine.settings.read_bytes() == before
    assert not machine.backup.exists()


def test_roles_stop_between_groups_report_them_and_a_second_run_finishes(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    machine = roles.Machine(tmp_path, monkeypatch)
    declared = yaml.safe_load(machine.registry.read_text())
    stems = [r["file"] for r in declared["gold_band"]["roles"]]
    assert len(stems) >= 2
    # Clear at the start and before the first group, running before the second.
    answers = iter([None, None, "gold-band-personal (process 7)"])
    real = converge._gold_band_running
    monkeypatch.setattr(converge, "_gold_band_running", lambda _dir: next(answers, "gold-band-personal (process 7)"))
    assert machine.run() == converge.GOLD_BAND_RUNNING_EXIT
    out = capsys.readouterr().out
    assert "started during setup" in out
    assert f"written: {stems[0]}.md" in out
    assert f"not yet written: {', '.join(f'{s}.md' for s in stems[1:])}" in out
    assert machine.profile(stems[0]).exists()
    assert not machine.profile(stems[1]).exists()
    # The instance quits; the second run writes the rest.
    monkeypatch.setattr(converge, "_gold_band_running", real)
    _listing(monkeypatch)  # the instance has quit: no process is listed
    assert machine.run() == 0
    assert all(machine.profile(s).exists() for s in stems)


def test_roles_removal_is_a_write_group_too(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    machine = roles.Machine(tmp_path, monkeypatch)
    _listing(monkeypatch)
    assert machine.run() == 0
    stale = machine.hand_made("stale", "old\n")
    answers = iter([None, "gold-band-personal (process 7)"])
    monkeypatch.setattr(converge, "_gold_band_running", lambda _dir: next(answers))
    assert machine.run() == converge.GOLD_BAND_RUNNING_EXIT
    assert "not yet written: stale.md" in capsys.readouterr().out
    assert stale.exists()
