"""The gate shim, proved headless against the pinned Gold Band (issue #445).

Gold Band has no command node, so a deterministic gate is a shim: a cheap
worker whose agent runs only the pinned wrapper ``gold-band-gate run``, which
writes a gate report the gate agent cannot write, followed by a read-only
copier worker that runs ``gold-band-gate verify`` and fails the run when the
report is missing, stale, from another run or attempt, or does not match
(docs/GOLD-BAND.md, "The gate shim"). The fixture
``tests/fixtures/gold-band/gate-proof.json`` is that shape: ``gate`` ->
``copy`` -> ``model-work``, the last a worker whose stub writes a marker file
when it runs. Neither shim node has a failure edge, so any failure ends the
run before the marker exists.

The stub gate agent (``tests/fixtures/gold_band_stub_acp.py``) answers with the
wrapper's exit status only. Real runs deny the gate agent id writes to the
report directory (the launcher's OS write limit, ticket #432); here the stub
does those writes, after the wrapper, to prove the copier catches each of
them. Nothing about the confinement itself is proved here.

The tests run the real headless ``gold-band run`` in a temporary git
repository with a temporary ``GOLD_BAND_HOME`` and ``HOME``. No model and no
network are used. They skip, with the reason, when ``GOLD_BAND_BIN`` is unset
or cargo is absent (the wrapper is built from this repository unless
``GOLD_BAND_GATE_BIN`` names one).
"""

from __future__ import annotations

import json
import os
import shutil
import subprocess
import sys
from pathlib import Path
from typing import Any

import pytest

KIT_ROOT = Path(__file__).resolve().parent.parent
FIXTURE = KIT_ROOT / "tests" / "fixtures" / "gold-band" / "gate-proof.json"
STUB = KIT_ROOT / "tests" / "fixtures" / "gold_band_stub_acp.py"
MANIFEST = KIT_ROOT / "scripts" / "Cargo.toml"

GOLD_BAND_BIN = os.environ.get("GOLD_BAND_BIN")
SKIP_NO_GOLD_BAND = "GOLD_BAND_BIN not set: point it at a headless gold-band binary (docs/GOLD-BAND.md)"
SKIP_NO_CARGO = "cargo not on PATH and GOLD_BAND_GATE_BIN not set: the pinned wrapper cannot be built"
NO_HEADLESS_PROOF = GOLD_BAND_BIN is None or (shutil.which("cargo") is None and not os.environ.get("GOLD_BAND_GATE_BIN"))
HEADLESS_SKIP_REASON = SKIP_NO_GOLD_BAND if GOLD_BAND_BIN is None else SKIP_NO_CARGO
pytestmark = pytest.mark.skipif(NO_HEADLESS_PROOF, reason=HEADLESS_SKIP_REASON)

# Gold Band v0.19.1: CURRENT_SETTINGS_SCHEMA_VERSION in src/config/mod.rs.
SETTINGS_SCHEMA_VERSION = 12
TIMEOUT_SECONDS = 600
# gold-band run: 0 success, 1 failure, 2 paused at a manual check.
GOLD_BAND_FAILURE_EXIT = 1
REPORT_KEYS = {"run_id", "attempt", "command", "exit_code", "output_hash", "timestamp"}
HASH_DIGITS = set("0123456789abcdef")


@pytest.fixture(scope="module")
def wrapper() -> str:
    """The pinned wrapper: a prebuilt one, or built from this repository."""
    named = os.environ.get("GOLD_BAND_GATE_BIN")
    if named:
        return named
    subprocess.run(
        ["cargo", "build", "-q", "--manifest-path", str(MANIFEST), "-p", "gold-band-gate"],
        check=True,
        capture_output=True,
        text=True,
        timeout=TIMEOUT_SECONDS,
    )
    target = os.environ.get("CARGO_TARGET_DIR") or str(KIT_ROOT / "scripts" / "target")
    return str(Path(target) / "debug" / "gold-band-gate")


def _agent(display: str, env: dict[str, str]) -> dict[str, Any]:
    return {
        "adapter": {
            "command": sys.executable,
            "args": [str(STUB)],
            "displayName": display,
            "env": {"STUB_ACP_MODELS": "stub-model", **env},
        }
    }


class Run:
    """One headless run of the gate proof workflow and what it left behind."""

    def __init__(self, tmp: Path, wrapper: str, command: list[str], timeout_secs: int = 30) -> None:
        self.tmp = tmp
        self.wrapper = wrapper
        self.command = command
        self.timeout_secs = timeout_secs
        self.home = tmp / "home"
        self.config = tmp / "config"
        self.repo = tmp / "repo"
        self.reports = tmp / "reports"
        self.marker = tmp / "model-ran.marker"
        self.run_id = "run-1"
        self.attempt = 1
        # What the copier is told to expect; the tests change these to
        # play a report from another run or attempt, or for another command.
        self.verify_run_id = self.run_id
        self.verify_attempt = self.attempt
        self.verify_command = " ".join(command)
        self.verify_extra: list[str] = []
        self.tamper: str | None = None
        self.repo.mkdir()
        (self.config / "agent-kit").mkdir(parents=True)
        subprocess.run(["git", "-C", str(self.repo), "init", "-q"], check=True, capture_output=True, text=True)

    def gate_command(self) -> list[str]:
        """The pinned wrapper command line the gate node runs."""
        return [
            self.wrapper,
            "run",
            "--run-id",
            self.run_id,
            "--attempt",
            str(self.attempt),
            "--timeout-secs",
            str(self.timeout_secs),
            "--report-dir",
            str(self.reports),
            "--",
            *self.command,
        ]

    def copy_command(self) -> list[str]:
        """The read-only copier command line the copy node runs."""
        return [
            self.wrapper,
            "verify",
            "--run-id",
            self.verify_run_id,
            "--attempt",
            str(self.verify_attempt),
            "--command",
            self.verify_command,
            "--report-dir",
            str(self.reports),
            *self.verify_extra,
        ]

    def start(self) -> subprocess.CompletedProcess[str]:
        workflow = self.tmp / "workflow.json"
        workflow.write_text(
            FIXTURE.read_text()
            .replace("GATE_AGENT", "stub-gate")
            .replace("COPY_AGENT", "stub-copy")
            .replace("MODEL_AGENT", "stub-model-agent")
        )
        band_home = self.home / ".gold-band"
        band_home.mkdir(parents=True)
        gate_env = {
            "STUB_ACP_GATE": json.dumps(self.gate_command()),
            "STUB_ACP_REPORT_DIR": str(self.reports),
        }
        if self.tamper:
            gate_env["STUB_ACP_TAMPER"] = self.tamper
        # Without the current schema version Gold Band migrates the settings
        # from version 0 and silently loads no custom agents.
        settings = {
            "settingsSchemaVersion": SETTINGS_SCHEMA_VERSION,
            "agents": {
                "stub-gate": _agent("Stub ACP (gate)", gate_env),
                "stub-copy": _agent("Stub ACP (copier)", {"STUB_ACP_COPY": json.dumps(self.copy_command())}),
                "stub-model-agent": _agent("Stub ACP (model)", {"STUB_ACP_MARKER": str(self.marker)}),
            },
        }
        (band_home / "settings.json").write_text(json.dumps(settings))
        requirement = self.repo / "requirement.md"
        requirement.write_text("# Gate proof\n\nThe requirement is not read by the stubs.\n")
        assert GOLD_BAND_BIN is not None
        env = dict(os.environ, GOLD_BAND_HOME=str(self.home), HOME=str(self.home), XDG_CONFIG_HOME=str(self.config))
        return subprocess.run(
            [GOLD_BAND_BIN, "run", "--requirement-file", str(requirement), "--workflow", str(workflow)],
            capture_output=True,
            text=True,
            timeout=TIMEOUT_SECONDS,
            env=env,
            cwd=str(self.repo),
            check=False,
        )

    def report(self) -> dict[str, Any]:
        report: dict[str, Any] = json.loads((self.reports / "gate-report.json").read_text())
        return report


def _said(run: subprocess.CompletedProcess[str]) -> str:
    return f"exit {run.returncode}\nstdout:\n{run.stdout[-3000:]}\nstderr:\n{run.stderr[-3000:]}"


def _failed(run: subprocess.CompletedProcess[str], proof: Run) -> None:
    assert run.returncode == GOLD_BAND_FAILURE_EXIT, _said(run)
    assert not proof.marker.exists(), "the model node ran after a failed gate"


def _assert_report(report: dict[str, Any], proof: Run, exit_code: int, failed_check: str | None) -> None:
    assert set(report) >= REPORT_KEYS
    assert report["run_id"] == proof.run_id and report["attempt"] == proof.attempt
    assert report["command"] == " ".join(proof.command)
    assert report["exit_code"] == exit_code
    assert len(report["output_hash"]) == 64 and set(report["output_hash"]) <= HASH_DIGITS
    assert report["timestamp"].endswith("Z") and "T" in report["timestamp"]
    assert report.get("failed_check") == failed_check


# ---- the gate passes -----------------------------------------------------


def test_a_green_command_reaches_the_model_node(tmp_path: Path, wrapper: str) -> None:
    proof = Run(tmp_path, wrapper, ["sh", "-c", "echo built; echo warned >&2"])
    run = proof.start()
    assert run.returncode == 0, _said(run)
    assert proof.marker.exists(), "the model node did not run after a green gate"
    _assert_report(proof.report(), proof, 0, None)
    assert (proof.reports / "gate-output.txt").read_text() == "built\nwarned\n"


# ---- the gate fails ------------------------------------------------------


def test_a_red_command_ends_the_run_in_failure(tmp_path: Path, wrapper: str) -> None:
    proof = Run(tmp_path, wrapper, ["sh", "-c", "echo broken; exit 3"])
    _failed(proof.start(), proof)
    _assert_report(proof.report(), proof, 3, None)


def test_a_command_that_cannot_start_ends_the_run_in_failure(tmp_path: Path, wrapper: str) -> None:
    proof = Run(tmp_path, wrapper, ["/no/such/gate-command", "--flag"])
    _failed(proof.start(), proof)
    _assert_report(proof.report(), proof, 127, "cannot-start")


def test_a_hung_command_times_out_into_a_failed_report(tmp_path: Path, wrapper: str) -> None:
    proof = Run(tmp_path, wrapper, ["sleep", "30"], timeout_secs=1)
    _failed(proof.start(), proof)
    _assert_report(proof.report(), proof, 124, "timeout")


# ---- the report is tampered with between the gate and the copier -----------


def test_a_deleted_report_ends_the_run_in_failure(tmp_path: Path, wrapper: str) -> None:
    proof = Run(tmp_path, wrapper, ["true"])
    proof.tamper = "delete-report"
    _failed(proof.start(), proof)
    assert not (proof.reports / "gate-report.json").exists()


def test_rewritten_output_ends_the_run_in_failure(tmp_path: Path, wrapper: str) -> None:
    proof = Run(tmp_path, wrapper, ["sh", "-c", "echo real output"])
    proof.tamper = "rewrite-output"
    _failed(proof.start(), proof)


def test_a_rewritten_command_ends_the_run_in_failure(tmp_path: Path, wrapper: str) -> None:
    proof = Run(tmp_path, wrapper, ["sh", "-c", "exit 0"])
    proof.tamper = "rewrite-command"
    _failed(proof.start(), proof)


def test_a_stale_report_ends_the_run_in_failure(tmp_path: Path, wrapper: str) -> None:
    proof = Run(tmp_path, wrapper, ["true"])
    proof.tamper = "age-report"
    _failed(proof.start(), proof)


def test_a_report_from_another_attempt_ends_the_run_in_failure(tmp_path: Path, wrapper: str) -> None:
    proof = Run(tmp_path, wrapper, ["true"])
    proof.verify_attempt = 2
    _failed(proof.start(), proof)


def test_a_report_from_another_run_ends_the_run_in_failure(tmp_path: Path, wrapper: str) -> None:
    proof = Run(tmp_path, wrapper, ["true"])
    proof.verify_run_id = "run-0"
    _failed(proof.start(), proof)


def test_a_copier_that_expects_another_command_ends_the_run_in_failure(tmp_path: Path, wrapper: str) -> None:
    proof = Run(tmp_path, wrapper, ["true"])
    proof.verify_command = "cargo test --locked"
    _failed(proof.start(), proof)


def test_a_report_older_than_the_copiers_max_age_ends_the_run_in_failure(tmp_path: Path, wrapper: str) -> None:
    proof = Run(tmp_path, wrapper, ["true"])
    proof.verify_extra = ["--now", "2099-01-01T00:00:00Z"]
    _failed(proof.start(), proof)
