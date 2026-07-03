#!/usr/bin/env python3
from __future__ import annotations

import importlib.util
import json
import shutil
import subprocess
import sys
from pathlib import Path
from types import SimpleNamespace
from typing import Any

ROOT = Path(__file__).resolve().parents[4]
OUT = Path(__file__).resolve().parent / "python-runs"
COUNCIL_PY = ROOT / "council" / "council.py"


SCENARIOS: list[dict[str, Any]] = [
    {
        "id": "legacy-ordinal-ids",
        "brief": "Keep legacy ordinal task ids stable.",
        "tasks": [
            {
                "id": "T1",
                "title": "Preserve first ordinal task",
                "objective": "Exercise a legacy T1 task id.",
                "output_format": "A committed deterministic worker result.",
                "paths": ["legacy/t1.txt"],
                "depends_on": [],
                "difficulty": "trivial",
                "model": "haiku",
                "verify": "test -f legacy/t1.txt",
                "boundaries": "Only touch legacy/t1.txt.",
            },
            {
                "id": "T2",
                "title": "Preserve dependent ordinal task",
                "objective": "Exercise a legacy T2 task id that depends on T1.",
                "output_format": "A committed deterministic worker result.",
                "paths": ["legacy/t2.txt"],
                "depends_on": ["T1"],
                "difficulty": "moderate",
                "model": "sonnet",
                "verify": "test -f legacy/t2.txt",
                "boundaries": "Only touch legacy/t2.txt.",
            },
        ],
    },
    {
        "id": "watchdog-table-config",
        "brief": "Update a [watchdog] table-bearing config without losing the table.",
        "tasks": [
            {
                "id": "ck-a100",
                "title": "Touch watchdog table config",
                "objective": "Exercise a task that mentions a [watchdog] TOML table.",
                "output_format": "A deterministic result for a table-bearing config.",
                "paths": ["config/service.toml"],
                "depends_on": [],
                "difficulty": "moderate",
                "model": "haiku",
                "verify": "rg '\\[watchdog\\]' config/service.toml",
                "boundaries": "Keep edits inside config/service.toml.",
                "acceptance_criteria": ["The [watchdog] table remains present."],
                "dev_notes": "[watchdog]\ninterval = \"30s\"",
            }
        ],
    },
    {
        "id": "grown-schema-task",
        "brief": "Carry grown task schema metadata through plan and fanout.",
        "tasks": [
            {
                "id": "ck-b200",
                "title": "Retain grown schema fields",
                "objective": "Exercise newer optional task metadata fields.",
                "output_format": "A deterministic result with metadata preserved.",
                "paths": ["schema/grown.json"],
                "depends_on": [],
                "difficulty": "hard",
                "model": "opus",
                "verify": "node -e 'JSON.parse(require(\"fs\").readFileSync(\"schema/grown.json\", \"utf8\"))'",
                "boundaries": "Only touch schema/grown.json.",
                "acceptance_criteria": ["Optional metadata round-trips through tasks.md."],
                "spec_ref": "specs/001-grown-schema-task",
                "context_refs": ["kb://council/tasks/grown-schema"],
                "archetype": "schema-maintenance",
                "context_profile": "contracts",
                "discovered_from": "golden-fixture",
                "supersedes": [],
                "content_hash": "sha256:fixture-grown-schema-task",
                "engine": {"cli": "codex", "model": "gpt-5.5"},
                "model_tier": "expensive",
            }
        ],
    },
]


def load_council() -> Any:
    original_run = subprocess.run

    def fake_run(cmd: Any, *args: Any, **kwargs: Any) -> subprocess.CompletedProcess[str]:
        if cmd == ["git", "rev-parse", "--show-toplevel"]:
            return subprocess.CompletedProcess(cmd, 0, stdout=str(ROOT), stderr="")
        return original_run(cmd, *args, **kwargs)

    subprocess.run = fake_run
    try:
        spec = importlib.util.spec_from_file_location("fixture_council", COUNCIL_PY)
        if spec is None or spec.loader is None:
            raise RuntimeError(f"cannot load {COUNCIL_PY}")
        module = importlib.util.module_from_spec(spec)
        sys.modules[spec.name] = module
        spec.loader.exec_module(module)
        return module
    finally:
        subprocess.run = original_run


def plan_payload(scenario: dict[str, Any], label: str) -> dict[str, Any]:
    return {
        "summary": f"{scenario['id']} plan from {label}",
        "approach": "Use deterministic canned model output.",
        "steps": ["plan", "fanout"],
        "risks": [],
        "parallelizable_tasks": [task["id"] for task in scenario["tasks"]],
        "open_questions": [],
        "engine": {"label": label},
        "model_tier": "fixture",
    }


def consolidated_payload(scenario: dict[str, Any]) -> dict[str, Any]:
    title = scenario["id"].replace("-", " ").title()
    return {
        "consolidated_plan_markdown": f"# {title}\n\nDeterministic consolidated fixture.\n",
        "spec_markdown": f"# Feature Specification: {title}\n\n{scenario['brief']}\n",
        "implementation_plan_markdown": f"# Implementation Plan: {title}\n\nFixture plan.\n",
        "tasks": scenario["tasks"],
    }


class FakeGit:
    def __init__(self) -> None:
        self.heads: dict[str, str] = {}

    def __call__(
        self,
        *args: str,
        cwd: Path | None = None,
        check: bool = True,
        timeout: int = 120,
    ) -> subprocess.CompletedProcess[str]:
        del check, timeout
        cwd_s = str(cwd) if cwd is not None else str(ROOT)
        stdout = ""

        if args[:2] == ("rev-parse", "HEAD"):
            stdout = self.heads.get(cwd_s, "0123456789abcdef0123456789abcdef01234567") + "\n"
        elif args[:2] == ("worktree", "add"):
            path = Path(args[-2])
            ref = args[-1]
            path.mkdir(parents=True, exist_ok=True)
            self.heads[str(path)] = self.heads.get(ref, "0123456789abcdef0123456789abcdef01234567")
        elif args[:2] == ("worktree", "remove"):
            shutil.rmtree(args[-1], ignore_errors=True)
        elif args[:2] == ("status", "--porcelain"):
            stdout = "M fixture\n" if "--" not in args else ""
        elif args[:2] == ("diff", "--name-only"):
            task_id = Path(cwd_s).name
            paths = CURRENT_PATHS.get(task_id, [])
            stdout = "".join(f"{path}\n" for path in paths)
        elif args and args[0] == "diff":
            stdout = "diff --git a/fixture b/fixture\n"
        elif args and args[0] == "commit":
            self.heads[cwd_s] = "fedcba9876543210fedcba9876543210fedcba98"
        elif args and args[0] == "merge":
            self.heads[cwd_s] = "89abcdef0123456789abcdef0123456789abcdef"

        return subprocess.CompletedProcess(["git", *args], 0, stdout=stdout, stderr="")


CURRENT_PATHS: dict[str, list[str]] = {}


def run_scenario(council: Any, scenario: dict[str, Any]) -> None:
    run_dir = OUT / scenario["id"]
    run_dir.mkdir(parents=True)
    CURRENT_PATHS.clear()
    CURRENT_PATHS.update({task["id"]: list(task["paths"]) for task in scenario["tasks"]})

    def fake_run_engine(engine: Any, prompt: str, **kwargs: Any) -> Any:
        del kwargs
        if "council-verdict" in prompt:
            return council.EngineResult(
                label=engine.label,
                text=json.dumps({"satisfied": True, "reasons": "fixture", "issues": []}),
                cost_usd=0.01,
            )
        if "council-consolidated" in prompt:
            return council.EngineResult(
                label=engine.label,
                text=json.dumps(consolidated_payload(scenario), sort_keys=True),
                cost_usd=0.01,
            )
        return council.EngineResult(
            label=engine.label,
            text=json.dumps(plan_payload(scenario, engine.label), sort_keys=True),
            cost_usd=0.01,
        )

    def fake_run_claude(
        prompt: str,
        model: str,
        *,
        cwd: Path | None = None,
        permission_mode: str = "plan",
        timeout: int = council.PLAN_TIMEOUT_S,
    ) -> Any:
        del cwd, permission_mode, timeout
        return fake_run_engine(council.Engine("claude", model), prompt)

    def fake_run_codex(
        prompt: str,
        model: str,
        *,
        cwd: Path | None = None,
        timeout: int = council.PLAN_TIMEOUT_S,
        sandbox: str = "read-only",
    ) -> Any:
        del cwd, timeout, sandbox
        return fake_run_engine(council.Engine("codex", model), prompt)

    council.REPO_ROOT = ROOT
    council.RUNS_ROOT = OUT
    council.WT_ROOT = Path("council/ts/test/fixtures/python-runs/.worktrees")
    council.TS_BUNDLE_PATH = Path("/nonexistent/council.mjs")
    council.run_engine = fake_run_engine
    council.run_claude = fake_run_claude
    council.run_codex = fake_run_codex
    council.git = FakeGit()
    council.run_verify = lambda _cmd, _cwd: (0, "fixture verify ok")
    council.constitution_failure = lambda repo=council.REPO_ROOT: None
    council.read_constitution_context = lambda repo=council.REPO_ROOT: "Fixture constitution."

    plan_args = SimpleNamespace(
        intensity="quick",
        planner_a="claude:opus",
        planner_b="codex:gpt-5.5",
        consolidator="claude:opus",
        rounds=1,
        codex_effort="low",
        estimate=False,
        brief=scenario["brief"],
        run=str(run_dir),
        slug=scenario["id"],
        spec_dir=None,
    )
    fanout_args = SimpleNamespace(
        run=str(run_dir),
        intensity="quick",
        max_workers=1,
        worker="claude:haiku",
        verifier="claude:sonnet",
        codex_effort="low",
        estimate=False,
        keep_worktrees=False,
    )

    if council.cmd_plan(plan_args) != 0:
        raise RuntimeError(f"plan failed for {scenario['id']}")
    if council.cmd_fanout(fanout_args) != 0:
        raise RuntimeError(f"fanout failed for {scenario['id']}")
    shutil.rmtree(council.WT_ROOT, ignore_errors=True)


def main() -> int:
    shutil.rmtree(OUT, ignore_errors=True)
    OUT.mkdir(parents=True)
    council = load_council()
    for scenario in SCENARIOS:
        run_scenario(council, scenario)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
