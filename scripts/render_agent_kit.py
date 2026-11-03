#!/usr/bin/env python3
"""Render the agent kit's repo skills, agents and runner runtime into a tree.

Two kinds of output:

- committed: the runner runtime under runner/manifests/runtime, checked in
  and compared by --check;
- local: .claude/skills, .agents/skills and .claude/agents, which are
  gitignored. --write renders them into this checkout and --output into a
  separate tree (the runtime-home package uses that). --write also renders
  this repository's own .council/context; --output does not.
"""

from __future__ import annotations

import argparse
import filecmp
import importlib.util
import re
import shutil
import stat
import sys
from dataclasses import dataclass
from pathlib import Path

KIT_ROOT = Path(__file__).resolve().parent.parent
REPOSITORY_ROOT = KIT_ROOT
# skills/repo/<name>/claude/ renders to .claude/skills/<name>/ and
# skills/repo/<name>/codex/ to .agents/skills/<name>/.
REPO_SKILLS_ROOT = KIT_ROOT / "skills" / "repo"
SKILL_SURFACES = (("claude", Path(".claude/skills")), ("codex", Path(".agents/skills")))
AGENTS_ROOT = KIT_ROOT / "agents"
AGENTS_DESTINATION = Path(".claude/agents")
# This repository's own council context; rendered by --write only.
COUNCIL_CONTEXT_ROOT = KIT_ROOT / "council" / "context"
COUNCIL_CONTEXT_DESTINATION = Path(".council/context")
RUNNER_RUNTIME_TEMPLATE_ROOT = KIT_ROOT / "runner" / "template"
RUNNER_RUNTIME_DESTINATION = Path("runner/manifests/runtime")
# installer/install.sh and installer/install-agents.sh (and the heredoc-bundle
# rendering they alone needed) were retired in agent-kit#40.
# setup-workstation.sh is rendered by scripts/render_registry.py instead.
COUNCIL_SRC = KIT_ROOT / "council"
MANIFEST_PATH = KIT_ROOT / "manifest.yaml"


@dataclass(frozen=True)
class RenderedFile:
    source: Path
    destination: Path
    relative_path: Path
    committed: bool


@dataclass(frozen=True)
class RenderFindings:
    missing: list[RenderedFile]
    drifted: list[RenderedFile]


@dataclass(frozen=True)
class DoctorCheck:
    name: str
    status: str
    detail: str


def files_under(source_root: Path, destination: Path, destination_root: Path, *, committed: bool) -> list[RenderedFile]:
    if not source_root.is_dir():
        raise FileNotFoundError(f"source root does not exist: {source_root}")
    files: list[RenderedFile] = []
    for source in sorted(source_root.rglob("*")):
        if not source.is_file():
            continue
        relative_path = destination / source.relative_to(source_root)
        files.append(
            RenderedFile(
                source=source,
                destination=destination_root / relative_path,
                relative_path=relative_path,
                committed=committed,
            ),
        )
    return files


def repo_skill_files(destination_root: Path) -> list[RenderedFile]:
    if not REPO_SKILLS_ROOT.is_dir():
        raise FileNotFoundError(f"repo skills root does not exist: {REPO_SKILLS_ROOT}")
    files: list[RenderedFile] = []
    for skill in sorted(path for path in REPO_SKILLS_ROOT.iterdir() if path.is_dir()):
        for agent, surface in SKILL_SURFACES:
            if (skill / agent).is_dir():
                files.extend(files_under(skill / agent, surface / skill.name, destination_root, committed=False))
    return files


def template_files(destination_root: Path, *, include_self: bool = False) -> list[RenderedFile]:
    """Every rendered file; include_self adds this repository's council context."""
    files = repo_skill_files(destination_root)
    files.extend(files_under(AGENTS_ROOT, AGENTS_DESTINATION, destination_root, committed=False))
    if include_self:
        files.extend(
            files_under(COUNCIL_CONTEXT_ROOT, COUNCIL_CONTEXT_DESTINATION, destination_root, committed=False),
        )
    files.extend(
        files_under(RUNNER_RUNTIME_TEMPLATE_ROOT, RUNNER_RUNTIME_DESTINATION, destination_root, committed=True),
    )
    return files


def council_toolkit_files() -> list[tuple[str, str]]:
    """(relpath, mode) for the shared council toolkit under council/."""
    files: list[tuple[str, str]] = []
    for path in sorted(COUNCIL_SRC.rglob("*")):
        if not path.is_file():
            continue
        rel_path = path.relative_to(COUNCIL_SRC)
        if (
            "__pycache__" in path.parts
            or "node_modules" in rel_path.parts
            or "coverage" in rel_path.parts
            or path.suffix == ".pyc"
            or path.suffix == ".map"
            or path.name.endswith(".tsbuildinfo")
            or (rel_path.parts and rel_path.parts[0] in {"ts", "ts-dist"})
        ):
            continue
        rel = rel_path.as_posix()
        if rel == "README.md":
            continue
        if rel not in {"council.mjs", "council.toml"} and (
            not rel_path.parts or rel_path.parts[0] not in {"prompts", "schemas"}
        ):
            continue
        files.append((rel, "0755" if rel == "council.mjs" else "0644"))
    return files


def render_findings(destination_root: Path, *, committed: bool) -> RenderFindings:
    """Missing and drifted outputs among the committed or the local ones."""
    drifted: list[RenderedFile] = []
    missing: list[RenderedFile] = []

    for rendered in template_files(destination_root, include_self=True):
        if rendered.committed != committed:
            continue
        if not rendered.destination.exists():
            missing.append(rendered)
        elif not filecmp.cmp(rendered.source, rendered.destination, shallow=False):
            drifted.append(rendered)

    return RenderFindings(missing=missing, drifted=drifted)


def check(destination_root: Path) -> int:
    # Only committed output is checked: the local output is gitignored.
    findings = render_findings(destination_root, committed=True)

    if not findings.missing and not findings.drifted:
        print("agent kit render check passed")
        return 0

    print_render_findings(findings)
    return 1


def print_render_findings(findings: RenderFindings) -> None:
    for rendered in findings.missing:
        print(f"missing: {rendered.relative_path}", file=sys.stderr)
    for rendered in findings.drifted:
        print(f"drifted: {rendered.relative_path}", file=sys.stderr)


def manifest_version() -> str:
    if not MANIFEST_PATH.exists():
        return "unknown"
    match = re.search(r"^version:\s*([^\s#]+)", MANIFEST_PATH.read_text(), re.MULTILINE)
    return match.group(1) if match else "unknown"


def manifest_check() -> DoctorCheck:
    if not MANIFEST_PATH.exists():
        return DoctorCheck(name="manifest", status="fail", detail="manifest.yaml is missing")

    validator_path = KIT_ROOT / "scripts" / "validate_manifest.py"
    spec = importlib.util.spec_from_file_location("validate_manifest", validator_path)
    if spec is None or spec.loader is None:
        return DoctorCheck(name="manifest", status="fail", detail="cannot load scripts/validate_manifest.py")

    validator = importlib.util.module_from_spec(spec)
    try:
        spec.loader.exec_module(validator)
        validator.validate_council_command_surface()
    except AssertionError as exc:
        return DoctorCheck(name="council-command-surface", status="fail", detail=str(exc))
    except (OSError, SystemExit) as exc:
        return DoctorCheck(name="manifest", status="fail", detail=str(exc))

    council_commands = getattr(validator, "REQUIRED_COUNCIL_CLI_COMMANDS_LABEL", "validated")
    return DoctorCheck(
        name="manifest",
        status="ok",
        detail=f"kit manifest version {manifest_version()}; council command surface validated: {council_commands}",
    )


def repo_skill_names(agent: str) -> set[str]:
    if not REPO_SKILLS_ROOT.is_dir():
        return set()
    return {path.parent.parent.name for path in REPO_SKILLS_ROOT.glob(f"*/{agent}/SKILL.md")}


def parity_check() -> DoctorCheck:
    claude_skills = repo_skill_names("claude")
    codex_skills = repo_skill_names("codex")
    skill_diff = sorted(claude_skills ^ codex_skills)

    if skill_diff:
        return DoctorCheck(name="parity", status="fail", detail="shared skill mismatch: " + ",".join(skill_diff))

    return DoctorCheck(
        name="parity",
        status="ok",
        detail=f"{len(claude_skills)} shared skills",
    )


# installer_artifact_check and grill_me_check were retired with install.sh/install-agents.sh (agent-kit#40);
# the knowledge-base probe went with the knowledge service (agent-kit#36).


def render_findings_detail(findings: RenderFindings) -> str:
    details = []
    if findings.missing:
        details.append("missing " + ",".join(str(item.relative_path) for item in findings.missing))
    if findings.drifted:
        details.append("drifted " + ",".join(str(item.relative_path) for item in findings.drifted))
    return "; ".join(details)


def render_doctor_check(name: str, *, committed: bool, ok: str, problem: str) -> DoctorCheck:
    findings = render_findings(REPOSITORY_ROOT, committed=committed)
    if not findings.missing and not findings.drifted:
        return DoctorCheck(name=name, status="ok", detail=ok)
    # A fresh clone has rendered nothing local yet; that is not a fault.
    local_total = sum(1 for item in template_files(REPOSITORY_ROOT, include_self=True) if not item.committed)
    if not committed and not findings.drifted and len(findings.missing) == local_total:
        detail = "not rendered; run scripts/render_agent_kit.py --write to use it"
        return DoctorCheck(name=name, status="ok", detail=detail)
    detail = render_findings_detail(findings)
    if not committed:
        detail = f"run scripts/render_agent_kit.py --write: {detail}"
    return DoctorCheck(name=name, status=problem, detail=detail)


def doctor(args: argparse.Namespace) -> int:
    checks = [
        render_doctor_check("render", committed=True, ok="generated files match templates", problem="fail"),
        render_doctor_check(
            "local-render",
            committed=False,
            ok="local skills, agents and council context match their sources",
            problem="warn",
        ),
    ]

    checks.append(manifest_check())
    checks.append(parity_check())

    print("agent kit doctor")
    for item in checks:
        print(f"{item.status:<4} {item.name}: {item.detail}")

    failures = sum(1 for item in checks if item.status == "fail")
    warnings = sum(1 for item in checks if item.status == "warn")
    print(f"summary: {len(checks) - failures - warnings} ok, {warnings} warn, {failures} fail")
    if failures or (args.strict and warnings):
        return 1
    return 0


def render(destination_root: Path, *, include_self: bool = False) -> int:
    rendered_count = 0
    for rendered in template_files(destination_root, include_self=include_self):
        source_mode = rendered.source.stat().st_mode
        mode = 0o755 if source_mode & stat.S_IXUSR else 0o644
        if rendered.destination.exists():
            same_mode = stat.S_IMODE(rendered.destination.stat().st_mode) == mode
            same_content = filecmp.cmp(rendered.source, rendered.destination, shallow=False)
            if same_content and same_mode:
                continue

        rendered.destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(rendered.source, rendered.destination)
        rendered.destination.chmod(mode)
        rendered_count += 1
    print(f"rendered {rendered_count} agent kit files into {destination_root}")
    return 0


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--check", action="store_true", help="verify the committed runner runtime matches its templates")
    mode.add_argument("--write", action="store_true", help="render everything, local output included, here")
    mode.add_argument("--output", type=Path, help="render skills, agents and the runner runtime into a directory")
    mode.add_argument("--doctor", action="store_true", help="run read-only agent kit diagnostics")
    parser.add_argument("--strict", action="store_true", help="make doctor warnings fail")
    return parser.parse_args(argv)


def main(argv: list[str]) -> int:
    args = parse_args(argv)
    if args.check:
        return check(REPOSITORY_ROOT)
    if args.write:
        return render(REPOSITORY_ROOT, include_self=True)
    if args.doctor:
        return doctor(args)
    return render(args.output.resolve())


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
