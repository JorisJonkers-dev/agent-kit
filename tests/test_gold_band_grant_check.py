"""The grant-check node, proved headless against the pinned Gold Band (issue #460).

Before every Codex node of a work-instance Gold Band workflow a grant-check node
runs the pinned wrapper ``gold-band-run-start grant-check``. It reserves one
Codex node start against the run's grant in the gateway's grant record (the
TypeScript gateway's ``grant reserve``, under the gateway's ledger lock) and
refuses when the reservation does not fit or the grant has lapsed.

The tests use the real gateway bundle (``skills/workstation/model-gateway``) in a
throw-away configuration, state and home directory, so the grants file is a
fake one outside any git repository (the gateway refuses a grant inside a
repository) and the real grants are never touched. No Codex and no model is
called: the Codex nodes are stub agents that write marker files.

* the lapse proof runs the real headless ``gold-band run`` (``GOLD_BAND_BIN``,
  in a throw-away ``GOLD_BAND_HOME``): a grant whose workflow ends while the
  first Codex node runs makes the next grant check fail, and the run pauses at
  the manual check (exit 2) before the second Codex node;
* the race proof starts many wrapper processes at once on a grant with room for
  fewer starts and counts how many pass.

They skip, with the reason, when node, cargo or (for the headless run)
``GOLD_BAND_BIN`` is missing; ``GOLD_BAND_RUN_START_BIN`` names a prebuilt wrapper.
"""

from __future__ import annotations

import json
import os
import shutil
import stat
import subprocess
import sys
from pathlib import Path
from typing import Any

import pytest

KIT_ROOT = Path(__file__).resolve().parent.parent
FIXTURE = KIT_ROOT / "tests" / "fixtures" / "gold-band" / "grant-check-proof.json"
STUB = KIT_ROOT / "tests" / "fixtures" / "gold_band_stub_acp.py"
MANIFEST = KIT_ROOT / "scripts" / "Cargo.toml"
GATEWAY_BUNDLE = KIT_ROOT / "skills" / "workstation" / "model-gateway" / "model-gateway.mjs"

GOLD_BAND_BIN = os.environ.get("GOLD_BAND_BIN")
NO_TOOLS = (
    shutil.which("node") is None
    or shutil.which("git") is None
    or (shutil.which("cargo") is None and not os.environ.get("GOLD_BAND_RUN_START_BIN"))
)
pytestmark = pytest.mark.skipif(
    NO_TOOLS, reason="node, git or cargo (or GOLD_BAND_RUN_START_BIN) is missing: the shim and the gateway cannot run"
)
needs_gold_band = pytest.mark.skipif(
    GOLD_BAND_BIN is None, reason="GOLD_BAND_BIN not set: point it at a headless gold-band binary (docs/GOLD-BAND.md)"
)

# Gold Band v0.19.1: CURRENT_SETTINGS_SCHEMA_VERSION in src/config/mod.rs.
SETTINGS_SCHEMA_VERSION = 12
TIMEOUT_SECONDS = 600
NEDAP = "https://github.com/nedap/healthcare.git"
WORKFLOW_NAME = "grant-check-proof"
RUN_ID = "run-1"
RUN_NAME = f"{WORKFLOW_NAME}@github.com/nedap/healthcare@{RUN_ID}"
REPORT_KEYS = {"run_id", "attempt", "command", "exit_code", "output_hash", "timestamp"}


@pytest.fixture(scope="module")
def wrapper() -> str:
    """The pinned wrapper: a prebuilt one, or built from this repository."""
    named = os.environ.get("GOLD_BAND_RUN_START_BIN")
    if named:
        return named
    subprocess.run(
        ["cargo", "build", "-q", "--manifest-path", str(MANIFEST), "-p", "gold-band-run-start"],
        check=True,
        capture_output=True,
        text=True,
        timeout=TIMEOUT_SECONDS,
    )
    target = os.environ.get("CARGO_TARGET_DIR") or str(KIT_ROOT / "scripts" / "target")
    return str(Path(target) / "debug" / "gold-band-run-start")


def _git(repo: Path, *args: str) -> None:
    subprocess.run(["git", "-C", str(repo), *args], check=True, capture_output=True, text=True)


class Machine:
    """A throw-away machine: home, configuration and state outside any repository, a work repository, a gateway."""

    def __init__(self, tmp: Path, wrapper: str) -> None:
        self.tmp = tmp
        self.wrapper = wrapper
        self.home = tmp / "home"
        self.config = tmp / "config"
        self.state = tmp / "state"
        self.repo = tmp / "repo"
        self.reports = tmp / "reports"
        for directory in (self.home, self.config, self.state, self.repo):
            directory.mkdir()
        _git(self.repo, "init", "-q")
        _git(self.repo, "remote", "add", "origin", NEDAP)
        self.gateway = tmp / "model-gateway"
        self.gateway.write_text(f'#!/bin/sh\nexec node "{GATEWAY_BUNDLE}" "$@"\n')
        self.gateway.chmod(self.gateway.stat().st_mode | stat.S_IXUSR)
        # The gateway reads these three, never the owner's real ones.
        self.env = dict(
            os.environ,
            HOME=str(self.home),
            XDG_CONFIG_HOME=str(self.config),
            XDG_STATE_HOME=str(self.state),
            GOLD_BAND_HOME=str(self.home),
        )
        self.env.pop("GOLD_BAND_MODEL_GATEWAY", None)

    @property
    def grants_file(self) -> Path:
        return self.config / "agent-kit" / "model-gateway-grants.json"

    def gateway_call(self, *args: str) -> dict[str, Any]:
        done = subprocess.run(
            [str(self.gateway), *args], capture_output=True, text=True, env=self.env, timeout=TIMEOUT_SECONDS, check=False
        )
        assert done.returncode == 0, f"model-gateway {' '.join(args)} failed: {done.stdout}{done.stderr}"
        result: dict[str, Any] = json.loads(done.stdout)
        return result

    def grant(self, starts: int) -> None:
        """The owner's grant for this run: its cap counts Codex node starts."""
        self.gateway_call("workflow", "start", "--json", json.dumps({"name": RUN_NAME}))
        self.gateway_call(
            "grant",
            "write",
            "--json",
            json.dumps(
                {
                    "workflow": RUN_NAME,
                    "client": "codex",
                    "cap_usd": starts,
                    "question": "may the proof run spend Codex node starts?",
                    "answer": f"yes, {starts} of them",
                }
            ),
        )

    def end_workflow(self) -> list[str]:
        """The command that ends the run's gateway workflow: its grant lapses with it."""
        return [str(self.gateway), "workflow", "end", "--json", json.dumps({"name": RUN_NAME})]

    def check_command(self, node: str, reports: Path) -> list[str]:
        """The pinned wrapper command line one grant-check node runs."""
        return [
            self.wrapper,
            "grant-check",
            "--repo",
            str(self.repo),
            "--workflow-name",
            WORKFLOW_NAME,
            "--run-id",
            RUN_ID,
            "--attempt",
            "1",
            "--node",
            node,
            "--gateway",
            str(self.gateway),
            "--report-dir",
            str(reports),
        ]

    def check(self, node: str, reports: Path | None = None) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            self.check_command(node, reports or self.reports / node),
            capture_output=True,
            text=True,
            env=self.env,
            timeout=TIMEOUT_SECONDS,
            check=False,
        )

    def reservations(self) -> list[dict[str, Any]]:
        """The reservation lines the gateway's ledger holds."""
        ledger = next(self.state.rglob("ledger*"), None)
        if ledger is None:
            return []
        lines = [json.loads(line) for line in ledger.read_text().splitlines() if line.strip()]
        return [line for line in lines if line.get("kind") == "reservation"]


@pytest.fixture
def machine(tmp_path: Path, wrapper: str) -> Machine:
    return Machine(tmp_path, wrapper)


def _report(path: Path) -> dict[str, Any]:
    report: dict[str, Any] = json.loads((path / "grant-check.json").read_text())
    return report


# ---- the reservation ----------------------------------------------------


def test_a_live_grant_lets_the_node_reserve_one_start(machine: Machine) -> None:
    machine.grant(2)
    done = machine.check("codex-1")
    assert done.returncode == 0, done.stdout + done.stderr
    report = _report(machine.reports / "codex-1")
    assert set(report) >= REPORT_KEYS and report["exit_code"] == 0 and "failed_check" not in report
    reserved = machine.reservations()
    assert len(reserved) == 1 and reserved[0]["workflow"] == RUN_NAME and reserved[0]["client"] == "codex"


def test_a_run_with_no_grant_is_refused(machine: Machine) -> None:
    done = machine.check("codex-1")
    assert done.returncode == 1, done.stdout + done.stderr
    assert _report(machine.reports / "codex-1")["failed_check"] == "grant-lapsed"
    assert machine.reservations() == []


def test_a_grant_whose_starts_are_used_up_refuses_the_next_node(machine: Machine) -> None:
    machine.grant(2)
    assert [machine.check(f"codex-{n}").returncode for n in (1, 2)] == [0, 0]
    third = machine.check("codex-3")
    assert third.returncode == 1, third.stdout + third.stderr
    assert _report(machine.reports / "codex-3")["failed_check"] == "grant-no-start-left"
    assert len(machine.reservations()) == 2


def test_a_node_whose_agent_runs_the_wrapper_twice_holds_one_start(machine: Machine) -> None:
    machine.grant(2)
    assert [machine.check("codex-1").returncode for _ in range(2)] == [0, 0]
    assert len(machine.reservations()) == 1
    assert machine.check("codex-2").returncode == 0
    assert len(machine.reservations()) == 2


def test_an_ended_workflow_lapses_its_grant(machine: Machine) -> None:
    machine.grant(5)
    assert machine.check("codex-1").returncode == 0
    subprocess.run(machine.end_workflow(), check=True, capture_output=True, env=machine.env)
    done = machine.check("codex-2")
    assert done.returncode == 1, done.stdout + done.stderr
    assert _report(machine.reports / "codex-2")["failed_check"] == "grant-lapsed"
    assert len(machine.reservations()) == 1


# ---- the race -----------------------------------------------------------


@pytest.mark.parametrize(("starts", "nodes"), [(1, 8), (3, 10)])
def test_parallel_nodes_cannot_pass_on_the_last_start(machine: Machine, starts: int, nodes: int) -> None:
    """Every node races for the grant at once; exactly `starts` of them pass."""
    machine.grant(starts)
    running = [
        subprocess.Popen(
            machine.check_command(f"codex-{n}", machine.reports / f"codex-{n}"),
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            env=machine.env,
        )
        for n in range(nodes)
    ]
    outcomes = []
    for process in running:
        out, err = process.communicate(timeout=TIMEOUT_SECONDS)
        outcomes.append((process.returncode, out + err))
    passed = [code for code, _ in outcomes if code == 0]
    assert len(passed) == starts, outcomes
    assert all(code == 1 for code, _ in outcomes if code != 0), outcomes
    assert len(machine.reservations()) == starts
    refused = [_report(machine.reports / f"codex-{n}") for n in range(nodes)]
    assert sorted(r["exit_code"] for r in refused).count(0) == starts
    assert {r["failed_check"] for r in refused if r["exit_code"] != 0} <= {"grant-no-start-left"}


# ---- a grant that lapses mid-run pauses the run -------------------------


def _agent(display: str, env: dict[str, str]) -> dict[str, Any]:
    return {
        "adapter": {
            "command": sys.executable,
            "args": [str(STUB)],
            "displayName": display,
            "env": {"STUB_ACP_MODELS": "stub-model", **env},
        }
    }


@needs_gold_band
def test_a_grant_that_lapses_mid_run_pauses_the_run_at_the_next_codex_node(machine: Machine) -> None:
    machine.grant(5)
    workflow = machine.tmp / "workflow.json"
    markers = {name: machine.tmp / f"{name}.marker" for name in ("codex-1", "codex-2", "manual")}
    agents = {
        "GRANT_AGENT_1": "grant-check-1",
        "GRANT_AGENT_2": "grant-check-2",
        "CODEX_AGENT_1": "codex-1",
        "CODEX_AGENT_2": "codex-2",
        "MANUAL_AGENT": "manual",
    }
    text = FIXTURE.read_text()
    for placeholder, agent_id in agents.items():
        text = text.replace(placeholder, f"stub-{agent_id}")
    workflow.write_text(text)
    settings: dict[str, Any] = {"settingsSchemaVersion": SETTINGS_SCHEMA_VERSION, "agents": {}}
    for node in ("grant-check-1", "grant-check-2"):
        reports = machine.reports / node
        number = node.rsplit("-", 1)[1]
        settings["agents"][f"stub-{node}"] = _agent(
            node,
            {
                "STUB_ACP_RUN": json.dumps(machine.check_command(f"codex-{number}", reports)),
                "STUB_ACP_ANSWER": str(reports / "grant-check.json"),
            },
        )
    # The first Codex node works; while it does, the run's gateway workflow
    # ends and its grant lapses with it.
    settings["agents"]["stub-codex-1"] = _agent(
        "codex-1",
        {"STUB_ACP_MARKER": str(markers["codex-1"]), "STUB_ACP_AFTER": json.dumps(machine.end_workflow())},
    )
    settings["agents"]["stub-codex-2"] = _agent("codex-2", {"STUB_ACP_MARKER": str(markers["codex-2"])})
    settings["agents"]["stub-manual"] = _agent("manual", {"STUB_ACP_MARKER": str(markers["manual"])})
    band_home = machine.home / ".gold-band"
    band_home.mkdir()
    (band_home / "settings.json").write_text(json.dumps(settings))
    requirement = machine.repo / "requirement.md"
    requirement.write_text("# Grant-check proof\n\nThe requirement is not read by the stubs.\n")
    assert GOLD_BAND_BIN is not None
    done = subprocess.run(
        [GOLD_BAND_BIN, "run", "--requirement-file", str(requirement), "--workflow", str(workflow)],
        capture_output=True,
        text=True,
        timeout=TIMEOUT_SECONDS,
        env=machine.env,
        cwd=str(machine.repo),
        check=False,
    )
    said = f"exit {done.returncode}\nstdout:\n{done.stdout[-3000:]}\nstderr:\n{done.stderr[-3000:]}"
    # Paused at the manual check: exit 2, and the second Codex node never ran.
    assert done.returncode == 2, said
    assert markers["codex-1"].exists(), "the first Codex node did not run on its live grant"
    assert not markers["codex-2"].exists(), "the second Codex node ran after its grant lapsed"
    assert _report(machine.reports / "grant-check-1")["exit_code"] == 0
    refused = _report(machine.reports / "grant-check-2")
    assert refused["exit_code"] == 1 and refused["failed_check"] == "grant-lapsed"
    assert len(machine.reservations()) == 1
