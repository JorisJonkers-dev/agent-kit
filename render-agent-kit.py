#!/usr/bin/env python3
"""Render checked-in agent kit templates into a repository tree."""

from __future__ import annotations

import argparse
import filecmp
import importlib.util
import json
import os
import re
import shutil
import stat
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Any, cast
from urllib import error, request

KIT_ROOT = Path(__file__).resolve().parent
REPOSITORY_ROOT = KIT_ROOT
REPO_TEMPLATE_ROOT = KIT_ROOT / "templates" / "repo"
RUNNER_RUNTIME_TEMPLATE_ROOT = KIT_ROOT / "templates" / "runner-runtime"
RUNNER_RUNTIME_DESTINATION = Path("runner-manifests/runtime")
# installer/install.sh and installer/install-agents.sh (and the heredoc-bundle
# rendering they alone needed) were retired with knowledge-api (agent-kit#40).
# setup-workstation.sh is rendered by scripts/render_registry.py instead.
COUNCIL_SRC = KIT_ROOT / "council"
SPECKIT_COMMANDS_SRC = REPO_TEMPLATE_ROOT / ".claude" / "commands"
CODEX_SPECKIT_SKILLS_SRC = REPO_TEMPLATE_ROOT / ".agents" / "skills"
MANIFEST_PATH = KIT_ROOT / "manifest.yaml"


@dataclass(frozen=True)
class RenderedFile:
    source: Path
    destination: Path
    relative_path: Path


@dataclass(frozen=True)
class RenderFindings:
    missing: list[RenderedFile]
    drifted: list[RenderedFile]


@dataclass(frozen=True)
class DoctorCheck:
    name: str
    status: str
    detail: str


def template_files(destination_root: Path) -> list[RenderedFile]:
    if not REPO_TEMPLATE_ROOT.is_dir():
        raise FileNotFoundError(f"template root does not exist: {REPO_TEMPLATE_ROOT}")

    files: list[RenderedFile] = []
    for source in sorted(REPO_TEMPLATE_ROOT.rglob("*")):
        if not source.is_file():
            continue
        relative_path = source.relative_to(REPO_TEMPLATE_ROOT)
        if relative_path.parts and relative_path.parts[0] == ".specify":
            continue
        files.append(
            RenderedFile(
                source=source,
                destination=destination_root / relative_path,
                relative_path=relative_path,
            ),
        )

    if not RUNNER_RUNTIME_TEMPLATE_ROOT.is_dir():
        raise FileNotFoundError(f"runner runtime template root does not exist: {RUNNER_RUNTIME_TEMPLATE_ROOT}")
    for source in sorted(RUNNER_RUNTIME_TEMPLATE_ROOT.rglob("*")):
        if not source.is_file():
            continue
        relative_path = RUNNER_RUNTIME_DESTINATION / source.relative_to(RUNNER_RUNTIME_TEMPLATE_ROOT)
        files.append(
            RenderedFile(
                source=source,
                destination=destination_root / relative_path,
                relative_path=relative_path,
            ),
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


def speckit_command_files() -> list[Path]:
    return sorted(SPECKIT_COMMANDS_SRC.glob("speckit.*.md"))


def codex_speckit_skill_files() -> list[Path]:
    return sorted(CODEX_SPECKIT_SKILLS_SRC.glob("speckit-*/SKILL.md"))


def render_findings(destination_root: Path) -> RenderFindings:
    drifted: list[RenderedFile] = []
    missing: list[RenderedFile] = []

    for rendered in template_files(destination_root):
        if not rendered.destination.exists():
            missing.append(rendered)
        elif not filecmp.cmp(rendered.source, rendered.destination, shallow=False):
            drifted.append(rendered)

    return RenderFindings(missing=missing, drifted=drifted)


def check(destination_root: Path) -> int:
    findings = render_findings(destination_root)

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


def skill_names(directory: Path) -> set[str]:
    if not directory.is_dir():
        return set()
    return {path.parent.name for path in directory.glob("*/SKILL.md")}


def speckit_command_names() -> set[str]:
    return {path.stem.removeprefix("speckit.") for path in speckit_command_files()}


def codex_speckit_names() -> set[str]:
    return {path.parent.name.removeprefix("speckit-") for path in codex_speckit_skill_files()}


# Deliberately one-sided surfaces; must match supported_agents in manifest.yaml.
SINGLE_AGENT_SKILLS = frozenset({"claude-worker"})


def parity_check() -> DoctorCheck:
    claude_skills = skill_names(REPO_TEMPLATE_ROOT / ".claude" / "skills")
    codex_skills = skill_names(REPO_TEMPLATE_ROOT / ".agents" / "skills")
    shared_codex_skills = {name for name in codex_skills if not name.startswith("speckit-")}
    skill_diff = sorted((claude_skills ^ shared_codex_skills) - SINGLE_AGENT_SKILLS)

    commands = speckit_command_names()
    codex_speckit = codex_speckit_names()
    speckit_diff = sorted(commands ^ codex_speckit)

    if skill_diff or speckit_diff:
        details: list[str] = []
        if skill_diff:
            details.append("shared skill mismatch: " + ",".join(skill_diff))
        if speckit_diff:
            details.append("Spec Kit command/skill mismatch: " + ",".join(speckit_diff))
        return DoctorCheck(name="parity", status="fail", detail="; ".join(details))

    return DoctorCheck(
        name="parity",
        status="ok",
        detail=f"{len(claude_skills)} shared skills; {len(commands)} Spec Kit command/skill pairs",
    )


# installer_artifact_check and grill_me_check were retired with install.sh/install-agents.sh (agent-kit#40).


def kb_reachability_check(require_live_kb: bool, timeout_seconds: float) -> DoctorCheck:
    kb_url = os.environ.get("KB_URL", "").rstrip("/")
    token = os.environ.get("KB_BEARER_TOKEN", "")
    live_failure_status = "fail" if require_live_kb else "warn"

    if not kb_url:
        return DoctorCheck(
            name="kb-live",
            status=live_failure_status,
            detail="KB_URL is not set; live MCP probe skipped",
        )
    if not token:
        return DoctorCheck(
            name="kb-live",
            status=live_failure_status,
            detail="KB_BEARER_TOKEN is not set; live MCP probe skipped",
        )

    try:
        tools_body = mcp_post(
            kb_url=kb_url,
            token=token,
            payload={"jsonrpc": "2.0", "id": "agent-kit-doctor-tools", "method": "tools/list"},
            timeout_seconds=timeout_seconds,
        )
    except (OSError, error.URLError, json.JSONDecodeError) as exc:
        return DoctorCheck(name="kb-live", status=live_failure_status, detail=f"MCP tools/list probe failed: {exc}")

    if "error" in tools_body:
        return DoctorCheck(
            name="kb-live",
            status=live_failure_status,
            detail=f"MCP tools/list returned error: {tools_body['error']}",
        )

    tool_names = {tool.get("name") for tool in tools_body.get("result", {}).get("tools", [])}
    if "knowledge.recall" not in tool_names:
        return DoctorCheck(
            name="kb-live",
            status=live_failure_status,
            detail="MCP tools/list did not include knowledge.recall",
        )

    recall_payload = {
        "jsonrpc": "2.0",
        "id": "agent-kit-doctor-recall",
        "method": "tools/call",
        "params": {
            "name": "knowledge.recall",
            "arguments": {
                "query": "agent kit doctor reachability",
                "scope": "project:personal-stack",
                "mode": "fast",
                "limit": 1,
            },
        },
    }
    try:
        recall_body = mcp_post(kb_url=kb_url, token=token, payload=recall_payload, timeout_seconds=timeout_seconds)
    except (OSError, error.URLError, json.JSONDecodeError) as exc:
        return DoctorCheck(
            name="kb-live",
            status=live_failure_status,
            detail=f"MCP knowledge.recall probe failed: {exc}",
        )

    if "error" in recall_body:
        return DoctorCheck(
            name="kb-live",
            status=live_failure_status,
            detail=f"MCP knowledge.recall returned error: {recall_body['error']}",
        )

    structured = recall_body.get("result", {}).get("structuredContent", {})
    hits = structured.get("hits")
    hit_count = len(hits) if isinstance(hits, list) else 0
    return DoctorCheck(
        name="kb-live",
        status="ok",
        detail=f"reachable at {kb_url}/mcp with {len(tool_names)} tools; fast recall returned {hit_count} hits",
    )


def mcp_post(kb_url: str, token: str, payload: dict[str, Any], timeout_seconds: float) -> dict[str, Any]:
    probe = request.Request(
        f"{kb_url}/mcp",
        data=json.dumps(payload).encode(),
        headers={
            "Authorization": f"Bearer {token}",
            "Content-Type": "application/json",
        },
        method="POST",
    )
    with request.urlopen(probe, timeout=timeout_seconds) as response:
        return cast(dict[str, Any], json.loads(response.read().decode()))


def doctor(args: argparse.Namespace) -> int:
    checks: list[DoctorCheck] = []
    findings = render_findings(REPOSITORY_ROOT)

    if not findings.missing and not findings.drifted:
        checks.append(DoctorCheck(name="render", status="ok", detail="generated files match templates"))
    else:
        details = []
        if findings.missing:
            details.append("missing " + ",".join(str(item.relative_path) for item in findings.missing))
        if findings.drifted:
            details.append("drifted " + ",".join(str(item.relative_path) for item in findings.drifted))
        checks.append(DoctorCheck(name="render", status="fail", detail="; ".join(details)))

    checks.append(manifest_check())
    checks.append(parity_check())
    checks.append(kb_reachability_check(require_live_kb=args.require_live_kb, timeout_seconds=args.kb_timeout_seconds))

    print("agent kit doctor")
    for item in checks:
        print(f"{item.status:<4} {item.name}: {item.detail}")

    failures = sum(1 for item in checks if item.status == "fail")
    warnings = sum(1 for item in checks if item.status == "warn")
    print(f"summary: {len(checks) - failures - warnings} ok, {warnings} warn, {failures} fail")
    if failures or (args.strict and warnings):
        return 1
    return 0


def render(destination_root: Path) -> int:
    rendered_count = 0
    for rendered in template_files(destination_root):
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
    mode.add_argument("--check", action="store_true", help="verify templates match the repository tree")
    mode.add_argument("--write", action="store_true", help="render templates into the repository tree")
    mode.add_argument("--output", type=Path, help="render templates into a separate output directory")
    mode.add_argument("--doctor", action="store_true", help="run read-only agent kit diagnostics")
    parser.add_argument("--strict", action="store_true", help="make doctor warnings fail")
    parser.add_argument(
        "--require-live-kb",
        action="store_true",
        help="make doctor fail unless the KB MCP probe succeeds",
    )
    parser.add_argument("--kb-timeout-seconds", type=float, default=5.0, help="timeout for the doctor KB MCP probe")
    return parser.parse_args(argv)


def main(argv: list[str]) -> int:
    args = parse_args(argv)
    if args.check:
        return check(REPOSITORY_ROOT)
    if args.write:
        return render(REPOSITORY_ROOT)
    if args.doctor:
        return doctor(args)
    return render(args.output.resolve())


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
