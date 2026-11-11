"""The run-start shim, proved headless against the pinned Gold Band (issue #444).

Every Gold Band workflow opens with a cheap node that runs the pinned wrapper
``gold-band-run-start`` (docs/GOLD-BAND.md). The fixture
``tests/fixtures/gold-band/run-start-proof.json`` is that shape: node one is a
worker whose stub ACP agent runs the wrapper and answers with the report the
wrapper wrote, its success condition is ``exit_code`` equal to ``0`` and it
has no failure edge; node two is a "model" worker whose stub agent writes a
marker file when it runs. A refused check therefore ends the run in failure,
and the marker file never exists.

The tests run the real headless ``gold-band run`` in a temporary git
repository with a temporary ``GOLD_BAND_HOME`` and ``HOME``, the stub
(``tests/fixtures/gold_band_stub_acp.py``) declared as the agents in the
temporary ``settings.json``. No model and no network are used. They skip, with
the reason, when ``GOLD_BAND_BIN`` is unset or cargo is absent (the wrapper is
built from this repository unless ``GOLD_BAND_RUN_START_BIN`` names one).
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
FIXTURE = KIT_ROOT / "tests" / "fixtures" / "gold-band" / "run-start-proof.json"
STUB = KIT_ROOT / "tests" / "fixtures" / "gold_band_stub_acp.py"
MANIFEST = KIT_ROOT / "scripts" / "Cargo.toml"

GOLD_BAND_BIN = os.environ.get("GOLD_BAND_BIN")
SKIP_NO_GOLD_BAND = "GOLD_BAND_BIN not set: point it at a headless gold-band binary (docs/GOLD-BAND.md)"
SKIP_NO_CARGO = "cargo not on PATH and GOLD_BAND_RUN_START_BIN not set: the pinned wrapper cannot be built"
NO_HEADLESS_PROOF = GOLD_BAND_BIN is None or (
    shutil.which("cargo") is None and not os.environ.get("GOLD_BAND_RUN_START_BIN")
)
HEADLESS_SKIP_REASON = SKIP_NO_GOLD_BAND if GOLD_BAND_BIN is None else SKIP_NO_CARGO
pytestmark = pytest.mark.skipif(NO_HEADLESS_PROOF, reason=HEADLESS_SKIP_REASON)

# Gold Band v0.19.1: CURRENT_SETTINGS_SCHEMA_VERSION in src/config/mod.rs.
SETTINGS_SCHEMA_VERSION = 12
TIMEOUT_SECONDS = 600
# gold-band run: 0 success, 1 failure, 2 paused at a manual check.
GOLD_BAND_FAILURE_EXIT = 1

NEDAP = "https://github.com/nedap/healthcare.git"
OTHER = "https://github.com/someone/personal-project.git"
OTHER_KEY = "github.com/someone/personal-project"
WORKFLOW_NAME = "run-start-proof"

# Work instance agents are the declared ones (ADR-0007); personal ids are free.
PERSONAL_AGENTS = ("stub-shim", "stub-model")
WORK_AGENTS = ("claude-work", "codex")
FAMILIES = "stub-shim\tstubfam\nstub-model\tmistral\nclaude-work\tanthropic\ncodex\topenai\n"
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
    """One headless run of the proof workflow and what it left behind."""

    def __init__(self, tmp: Path, wrapper: str, instance: str, agents: tuple[str, str]) -> None:
        self.tmp = tmp
        self.wrapper = wrapper
        self.instance = instance
        self.shim_agent, self.model_agent = agents
        self.home = tmp / "home"
        self.config = tmp / "config"
        self.repo = tmp / "repo"
        self.reports = tmp / "reports"
        self.marker = tmp / "model-ran.marker"
        self.allowlist = self.config / "agent-kit" / "critique-allowlist.json"
        self.grants = self.config / "agent-kit" / "model-gateway-grants.json"
        self.families = tmp / "families.tsv"
        self.run_id = "run-1"
        self.attempt = 1
        self.repo.mkdir()
        (self.config / "agent-kit").mkdir(parents=True)
        self.families.write_text(FAMILIES)
        _git(self.repo, "init", "-q")

    def remote(self, name: str, url: str) -> None:
        _git(self.repo, "remote", "add", name, url)

    def write_workflow(self) -> Path:
        text = FIXTURE.read_text().replace("SHIM_AGENT", self.shim_agent).replace("MODEL_AGENT", self.model_agent)
        path = self.tmp / "workflow.json"
        path.write_text(text)
        return path

    def command(self, workflow: Path) -> list[str]:
        """The pinned wrapper command line the shim node runs."""
        return [
            self.wrapper,
            "check",
            "--instance",
            self.instance,
            "--repo",
            str(self.repo),
            "--workflow-name",
            WORKFLOW_NAME,
            "--run-id",
            self.run_id,
            "--attempt",
            str(self.attempt),
            "--workflow",
            str(workflow),
            "--families",
            str(self.families),
            "--allowlist",
            str(self.allowlist),
            "--grants",
            str(self.grants),
            "--report-dir",
            str(self.reports),
        ]

    def start(self) -> subprocess.CompletedProcess[str]:
        workflow = self.write_workflow()
        band_home = self.home / ".gold-band"
        band_home.mkdir(parents=True)
        # Without the current schema version Gold Band migrates the settings
        # from version 0 and silently loads no custom agents.
        settings = {
            "settingsSchemaVersion": SETTINGS_SCHEMA_VERSION,
            "agents": {
                self.shim_agent: _agent(
                    "Stub ACP (shim)",
                    {
                        "STUB_ACP_RUN": json.dumps(self.command(workflow)),
                        "STUB_ACP_ANSWER": str(self.reports / "run-start.json"),
                    },
                ),
                self.model_agent: _agent("Stub ACP (model)", {"STUB_ACP_MARKER": str(self.marker)}),
            },
        }
        (band_home / "settings.json").write_text(json.dumps(settings))
        requirement = self.repo / "requirement.md"
        requirement.write_text("# Run-start proof\n\nThe requirement is not read by the stubs.\n")
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
        report: dict[str, Any] = json.loads((self.reports / "run-start.json").read_text())
        return report


def _said(run: subprocess.CompletedProcess[str]) -> str:
    return f"exit {run.returncode}\nstdout:\n{run.stdout[-3000:]}\nstderr:\n{run.stderr[-3000:]}"


def _assert_report(report: dict[str, Any], run_id: str, exit_code: int, failed_check: str | None) -> None:
    assert set(report) >= REPORT_KEYS
    assert report["run_id"] == run_id and report["attempt"] == 1
    assert report["command"].startswith("gold-band-run-start check --instance ")
    assert report["exit_code"] == exit_code
    assert len(report["output_hash"]) == 64 and set(report["output_hash"]) <= set("0123456789abcdef")
    assert report["timestamp"].endswith("Z") and "T" in report["timestamp"]
    assert report.get("failed_check") == failed_check


def _refused(run: subprocess.CompletedProcess[str], proof: Run, check: str) -> None:
    assert run.returncode == GOLD_BAND_FAILURE_EXIT, _said(run)
    assert not proof.marker.exists(), "the model node ran after a refused run-start check"
    _assert_report(proof.report(), proof.run_id, 1, check)


def _passed(run: subprocess.CompletedProcess[str], proof: Run) -> None:
    assert run.returncode == 0, _said(run)
    assert proof.marker.exists(), "the model node did not run after a passing run-start check"
    _assert_report(proof.report(), proof.run_id, 0, None)


# ---- the passing runs ----------------------------------------------------


def test_a_personal_run_on_a_personal_repository_reaches_the_model_node(tmp_path: Path, wrapper: str) -> None:
    proof = Run(tmp_path, wrapper, "gold-band-personal", PERSONAL_AGENTS)
    proof.remote("origin", OTHER)
    _passed(proof.start(), proof)


def test_a_personal_run_with_no_remote_reaches_the_model_node(tmp_path: Path, wrapper: str) -> None:
    proof = Run(tmp_path, wrapper, "gold-band-personal", PERSONAL_AGENTS)
    _passed(proof.start(), proof)


def test_a_work_run_with_a_live_grant_reaches_the_model_node(tmp_path: Path, wrapper: str) -> None:
    proof = Run(tmp_path, wrapper, "gold-band", WORK_AGENTS)
    proof.remote("origin", NEDAP)
    name = f"{WORKFLOW_NAME}@github.com/nedap/healthcare@{proof.run_id}"
    proof.grants.write_text(
        json.dumps(
            {
                "version": 1,
                "grants": [
                    {
                        "answer": "yes",
                        "cap_usd": 5,
                        "client": "codex",
                        "expires_at": "2099-01-01T00:00:00Z",
                        "id": "g-1",
                        "question": "may codex run?",
                        "workflow": name,
                        "written_at": "2026-11-11T08:00:00Z",
                    }
                ],
            }
        )
    )
    _passed(proof.start(), proof)


# ---- (a) a repository of the wrong instance --------------------------------


def test_the_personal_instance_refuses_a_nedap_repository(tmp_path: Path, wrapper: str) -> None:
    proof = Run(tmp_path, wrapper, "gold-band-personal", PERSONAL_AGENTS)
    proof.remote("origin", NEDAP)
    _refused(proof.start(), proof, "repository-rule")


def test_the_personal_instance_refuses_a_second_remote_owned_by_nedap(tmp_path: Path, wrapper: str) -> None:
    proof = Run(tmp_path, wrapper, "gold-band-personal", PERSONAL_AGENTS)
    proof.remote("origin", OTHER)
    proof.remote("work", "git@github.com:Nedap/healthcare.git")
    _refused(proof.start(), proof, "repository-rule")


def test_the_work_instance_refuses_a_repository_that_is_not_nedap(tmp_path: Path, wrapper: str) -> None:
    proof = Run(tmp_path, wrapper, "gold-band", WORK_AGENTS)
    proof.remote("origin", OTHER)
    _refused(proof.start(), proof, "repository-rule")


def test_the_work_instance_refuses_a_repository_with_no_remote(tmp_path: Path, wrapper: str) -> None:
    proof = Run(tmp_path, wrapper, "gold-band", WORK_AGENTS)
    _refused(proof.start(), proof, "repository-rule")


# ---- (b) a forbidden family -------------------------------------------------


def test_an_entry_without_the_bound_agents_family_refuses_the_run(tmp_path: Path, wrapper: str) -> None:
    proof = Run(tmp_path, wrapper, "gold-band-personal", PERSONAL_AGENTS)
    proof.remote("origin", OTHER)
    proof.allowlist.write_text(
        json.dumps(
            {
                "version": 1,
                "repositories": {
                    OTHER_KEY: {
                        "code": True,
                        "families": ["stubfam"],
                        "history": False,
                        "updated_at": "2026-11-01T00:00:00Z",
                    }
                },
            }
        )
    )
    _refused(proof.start(), proof, "family-allowlist")


def test_an_entry_without_code_refuses_the_run(tmp_path: Path, wrapper: str) -> None:
    proof = Run(tmp_path, wrapper, "gold-band-personal", PERSONAL_AGENTS)
    proof.remote("origin", OTHER)
    proof.allowlist.write_text(
        json.dumps(
            {
                "version": 1,
                "repositories": {
                    OTHER_KEY: {
                        "code": False,
                        "families": ["stubfam", "mistral"],
                        "history": False,
                        "updated_at": "2026-11-01T00:00:00Z",
                    }
                },
            }
        )
    )
    _refused(proof.start(), proof, "family-allowlist")


# ---- (c) a work run with no grant for the bound Codex agent ------------------


def test_a_work_run_with_no_grant_for_codex_is_refused(tmp_path: Path, wrapper: str) -> None:
    proof = Run(tmp_path, wrapper, "gold-band", WORK_AGENTS)
    proof.remote("origin", NEDAP)
    _refused(proof.start(), proof, "grant")


def test_a_grant_for_another_run_does_not_cover_this_one(tmp_path: Path, wrapper: str) -> None:
    proof = Run(tmp_path, wrapper, "gold-band", WORK_AGENTS)
    proof.remote("origin", NEDAP)
    other = f"{WORKFLOW_NAME}@github.com/nedap/healthcare@run-0"
    proof.grants.write_text(
        json.dumps(
            {
                "version": 1,
                "grants": [
                    {
                        "answer": "yes",
                        "cap_usd": 5,
                        "client": "codex",
                        "expires_at": "2099-01-01T00:00:00Z",
                        "id": "g-0",
                        "question": "may codex run?",
                        "workflow": other,
                        "written_at": "2026-11-11T08:00:00Z",
                    }
                ],
            }
        )
    )
    _refused(proof.start(), proof, "grant")
