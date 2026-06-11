#!/usr/bin/env python3
"""council — cross-model planning + fan-out orchestrator.

`plan` runs stages 1-4: two different model families plan a brief independently,
critique each other's plan for two rounds, then a single judge consolidates one
plan plus a parallel task DAG. `fanout` (added separately) executes that DAG.

The script is engine-agnostic — it shells out to `claude -p` and `codex exec`
and runs identically whether the host session is Claude or Codex. All state is
plain JSON/Markdown under .council/runs/<id>/ so a run is resumable and the
hand-offs between stages are structured rather than free-text.

Stdlib only. See platform/agents/council/README.md.
"""

from __future__ import annotations

import argparse
import concurrent.futures
import json
import os
import re
import shlex
import shutil
import subprocess
import sys
import tempfile
import time
import tomllib
from dataclasses import dataclass, field
from pathlib import Path
from typing import Callable, Optional, TypeVar

HERE = Path(__file__).resolve().parent
PROMPTS_DIR = HERE / "prompts"
SCHEMAS_DIR = HERE / "schemas"


def repo_root() -> Path:
    """The repository council operates on — the git toplevel of the CURRENT
    working directory (i.e. the project the agent invoked council from), NOT the
    directory the toolkit happens to live in. This is what makes a globally
    installed council orchestrate whatever project you're in."""
    try:
        out = subprocess.run(
            ["git", "rev-parse", "--show-toplevel"],
            capture_output=True, text=True, check=True,
        )
        return Path(out.stdout.strip())
    except Exception:
        return Path.cwd()


# HERE is where the toolkit lives (prompts, schemas, user config); REPO_ROOT is
# the project being worked on. They differ once council is installed globally.
REPO_ROOT = repo_root()
RUNS_ROOT = REPO_ROOT / ".council" / "runs"


# --------------------------------------------------------------------------
# engines
# --------------------------------------------------------------------------

@dataclass(frozen=True)
class Engine:
    """A model behind one of the two CLIs."""
    cli: str      # "claude" | "codex"
    model: str    # alias (claude) or model id (codex)

    @property
    def label(self) -> str:
        return f"{self.cli}:{self.model}"


# Model tiers are driven by an intensity preset plus optional per-role overrides
# (council.toml + CLI flags). A preset bundles the dials that scale with effort;
# the expensive-tier role models stay constant across presets. Expensive models
# plan / critique / judge (errors propagate there); cheap models do the fan-out.
BASE_ROLES = {
    "planner_a": "claude:opus",
    "planner_b": "codex:gpt-5.5",
    "consolidator": "claude:opus",
    "verifier": "claude:sonnet",
}
PRESETS = {
    "quick":    {"rounds": 1, "codex_effort": "low",   "worker": "claude:sonnet", "max_workers": 4},
    "standard": {"rounds": 2, "codex_effort": "high",  "worker": "claude:sonnet", "max_workers": 6},
    "thorough": {"rounds": 3, "codex_effort": "high",  "worker": "claude:sonnet", "max_workers": 6},
    "max":      {"rounds": 3, "codex_effort": "xhigh", "worker": "claude:sonnet", "max_workers": 8},
}
DEFAULT_INTENSITY = "standard"
ROLE_KEYS = ("planner_a", "planner_b", "consolidator", "worker", "verifier")
INT_KEYS = ("rounds", "max_workers")
CODEX_EFFORTS = ("low", "medium", "high", "xhigh")
CONFIG_KEYS = ("intensity",) + ROLE_KEYS + ("codex_effort",) + INT_KEYS
# User-global config lives next to the toolkit (council.toml beside the script,
# i.e. the committed default in-repo, or ~/.claude/skills/council/ when
# installed). A per-project ./.council.toml in the target repo overrides it.
USER_CONFIG_PATH = HERE / "council.toml"
PROJECT_CONFIG_PATH = REPO_ROOT / ".council.toml"

# codex reasoning effort; resolved per-run from config and set at command entry.
CODEX_REASONING = os.environ.get("COUNCIL_CODEX_REASONING", "high")
PLAN_TIMEOUT_S = int(os.environ.get("COUNCIL_PLAN_TIMEOUT_S", "1200"))

# fan-out tier
WORKER_PERMISSION_MODE = "bypassPermissions"   # YOLO: bypass approvals + sandbox so workers can build/network
WORKER_TIMEOUT_S = int(os.environ.get("COUNCIL_WORKER_TIMEOUT_S", "1800"))
VERIFY_TIMEOUT_S = int(os.environ.get("COUNCIL_VERIFY_TIMEOUT_S", "600"))
WT_ROOT = Path(tempfile.gettempdir()) / "council-worktrees"
SPEC_DIR_RE = re.compile(r"^(\d{3})-([a-z0-9][a-z0-9-]*)$")
TASK_BLOCK_RE = re.compile(
    r"^## (?P<header_id>[^\n:]+)(?::[^\n]*)?\n"
    r"<!-- council-task-id: (?P<marker_id>[^>]+) -->\n"
    r"```json\n(?P<body>.*?)\n```",
    re.MULTILINE | re.DOTALL,
)
MAX_CONSTITUTION_CHARS = 6000
MAX_TEMPLATE_FIELD_CHARS = 12000
EMBEDDED_CONSTITUTION = """# Constitution

No project `.specify/memory/constitution.md` was found. Apply the repository's
agent guide, keep changes minimal, validate against real files, and preserve
human authorship.
"""
EMBEDDED_SPEC_TEMPLATE = """# Feature Specification: {{feature_name}}

**Feature Branch**: `{{feature_id}}`
**Created**: {{date}}

## User Brief

{{brief}}

## Requirements

- Implement the brief as described, grounded in the real repository.
- Keep scope additive and preserve existing council canonical artifacts.

## Success Criteria

- `consolidated_plan.md` and `tasks.json` remain canonical.
- `tasks.md` round-trips exactly to `tasks.json` through council markers.
"""
EMBEDDED_PLAN_TEMPLATE = """# Implementation Plan: {{feature_name}}

**Feature Branch**: `{{feature_id}}`
**Created**: {{date}}

## Summary

{{summary}}

## Consolidated Plan

{{consolidated_plan}}
"""
EMBEDDED_TASKS_TEMPLATE = """# Tasks: {{feature_id}}

<!-- council-tasks-format: v1 -->
"""


@dataclass(frozen=True)
class SpecRef:
    number: int
    slug: str

    @property
    def name(self) -> str:
        return f"{self.number:03d}-{self.slug}"

    @property
    def relpath(self) -> str:
        return f"specs/{self.name}"


@dataclass
class EngineResult:
    label: str
    text: str
    cost_usd: Optional[float] = None
    raw: Optional[dict] = None


def child_env() -> dict:
    """Environment for sub-invocations: silence the KB hooks so council's own
    prompts never get recalled into or digested out to the knowledge base."""
    env = dict(os.environ)
    env["KB_AUTO_MCP_DISABLED"] = "1"
    return env


def run_claude(prompt: str, model: str, *, cwd: Optional[Path] = None,
               permission_mode: str = "plan",
               timeout: int = PLAN_TIMEOUT_S) -> EngineResult:
    # "plan" = read-only repo access, no edits: correct for the planning tier.
    cmd = ["claude", "-p", "--model", model, "--output-format", "json",
           "--permission-mode", permission_mode]
    proc = subprocess.run(
        cmd, input=prompt, capture_output=True, text=True,
        cwd=str(cwd or REPO_ROOT), env=child_env(), timeout=timeout,
    )
    if proc.returncode != 0:
        raise RuntimeError(f"claude:{model} exited {proc.returncode}: "
                           f"{proc.stderr.strip()[:500]}")
    try:
        data = json.loads(proc.stdout)
    except json.JSONDecodeError as exc:
        raise RuntimeError(f"claude:{model} non-JSON output: {exc}: "
                           f"{proc.stdout[:300]}") from exc
    return EngineResult(
        label=f"claude:{model}",
        text=str(data.get("result", "")),
        cost_usd=data.get("total_cost_usd"),
        raw=data,
    )


def run_codex(prompt: str, model: str, *, cwd: Optional[Path] = None,
              timeout: int = PLAN_TIMEOUT_S, sandbox: str = "read-only") -> EngineResult:
    # `-o` writes only the final assistant message to a file; stdout is noisy
    # (banner + hook wrappers) so we read the message back from the file.
    # sandbox is "read-only" for planning and "workspace-write" for a worker
    # that must edit files in its worktree.
    last = Path(
        subprocess.run(["mktemp"], capture_output=True, text=True, check=True)
        .stdout.strip()
    )
    cmd = [
        "codex", "exec", "-m", model,
        "-c", f"model_reasoning_effort={CODEX_REASONING}",
        "--skip-git-repo-check",
        "-o", str(last), prompt,
    ]
    if sandbox == "bypass":
        # YOLO: full access (network + fs), no approvals — used for fan-out
        # workers so they can run gradle / fetch deps / build themselves.
        cmd[6:6] = ["--dangerously-bypass-approvals-and-sandbox"]
    else:
        cmd[6:6] = ["-s", sandbox]
    try:
        proc = subprocess.run(
            cmd, capture_output=True, text=True,
            cwd=str(cwd or REPO_ROOT), env=child_env(), timeout=timeout,
        )
        if proc.returncode != 0:
            raise RuntimeError(f"codex:{model} exited {proc.returncode}: "
                               f"{proc.stderr.strip()[:500]}")
        text = last.read_text().strip()
    finally:
        last.unlink(missing_ok=True)
    return EngineResult(label=f"codex:{model}", text=text)


def run_engine(engine: Engine, prompt: str, *, cwd: Optional[Path] = None,
               timeout: int = PLAN_TIMEOUT_S, retries: int = 1) -> EngineResult:
    def once() -> EngineResult:
        if engine.cli == "claude":
            return run_claude(prompt, engine.model, cwd=cwd, timeout=timeout)
        if engine.cli == "codex":
            return run_codex(prompt, engine.model, cwd=cwd, timeout=timeout)
        raise ValueError(f"unknown cli: {engine.cli}")

    last: Exception = RuntimeError("no attempt")
    for attempt in range(retries + 1):
        try:
            return once()
        except (RuntimeError, ValueError) as exc:
            last = exc
            if attempt < retries:
                log(f"{engine.label} attempt {attempt + 1} failed ({exc}); retrying")
                time.sleep(3)
    raise last


# --------------------------------------------------------------------------
# small helpers
# --------------------------------------------------------------------------

T = TypeVar("T")


def parallel(thunks: list[Callable[[], T]]) -> list[T]:
    """Run thunks concurrently, return results in order. Raises if any raises."""
    with concurrent.futures.ThreadPoolExecutor(max_workers=len(thunks)) as ex:
        futs = [ex.submit(t) for t in thunks]
        return [f.result() for f in futs]


def render(template: str, **values: str) -> str:
    """Replace {{key}} tokens. Double braces avoid clashing with JSON braces."""
    out = template
    for key, val in values.items():
        out = out.replace("{{" + key + "}}", val)
    return out


def load_prompt(name: str) -> str:
    return (PROMPTS_DIR / f"{name}.md").read_text()


# Durable rules every agent must follow regardless of role (no attribution,
# match conventions, stay in scope, validate against real code). Injected into
# every prompt via the {{baseline}} token. Loaded once at import.
BASELINE_PROMPT = load_prompt("_baseline")


def load_schema_text(name: str) -> str:
    return json.dumps(json.loads((SCHEMAS_DIR / f"{name}.schema.json").read_text()),
                      indent=2)


def extract_json(text: str) -> dict:
    """Pull a JSON object out of a model reply, tolerating ```json fences and
    surrounding prose. Tries a clean parse first (so backticks or braces inside
    string values survive), then a string-aware scan from the first brace."""
    stripped = text.strip()
    try:
        obj = json.loads(stripped)
        if isinstance(obj, dict):
            return obj
    except json.JSONDecodeError:
        pass
    start = stripped.find("{")
    if start == -1:
        raise ValueError(f"no JSON object found in reply: {text[:200]}")
    depth = 0
    in_str = False
    esc = False
    for i in range(start, len(stripped)):
        ch = stripped[i]
        if in_str:
            if esc:
                esc = False
            elif ch == "\\":
                esc = True
            elif ch == '"':
                in_str = False
            continue
        if ch == '"':
            in_str = True
        elif ch == "{":
            depth += 1
        elif ch == "}":
            depth -= 1
            if depth == 0:
                return json.loads(stripped[start:i + 1])
    raise ValueError(f"unbalanced JSON in reply: {text[:200]}")


def plan_waves(tasks: list[dict]) -> list[list[str]]:
    """Topologically group task ids into parallel waves (Kahn's algorithm).
    Raises on unknown dependency or cycle. Pure — covered by --self-test."""
    ids = {t["id"] for t in tasks}
    deps = {t["id"]: list(t.get("depends_on", [])) for t in tasks}
    for tid, ds in deps.items():
        for d in ds:
            if d not in ids:
                raise ValueError(f"task {tid!r} depends on unknown task {d!r}")
    remaining = dict(deps)
    done: set[str] = set()
    waves: list[list[str]] = []
    while remaining:
        ready = sorted(t for t, ds in remaining.items()
                       if all(d in done for d in ds))
        if not ready:
            raise ValueError(f"dependency cycle among tasks: "
                             f"{sorted(remaining)}")
        waves.append(ready)
        for t in ready:
            done.add(t)
            del remaining[t]
    return waves


# --------------------------------------------------------------------------
# run directory / state
# --------------------------------------------------------------------------

@dataclass
class Run:
    path: Path
    costs: list[tuple[str, float]] = field(default_factory=list)

    @classmethod
    def create(cls, brief: str, slug: Optional[str]) -> "Run":
        stamp = time.strftime("%Y%m%d-%H%M%S")
        slug = slug or _slugify(brief.splitlines()[0] if brief.strip() else "run")
        path = RUNS_ROOT / f"{stamp}-{slug}"
        path.mkdir(parents=True, exist_ok=True)
        return cls(path)

    @classmethod
    def open(cls, path: Path) -> "Run":
        if not path.exists():
            raise SystemExit(f"run dir not found: {path}")
        return cls(path)

    def write_text(self, name: str, text: str) -> None:
        (self.path / name).write_text(text)

    def write_json(self, name: str, obj: object) -> None:
        (self.path / name).write_text(json.dumps(obj, indent=2))

    def read_json(self, name: str) -> dict:
        return json.loads((self.path / name).read_text())

    def has(self, name: str) -> bool:
        return (self.path / name).exists()

    def record(self, res: EngineResult) -> EngineResult:
        if res.cost_usd is not None:
            self.costs.append((res.label, res.cost_usd))
        return res

    def set_state(self, **kw: object) -> None:
        state = {}
        if self.has("state.json"):
            state = self.read_json("state.json")
        state.update(kw)
        self.write_json("state.json", state)


def _slugify(text: str) -> str:
    s = re.sub(r"[^a-z0-9]+", "-", text.lower()).strip("-")
    return (s[:48] or "run")


def _first_line(text: str) -> str:
    for line in text.splitlines():
        if line.strip():
            return line.strip()
    return "run"


def derive_feature_slug(brief: str, explicit_slug: Optional[str]) -> str:
    return _slugify(explicit_slug or _first_line(brief))


def _bounded(text: str, limit: int) -> str:
    if len(text) <= limit:
        return text
    return text[:limit].rstrip() + "\n\n[truncated]"


def _constitution_path(repo: Path = REPO_ROOT) -> Path:
    return repo / ".specify" / "memory" / "constitution.md"


def read_constitution_context(repo: Path = REPO_ROOT) -> str:
    path = _constitution_path(repo)
    text = path.read_text() if path.exists() else EMBEDDED_CONSTITUTION
    return _bounded(text.strip(), MAX_CONSTITUTION_CHARS)


def _constitution_placeholder_reason(text: str) -> Optional[str]:
    stripped = text.strip()
    if not stripped:
        return "constitution is empty"
    lower = stripped.lower()
    markers = (
        "[project name]",
        "[insert",
        "[fill",
        "[todo",
        "{{",
        "todo:",
        "tbd",
        "placeholder",
    )
    for marker in markers:
        if marker in lower:
            return f"constitution contains placeholder marker {marker!r}"
    return None


def constitution_failure(repo: Path = REPO_ROOT) -> Optional[str]:
    path = _constitution_path(repo)
    if not path.exists():
        return f"missing constitution at {path}"
    reason = _constitution_placeholder_reason(path.read_text())
    if reason:
        return f"{reason} at {path}"
    return None


def _spec_numbers(specs_root: Path) -> list[int]:
    if not specs_root.exists():
        return []
    nums = []
    for child in specs_root.iterdir():
        m = SPEC_DIR_RE.match(child.name)
        if m:
            nums.append(int(m.group(1)))
    return nums


def allocate_spec_ref(slug: str, specs_root: Path) -> SpecRef:
    slug = _slugify(slug)
    children = specs_root.iterdir() if specs_root.exists() else ()
    for child in children:
        m = SPEC_DIR_RE.match(child.name)
        if m and m.group(2) == slug:
            raise ValueError(f"spec path already exists: {child}")
    ref = SpecRef((max(_spec_numbers(specs_root)) if specs_root.exists() else 0) + 1,
                  slug)
    path = specs_root / ref.name
    if path.exists():
        raise ValueError(f"spec path already exists: {path}")
    return ref


def spec_ref_from_state(state: dict) -> Optional[SpecRef]:
    rel = state.get("spec_relpath")
    if not isinstance(rel, str):
        return None
    name = Path(rel).name
    m = SPEC_DIR_RE.match(name)
    if not m:
        return None
    return SpecRef(int(m.group(1)), m.group(2))


def prepare_spec_ref(run: Run, brief: str, explicit_slug: Optional[str]) -> SpecRef:
    state = run.read_json("state.json") if run.has("state.json") else {}
    existing = spec_ref_from_state(state)
    if existing:
        return existing
    ref = allocate_spec_ref(derive_feature_slug(brief, explicit_slug),
                            REPO_ROOT / "specs")
    run_target = run.path / ref.relpath
    repo_target = REPO_ROOT / ref.relpath
    if run_target.exists():
        raise ValueError(f"spec path already exists: {run_target}")
    if repo_target.exists():
        raise ValueError(f"spec path already exists: {repo_target}")
    run.set_state(spec_id=ref.name, spec_slug=ref.slug, spec_relpath=ref.relpath)
    return ref


def read_spec_dir(path_s: Optional[str]) -> dict[str, str]:
    if not path_s:
        return {}
    path = Path(path_s)
    if not path.exists():
        raise SystemExit(f"spec dir not found: {path}")
    if not path.is_dir():
        raise SystemExit(f"--spec-dir must be a directory: {path}")
    out = {}
    for name in ("spec.md", "plan.md", "tasks.md"):
        p = path / name
        if p.exists():
            out[name] = p.read_text()
    return out


def load_sdd_template(name: str, fallback: str) -> str:
    path = REPO_ROOT / ".specify" / "templates" / name
    return path.read_text() if path.exists() else fallback


def render_sdd_template(template: str, values: dict[str, str]) -> str:
    out = template
    for key, val in values.items():
        bounded = _bounded(val, MAX_TEMPLATE_FIELD_CHARS)
        out = out.replace("{{" + key + "}}", bounded)
        out = out.replace("[" + key.upper() + "]", bounded)
        out = out.replace("[" + key + "]", bounded)
    return out


def render_tasks_md(tasks: list[dict], spec_ref: Optional[SpecRef] = None) -> str:
    feature_id = spec_ref.name if spec_ref else "council"
    template = load_sdd_template("tasks-template.md", EMBEDDED_TASKS_TEMPLATE)
    header = render_sdd_template(template, {
        "feature_id": feature_id,
        "feature_name": feature_id,
    }).strip()
    if "<!-- council-tasks-format: v1 -->" not in header:
        header += "\n\n<!-- council-tasks-format: v1 -->"
    lines = [header, ""]
    for task in tasks:
        tid = str(task["id"])
        task_title = str(task.get("title", tid)).replace("\n", " ").strip() or tid
        lines += [
            f"## {tid}: {task_title}",
            f"<!-- council-task-id: {tid} -->",
            "```json",
            json.dumps(task, indent=2, sort_keys=True),
            "```",
            "",
        ]
    return "\n".join(lines).rstrip() + "\n"


def parse_tasks_md(text: str) -> list[dict]:
    tasks: list[dict] = []
    for match in TASK_BLOCK_RE.finditer(text):
        header_id = match.group("header_id").strip()
        marker_id = match.group("marker_id").strip()
        if header_id != marker_id:
            raise ValueError(f"task marker mismatch: header {header_id!r}, "
                             f"marker {marker_id!r}")
        try:
            task = json.loads(match.group("body"))
        except json.JSONDecodeError as exc:
            raise ValueError(f"task {marker_id!r} JSON block is invalid: {exc}") from exc
        if not isinstance(task, dict):
            raise ValueError(f"task {marker_id!r} JSON block must be an object")
        if str(task.get("id", "")).strip() != marker_id:
            raise ValueError(f"task {marker_id!r} JSON id does not match marker")
        tasks.append(task)
    if not tasks:
        raise ValueError("no council task JSON blocks found in tasks.md")
    seen: set[str] = set()
    for task in tasks:
        tid = str(task["id"])
        if tid in seen:
            raise ValueError(f"duplicate task id in tasks.md: {tid}")
        seen.add(tid)
    return tasks


def _normalise_tasks(tasks: list[dict]) -> object:
    return json.loads(json.dumps(tasks, sort_keys=True))


def assert_tasks_bijection(tasks: list[dict], tasks_md_text: str) -> None:
    parsed = parse_tasks_md(tasks_md_text)
    validate_tasks(parsed)
    if _normalise_tasks(parsed) != _normalise_tasks(tasks):
        raise ValueError("tasks.md does not match tasks.json")


def run_spec_dir(run: Run) -> Optional[Path]:
    state = run.read_json("state.json") if run.has("state.json") else {}
    ref = spec_ref_from_state(state)
    if ref:
        return run.path / ref.relpath
    specs = run.path / "specs"
    matches = sorted(p for p in specs.glob("[0-9][0-9][0-9]-*") if p.is_dir())
    return matches[-1] if matches else None


def regenerate_command(run: Run) -> str:
    return ("council plan --run " + shlex.quote(str(run.path)) +
            " --brief " + shlex.quote(str(run.path / "brief.md")))


def analyze_checkpoint(run: Run, tasks: list[dict]) -> None:
    failures = []
    constitution = constitution_failure()
    if constitution:
        failures.append(constitution)
    spec_dir = run_spec_dir(run)
    tasks_md = spec_dir / "tasks.md" if spec_dir else None
    if not tasks_md or not tasks_md.exists():
        failures.append("missing tasks.md for tasks.json")
    else:
        try:
            assert_tasks_bijection(tasks, tasks_md.read_text())
        except ValueError as exc:
            failures.append(str(exc))
    if failures:
        raise ValueError("analyze gate checkpoint 1 failed:\n- "
                         + "\n- ".join(failures)
                         + f"\nRegenerate with: {regenerate_command(run)}")


def analyze_tasks_file(tasks: list[dict], tasks_path: Path) -> None:
    failures = []
    constitution = constitution_failure()
    if constitution:
        failures.append(constitution)
    tasks_md = tasks_path.with_name("tasks.md")
    if tasks_md.exists():
        try:
            assert_tasks_bijection(tasks, tasks_md.read_text())
        except ValueError as exc:
            failures.append(str(exc))
    if failures:
        cmd = ("council plan --brief " + shlex.quote(str(tasks_path.parent / "spec.md"))
               + " --spec-dir " + shlex.quote(str(tasks_path.parent)))
        raise ValueError("analyze gate checkpoint 1 failed:\n- "
                         + "\n- ".join(failures)
                         + f"\nRegenerate with: {cmd}")


def build_spec_md(brief: str, obj: dict, ref: SpecRef,
                  seed: dict[str, str]) -> str:
    if seed.get("spec.md"):
        return seed["spec.md"]
    if obj.get("spec_markdown"):
        return str(obj["spec_markdown"]).strip() + "\n"
    template = load_sdd_template("spec-template.md", EMBEDDED_SPEC_TEMPLATE)
    return render_sdd_template(template, {
        "feature_name": ref.slug.replace("-", " ").title(),
        "feature_id": ref.name,
        "date": time.strftime("%Y-%m-%d"),
        "brief": brief.strip(),
    }).rstrip() + "\n"


def build_plan_md(brief: str, obj: dict, ref: SpecRef,
                  seed: dict[str, str]) -> str:
    if seed.get("plan.md"):
        return seed["plan.md"]
    for key in ("implementation_plan_markdown", "plan_markdown"):
        if obj.get(key):
            return str(obj[key]).strip() + "\n"
    template = load_sdd_template("plan-template.md", EMBEDDED_PLAN_TEMPLATE)
    return render_sdd_template(template, {
        "feature_name": ref.slug.replace("-", " ").title(),
        "feature_id": ref.name,
        "date": time.strftime("%Y-%m-%d"),
        "brief": brief.strip(),
        "summary": str(obj.get("summary", "")).strip() or _first_line(brief),
        "consolidated_plan": str(obj.get("consolidated_plan_markdown", "")).strip(),
    }).rstrip() + "\n"


def write_sdd_artifacts(run: Run, brief: str, obj: dict, ref: SpecRef,
                        seed: dict[str, str]) -> None:
    tasks = obj.get("tasks", [])
    if seed.get("tasks.md"):
        assert_tasks_bijection(tasks, seed["tasks.md"])
    spec_dir = run.path / ref.relpath
    spec_dir.mkdir(parents=True, exist_ok=True)
    (spec_dir / "spec.md").write_text(build_spec_md(brief, obj, ref, seed))
    (spec_dir / "plan.md").write_text(build_plan_md(brief, obj, ref, seed))
    (spec_dir / "tasks.md").write_text(render_tasks_md(tasks, ref))


def copy_run_specs_to_worktree(run: Run, worktree: Path) -> None:
    specs_root = run.path / "specs"
    if not specs_root.exists():
        return
    copied = []
    for src in sorted(p for p in specs_root.iterdir()
                      if p.is_dir() and SPEC_DIR_RE.match(p.name)):
        dest = worktree / "specs" / src.name
        if dest.exists():
            raise ValueError(f"spec path already exists in integration worktree: "
                             f"specs/{src.name}")
        dest.parent.mkdir(parents=True, exist_ok=True)
        shutil.copytree(src, dest)
        copied.append(f"specs/{src.name}")
    if not copied:
        return
    git("add", "-A", "--", "specs", cwd=worktree)
    dirty = git("status", "--porcelain", "--", "specs", cwd=worktree).stdout.strip()
    if dirty:
        git("-c", "user.name=council", "-c", "user.email=council@local",
            "commit", "-q", "-m", "council: add spec artifacts", cwd=worktree)
        log(f"committed Spec Kit artifacts: {', '.join(copied)}")


def _split_dest_url(owner: str, name: str) -> str:
    """Canonical SSH remote for a GitHub owner/name. Pure (covered by
    --self-test). In runner workspaces git rewrites git@github.com: to https so
    the App-token credential helper serves the push."""
    return f"git@github.com:{owner}/{name}.git"


# --------------------------------------------------------------------------
# stages 1-4
# --------------------------------------------------------------------------

def stage_plan(run: Run, brief: str, a: Engine, b: Engine,
               constitution_context: str) -> tuple[dict, dict]:
    if run.has("planA.v1.json") and run.has("planB.v1.json"):
        log("stage 1: dual plans already present, skipping")
        return run.read_json("planA.v1.json"), run.read_json("planB.v1.json")
    log(f"stage 1: independent plans  {a.label} ║ {b.label}")
    schema = load_schema_text("plan")
    tmpl = load_prompt("planner")

    def mk(engine: Engine) -> Callable[[], EngineResult]:
        prompt = render(tmpl, engine_label=engine.label, brief=brief,
                        repo_root=str(REPO_ROOT), schema=schema,
                        baseline=BASELINE_PROMPT,
                        constitution=constitution_context)
        return lambda: run.record(run_engine(engine, prompt))

    res_a, res_b = parallel([mk(a), mk(b)])
    plan_a, plan_b = extract_json(res_a.text), extract_json(res_b.text)
    run.write_json("planA.v1.json", plan_a)
    run.write_json("planB.v1.json", plan_b)
    return plan_a, plan_b


def stage_critique_round(run: Run, brief: str, a: Engine, b: Engine,
                         plan_a: dict, plan_b: dict, rnd: int,
                         constitution_context: str) -> tuple[dict, dict]:
    out_a, out_b = f"planA.v{rnd + 1}.json", f"planB.v{rnd + 1}.json"
    if run.has(out_a) and run.has(out_b):
        log(f"stage 2: critique round {rnd} already present, skipping")
        return run.read_json(out_a), run.read_json(out_b)
    log(f"stage 2: cross-critique round {rnd}  ({a.label} ⇄ {b.label})")
    critic_tmpl = load_prompt("critic")
    schema = load_schema_text("plan")

    # Cross: each model critiques the OTHER's plan.
    def crit(critic: Engine, plan: dict) -> Callable[[], EngineResult]:
        prompt = render(critic_tmpl, engine_label=critic.label, brief=brief,
                        repo_root=str(REPO_ROOT),
                        plan=json.dumps(plan, indent=2),
                        baseline=BASELINE_PROMPT,
                        constitution=constitution_context)
        return lambda: run.record(run_engine(critic, prompt))

    crit_of_a, crit_of_b = parallel([crit(b, plan_a), crit(a, plan_b)])
    run.write_text(f"critique-of-A.r{rnd}.md", crit_of_a.text)
    run.write_text(f"critique-of-B.r{rnd}.md", crit_of_b.text)

    # Each author revises its own plan using the critique it received.
    rev_tmpl = load_prompt("reviser")

    def rev(author: Engine, plan: dict, critique: str) -> Callable[[], EngineResult]:
        prompt = render(rev_tmpl, engine_label=author.label, brief=brief,
                        repo_root=str(REPO_ROOT), plan=json.dumps(plan, indent=2),
                        critique=critique, schema=schema,
                        baseline=BASELINE_PROMPT,
                        constitution=constitution_context)
        return lambda: run.record(run_engine(author, prompt))

    rev_a, rev_b = parallel([rev(a, plan_a, crit_of_a.text),
                             rev(b, plan_b, crit_of_b.text)])
    next_a, next_b = extract_json(rev_a.text), extract_json(rev_b.text)
    run.write_json(out_a, next_a)
    run.write_json(out_b, next_b)
    return next_a, next_b


def stage_consolidate(run: Run, brief: str, plan_a: dict, plan_b: dict,
                      rounds: int, consolidator: Engine,
                      constitution_context: str, spec_ref: SpecRef,
                      spec_seed: dict[str, str]) -> dict:
    if run.has("tasks.json") and run.has("consolidated_plan.md"):
        log("stage 4: consolidation already present, skipping")
        tasks = run.read_json("tasks.json")
        obj = {
            "consolidated_plan_markdown": (run.path / "consolidated_plan.md").read_text(),
            "tasks": tasks,
        }
        write_sdd_artifacts(run, brief, obj, spec_ref, spec_seed)
        analyze_checkpoint(run, tasks)
        return tasks
    log(f"stage 4: consolidation  ({consolidator.label})")
    history_parts = []
    for r in range(1, rounds + 1):
        for side in ("A", "B"):
            name = f"critique-of-{side}.r{r}.md"
            if run.has(name):
                history_parts.append(f"## Round {r} — critique of plan {side}\n"
                                     + (run.path / name).read_text())
    prompt = render(
        load_prompt("consolidator"), brief=brief, repo_root=str(REPO_ROOT),
        plan_a=json.dumps(plan_a, indent=2), plan_b=json.dumps(plan_b, indent=2),
        history="\n\n".join(history_parts) or "(no critiques recorded)",
        schema=load_schema_text("consolidated"),
        baseline=BASELINE_PROMPT,
        constitution=constitution_context,
    )
    res = run.record(run_engine(consolidator, prompt))
    obj = extract_json(res.text)
    tasks = obj.get("tasks", [])
    validate_tasks(tasks)
    run.write_text("consolidated_plan.md", obj.get("consolidated_plan_markdown", ""))
    run.write_json("tasks.json", tasks)
    write_sdd_artifacts(run, brief, obj, spec_ref, spec_seed)
    analyze_checkpoint(run, tasks)
    return tasks


def validate_tasks(tasks: list[dict]) -> None:
    """Structural + DAG validation (we don't ship a full JSON-Schema validator;
    this checks the fields fan-out actually relies on)."""
    if not isinstance(tasks, list) or not tasks:
        raise ValueError("consolidator returned no tasks")
    required = {"id", "objective", "depends_on", "paths", "model", "verify"}
    seen: set[str] = set()
    for t in tasks:
        missing = required - t.keys()
        if missing:
            raise ValueError(f"task {t.get('id', '?')} missing fields: {sorted(missing)}")
        if t["id"] in seen:
            raise ValueError(f"duplicate task id: {t['id']}")
        seen.add(t["id"])
        if not str(t.get("verify", "")).strip():
            log(f"warning: task {t['id']} has no verify command — its result "
                "is unchecked except by the adversarial verifier")
    plan_waves(tasks)  # raises on cycle / unknown dep


# --------------------------------------------------------------------------
# stages 5-6: fan-out + verify + reconcile
# --------------------------------------------------------------------------

def git(*args: str, cwd: Optional[Path] = None, check: bool = True,
        timeout: int = 120) -> subprocess.CompletedProcess:
    return subprocess.run(["git", *args], cwd=str(cwd or REPO_ROOT),
                          capture_output=True, text=True, check=check,
                          timeout=timeout)


def gh(*args: str, check: bool = True,
       timeout: int = 120) -> subprocess.CompletedProcess:
    return subprocess.run(["gh", *args], cwd=str(REPO_ROOT),
                          capture_output=True, text=True, check=check,
                          timeout=timeout)


def have_git_subtree() -> bool:
    r = subprocess.run(["git", "subtree", "-h"], capture_output=True, text=True)
    return "is not a git command" not in (r.stdout + r.stderr).lower()


def parallel_bounded(thunks: list[Callable[[], T]], cap: int) -> list[T]:
    """Run thunks with at most `cap` concurrent, preserving order. Thunks must
    not raise (wrap failures into return values)."""
    with concurrent.futures.ThreadPoolExecutor(max_workers=max(1, cap)) as ex:
        return list(ex.map(lambda t: t(), thunks))


def localize_verify(cmd: str, repo_root: str, cwd: str) -> str:
    """Point a verify command at the worktree it runs in. The consolidator is
    told to write repo-relative commands, but a stray absolute repo-root path
    (e.g. `cd /workspace/services/foo`) would otherwise check the host tree, not
    the worker's worktree. Rewrite the repo root to the worktree so such a
    command still verifies the right files. Pure (covered by --self-test)."""
    if repo_root and repo_root != cwd and repo_root in cmd:
        return cmd.replace(repo_root, cwd)
    return cmd


def run_verify(cmd: str, cwd: Path) -> tuple[Optional[int], str]:
    if not cmd.strip():
        return None, "(no verify command)"
    cmd = localize_verify(cmd, str(REPO_ROOT), str(cwd))
    try:
        proc = subprocess.run(["bash", "-lc", cmd], cwd=str(cwd),
                              capture_output=True, text=True, env=child_env(),
                              timeout=VERIFY_TIMEOUT_S)
        return proc.returncode, (proc.stdout + proc.stderr)[-8000:]
    except subprocess.TimeoutExpired:
        return 124, f"(verify timed out after {VERIFY_TIMEOUT_S}s)"


def run_verifier(run: Run, task: dict, diff: str, verify_cmd: str,
                 verify_rc: Optional[int], verify_out: str,
                 verifier: Engine) -> Optional[dict]:
    prompt = render(
        load_prompt("verifier"), objective=task.get("objective", ""),
        output_format=task.get("output_format", ""),
        paths="\n".join(f"- {p}" for p in task.get("paths", [])) or "(none)",
        diff=diff[:16000] or "(no changes)", verify_cmd=verify_cmd or "(none)",
        verify_rc=str(verify_rc), verify_output=verify_out[:6000] or "(none)",
        schema=load_schema_text("verdict"),
        baseline=BASELINE_PROMPT,
    )
    try:
        res = run.record(run_engine(verifier, prompt, retries=0))
        return extract_json(res.text)
    except Exception as exc:  # verifier is advisory; never fail the run on it
        log(f"verifier for {task['id']} errored: {exc}")
        return None


def run_worker(run: Run, task: dict, base_ref: str, run_name: str,
               worker: Engine, verifier: Engine) -> dict:
    tid = task["id"]
    paths = list(task.get("paths", []))
    branch = f"council/{run_name}/{tid}"
    wt = WT_ROOT / run_name / tid
    result: dict = {"task_id": tid, "title": task.get("title", tid),
                    "model": worker.label, "suggested_model": task.get("model"),
                    "branch": branch, "worktree": str(wt), "committed": False}
    try:
        wt.parent.mkdir(parents=True, exist_ok=True)
        git("worktree", "remove", "--force", str(wt), check=False)
        git("branch", "-D", branch, check=False)
        git("worktree", "add", "--force", "-b", branch, str(wt), base_ref)

        if paths:
            prompt = render(
                load_prompt("worker"), title=task.get("title", tid),
                objective=task["objective"],
                paths="\n".join(f"- {p}" for p in paths),
                boundaries=task.get("boundaries", ""),
                output_format=task.get("output_format", ""), cwd=str(wt),
                baseline=BASELINE_PROMPT)
            # Engine-agnostic: claude with auto-accepted edits, or codex with a
            # writable sandbox. Either way the orchestrator (not the worker)
            # commits the worktree below.
            if worker.cli == "codex":
                res = run.record(run_codex(prompt, worker.model, cwd=wt,
                                           sandbox="bypass",
                                           timeout=WORKER_TIMEOUT_S))
            else:
                res = run.record(run_claude(prompt, worker.model, cwd=wt,
                                            permission_mode=WORKER_PERMISSION_MODE,
                                            timeout=WORKER_TIMEOUT_S))
            result["summary"] = res.text[-2000:]
        else:
            result["summary"] = "(verify-only task: no files to edit)"

        git("add", "-A", cwd=wt)
        dirty = git("status", "--porcelain", cwd=wt).stdout.strip()
        if dirty:
            git("-c", "user.name=council", "-c", "user.email=council@local",
                "commit", "-q", "-m", f"council: {tid}", cwd=wt)
            result["committed"] = True
            result["files_changed"] = git(
                "diff", "--name-only", f"{base_ref}..HEAD", cwd=wt
            ).stdout.split()
            diff = git("diff", f"{base_ref}..HEAD", cwd=wt, timeout=120).stdout
        else:
            result["files_changed"] = []
            diff = ""

        out_of_bounds = [f for f in result["files_changed"] if f not in paths]
        result["out_of_bounds"] = out_of_bounds

        verify_cmd = task.get("verify", "")
        rc, out = run_verify(verify_cmd, wt)
        result["verify_rc"] = rc
        result["verify_output"] = out[-4000:]

        verdict = run_verifier(run, task, diff, verify_cmd, rc, out, verifier)
        result["verdict"] = verdict

        if out_of_bounds:
            result["status"] = "out-of-bounds"
        elif paths and not result["committed"]:
            result["status"] = "no-op"
        elif rc not in (None, 0):
            result["status"] = "verify-failed"
        elif verdict is not None and not verdict.get("satisfied", True):
            result["status"] = "rejected"
        else:
            result["status"] = "ok"
    except Exception as exc:
        result["status"] = "error"
        result["error"] = str(exc)[:500]
    finally:
        wdir = run.path / "workers" / tid
        wdir.mkdir(parents=True, exist_ok=True)
        (wdir / "result.json").write_text(json.dumps(result, indent=2))
    return result


def execute_dag(run: Run, tasks: list[dict], worker_for: Callable[[str], Engine],
                verifier: Engine, cap: int, keep_worktrees: bool) -> tuple[dict, str]:
    """Execute a validated task DAG: topologically sort into waves, run each
    wave's tasks concurrently in isolated worktrees (worker chosen per task by
    worker_for(task_id)), verify, then reconcile committed worktrees onto an
    integration branch in dependency order. Nothing touches the host branch.
    Shared by fanout (constant worker) and fleet (round-robin pool). Returns
    (report, integration_branch)."""
    by_id = {t["id"]: t for t in tasks}
    waves = plan_waves(tasks)
    run_name = run.path.name
    base = git("rev-parse", "HEAD").stdout.strip()
    integ_branch = f"council/{run_name}/integration"
    integ_wt = WT_ROOT / run_name / "_integration"
    integ_wt.parent.mkdir(parents=True, exist_ok=True)
    git("worktree", "remove", "--force", str(integ_wt), check=False)
    git("branch", "-D", integ_branch, check=False)
    git("worktree", "add", "--force", "-b", integ_branch, str(integ_wt), base)
    copy_run_specs_to_worktree(run, integ_wt)
    log(f"exec: {len(tasks)} task(s) in {len(waves)} wave(s); base {base[:8]}; "
        f"integration branch {integ_branch}; concurrency {cap}")

    results: dict[str, dict] = {}
    for wi, wave in enumerate(waves, 1):
        wave_base = git("rev-parse", "HEAD", cwd=integ_wt).stdout.strip()
        log(f"wave {wi}/{len(waves)}: {wave}  (base {wave_base[:8]})")
        thunks = [(lambda t=t: run_worker(run, by_id[t], wave_base, run_name,
                                           worker_for(t), verifier))
                  for t in wave]
        for tid, res in zip(wave, parallel_bounded(thunks, cap)):
            results[tid] = res
            log(f"  [{tid}] {res['status']} ({res['model']})"
                + (f" ({len(res.get('files_changed', []))} files)"
                   if res.get("committed") else ""))
        # reconcile this wave into the integration branch, in order
        for tid in wave:
            res = results[tid]
            if not res.get("committed"):
                res["merge"] = "nothing-to-merge"
                continue
            m = git("merge", "--no-ff", "-m", f"council merge {tid}",
                    f"council/{run_name}/{tid}", cwd=integ_wt, check=False)
            if m.returncode != 0:
                git("merge", "--abort", cwd=integ_wt, check=False)
                res["merge"] = "conflict"
                log(f"  [{tid}] merge CONFLICT — left out of integration")
            else:
                res["merge"] = "ok"

    if not keep_worktrees:
        for tid in by_id:
            git("worktree", "remove", "--force", str(WT_ROOT / run_name / tid),
                check=False)

    report = build_report(run, integ_branch, str(integ_wt), waves, results, tasks)
    run.write_json("report.json", report)
    run.write_text("report.md", render_report_md(report))
    run.set_state(stage="fanned-out", integration_branch=integ_branch)
    s = report["summary"]
    log(f"done: {s['ok']}/{s['total']} ok, {s['failed']} failed, "
        f"{s['merged']} merged into {integ_branch}")
    return report, integ_branch


def cmd_fanout(args: argparse.Namespace) -> int:
    run = Run.open(Path(args.run))
    if not run.has("tasks.json"):
        raise SystemExit(f"no tasks.json in {run.path}; run `plan` first")
    tasks = run.read_json("tasks.json")
    validate_tasks(tasks)
    analyze_checkpoint(run, tasks)
    waves = plan_waves(tasks)

    cfg = resolve_config({"intensity": args.intensity, "worker": args.worker,
                          "verifier": args.verifier, "codex_effort": args.codex_effort,
                          "max_workers": args.max_workers})
    global CODEX_REASONING
    CODEX_REASONING = cfg["codex_effort"]
    worker = parse_engine_value(cfg["worker"])
    verifier = parse_engine_value(cfg["verifier"])
    cores = max(1, (os.cpu_count() or 3) - 2)
    cap = min(cfg["max_workers"], cores)

    if args.estimate:
        print(f"council fanout — {len(tasks)} tasks in {len(waves)} wave(s); "
              f"intensity {cfg['intensity']}; worker {worker.label}; "
              f"verifier {verifier.label}; concurrency {cap}")
        for i, wave in enumerate(waves, 1):
            print(f"  wave {i}: {', '.join(wave)}")
        print("Each task spawns one worker + one verifier. Worktrees are "
              "isolated; nothing is merged into your branch — results land on an "
              "integration branch for review.")
        return 0

    _report, integ_branch = execute_dag(run, tasks, lambda _tid: worker,
                                        verifier, cap, args.keep_worktrees)
    print(integ_branch)  # stdout: integration branch for the host to surface
    return 0


def parse_agents_pool(spec: str) -> list[Engine]:
    """Expand an agent-pool spec into an ordered list of engines. Grammar:
    "<cli>:<model>[*<count>](,<cli>:<model>[*<count>])*", e.g.
    "codex:gpt-5.5*3,claude:haiku*2" -> three codex + two claude engines. Pure
    (covered by --self-test). Raises ValueError on a malformed spec."""
    pool: list[Engine] = []
    for raw in spec.split(","):
        part = raw.strip()
        if not part:
            continue
        engine_spec, star, count_s = part.partition("*")
        cli, _, model = engine_spec.strip().partition(":")
        if cli not in ("claude", "codex") or not model:
            raise ValueError(f"agent must be claude:<model> or codex:<model>, "
                             f"got {engine_spec.strip()!r}")
        if star:
            try:
                count = int(count_s)
            except ValueError as exc:
                raise ValueError(f"bad count in agent spec {part!r}") from exc
        else:
            count = 1
        if count <= 0:
            raise ValueError(f"agent count must be >= 1 in {part!r}")
        pool.extend(Engine(cli, model) for _ in range(count))
    if not pool:
        raise ValueError(f"empty agent pool from spec {spec!r}")
    return pool


def assign_agents(task_ids: list[str], pool: list[Engine]) -> dict[str, Engine]:
    """Round-robin assign each task id to an engine from the pool. Pure."""
    if not pool:
        raise ValueError("cannot assign tasks to an empty agent pool")
    return {tid: pool[i % len(pool)] for i, tid in enumerate(task_ids)}


def cmd_fleet(args: argparse.Namespace) -> int:
    tasks_path = Path(args.tasks)
    if not tasks_path.exists():
        raise SystemExit(f"tasks file not found: {tasks_path}")
    tasks = json.loads(tasks_path.read_text())
    validate_tasks(tasks)
    analyze_tasks_file(tasks, tasks_path)
    waves = plan_waves(tasks)
    pool = parse_agents_pool(args.agents)

    cfg = resolve_config({"intensity": args.intensity, "verifier": args.verifier,
                          "codex_effort": args.codex_effort,
                          "max_workers": args.max_workers})
    global CODEX_REASONING
    CODEX_REASONING = cfg["codex_effort"]
    verifier = parse_engine_value(cfg["verifier"])
    cap = min(cfg["max_workers"], max(1, (os.cpu_count() or 3) - 2))
    ordered_ids = [tid for wave in waves for tid in wave]
    assignment = assign_agents(ordered_ids, pool)

    if args.estimate:
        print(f"council fleet — {len(tasks)} tasks in {len(waves)} wave(s); "
              f"pool [{', '.join(e.label for e in pool)}]; "
              f"verifier {verifier.label}; concurrency {cap}")
        for i, wave in enumerate(waves, 1):
            print(f"  wave {i}: "
                  + ", ".join(f"{t}->{assignment[t].label}" for t in wave))
        return 0

    run = Run.create(f"fleet-{tasks_path.stem}", args.slug)
    run.write_json("tasks.json", tasks)
    run.set_state(stage="fleet", agents=[e.label for e in pool])
    log(f"run dir: {run.path}")
    _report, integ_branch = execute_dag(run, tasks, lambda tid: assignment[tid],
                                        verifier, cap, args.keep_worktrees)
    print(integ_branch)
    return 0


def cmd_split(args: argparse.Namespace) -> int:
    """Carve a path subtree out into a new GitHub repo with its history
    preserved (git subtree split). Never touches the host branch — it works on
    a throwaway council/split/<name> branch."""
    path = args.path.rstrip("/")
    if not (REPO_ROOT / path).exists():
        raise SystemExit(f"path not found in repo: {path}")
    owner, sep, name = args.dest.partition("/")
    if not sep or not owner or not name or "/" in name:
        raise SystemExit(f"--dest must be owner/name, got {args.dest!r}")
    if not have_git_subtree():
        raise SystemExit("git subtree is unavailable; install git-subtree "
                         "(git contrib / git-extras package)")
    dest_url = _split_dest_url(owner, name)
    branch = f"council/split/{_slugify(name)}"

    if args.dry_run:
        print(f"[dry-run] extract '{path}' into {owner}/{name}, history preserved:")
        print(f"  git subtree split --prefix {path} -b {branch}")
        if args.push:
            print(f"  gh repo create {owner}/{name} --{args.visibility}   "
                  "# if it does not already exist")
            print(f"  git push {dest_url} {branch}:main")
        print(f"  # then, as a separate change, optionally replace the in-repo "
              f"copy:\n  #   git rm -r {path} && git submodule add {dest_url} {path}")
        return 0

    git("branch", "-D", branch, check=False)
    log(f"splitting {path} -> {branch} (history-preserving)")
    git("subtree", "split", "--prefix", path, "-b", branch, timeout=600)

    if not args.push:
        print(f"created local branch {branch} with the extracted history of "
              f"{path}. Push it to a new repo when ready:")
        print(f"  gh repo create {owner}/{name} --{args.visibility}")
        print(f"  git push {dest_url} {branch}:main")
        return 0

    try:
        if gh("repo", "view", f"{owner}/{name}", check=False).returncode == 0:
            log(f"{owner}/{name} already exists; skipping create")
        else:
            log(f"creating {owner}/{name} ({args.visibility})")
            gh("repo", "create", f"{owner}/{name}", f"--{args.visibility}")
        log(f"pushing {branch} -> {dest_url}:main")
        git("push", dest_url, f"{branch}:main")
    except subprocess.CalledProcessError as exc:
        detail = (exc.stderr or exc.stdout or str(exc)).strip()[:300]
        raise SystemExit(f"split push failed (branch {branch} kept for retry): "
                         f"{detail}")
    git("branch", "-D", branch, check=False)

    print(f"extracted {path} into {owner}/{name} with history preserved.")
    print("To replace the in-repo copy with a reference, in a separate change:")
    print(f"  git rm -r {path}")
    print(f"  git submodule add {dest_url} {path}")
    return 0


def build_report(run: Run, integ_branch: str, integ_wt: str,
                 waves: list[list[str]], results: dict[str, dict],
                 task_defs: list[dict]) -> dict:
    task_map = {t["id"]: t for t in task_defs}
    no_verify = sorted(tid for tid in results
                       if not str(task_map.get(tid, {}).get("verify", "")).strip())
    rows = []
    ok = failed = merged = 0
    for tid, r in results.items():
        good = r.get("status") == "ok" and r.get("merge") in ("ok", None)
        ok += 1 if r.get("status") == "ok" else 0
        failed += 0 if r.get("status") == "ok" else 1
        merged += 1 if r.get("merge") == "ok" else 0
        rows.append({
            "task_id": tid, "status": r.get("status"),
            "merge": r.get("merge"), "model": r.get("model"),
            "files_changed": r.get("files_changed", []),
            "verify_rc": r.get("verify_rc"),
            "verifier_satisfied": (r.get("verdict") or {}).get("satisfied"),
            "out_of_bounds": r.get("out_of_bounds", []),
            "branch": r.get("branch"), "good": good,
        })
    return {
        "run": run.path.name, "integration_branch": integ_branch,
        "integration_worktree": integ_wt, "waves": waves, "tasks": rows,
        "no_verify": no_verify,
        "summary": {"total": len(results), "ok": ok, "failed": failed,
                    "merged": merged},
    }


def render_report_md(report: dict) -> str:
    s = report["summary"]
    lines = [f"# council fan-out report — {report['run']}", "",
             f"- integration branch: `{report['integration_branch']}`",
             f"- worktree: `{report['integration_worktree']}`",
             f"- result: **{s['ok']}/{s['total']} ok**, {s['failed']} failed, "
             f"{s['merged']} merged", "", "## Tasks", "",
             "| task | status | merge | model | files | verify | verifier |",
             "|---|---|---|---|---|---|---|"]
    for t in report["tasks"]:
        lines.append(
            f"| {t['task_id']} | {t['status']} | {t['merge']} | {t['model']} "
            f"| {len(t['files_changed'])} | "
            f"{'-' if t['verify_rc'] is None else t['verify_rc']} "
            f"| {t['verifier_satisfied']} |")
    failures = [t for t in report["tasks"] if not t["good"]]
    if failures:
        lines += ["", "## Needs attention", ""]
        for t in failures:
            note = t["status"]
            if t["merge"] == "conflict":
                note += " + merge conflict"
            if t["out_of_bounds"]:
                note += f" (touched out-of-bounds: {t['out_of_bounds']})"
            lines.append(f"- `{t['task_id']}`: {note}")
    no_verify = report.get("no_verify", [])
    if no_verify:
        lines += ["", "## Tasks with no verify command", "",
                  "These ran without an automated check — only the adversarial "
                  "verifier reviewed them:", ""]
        lines += [f"- `{tid}`" for tid in no_verify]
    lines += ["", f"Review: `git -C {report['integration_worktree']} log --oneline`"
              f" or `git checkout {report['integration_branch']}`."]
    return "\n".join(lines) + "\n"


# --------------------------------------------------------------------------
# commands
# --------------------------------------------------------------------------

def log(msg: str) -> None:
    print(f"[council] {msg}", file=sys.stderr, flush=True)


def read_brief(arg: str) -> str:
    if arg == "-":
        return sys.stdin.read()
    p = Path(arg)
    if p.exists():
        return p.read_text()
    return arg


def cmd_plan(args: argparse.Namespace) -> int:
    cfg = resolve_config({"intensity": args.intensity, "planner_a": args.planner_a,
                          "planner_b": args.planner_b, "consolidator": args.consolidator,
                          "rounds": args.rounds, "codex_effort": args.codex_effort})
    global CODEX_REASONING
    CODEX_REASONING = cfg["codex_effort"]
    a = parse_engine_value(cfg["planner_a"])
    b = parse_engine_value(cfg["planner_b"])
    consolidator = parse_engine_value(cfg["consolidator"])
    rounds = cfg["rounds"]

    if args.estimate:
        calls = 2 + rounds * 4 + 1
        print(f"council plan — intensity {cfg['intensity']}, "
              f"estimated model calls: {calls}")
        print(f"  stage 1 dual plans      : 2  ({a.label}, {b.label})")
        print(f"  stage 2 critique+revise : {rounds * 4}  ({rounds} rounds x "
              f"[2 critiques + 2 revisions])")
        print(f"  stage 4 consolidation   : 1  ({consolidator.label})")
        print(f"  codex reasoning effort  : {cfg['codex_effort']}")
        print("These are expensive-tier calls; fan-out (cheap workers) is "
              "separate. Multi-agent runs ~15x the tokens of a single chat — "
              "use council only for large, decomposable work.")
        return 0

    brief = read_brief(args.brief)
    run = Run.open(Path(args.run)) if args.run else Run.create(brief, args.slug)
    run.write_text("brief.md", brief)
    spec_ref = prepare_spec_ref(run, brief, args.slug)
    spec_seed = read_spec_dir(args.spec_dir)
    constitution_context = read_constitution_context()
    run.set_state(stage="plan", intensity=cfg["intensity"], rounds=rounds,
                  planner_a=a.label, planner_b=b.label,
                  spec_id=spec_ref.name, spec_slug=spec_ref.slug,
                  spec_relpath=spec_ref.relpath,
                  spec_dir=args.spec_dir)
    log(f"run dir: {run.path}")
    log(f"intensity {cfg['intensity']}: {a.label} ║ {b.label}, {rounds} round(s), "
        f"codex effort {cfg['codex_effort']}")

    plan_a, plan_b = stage_plan(run, brief, a, b, constitution_context)
    for rnd in range(1, rounds + 1):
        plan_a, plan_b = stage_critique_round(
            run, brief, a, b, plan_a, plan_b, rnd, constitution_context)
    tasks = stage_consolidate(run, brief, plan_a, plan_b, rounds, consolidator,
                              constitution_context, spec_ref, spec_seed)
    run.set_state(stage="planned", task_count=len(tasks))

    waves = plan_waves(tasks)
    total = sum(c for _, c in run.costs)
    log(f"done: {len(tasks)} tasks in {len(waves)} wave(s); "
        f"recorded claude cost ${total:.2f} (codex cost not reported by CLI)")
    print(str(run.path))  # stdout: the run dir, for the host to pick up
    return 0


def parse_engine_value(spec: str) -> Engine:
    """spec form: "cli:model" e.g. "claude:opus" or "codex:gpt-5.5"."""
    cli, _, model = spec.partition(":")
    if cli not in ("claude", "codex") or not model:
        raise SystemExit(f"engine must be claude:<model> or codex:<model>, "
                         f"got {spec!r}")
    return Engine(cli, model)


# --------------------------------------------------------------------------
# config: intensity presets + per-role overrides (council.toml)
# --------------------------------------------------------------------------

def load_config_at(path: Path) -> dict:
    if not path.exists():
        return {}
    with path.open("rb") as fh:
        raw = tomllib.load(fh)
    return {k: v for k, v in raw.items() if k in CONFIG_KEYS}


def merge_config(file_cfg: dict, cli_overrides: dict) -> dict:
    """Resolve final settings: intensity preset < council.toml < CLI flags.
    Pure (no IO) so it is covered by --self-test."""
    intensity = (cli_overrides.get("intensity") or file_cfg.get("intensity")
                 or DEFAULT_INTENSITY)
    if intensity not in PRESETS:
        raise ValueError(f"unknown intensity {intensity!r}; choose from "
                         f"{', '.join(PRESETS)}")
    resolved = dict(BASE_ROLES)
    resolved.update(PRESETS[intensity])
    for src in (file_cfg, cli_overrides):
        for key, val in src.items():
            if key == "intensity" or val is None or key not in CONFIG_KEYS:
                continue
            resolved[key] = val
    resolved["intensity"] = intensity
    return resolved


def resolve_config(cli_overrides: dict) -> dict:
    # precedence: preset < user council.toml < project .council.toml < CLI
    file_cfg = {**load_config_at(USER_CONFIG_PATH),
                **load_config_at(PROJECT_CONFIG_PATH)}
    return merge_config(file_cfg, cli_overrides)


def coerce_config_value(key: str, raw: str):
    """Validate + type a `config set` value. Raises ValueError on bad input
    (callers convert to a clean CLI exit)."""
    if key not in CONFIG_KEYS:
        raise ValueError(f"unknown key {key!r}; choose from {', '.join(CONFIG_KEYS)}")
    if key == "intensity":
        if raw not in PRESETS:
            raise ValueError(f"intensity must be one of {', '.join(PRESETS)}")
        return raw
    if key in INT_KEYS:
        try:
            return int(raw)
        except ValueError:
            raise ValueError(f"{key} must be an integer, got {raw!r}")
    if key == "codex_effort":
        if raw not in CODEX_EFFORTS:
            raise ValueError(f"codex_effort must be one of {', '.join(CODEX_EFFORTS)}")
        return raw
    if key in ROLE_KEYS:
        if ":" not in raw or raw.split(":", 1)[0] not in ("claude", "codex"):
            raise ValueError(f"{key} must be claude:<model> or codex:<model>, "
                             f"got {raw!r}")
        return raw
    return raw


def save_config_at(path: Path, cfg: dict) -> None:
    lines = ["# council configuration. CLI flags override these per run;",
             "# `council config set <key> <value>` edits this file. Keys not",
             "# listed follow the chosen intensity preset (quick|standard|"
             "thorough|max).", ""]
    for key in CONFIG_KEYS:
        if key in cfg and cfg[key] is not None:
            val = cfg[key]
            if isinstance(val, bool):  # not expected, but keep TOML valid
                lines.append(f"{key} = {str(val).lower()}")
            elif isinstance(val, int):
                lines.append(f"{key} = {val}")
            else:
                lines.append(f'{key} = "{val}"')
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text("\n".join(lines) + "\n")


def cmd_config(args: argparse.Namespace) -> int:
    target = PROJECT_CONFIG_PATH if getattr(args, "project", False) else USER_CONFIG_PATH
    action = args.action
    if action == "path":
        print(f"user:    {USER_CONFIG_PATH}")
        print(f"project: {PROJECT_CONFIG_PATH}")
        return 0
    if action == "show":
        user_cfg = load_config_at(USER_CONFIG_PATH)
        proj_cfg = load_config_at(PROJECT_CONFIG_PATH)
        resolved = resolve_config({})
        print(f"user config:    {USER_CONFIG_PATH}"
              f"{'' if USER_CONFIG_PATH.exists() else '  (none)'}")
        print(f"project config: {PROJECT_CONFIG_PATH}"
              f"{'' if PROJECT_CONFIG_PATH.exists() else '  (none)'}")
        print(f"intensity: {resolved['intensity']}")
        for key in CONFIG_KEYS[1:]:
            src = (" (project)" if key in proj_cfg
                   else " (user)" if key in user_cfg else "")
            print(f"  {key} = {resolved[key]}{src}")
        return 0
    if action == "get":
        if not args.key or args.key not in CONFIG_KEYS:
            raise SystemExit(f"config get requires a known key "
                             f"({', '.join(CONFIG_KEYS)})")
        print(resolve_config({})[args.key])
        return 0
    if action == "set":
        if not args.key or args.value is None:
            raise SystemExit("config set requires <key> <value>")
        file_cfg = load_config_at(target)
        try:
            file_cfg[args.key] = coerce_config_value(args.key, args.value)
        except ValueError as exc:
            raise SystemExit(str(exc))
        save_config_at(target, file_cfg)
        print(f"set {args.key} = {file_cfg[args.key]!r} in {target}")
        return 0
    if action == "unset":
        if not args.key:
            raise SystemExit("config unset requires a key")
        file_cfg = load_config_at(target)
        if args.key in file_cfg:
            del file_cfg[args.key]
            save_config_at(target, file_cfg)
            print(f"unset {args.key} in {target}")
        else:
            print(f"{args.key} not set in {target}")
        return 0
    raise SystemExit(f"unknown config action {action!r}")


def cmd_self_test(_args: argparse.Namespace) -> int:
    failures: list[str] = []

    def check(name: str, cond: bool) -> None:
        if not cond:
            failures.append(name)
        print(f"  {'ok  ' if cond else 'FAIL'} {name}")

    # plan_waves
    tasks = [
        {"id": "a", "depends_on": []},
        {"id": "b", "depends_on": ["a"]},
        {"id": "c", "depends_on": ["a"]},
        {"id": "d", "depends_on": ["b", "c"]},
    ]
    check("plan_waves groups by dependency",
          plan_waves(tasks) == [["a"], ["b", "c"], ["d"]])
    check("plan_waves rejects cycle", _raises(
        lambda: plan_waves([{"id": "x", "depends_on": ["y"]},
                            {"id": "y", "depends_on": ["x"]}])))
    check("plan_waves rejects unknown dep", _raises(
        lambda: plan_waves([{"id": "x", "depends_on": ["nope"]}])))

    # extract_json
    check("extract_json plain", extract_json('{"a": 1}') == {"a": 1})
    check("extract_json fenced",
          extract_json('text\n```json\n{"a": [1,2]}\n```\nmore') == {"a": [1, 2]})
    check("extract_json with braces in string",
          extract_json('{"k": "a{b}c"}') == {"k": "a{b}c"})
    check("extract_json keeps code fences inside string value",
          extract_json('{"md": "see ```bash\\nx\\n``` end", "n": 1}')
          == {"md": "see ```bash\nx\n``` end", "n": 1})
    check("extract_json outer fence with inner fences",
          extract_json('```json\n{"md": "a ```inner``` b"}\n```')
          == {"md": "a ```inner``` b"})
    check("extract_json none raises", _raises(lambda: extract_json("no json here")))

    # render
    check("render replaces tokens",
          render("hi {{name}} {{name}}", name="x") == "hi x x")
    check("render leaves JSON braces",
          render('{"x": {{v}}}', v="1") == '{"x": 1}')

    # validate_tasks
    check("validate_tasks rejects empty", _raises(lambda: validate_tasks([])))
    check("validate_tasks rejects missing fields",
          _raises(lambda: validate_tasks([{"id": "a"}])))

    # baseline rules + verify checks
    check("_baseline loadable and non-empty", bool(BASELINE_PROMPT.strip()))
    global log
    captured: list[str] = []
    orig_log = log
    log = lambda m: captured.append(m)  # noqa: E731
    try:
        validate_tasks([{"id": "t1", "objective": "o", "depends_on": [],
                         "paths": [], "model": "haiku", "verify": "  "}])
    finally:
        log = orig_log
    check("validate_tasks warns on empty verify",
          any("no verify" in m for m in captured))

    # Spec Kit task markdown + analyze helpers
    sample_task = {
        "id": "T1",
        "title": "Implement one thing",
        "objective": "Change exactly one thing",
        "output_format": "Code edits",
        "paths": ["platform/agents/council/council.py"],
        "depends_on": [],
        "difficulty": "moderate",
        "model": "haiku",
        "verify": "python3 platform/agents/council/council.py --self-test",
        "boundaries": "Stay in scope",
    }
    rendered_tasks = render_tasks_md([sample_task], SpecRef(7, "sdd-aware-council"))
    check("tasks.json -> tasks.md -> parse roundtrips",
          parse_tasks_md(rendered_tasks) == [sample_task])
    edited_tasks = rendered_tasks.replace("Change exactly one thing",
                                          "Change a different thing")
    check("tasks.md bijection mismatch hard-fails",
          _raises(lambda: assert_tasks_bijection([sample_task], edited_tasks)))
    bad_marker = rendered_tasks.replace("<!-- council-task-id: T1 -->",
                                        "<!-- council-task-id: T2 -->")
    check("tasks.md parser rejects marker/header mismatch",
          _raises(lambda: parse_tasks_md(bad_marker)))
    gate_msg = ""
    try:
        analyze_checkpoint(Run(Path("/tmp/council-self-test-run")), [sample_task])
    except ValueError as exc:
        gate_msg = str(exc)
    check("analyze gate names checkpoint 1 and regenerate command",
          "analyze gate checkpoint 1 failed" in gate_msg
          and "Regenerate with: council plan --run" in gate_msg)

    # constitution handling is bounded and limited to reasoning roles.
    check("constitution context is bounded",
          len(read_constitution_context()) <= MAX_CONSTITUTION_CHARS + 20)
    for role in ("planner", "critic", "reviser", "consolidator"):
        check(f"constitution token present in {role}",
              "{{constitution}}" in load_prompt(role))
    check("constitution token absent from worker",
          "{{constitution}}" not in load_prompt("worker"))
    check("constitution token absent from verifier",
          "{{constitution}}" not in load_prompt("verifier"))
    with tempfile.TemporaryDirectory() as td:
        repo = Path(td)
        check("constitution failure detects missing file",
              "missing constitution" in (constitution_failure(repo) or ""))
        cpath = repo / ".specify" / "memory" / "constitution.md"
        cpath.parent.mkdir(parents=True)
        cpath.write_text("# Constitution\n\n[PROJECT NAME]\n")
        check("constitution failure detects placeholder",
              "placeholder" in (constitution_failure(repo) or ""))
        cpath.write_text("# Constitution\n\nShip small, verified changes.\n")
        check("constitution failure accepts concrete file",
              constitution_failure(repo) is None)

    # Spec numbering and slug derivation
    check("free-text brief derives slug from first line",
          derive_feature_slug("Fuse council with Spec Kit\nextra", None)
          == "fuse-council-with-spec-kit")
    check("explicit --slug is reused for spec slug",
          derive_feature_slug("ignored", "My Feature") == "my-feature")
    with tempfile.TemporaryDirectory() as td:
        specs = Path(td) / "specs"
        specs.mkdir()
        (specs / "001-old").mkdir()
        ref = allocate_spec_ref("new feature", specs)
        check("NNN allocation uses max(existing)+1",
              ref == SpecRef(2, "new-feature"))
        (specs / "003-duplicate").mkdir()
        check("NNN allocation fail-fast on existing slug",
              _raises(lambda: allocate_spec_ref("duplicate", specs)))

    # merge_config (intensity presets + precedence)
    std = merge_config({}, {})
    check("default intensity is standard",
          std["intensity"] == "standard" and std["rounds"] == 2
          and std["worker"] == "claude:sonnet" and std["codex_effort"] == "high")
    thorough = merge_config({"intensity": "thorough"}, {})
    check("thorough preset bumps rounds + worker",
          thorough["rounds"] == 3 and thorough["worker"] == "claude:sonnet")
    check("cli overrides preset",
          merge_config({"intensity": "quick"}, {"rounds": 5})["rounds"] == 5)
    check("file overrides preset, cli overrides file",
          merge_config({"worker": "claude:sonnet"},
                       {"worker": "claude:opus"})["worker"] == "claude:opus")
    check("file intensity used when no cli",
          merge_config({"intensity": "max"}, {})["codex_effort"] == "xhigh")
    check("merge_config rejects bad intensity",
          _raises(lambda: merge_config({}, {"intensity": "nope"})))
    check("coerce rejects unknown key",
          _raises(lambda: coerce_config_value("bogus", "x")))
    check("coerce accepts codex worker",
          coerce_config_value("worker", "codex:gpt-5.5") == "codex:gpt-5.5")
    check("coerce types ints", coerce_config_value("rounds", "3") == 3)

    # parse_agents_pool + assign_agents (engine-agnostic fleet)
    check("parse_agents_pool expands counts in order",
          parse_agents_pool("codex:gpt-5.5*2,claude:haiku*1")
          == [Engine("codex", "gpt-5.5"), Engine("codex", "gpt-5.5"),
              Engine("claude", "haiku")])
    check("parse_agents_pool defaults count to 1",
          parse_agents_pool("claude:opus") == [Engine("claude", "opus")])
    check("parse_agents_pool rejects zero count",
          _raises(lambda: parse_agents_pool("codex:x*0")))
    check("parse_agents_pool rejects unknown cli",
          _raises(lambda: parse_agents_pool("ollama:x*1")))
    check("parse_agents_pool rejects malformed spec",
          _raises(lambda: parse_agents_pool("notvalid")))
    check("assign_agents round-robins",
          assign_agents(["t1", "t2", "t3"],
                        [Engine("claude", "haiku"), Engine("codex", "gpt-5.5")])
          == {"t1": Engine("claude", "haiku"),
              "t2": Engine("codex", "gpt-5.5"),
              "t3": Engine("claude", "haiku")})
    check("assign_agents rejects empty pool",
          _raises(lambda: assign_agents(["t1"], [])))

    # split
    check("_split_dest_url canonical ssh remote",
          _split_dest_url("o", "n") == "git@github.com:o/n.git")

    # localize_verify (verify runs in the worktree, not the host repo root)
    check("localize_verify rewrites repo root to the worktree",
          localize_verify("cd /workspace/services/foo && npm test",
                          "/workspace", "/tmp/wt/T1")
          == "cd /tmp/wt/T1/services/foo && npm test")
    check("localize_verify leaves relative commands untouched",
          localize_verify("npm test", "/workspace", "/tmp/wt/T1") == "npm test")

    print(f"\n{'PASS' if not failures else 'FAIL: ' + ', '.join(failures)}")
    return 1 if failures else 0


def _raises(fn: Callable[[], object]) -> bool:
    try:
        fn()
        return False
    except Exception:
        return True


def build_parser() -> argparse.ArgumentParser:
    p = argparse.ArgumentParser(prog="council", description=__doc__)
    p.add_argument("--self-test", action="store_true",
                   help="run pure-function checks (no model calls) and exit")
    sub = p.add_subparsers(dest="command")

    pl = sub.add_parser("plan", help="stages 1-4: dual plans, critique, consolidate")
    pl.add_argument("--brief", help="brief file path, or - for stdin")
    pl.add_argument("--run", help="existing run dir to resume")
    pl.add_argument("--slug", help="slug for the run dir name")
    pl.add_argument("--spec-dir", help="existing specs/NNN-slug dir to consume")
    pl.add_argument("--intensity", choices=list(PRESETS),
                    help="preset (overrides council.toml for this run)")
    pl.add_argument("--rounds", type=int, default=None, help="override critique rounds")
    pl.add_argument("--planner-a", default=None, help="override, form cli:model")
    pl.add_argument("--planner-b", default=None, help="override, form cli:model")
    pl.add_argument("--consolidator", default=None, help="override, form cli:model")
    pl.add_argument("--codex-effort", default=None, choices=list(CODEX_EFFORTS),
                    help="override codex reasoning effort")
    pl.add_argument("--estimate", action="store_true",
                    help="print planned call count and exit without spending")
    pl.set_defaults(func=cmd_plan)

    fo = sub.add_parser("fanout", help="stages 5-6: execute the task DAG, "
                                       "verify, reconcile onto a branch")
    fo.add_argument("--run", required=True, help="run dir with a tasks.json")
    fo.add_argument("--intensity", choices=list(PRESETS),
                    help="preset (overrides council.toml for this run)")
    fo.add_argument("--max-workers", type=int, default=None,
                    help="override max concurrent workers (clamped to cores-2)")
    fo.add_argument("--worker", default=None,
                    help="override worker engine, form claude:model or codex:model")
    fo.add_argument("--verifier", default=None, help="override verifier, form cli:model")
    fo.add_argument("--codex-effort", default=None, choices=list(CODEX_EFFORTS),
                    help="override codex reasoning effort")
    fo.add_argument("--keep-worktrees", action="store_true",
                    help="keep per-task worktrees for inspection")
    fo.add_argument("--estimate", action="store_true",
                    help="print the wave/worker plan and exit without spending")
    fo.set_defaults(func=cmd_fanout)

    fl = sub.add_parser("fleet", help="run a task DAG against an ad-hoc, "
                                      "engine-agnostic worker pool (no plan phase)")
    fl.add_argument("--tasks", required=True,
                    help="path to a tasks.json (any DAG; need not come from `plan`)")
    fl.add_argument("--agents", required=True,
                    help="pool spec, e.g. 'codex:gpt-5.5*3,claude:haiku*2' — "
                         "round-robined across the tasks")
    fl.add_argument("--verifier", default=None, help="override verifier, form cli:model")
    fl.add_argument("--intensity", choices=list(PRESETS),
                    help="preset (only its verifier/max-workers/codex-effort apply)")
    fl.add_argument("--codex-effort", default=None, choices=list(CODEX_EFFORTS),
                    help="codex reasoning effort for codex agents in the pool")
    fl.add_argument("--max-workers", type=int, default=None,
                    help="override max concurrent workers (clamped to cores-2)")
    fl.add_argument("--keep-worktrees", action="store_true",
                    help="keep per-task worktrees for inspection")
    fl.add_argument("--slug", help="slug for the run dir name")
    fl.add_argument("--estimate", action="store_true",
                    help="print the pool/wave/assignment plan and exit")
    fl.set_defaults(func=cmd_fleet)

    sp = sub.add_parser("split", help="extract a path subtree into a new GitHub "
                                      "repo, preserving that path's history")
    sp.add_argument("--path", required=True,
                    help="path under the repo to extract, e.g. services/foo")
    sp.add_argument("--dest", required=True, help="new repo as owner/name")
    sp.add_argument("--visibility", choices=["private", "public"],
                    default="private", help="new repo visibility (default private)")
    sp.add_argument("--no-push", dest="push", action="store_false",
                    help="only create the local extracted branch; don't create "
                         "or push the remote")
    sp.add_argument("--dry-run", action="store_true",
                    help="print the commands and exit without touching anything")
    sp.set_defaults(func=cmd_split, push=True)

    cf = sub.add_parser("config", help="show or change model/intensity config "
                                       "(council.toml)")
    cf.add_argument("action", choices=["show", "get", "set", "unset", "path"])
    cf.add_argument("key", nargs="?", help="config key (see `config show`)")
    cf.add_argument("value", nargs="?", help="value (for `set`)")
    cf.add_argument("--project", action="store_true",
                    help="target the per-project .council.toml instead of the "
                         "user-global config (for set/unset)")
    cf.set_defaults(func=cmd_config)
    return p


def main(argv: Optional[list[str]] = None) -> int:
    args = build_parser().parse_args(argv)
    if args.self_test:
        return cmd_self_test(args)
    if not getattr(args, "command", None):
        build_parser().print_help()
        return 2
    if args.command == "plan" and not args.estimate and not args.brief:
        raise SystemExit("plan requires --brief (or --brief -)")
    try:
        return args.func(args)
    except ValueError as exc:  # e.g. bad intensity in council.toml
        print(f"council: {exc}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
