"""The Gold Band launcher: a managed start that enforces the reach (#339).

converge.py's ``gold-band-launch`` subcommand flattens the registry's Gold
Band agents and reach support record into the gold-band-registry crate's
line protocol and runs its ``launch`` command, which either refuses the id
BEFORE STARTING ANYTHING (exit 1, one reason line on stderr) or exec's its
adapter command, so the started agent inherits stdin, stdout and stderr
and the caller sees the agent's own exit code. A fake agent script in a
temporary directory is the started agent: it echoes a line and writes a
marker file when it starts, so every case proves both what came back --
the exit code, the reason line -- and that the launcher started nothing
when it should not have (the marker never appears). The crate builds the
way the other Gold Band tests build it, and the run skips with setup's
own reason where no Rust toolchain is there.
"""

from __future__ import annotations

import argparse
import os
import shutil
import subprocess
import sys
from pathlib import Path
from typing import Any

import pytest
import yaml

KIT_ROOT = Path(__file__).resolve().parent.parent
CONVERGE = KIT_ROOT / "scripts" / "converge.py"
sys.path.insert(0, str(KIT_ROOT / "scripts"))

import converge  # noqa: E402


@pytest.fixture(scope="module")
def built_crate() -> None:
    """Build the workspace's crates, the way setup's own skip line asks for (docs/RUST.md)."""
    if shutil.which("cargo") is None:
        pytest.skip("Rust toolchain (cargo) is absent: gold-band-registry cannot be built and run")
    build = subprocess.run(
        ["cargo", "build", "--release", "--manifest-path", str(KIT_ROOT / "scripts" / "Cargo.toml")],
        capture_output=True, text=True, check=False,
    )
    assert build.returncode == 0, build.stderr


def _fake_agent(root: Path) -> Path:
    """The agent to start: echoes a line, writes its marker (argv 1) and exits.

    The exit code comes from ``$FAKE_AGENT_EXIT``, which the write case sets
    through the declaration's own ``adapter.env`` -- the environment a real
    ACP adapter is started with.
    """
    script = root / "fake-agent.sh"
    script.write_text(
        "#!/usr/bin/env bash\n"
        'echo "fake agent started: $*"\n'
        ': > "$1"\n'
        'exit "${FAKE_AGENT_EXIT:-0}"\n'
    )
    script.chmod(0o755)
    return script


def _launch(root: Path, agents: list[dict[str, Any]], reach_support: dict[str, Any],
            agent_id: str) -> subprocess.CompletedProcess[str]:
    """One gold-band-launch run against a registry the test wrote."""
    registry = root / "gold-band-launch.yaml"
    registry.write_text(yaml.safe_dump(
        {"gold_band": {"agents": agents, "reach_support": reach_support}}, sort_keys=False))
    return subprocess.run(
        [sys.executable, str(CONVERGE), "gold-band-launch", "--id", agent_id,
         "--registry", str(registry)],
        capture_output=True, text=True, check=False,
        env={**os.environ, "FAKE_AGENT_EXIT": ""},
    )


def _agent(name: str, marker: Path, command: Path, **fields: Any) -> dict[str, Any]:
    """A declared fake agent writing ``marker`` when it starts."""
    return {
        "id": name,
        "adapter": {"command": str(command), "args": [str(marker)], "displayName": "Fake"},
        "reach": "write",
        **fields,
    }


def _support() -> dict[str, Any]:
    """The cell states the launch cases need: write, research and full
    enforced for the fake vibe kind, read refused, hermes barely covered."""
    def verified() -> dict[str, Any]:
        return {"state": "verified", "evidence": "test"}

    return {
        "vibe": {
            "write": verified(),
            "research": verified(),
            "read": {"state": "unverified", "evidence": "test"},
            "full": verified(),
        },
        "hermes": {"write": verified()},
    }


def test_a_verified_write_id_starts_the_fake_agent_and_the_exit_code_passes_through(
    tmp_path: Path, built_crate: None
) -> None:
    """(a) The marker appears, the agent's line reaches stdout untouched, and
    the exit code of the run is the agent's own, set through adapter.env."""
    marker = tmp_path / "started.marker"
    agent = _agent("fake-write", marker, _fake_agent(tmp_path), agent="vibe")
    # The agent's exit code comes through the adapter's own environment,
    # the way an ACP launcher hands a config directory to its binary.
    agent["adapter"]["env"] = {"FAKE_AGENT_EXIT": "3"}
    run = _launch(tmp_path, [agent], _support(), "fake-write")
    assert run.returncode == 3, run.stdout + run.stderr
    assert marker.exists()
    assert "fake agent started:" in run.stdout


def test_launch_refuses_an_unknown_id_without_starting_anything(
    tmp_path: Path, built_crate: None
) -> None:
    marker = tmp_path / "unknown.marker"
    agent = _agent("fake-write", marker, _fake_agent(tmp_path))
    run = _launch(tmp_path, [agent], _support(), "no-such-id")
    assert run.returncode == 1, run.stdout + run.stderr
    assert "no-such-id: unknown agent id" in run.stderr
    assert not marker.exists()


def test_launch_refuses_a_reach_the_kind_does_not_support(
    tmp_path: Path, built_crate: None
) -> None:
    """hermes has no read cell at all, so there is nothing to enforce."""
    marker = tmp_path / "hermes.marker"
    agent = _agent("fake-hermes", marker, _fake_agent(tmp_path), agent="hermes", reach="read")
    run = _launch(tmp_path, [agent], _support(), "fake-hermes")
    assert run.returncode == 1, run.stdout + run.stderr
    assert "fake-hermes: agent hermes does not support reach read" in run.stderr
    assert not marker.exists()


def test_launch_refuses_an_unverified_cell(
    tmp_path: Path, built_crate: None
) -> None:
    marker = tmp_path / "read.marker"
    agent = _agent("fake-read", marker, _fake_agent(tmp_path), agent="vibe", reach="read")
    run = _launch(tmp_path, [agent], _support(), "fake-read")
    assert run.returncode == 1, run.stdout + run.stderr
    assert "fake-read: reach read for agent vibe is unverified; refused" in run.stderr
    assert "fake agent started" not in run.stdout
    assert not marker.exists()


def test_launch_refuses_a_research_id_without_an_output_dir(
    tmp_path: Path, built_crate: None
) -> None:
    marker = tmp_path / "research.marker"
    agent = _agent("fake-research", marker, _fake_agent(tmp_path), agent="vibe", reach="research")
    run = _launch(tmp_path, [agent], _support(), "fake-research")
    assert run.returncode == 1, run.stdout + run.stderr
    assert 'fake-research: reach "research" needs an outputDir field' in run.stderr
    assert not marker.exists()


def test_launch_refuses_a_full_id_that_is_not_throwaway_only(
    tmp_path: Path, built_crate: None
) -> None:
    marker = tmp_path / "full.marker"
    agent = _agent("fake-full", marker, _fake_agent(tmp_path), agent="vibe", reach="full")
    run = _launch(tmp_path, [agent], _support(), "fake-full")
    assert run.returncode == 1, run.stdout + run.stderr
    assert 'fake-full: reach "full" needs throwawayOnly "true"' in run.stderr
    assert not marker.exists()


def test_the_flattened_support_lines_are_sorted_and_carry_only_the_state() -> None:
    """One support line per cell, kinds and levels sorted, evidence never sent."""
    support = {
        "vibe": {"read": {"state": "unverified", "evidence": "a sentence"},
                 "write": {"state": "verified", "evidence": "a sentence"}},
        "agy": {"write": {"state": "unverified", "evidence": "a sentence"}},
    }
    assert converge.gold_band_support_lines(support) == (
        "support\tagy\twrite\tunverified\n"
        "support\tvibe\tread\tunverified\n"
        "support\tvibe\twrite\tverified\n"
    )


def test_a_launch_without_a_built_crate_fails_the_step(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """A launch is an action: no crate means the step failed, never a skip."""
    monkeypatch.setattr(converge, "KIT_ROOT", tmp_path)
    registry = tmp_path / "registry.yaml"
    registry.write_text(yaml.safe_dump({"gold_band": {"agents": [
        {"id": "fake-write", "agent": "vibe", "reach": "write"}]}}))
    args = argparse.Namespace(id="fake-write", registry=registry)
    assert converge.gold_band_launch(args, cargo=True) == (
        converge.GOLD_BAND_CANNOT_VALIDATE_EXIT
    )


def test_the_launcher_refuses_a_reach_that_is_not_a_known_level(
    tmp_path: Path, built_crate: None
) -> None:
    """A verified cell cannot make a non-level enforceable (review of #339)."""
    marker = tmp_path / "banana.marker"
    agent = _agent("fake-banana", marker, _fake_agent(tmp_path), agent="vibe", reach="banana")
    support = _support()
    support["vibe"]["banana"] = {"state": "verified", "evidence": "test"}
    run = _launch(tmp_path, [agent], support, "fake-banana")
    assert run.returncode == 1, run.stdout + run.stderr
    assert "fake-banana" in run.stderr
    assert not marker.exists()


def test_an_agent_killed_by_a_signal_comes_back_as_128_plus_the_signal(
    tmp_path: Path, built_crate: None
) -> None:
    """The exit code that comes back is the agent's own (#339), signal deaths
    included: 143 for SIGTERM, not Python's rewrite of the negative code."""
    script = tmp_path / "kills-itself.sh"
    script.write_text("#!/usr/bin/env bash\nkill -TERM $$\n")
    script.chmod(0o755)
    marker = tmp_path / "signal.marker"
    agent = _agent("fake-signal", marker, script, agent="vibe")
    run = _launch(tmp_path, [agent], _support(), "fake-signal")
    assert run.returncode == 143, run.stdout + run.stderr
