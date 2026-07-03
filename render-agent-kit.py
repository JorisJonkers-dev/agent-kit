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
EXTRA_TEMPLATES = (
    (
        KIT_ROOT / "templates" / "installer" / "install.sh.tpl",
        Path("installer/install.sh"),
        True,
    ),
    (
        KIT_ROOT / "templates" / "installer" / "install-agents.sh.tpl",
        Path("installer/install-agents.sh"),
        False,
    ),
)
INCLUDE_PATTERN = re.compile(r"^# @agent-kit-include (?P<path>[A-Za-z0-9_./-]+)$")
# council is a multi-file skill; its install block + uninstall manifest lines are
# generated from the canonical source (no duplication into partials).
COUNCIL_BUNDLE_PATTERN = re.compile(r"^# @agent-kit-council-bundle$")
COUNCIL_MANAGED_PATTERN = re.compile(r"^# @agent-kit-council-managed (?P<agent>claude|codex)$")
SPECKIT_COMMANDS_BUNDLE_PATTERN = re.compile(r"^# @agent-kit-speckit-commands-bundle$")
SPECKIT_COMMANDS_MANAGED_PATTERN = re.compile(r"^# @agent-kit-speckit-commands-managed$")
CODEX_SPECKIT_BUNDLE_PATTERN = re.compile(r"^# @agent-kit-codex-speckit-bundle$")
CODEX_SPECKIT_MANAGED_PATTERN = re.compile(r"^# @agent-kit-codex-speckit-managed$")
SPECIFY_SEED_PATTERN = re.compile(r"^# @agent-kit-specify-seed$")
COUNCIL_SRC = KIT_ROOT / "council"
COUNCIL_SKILL_SRC = {
    "claude": REPO_TEMPLATE_ROOT / ".claude" / "skills" / "council" / "SKILL.md",
    "codex": REPO_TEMPLATE_ROOT / ".agents" / "skills" / "council" / "SKILL.md",
}
SPECKIT_COMMANDS_SRC = REPO_TEMPLATE_ROOT / ".claude" / "commands"
CODEX_SPECKIT_SKILLS_SRC = REPO_TEMPLATE_ROOT / ".agents" / "skills"
SPECIFY_SRC = REPO_TEMPLATE_ROOT / ".specify"
SPECIFY_CONSTITUTION_SRC = SPECIFY_SRC / "memory" / "constitution.md"
MANIFEST_PATH = KIT_ROOT / "manifest.yaml"


@dataclass(frozen=True)
class RenderedFile:
    source: Path
    destination: Path
    relative_path: Path
    expand_includes: bool = False


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

    for source, relative_path, expand_includes in EXTRA_TEMPLATES:
        if not source.is_file():
            raise FileNotFoundError(f"template file does not exist: {source}")
        files.append(
            RenderedFile(
                source=source,
                destination=destination_root / relative_path,
                relative_path=relative_path,
                expand_includes=expand_includes,
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


def _council_var(rel: str) -> str:
    return "COUNCIL_FILE_" + re.sub(r"[^A-Za-z0-9]", "_", rel)


def _shell_var(prefix: str, rel: str) -> str:
    return prefix + "_" + re.sub(r"[^A-Za-z0-9]", "_", rel)


def _heredoc(var: str, content: str) -> list[str]:
    delim = f"{var}_EOF"
    return [f"read -r -d '' {var} <<'{delim}' || true", content.rstrip("\n"), delim]


def render_council_bundle() -> str:
    """Generate the install.sh block that writes the council skill directory
    (SKILL.md + driver + prompts + schemas + default config) for both agents."""
    toolkit = council_toolkit_files()
    lines = ["# council — generated by render-agent-kit.py from "
             "council/. Edit the source, not here."]
    for rel, _mode in toolkit:
        lines += _heredoc(_council_var(rel), (COUNCIL_SRC / rel).read_text())
    for agent in ("claude", "codex"):
        lines += _heredoc(f"COUNCIL_SKILL_{agent}",
                          COUNCIL_SKILL_SRC[agent].read_text())
    lines += [
        "install_council() {",
        '  local dir="$1" skill="$2"',
    ]
    for rel, mode in toolkit:
        if rel == "council.toml":
            continue  # user config: preserved below, never clobbered on upgrade
        lines.append(f'  write_file "${{dir}}/council/{rel}" {mode} "${{{_council_var(rel)}}}"')
    lines.append('  write_file "${dir}/council/SKILL.md" 0644 "$skill"')
    toml_var = _council_var("council.toml")
    lines += [
        '  if [ ! -e "${dir}/council/council.toml" ]; then',
        f'    write_file "${{dir}}/council/council.toml" 0644 "${{{toml_var}}}"',
        "  else",
        '    log "preserving existing ${dir}/council/council.toml"',
        "  fi",
        "}",
        'if [ "${INSTALL_CLAUDE}" = 1 ]; then install_council "${SKILLS_DIR}" "${COUNCIL_SKILL_claude}"; fi',
        'if [ "${INSTALL_CODEX}" = 1 ]; then install_council "${CODEX_SKILLS_DIR}" "${COUNCIL_SKILL_codex}"; fi',
    ]
    return "\n".join(lines) + "\n"


def render_council_managed(agent: str) -> str:
    base = "${SKILLS_DIR}" if agent == "claude" else "${CODEX_SKILLS_DIR}"
    rels = ["SKILL.md"] + [rel for rel, _ in council_toolkit_files()]
    return "\n".join(f'  "{base}/council/{rel}"' for rel in rels) + "\n"


def speckit_command_files() -> list[Path]:
    return sorted(SPECKIT_COMMANDS_SRC.glob("speckit.*.md"))


def codex_speckit_skill_files() -> list[Path]:
    return sorted(CODEX_SPECKIT_SKILLS_SRC.glob("speckit-*/SKILL.md"))


def specify_seed_files() -> list[tuple[Path, str, str, bool]]:
    files: list[tuple[Path, str, str, bool]] = [
        (SPECIFY_CONSTITUTION_SRC, ".specify/memory/constitution.md", "0644", True),
    ]
    for path in sorted(SPECIFY_SRC.rglob("*")):
        if not path.is_file():
            continue
        if path == SPECIFY_CONSTITUTION_SRC:
            continue
        rel = path.relative_to(REPO_TEMPLATE_ROOT).as_posix()
        mode = "0755" if path.stat().st_mode & stat.S_IXUSR else "0644"
        files.append((path, rel, mode, False))
    return files


def render_speckit_commands_bundle() -> str:
    lines = ["# Spec Kit Claude commands — generated by render-agent-kit.py from repo templates."]
    commands = speckit_command_files()
    for path in commands:
        rel = path.relative_to(SPECKIT_COMMANDS_SRC).as_posix()
        lines += _heredoc(_shell_var("SPECKIT_COMMAND", rel), path.read_text())
    lines += ['if [ "${INSTALL_CLAUDE}" = 1 ]; then']
    for path in commands:
        rel = path.relative_to(SPECKIT_COMMANDS_SRC).as_posix()
        var = _shell_var("SPECKIT_COMMAND", rel)
        lines.append(f'  write_file "${{COMMANDS_DIR}}/{rel}" 0644 "${{{var}}}"')
    lines.append("fi")
    return "\n".join(lines) + "\n"


def render_speckit_commands_managed() -> str:
    return "\n".join(
        f'  "${{COMMANDS_DIR}}/{path.relative_to(SPECKIT_COMMANDS_SRC).as_posix()}"'
        for path in speckit_command_files()
    ) + "\n"


def render_codex_speckit_bundle() -> str:
    lines = ["# Spec Kit Codex skills — generated by render-agent-kit.py from repo templates."]
    skills = codex_speckit_skill_files()
    for path in skills:
        rel = path.relative_to(CODEX_SPECKIT_SKILLS_SRC).as_posix()
        lines += _heredoc(_shell_var("CODEX_SPECKIT", rel), path.read_text())
    lines += ['if [ "${INSTALL_CODEX}" = 1 ]; then']
    for path in skills:
        rel = path.relative_to(CODEX_SPECKIT_SKILLS_SRC).as_posix()
        var = _shell_var("CODEX_SPECKIT", rel)
        lines.append(f'  write_file "${{CODEX_SKILLS_DIR}}/{rel}" 0644 "${{{var}}}"')
    lines.append("fi")
    return "\n".join(lines) + "\n"


def render_codex_speckit_managed() -> str:
    return "\n".join(
        f'  "${{CODEX_SKILLS_DIR}}/{path.relative_to(CODEX_SPECKIT_SKILLS_SRC).as_posix()}"'
        for path in codex_speckit_skill_files()
    ) + "\n"


def render_specify_seed() -> str:
    lines = ["# Spec Kit project scaffold seed — generated by render-agent-kit.py from repo templates."]
    seed_files = specify_seed_files()
    for path, rel, _mode, _seed_if_absent in seed_files:
        lines += _heredoc(_shell_var("SPECIFY_SEED", rel), path.read_text())
    lines += ['if [ "${SCOPE}" = "project" ]; then']
    for _path, rel, mode, seed_if_absent in seed_files:
        var = _shell_var("SPECIFY_SEED", rel)
        target = f"${{PROJECT_ROOT}}/{rel}"
        if seed_if_absent:
            lines += [
                f'  if [ ! -e "{target}" ]; then',
                f'    write_file "{target}" {mode} "${{{var}}}"',
                "  else",
                f'    log "preserving existing {target}"',
                "  fi",
            ]
        else:
            lines.append(f'  write_file "{target}" {mode} "${{{var}}}"')
    lines.append("fi")
    return "\n".join(lines) + "\n"


def rendered_content(rendered: RenderedFile) -> bytes:
    if not rendered.expand_includes:
        return rendered.source.read_bytes()

    parts: list[str] = []
    template_root = rendered.source.parent
    for raw_line in rendered.source.read_text().splitlines(keepends=True):
        line = raw_line.rstrip("\r\n")
        if COUNCIL_BUNDLE_PATTERN.match(line):
            parts.append(render_council_bundle())
            continue
        managed = COUNCIL_MANAGED_PATTERN.match(line)
        if managed:
            parts.append(render_council_managed(managed.group("agent")))
            continue
        if SPECKIT_COMMANDS_BUNDLE_PATTERN.match(line):
            parts.append(render_speckit_commands_bundle())
            continue
        if SPECKIT_COMMANDS_MANAGED_PATTERN.match(line):
            parts.append(render_speckit_commands_managed())
            continue
        if CODEX_SPECKIT_BUNDLE_PATTERN.match(line):
            parts.append(render_codex_speckit_bundle())
            continue
        if CODEX_SPECKIT_MANAGED_PATTERN.match(line):
            parts.append(render_codex_speckit_managed())
            continue
        if SPECIFY_SEED_PATTERN.match(line):
            parts.append(render_specify_seed())
            continue
        match = INCLUDE_PATTERN.match(line)
        if not match:
            parts.append(raw_line)
            continue

        include_path = (template_root / match.group("path")).resolve()
        if not include_path.is_relative_to(template_root.resolve()):
            raise ValueError(f"include escapes template root: {include_path}")
        if not include_path.is_file():
            raise FileNotFoundError(f"include file does not exist: {include_path}")

        content = include_path.read_text()
        parts.append(content)
        if not content.endswith("\n"):
            parts.append("\n")

    return "".join(parts).encode()


def render_findings(destination_root: Path) -> RenderFindings:
    drifted: list[RenderedFile] = []
    missing: list[RenderedFile] = []

    for rendered in template_files(destination_root):
        if not rendered.destination.exists():
            missing.append(rendered)
        elif rendered.expand_includes:
            if rendered_content(rendered) != rendered.destination.read_bytes():
                drifted.append(rendered)
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

    return DoctorCheck(
        name="manifest",
        status="ok",
        detail=f"kit manifest version {manifest_version()}; council command surface validated",
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


SERVED_INSTALLERS = (
    Path("installer/install.sh"),
    Path("installer/install-agents.sh"),
)


def installer_artifact_check() -> DoctorCheck:
    secret_patterns = {
        "bearer token": re.compile(r"Bearer\s+[A-Za-z0-9._~+/-]{16,}=*"),
        "private key": re.compile(r"-----BEGIN [A-Z ]*PRIVATE KEY-----"),
        "transcript block": re.compile(r"BEGIN RAW TRANSCRIPT|END RAW TRANSCRIPT", re.IGNORECASE),
    }

    for relative in SERVED_INSTALLERS:
        installer = REPOSITORY_ROOT / relative
        if not installer.is_file():
            return DoctorCheck(name="installer", status="fail", detail=f"missing {relative}")

        body = installer.read_text(errors="replace")
        missing = [token for token in ("@VERSION@", "@KB_URL@") if token not in body]
        if missing:
            return DoctorCheck(
                name="installer",
                status="fail",
                detail=f"{relative} missing placeholders: " + ",".join(missing),
            )

        matches = [name for name, pattern in secret_patterns.items() if pattern.search(body)]
        if matches:
            return DoctorCheck(
                name="installer",
                status="fail",
                detail=f"{relative} secret-like markers: " + ",".join(matches),
            )

    return DoctorCheck(
        name="installer",
        status="ok",
        detail=f"{len(SERVED_INSTALLERS)} serving artifacts passed placeholder and secret scans",
    )


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
    checks.append(installer_artifact_check())
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
            if rendered.expand_includes:
                same_content = rendered_content(rendered) == rendered.destination.read_bytes()
            else:
                same_content = filecmp.cmp(rendered.source, rendered.destination, shallow=False)
            if same_content and same_mode:
                continue

        rendered.destination.parent.mkdir(parents=True, exist_ok=True)
        if rendered.expand_includes:
            rendered.destination.write_bytes(rendered_content(rendered))
        else:
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
