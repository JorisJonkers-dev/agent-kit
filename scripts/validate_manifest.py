#!/usr/bin/env python3
"""Validate the agent-kit manifest against checked-in sources."""

from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import tomllib
from collections.abc import Iterable
from pathlib import Path
from typing import Any, NoReturn, cast

try:
    import yaml
except ModuleNotFoundError as exc:  # pragma: no cover - exercised in CI setup failures
    raise SystemExit("PyYAML is required: python3 -m pip install -r requirements-dev.txt") from exc


ROOT = Path(__file__).resolve().parents[1]
RUNTIME_SELFTEST_ENV = "AGENT_RUNTIME_SELFTEST"
MANIFEST_PATH = ROOT / "manifest.yaml"
COUNCIL_CLI_INDEX_PATH = ROOT / "council" / "ts" / "src" / "cli" / "index.ts"
ROUND3_SKELETON_DIR = ROOT / "runner-manifests"
ROUND3_VALIDATED_DIRS = (ROUND3_SKELETON_DIR,)

ROUND3_FORBIDDEN_PATTERNS = {
    "personal domain or hostname": re.compile(
        r"\b(?:esa-blueshell|blueshell|enschede|frankfurt|contabo)\b",
        re.IGNORECASE,
    ),
    "concrete namespace": re.compile(r"\b(?:agents-system|assistant-system|knowledge-system(?!-version))\b"),
    "personal image prefix": re.compile(r"\bghcr\.io/extratoast/[^<\s]+", re.IGNORECASE),
    "personal label": re.compile(r"\bpersonal-stack/[A-Za-z0-9_.-]+", re.IGNORECASE),
    "vault path": re.compile(r"\bsecret/(?:agents|knowledge-system|platform)\b", re.IGNORECASE),
    "ip address": re.compile(r"\b(?:\d{1,3}\.){3}\d{1,3}\b"),
    "endpoint url": re.compile(r"\bhttps?://[^\s)\"']+", re.IGNORECASE),
}

KUBERNETES_API_PREFIXES = (
    "v1",
    "apps/",
    "batch/",
    "rbac.authorization.k8s.io/",
    "networking.k8s.io/",
    "kustomize.config.k8s.io/",
    "helm.toolkit.fluxcd.io/",
    "kustomize.toolkit.fluxcd.io/",
)
KUBERNETES_RESOURCE_KINDS = {
    "ClusterRole",
    "ClusterRoleBinding",
    "ConfigMap",
    "CronJob",
    "Deployment",
    "HelmRelease",
    "Ingress",
    "Kustomization",
    "Namespace",
    "NetworkPolicy",
    "PersistentVolumeClaim",
    "Pod",
    "Role",
    "RoleBinding",
    "Secret",
    "Service",
    "ServiceAccount",
    "StatefulSet",
}
ATTACHMENT_ACTIVE_LIMIT = 12
ROUTING_CARD_MIN_SIGNALS = 2
ROUTING_CARD_RISKS = {"low", "medium", "high"}
ATTACHMENT_PROFILE_KEYS = {"name", "mcpProfile", "skillCards", "fullSkills"}
ROUTING_CARD_KEYS = {
    "name",
    "purpose",
    "positiveTriggers",
    "negativeTriggers",
    "requiredMcpProfile",
    "risk",
    "expectedOutputs",
}
REQUIRED_COUNCIL_CLI_COMMANDS = ("eval", "status", "tail", "triage")
REQUIRED_COUNCIL_CLI_COMMANDS_LABEL = ", ".join(REQUIRED_COUNCIL_CLI_COMMANDS)


def load_renderer() -> Any:
    spec = importlib.util.spec_from_file_location("render_agent_kit", ROOT / "render-agent-kit.py")
    if spec is None or spec.loader is None:
        raise AssertionError("cannot load render-agent-kit.py")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


def load_manifest() -> dict[str, Any]:
    data = yaml.safe_load(MANIFEST_PATH.read_text())
    if not isinstance(data, dict):
        raise AssertionError("manifest root must be a mapping")
    return data


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(65536), b""):
            digest.update(chunk)
    return digest.hexdigest()


def fail(message: str) -> NoReturn:
    raise AssertionError(message)


def as_list(value: Any, name: str) -> list[Any]:
    if not isinstance(value, list):
        fail(f"{name} must be a list")
    return value


def as_mapping(value: Any, name: str) -> dict[str, Any]:
    if not isinstance(value, dict):
        fail(f"{name} must be a mapping")
    return cast(dict[str, Any], value)


def as_non_empty_string(value: Any, name: str) -> str:
    if not isinstance(value, str) or not value.strip():
        fail(f"{name} must be a non-empty string")
    return value


def as_string_list(value: Any, name: str, *, min_items: int = 0) -> list[str]:
    items = as_list(value, name)
    if len(items) < min_items:
        fail(f"{name} must contain at least {min_items} item(s)")
    result: list[str] = []
    for index, item in enumerate(items):
        result.append(as_non_empty_string(item, f"{name}[{index}]"))
    return result


def all_mappings(value: Any) -> Iterable[dict[str, Any]]:
    if isinstance(value, dict):
        yield value
        for child in value.values():
            yield from all_mappings(child)
    elif isinstance(value, list):
        for item in value:
            yield from all_mappings(item)


def pinned_path_entries(manifest: dict[str, Any]) -> list[dict[str, str]]:
    """Each pin: the path a file lands at, the file the sha256 is taken of, the digest.

    A file that lands in a gitignored directory names its tracked `source`.
    """
    entries: list[dict[str, str]] = []
    for item in all_mappings(manifest):
        path = item.get("path")
        digest = item.get("sha256")
        if isinstance(path, str) and isinstance(digest, str):
            source = item.get("source", path)
            if not isinstance(source, str):
                fail(f"pinned path {path} has a source that is not a string")
            entries.append({"path": path, "file": source, "sha256": digest})
    return entries


def validate_artifact(manifest: dict[str, Any]) -> None:
    artifact = as_mapping(manifest.get("artifact"), "artifact")
    coordinate = artifact.get("short_coordinate")
    if not isinstance(coordinate, str):
        fail("artifact.short_coordinate must be a string")
    if artifact.get("repository") != "https://github.com/JorisJonkers-dev/agent-kit":
        fail("artifact.repository must be https://github.com/JorisJonkers-dev/agent-kit")
    if coordinate != "github:JorisJonkers-dev/agent-kit":
        fail("artifact.short_coordinate must be github:JorisJonkers-dev/agent-kit")
    if re.search(r"agent-kit[-_/]agent-kit", coordinate, re.IGNORECASE):
        fail("artifact.short_coordinate repeats the agent-kit segment")
    if artifact.get("package_publish") != "ghcr-oci-runtime-home":
        fail("artifact.package_publish must be ghcr-oci-runtime-home")
    if artifact.get("runtime_home_repository") != "ghcr.io/jorisjonkers-dev/agent-kit/runtime-home":
        fail("artifact.runtime_home_repository must be ghcr.io/jorisjonkers-dev/agent-kit/runtime-home")


def validate_renderer(manifest: dict[str, Any]) -> set[str]:
    renderer = load_renderer()
    renderer_manifest = as_mapping(manifest.get("renderer"), "renderer")
    if renderer_manifest.get("script_path") != "render-agent-kit.py":
        fail("renderer.script_path must be render-agent-kit.py")
    if renderer_manifest.get("repo_skill_root") != "skills/repo":
        fail("renderer.repo_skill_root must be skills/repo")
    if renderer_manifest.get("agents_root") != "agents":
        fail("renderer.agents_root must be agents")
    if "template_root" in renderer_manifest:
        fail("renderer.template_root is retired: templates/repo moved to skills/repo and agents (agent-kit#277)")
    if renderer_manifest.get("runtime_template_root") != "templates/runner-runtime":
        fail("renderer.runtime_template_root must be templates/runner-runtime")

    managed_paths = set(as_list(renderer_manifest.get("managed_paths"), "renderer.managed_paths"))
    rendered = renderer.template_files(ROOT)
    expected = {item.relative_path.as_posix() for item in rendered}
    if managed_paths != expected:
        missing = sorted(expected - managed_paths)
        extra = sorted(managed_paths - expected)
        fail(f"renderer.managed_paths mismatch; missing={missing}; extra={extra}")

    # extra_templates/include_templates existed only to render install.sh
    # from heredoc partials; both are retired along with it (agent-kit#40).
    for retired_key in ("extra_templates", "include_templates"):
        if retired_key in renderer_manifest:
            fail(f"renderer.{retired_key} is retired along with install.sh (agent-kit#40)")

    # Local output is gitignored, so its pin must name the tracked file it is
    # rendered from.
    sources = {item.relative_path.as_posix(): item.source.relative_to(ROOT).as_posix() for item in rendered}
    local = {item.relative_path.as_posix() for item in rendered if not item.committed}
    for entry in pinned_path_entries(manifest):
        path = entry["path"]
        if path in local and entry["file"] != sources[path]:
            fail(f"pinned path {path} must name its source {sources[path]}, not {entry['file']}")

    return managed_paths


def validate_checksums(manifest: dict[str, Any], managed_paths: set[str]) -> None:
    pinned_entries = pinned_path_entries(manifest)
    seen: dict[str, str] = {}
    for entry in pinned_entries:
        path = entry["path"]
        digest = entry["sha256"]
        full_path = ROOT / entry["file"]
        if not full_path.is_file():
            fail(f"pinned path does not exist: {entry['file']}")
        actual = sha256(full_path)
        if actual != digest:
            fail(f"sha256 mismatch for {entry['file']}: manifest={digest} actual={actual}")
        if path in seen and seen[path] != digest:
            fail(f"pinned path has conflicting sha256 values: {path}")
        seen[path] = digest

    missing = sorted(managed_paths - set(seen))
    if missing:
        fail(f"managed paths are not pinned with sha256: {missing}")


def validate_surface_parity(manifest: dict[str, Any]) -> None:
    skills = as_list(manifest.get("skills"), "skills")
    for skill in skills:
        name = str(skill.get("name"))
        supported = skill.get("supported_agents")
        targets_value = skill.get("targets", [])
        targets = {target.get("agent") for target in as_list(targets_value, f"skill {name}.targets")}
        # install.sh's `installer:` sub-block (and the installer-only
        # skills that had no repo target and relied on it alone) are
        # retired along with it (agent-kit#40); every remaining skill
        # must have a real repo-file target.
        supported_set = set(supported or [])
        if not supported_set or not supported_set <= {"claude", "codex"}:
            fail(f"skill {name} must declare supported_agents from claude/codex")
        if not targets:
            fail(f"skill {name} must declare repo-file targets")
        if targets != supported_set:
            fail(f"skill {name} targets {sorted(targets)} but supports {sorted(supported_set)}")

    # The `settings` ledger existed only to wire the retired knowledge hooks;
    # the critique hooks are merged into the operator's files by setup, so it
    # stays absent. The `hooks` section is checked by validate_hooks.
    if "settings" in manifest:
        fail(
            "manifest must not declare a `settings` section: it only ever wired the "
            "retired knowledge hooks (see manifest.yaml notes)",
        )

    # The top-level `installer:` block (install.sh/install-agents.sh pins) is
    # retired along with those files (agent-kit#40); it must stay absent.
    if "installer" in manifest:
        fail("manifest must not declare a top-level `installer` section: install.sh is retired")

    # The `commands:` ledger pinned the Spec Kit slash commands, which are
    # retired (agent-kit#253); it must stay absent.
    if "commands" in manifest:
        fail("manifest must not declare a `commands` section: the Spec Kit commands are retired")


HOOKS_ADR = "docs/adr/0002-exactly-three-critique-hooks.md"
# ADR-0002 allows exactly these hooks, each on one event. Any other name
# needs its own ADR before it can be added here.
ALLOWED_HOOK_EVENTS = {
    "critique-commit-gate": "PreToolUse",
    "critique-pr-gate": "PreToolUse",
    "critique-session-start": "SessionStart",
}


def validate_hooks(manifest: dict[str, Any]) -> None:
    hooks = as_mapping(manifest.get("hooks", {}), "hooks")
    for name, value in hooks.items():
        if name not in ALLOWED_HOOK_EVENTS:
            fail(f"hook {name} is not allowed: {HOOKS_ADR} allows only {', '.join(ALLOWED_HOOK_EVENTS)}")
        hook = as_mapping(value, f"hook {name}")
        if hook.get("adr") != HOOKS_ADR or not (ROOT / HOOKS_ADR).is_file():
            fail(f"hook {name} must cite {HOOKS_ADR}")
        if hook.get("event") != ALLOWED_HOOK_EVENTS[name]:
            fail(f"hook {name} must run on {ALLOWED_HOOK_EVENTS[name]}, not {hook.get('event')}")
        agents = as_string_list(hook.get("agents"), f"hook {name}.agents", min_items=1)
        # Vibe and Antigravity stay out until their hook events are checked.
        if not set(agents) <= {"claude", "codex"}:
            fail(f"hook {name} agents must be claude and/or codex, not {agents}")


def _has_council_command_spec(source: str, command: str) -> bool:
    command_literal = re.escape(command)
    pattern = re.compile(
        r"\{[^{}]*\bname:\s*(['\"])" + command_literal + r"\1[^{}]*\}",
        re.MULTILINE | re.DOTALL,
    )
    return pattern.search(source) is not None


def _has_council_command_dispatch(source: str, command: str) -> bool:
    command_literal = re.escape(command)
    pattern = re.compile(
        r"case\s+(['\"])"
        + command_literal
        + r"\1\s*:\s*return\s+okJson\s*\(\s*await\s+app\."
        + command_literal
        + r"\s*\(",
        re.MULTILINE | re.DOTALL,
    )
    return pattern.search(source) is not None


def _has_council_command_handler_dispatch(source: str, command: str) -> bool:
    command_literal = re.escape(command)
    handler_literal = "run" + "".join(part.capitalize() for part in command.split("-")) + "Command"
    parser_literal = "parse" + "".join(part.capitalize() for part in command.split("-"))
    pattern = re.compile(
        r"case\s+(['\"])"
        + command_literal
        + r"\1\s*:\s*return\s+await\s+"
        + re.escape(handler_literal)
        + r"\s*\(\s*app\s*,\s*"
        + re.escape(parser_literal)
        + r"\s*\(",
        re.MULTILINE | re.DOTALL,
    )
    return pattern.search(source) is not None


def _has_required_council_command_dispatch(source: str, command: str) -> bool:
    return _has_council_command_dispatch(source, command) or _has_council_command_handler_dispatch(source, command)


def validate_council_command_surface_source(source: str) -> None:
    missing_specs = [
        command for command in REQUIRED_COUNCIL_CLI_COMMANDS if not _has_council_command_spec(source, command)
    ]
    if missing_specs:
        fail(
            "council command surface required commands "
            f"({REQUIRED_COUNCIL_CLI_COMMANDS_LABEL}) missing command registry specs: "
            + ", ".join(missing_specs),
        )

    missing_dispatches = [
        command
        for command in REQUIRED_COUNCIL_CLI_COMMANDS
        if not _has_required_council_command_dispatch(source, command)
    ]
    if missing_dispatches:
        fail(
            "council command surface required commands "
            f"({REQUIRED_COUNCIL_CLI_COMMANDS_LABEL}) missing command dispatch branches: "
            + ", ".join(missing_dispatches),
        )


def validate_council_command_surface() -> None:
    if not COUNCIL_CLI_INDEX_PATH.is_file():
        fail(f"council command surface source is missing: {COUNCIL_CLI_INDEX_PATH.relative_to(ROOT)}")
    validate_council_command_surface_source(COUNCIL_CLI_INDEX_PATH.read_text())


def validate_council(manifest: dict[str, Any]) -> None:
    renderer = load_renderer()
    council = as_mapping(manifest.get("council"), "council")
    files = as_list(council.get("files"), "council.files")
    manifest_files = {item.get("path") for item in files}
    rendered_files = {f"council/{rel}" for rel, _mode in renderer.council_toolkit_files()}
    actual_files = {
        path.relative_to(ROOT).as_posix()
        for path in (ROOT / "council").rglob("*")
        if (
            path.is_file()
            and path.name != "README.md"
            and "__pycache__" not in path.parts
            and "node_modules" not in path.relative_to(ROOT / "council").parts
            and "coverage" not in path.relative_to(ROOT / "council").parts
            and path.suffix != ".pyc"
            and path.suffix != ".map"
            and not path.name.endswith(".tsbuildinfo")
            and path.relative_to(ROOT / "council").parts[:1] not in (("ts",), ("ts-dist",))
            and (
                path.relative_to(ROOT / "council").as_posix() in {"council.mjs", "council.toml"}
                or path.relative_to(ROOT / "council").parts[:1] in (("prompts",), ("schemas",))
            )
        )
    }
    if rendered_files != actual_files:
        fail(f"council renderer mismatch: rendered={sorted(rendered_files)} actual={sorted(actual_files)}")
    if manifest_files != actual_files:
        fail(f"council.files mismatch: manifest={sorted(manifest_files)} actual={sorted(actual_files)}")


def validate_critique(manifest: dict[str, Any]) -> None:
    """critique.files pins every file of skills/workstation/critique but its SKILL.md.

    The sha256 of each entry is checked with every other pin, by
    validate_checksums. This checks the set: a file the skill ships and the
    manifest does not pin is a file setup installs unreviewed.
    """
    critique = as_mapping(manifest.get("critique"), "critique")
    files = as_list(critique.get("files"), "critique.files")
    manifest_files = {item.get("path") for item in files}
    root = ROOT / "skills" / "workstation" / "critique"
    actual_files = {
        path.relative_to(ROOT).as_posix()
        for path in root.rglob("*")
        if path.is_file() and path.relative_to(root).as_posix() != "SKILL.md"
    }
    if manifest_files != actual_files:
        fail(f"critique.files mismatch: manifest={sorted(manifest_files)} actual={sorted(actual_files)}")


def validate_build_tracker(manifest: dict[str, Any]) -> None:
    """build_tracker.files pins every file of skills/workstation/build-tracker but its SKILL.md.

    As with critique.files, validate_checksums checks each sha256; this
    checks the set.
    """
    section = as_mapping(manifest.get("build_tracker"), "build_tracker")
    files = as_list(section.get("files"), "build_tracker.files")
    manifest_files = {item.get("path") for item in files}
    root = ROOT / "skills" / "workstation" / "build-tracker"
    actual_files = {
        path.relative_to(ROOT).as_posix()
        for path in root.rglob("*")
        if path.is_file() and path.relative_to(root).as_posix() != "SKILL.md"
    }
    if manifest_files != actual_files:
        fail(f"build_tracker.files mismatch: manifest={sorted(manifest_files)} actual={sorted(actual_files)}")


# _scan_served_installer/validate_installer scanned install.sh/install-agents.sh; retired with them (agent-kit#40).


def round3_files() -> list[Path]:
    files: list[Path] = []
    for directory in ROUND3_VALIDATED_DIRS:
        if not directory.is_dir():
            fail(f"round-3 required directory is missing: {directory.relative_to(ROOT)}")
        files.extend(path for path in directory.rglob("*") if path.is_file())
    return sorted(files)


def validate_round3_no_concrete_values() -> None:
    for path in round3_files():
        rel = path.relative_to(ROOT).as_posix()
        text = path.read_text(errors="replace")
        for label, pattern in ROUND3_FORBIDDEN_PATTERNS.items():
            match = pattern.search(text)
            if match:
                fail(f"round-3 skeleton contains {label} in {rel}: {match.group(0)}")


def validate_round3_parses() -> None:
    schema_path = ROUND3_SKELETON_DIR / "runtime-package.schema.json"
    if not schema_path.is_file():
        fail("round-3 runtime package schema is missing")
    schema = json.loads(schema_path.read_text())
    if schema.get("title") != "AgentRuntimePackage":
        fail("round-3 runtime schema title must be AgentRuntimePackage")
    api_version = as_mapping(schema.get("properties"), "round-3 schema.properties").get("apiVersion", {})
    if not isinstance(api_version, dict) or api_version.get("const") != "agent-kit.runtime/v1alpha1":
        fail("runtime schema apiVersion const must be agent-kit.runtime/v1alpha1")

    fixtures_dir = ROUND3_SKELETON_DIR / "fixtures"
    if not fixtures_dir.is_dir():
        fail("round-3 fixtures directory is missing")
    for path in sorted(fixtures_dir.glob("*.yaml")):
        loaded = list(yaml.safe_load_all(path.read_text()))
        if not loaded or any(item is None for item in loaded):
            fail(f"round-3 fixture must contain at least one YAML document: {path.relative_to(ROOT)}")


def validate_round3_non_deployable() -> None:
    for path in sorted((ROUND3_SKELETON_DIR / "fixtures").glob("*.yaml")):
        for document in yaml.safe_load_all(path.read_text()):
            if not isinstance(document, dict):
                continue
            api_version = str(document.get("apiVersion", ""))
            kind = str(document.get("kind", ""))
            if kind in KUBERNETES_RESOURCE_KINDS:
                fail(f"round-3 fixture uses deployable Kubernetes kind {kind}: {path.relative_to(ROOT)}")
            if any(api_version == prefix or api_version.startswith(prefix) for prefix in KUBERNETES_API_PREFIXES):
                fail(f"round-3 fixture uses Kubernetes apiVersion {api_version}: {path.relative_to(ROOT)}")


def validate_round3_skeleton() -> None:
    validate_round3_no_concrete_values()
    validate_round3_parses()
    validate_round3_non_deployable()


def validate_runtime_package_manifest(manifest: dict[str, Any]) -> None:
    runtime_section = as_mapping(manifest.get("agent_runner_runtime"), "agent_runner_runtime")
    files = as_list(runtime_section.get("files"), "agent_runner_runtime.files")
    expected_files = {
        path.relative_to(ROOT).as_posix()
        for path in (ROOT / "runner-manifests" / "runtime").rglob("*")
        if path.is_file()
    }
    manifest_files = {
        path
        for item in files
        if isinstance(item, dict) and isinstance((path := item.get("path")), str)
    }
    if manifest_files != expected_files:
        fail(
            "agent_runner_runtime.files mismatch: "
            f"manifest={sorted(manifest_files)} actual={sorted(expected_files)}",
        )


def _validate_unique_name(name: str, seen: set[str], path: str, label: str) -> None:
    if name in seen:
        fail(f"{path}.name duplicates {label} {name!r}")
    seen.add(name)


def _validate_known_keys(item: dict[str, Any], path: str, allowed_keys: set[str]) -> None:
    extra = sorted(set(item) - allowed_keys)
    if extra:
        fail(f"{path} contains unsupported field(s): {extra}")


def _validate_routing_card_fit(card: dict[str, Any], path: str) -> None:
    purpose = as_non_empty_string(card.get("purpose"), f"{path}.purpose")
    if "\n" in purpose or "\r" in purpose:
        fail(f"{path}.purpose must be one line")

    positive_triggers = as_string_list(
        card.get("positiveTriggers"),
        f"{path}.positiveTriggers",
        min_items=ROUTING_CARD_MIN_SIGNALS,
    )
    negative_triggers = as_string_list(
        card.get("negativeTriggers"),
        f"{path}.negativeTriggers",
        min_items=ROUTING_CARD_MIN_SIGNALS,
    )
    expected_outputs = as_string_list(
        card.get("expectedOutputs"),
        f"{path}.expectedOutputs",
        min_items=ROUTING_CARD_MIN_SIGNALS,
    )
    if set(positive_triggers) & set(negative_triggers):
        fail(f"{path} has overlapping positive and negative triggers")
    if len(set(positive_triggers)) != len(positive_triggers):
        fail(f"{path}.positiveTriggers contains duplicate routing signals")
    if len(set(negative_triggers)) != len(negative_triggers):
        fail(f"{path}.negativeTriggers contains duplicate routing signals")
    if len(set(expected_outputs)) != len(expected_outputs):
        fail(f"{path}.expectedOutputs contains duplicate routing outputs")


def _validate_runtime_attachment_profiles(runtime: dict[str, Any], mcp_profile_names: set[str]) -> None:
    attachment_section = as_mapping(runtime.get("attachmentProfiles"), "runtime package attachmentProfiles")
    _validate_known_keys(attachment_section, "runtime package attachmentProfiles", {"active", "profiles"})
    profiles = as_list(attachment_section.get("profiles"), "runtime package attachmentProfiles.profiles")
    if not profiles:
        fail("runtime package attachmentProfiles.profiles must be non-empty")

    profile_names: set[str] = set()
    skill_card_names: set[str] = set()
    full_skill_names: set[str] = set()
    for profile_index, item in enumerate(profiles):
        profile_path = f"runtime package attachmentProfiles.profiles[{profile_index}]"
        profile = as_mapping(item, profile_path)
        _validate_known_keys(profile, profile_path, ATTACHMENT_PROFILE_KEYS)
        profile_name = as_non_empty_string(profile.get("name"), f"{profile_path}.name")
        _validate_unique_name(profile_name, profile_names, profile_path, "attachment profile")
        mcp_profile = as_non_empty_string(profile.get("mcpProfile"), f"{profile_path}.mcpProfile")
        if mcp_profile not in mcp_profile_names:
            fail(f"{profile_path}.mcpProfile references unknown MCP profile {mcp_profile!r}")

        for card_index, card_value in enumerate(as_list(profile.get("skillCards"), f"{profile_path}.skillCards")):
            card_path = f"{profile_path}.skillCards[{card_index}]"
            card = as_mapping(card_value, card_path)
            _validate_known_keys(card, card_path, ROUTING_CARD_KEYS)
            card_name = as_non_empty_string(card.get("name"), f"{card_path}.name")
            _validate_unique_name(card_name, skill_card_names, card_path, "routing card")
            required_mcp_profile = as_non_empty_string(
                card.get("requiredMcpProfile"),
                f"{card_path}.requiredMcpProfile",
            )
            if required_mcp_profile not in mcp_profile_names:
                fail(f"{card_path}.requiredMcpProfile references unknown MCP profile {required_mcp_profile!r}")
            risk = as_non_empty_string(card.get("risk"), f"{card_path}.risk")
            if risk not in ROUTING_CARD_RISKS:
                fail(f"{card_path}.risk must be one of {sorted(ROUTING_CARD_RISKS)}")
            _validate_routing_card_fit(card, card_path)

        full_skill_names.update(as_string_list(profile.get("fullSkills"), f"{profile_path}.fullSkills"))

    active = attachment_section.get("active", {})
    active_section = as_mapping(active, "runtime package attachmentProfiles.active")
    active_sets = {
        "profiles": profile_names,
        "skillCards": skill_card_names,
        "fullSkills": full_skill_names,
    }
    _validate_known_keys(active_section, "runtime package attachmentProfiles.active", set(active_sets))
    for key, known_values in active_sets.items():
        path = f"runtime package attachmentProfiles.active.{key}"
        refs = as_string_list(active_section.get(key, []), path)
        if len(refs) > ATTACHMENT_ACTIVE_LIMIT:
            fail(f"{path} must contain at most {ATTACHMENT_ACTIVE_LIMIT} item(s)")
        if len(set(refs)) != len(refs):
            fail(f"{path} contains duplicate active references")
        for index, ref in enumerate(refs):
            if ref not in known_values:
                fail(f"{path}[{index}] references unknown {key} entry {ref!r}")


def validate_runtime_package_artifacts() -> None:
    package_path = ROOT / "runner-manifests" / "runtime" / "runtime-package.yaml"
    package = yaml.safe_load(package_path.read_text())
    package = as_mapping(package, "runtime-package.yaml")
    if package.get("apiVersion") != "agent-kit.runtime/v1alpha1":
        fail("runtime package apiVersion must be agent-kit.runtime/v1alpha1")
    if package.get("kind") != "AgentRuntimePackage":
        fail("runtime package kind must be AgentRuntimePackage")

    artifact = as_mapping(package.get("artifact"), "runtime package artifact")
    expected_artifact = {
        "repository": "ghcr.io/jorisjonkers-dev/agent-kit/runtime-home",
        "mediaType": "application/vnd.jorisjonkers.agent-kit.runtime-home.v1.tar+gzip",
        "tagPattern": "v{version}",
        "releaseAssetPattern": "agent-kit-runtime-home-v{version}.tar.gz",
        "digestAlgorithm": "sha256",
    }
    if artifact != expected_artifact:
        fail(f"runtime package artifact mismatch: {artifact}")

    build = as_mapping(package.get("build"), "runtime package build")
    for key in ("containerfile", "context"):
        path = ROOT / str(build.get(key, ""))
        if not path.exists():
            fail(f"runtime package build.{key} does not exist: {build.get(key)}")

    runtime = as_mapping(package.get("runtime"), "runtime package runtime")
    entrypoint = as_mapping(runtime.get("entrypoint"), "runtime package entrypoint")
    entrypoint_path = ROOT / str(entrypoint.get("path", ""))
    if not entrypoint_path.is_file():
        fail(f"runtime package entrypoint path does not exist: {entrypoint.get('path')}")
    required_self_tests = {"agent-kit-manifest", "repo-allow", "repo-dir"}
    actual_self_tests = set(as_list(entrypoint.get("selfTests"), "runtime package entrypoint.selfTests"))
    if actual_self_tests != required_self_tests:
        fail(f"runtime package selfTests mismatch: {sorted(actual_self_tests)}")

    home_bundle = as_mapping(runtime.get("homeBundle"), "runtime package homeBundle")
    expected_home_bundle = {
        "archiveRoot": ".",
        "homeSource": "home",
        "mcpSource": "mcp",
        "markerFile": ".agent-kit-home-install.sha256",
        "versionMarkerFile": ".knowledge-system-version",
        "installPolicy": "update-if-unmodified",
        "preserveCredentialFiles": [
            ".claude/.credentials.json",
            ".codex/auth.json",
            ".codex/config.toml",
        ],
    }
    if home_bundle != expected_home_bundle:
        fail(f"runtime package homeBundle mismatch: {home_bundle}")

    helpers = as_mapping(runtime.get("githubTokenHelper"), "runtime package githubTokenHelper")
    for key in ("helperPath", "ghWrapperPath", "gitCredentialHelperPath", "mcpWrapperPath"):
        helper_path = ROOT / str(helpers.get(key, ""))
        if not helper_path.is_file():
            fail(f"runtime package helper path does not exist: {helpers.get(key)}")
        if not os.access(helper_path, os.X_OK):
            fail(f"runtime package helper is not executable: {helpers.get(key)}")

    profile_section = as_mapping(runtime.get("mcpProfiles"), "runtime package mcpProfiles")
    placeholders = {
        name
        for item in as_list(profile_section.get("placeholders"), "runtime package mcpProfiles.placeholders")
        if isinstance(item, dict) and isinstance((name := item.get("name")), str)
    }
    required_placeholders = {
        "CLUSTER_MCP_URL",
        "FRONTEND_DOCS_MCP_URL",
        "UI_DOCS_MCP_URL",
    }
    if placeholders != required_placeholders:
        fail(f"runtime MCP placeholders mismatch: {sorted(placeholders)}")

    profiles = as_list(profile_section.get("profiles"), "runtime package mcpProfiles.profiles")
    mcp_profile_names: set[str] = set()
    for index, item in enumerate(profiles):
        profile = as_mapping(item, f"runtime package mcpProfiles.profiles[{index}]")
        profile_name = as_non_empty_string(profile.get("name"), f"runtime package mcpProfiles.profiles[{index}].name")
        _validate_unique_name(
            profile_name,
            mcp_profile_names,
            f"runtime package mcpProfiles.profiles[{index}]",
            "MCP profile",
        )
    if mcp_profile_names != {
        "minimal",
        "frontend",
        "cluster",
        "code-intel",
        "full-diagnostic",
    }:
        fail("runtime MCP profiles must include minimal, frontend, cluster, code-intel, and full-diagnostic")
    for index, item in enumerate(profiles):
        profile = as_mapping(item, f"runtime package mcpProfiles.profiles[{index}]")
        claude_path = ROOT / str(profile.get("claude", ""))
        codex_path = ROOT / str(profile.get("codex", ""))
        if not claude_path.is_file() or not codex_path.is_file():
            fail(f"runtime MCP profile files are missing for {profile.get('name')}")
        json.loads(claude_path.read_text())
        tomllib.loads(codex_path.read_text())
    _validate_runtime_attachment_profiles(runtime, mcp_profile_names)


def run_checked(command: list[str], env: dict[str, str] | None = None, input_text: str | None = None) -> str:
    completed = subprocess.run(
        command,
        cwd=ROOT,
        env=env,
        input=input_text,
        text=True,
        capture_output=True,
        check=False,
    )
    if completed.returncode != 0:
        fail(
            "command failed: "
            + " ".join(command)
            + f"\nstdout={completed.stdout}\nstderr={completed.stderr}",
        )
    return completed.stdout


def validate_git_credential_preseeded_token(
    runtime_dir: Path,
    env: dict[str, str],
    *,
    expected_token: str = "preseeded-token",
) -> None:
    credential = runtime_dir / "bin" / "git-credential-agent-gh-app"
    credential_output = run_checked(
        ["bash", str(credential), "get"],
        env={**env, "REPO_ALLOW": "owner/repo"},
        input_text="protocol=https\nhost=git-host\npath=owner/repo.git\n\n",
    )
    if f"password={expected_token}" not in credential_output:
        fail("git credential helper did not return a preseeded token for an allowed repo")


# A container runtime leaves one of these in the root of every container it
# starts; the entrypoint treats either as the runner container.
CONTAINER_MARKER_FILES = (Path("/.dockerenv"), Path("/run/.containerenv"))


def _home_tree(home: Path) -> dict[str, bytes | None]:
    """Every path under ``home`` with its bytes, or None for a directory."""
    return {
        str(path.relative_to(home)): (None if path.is_dir() else path.read_bytes()) for path in home.rglob("*")
    }


def validate_entrypoint_home_isolation(entrypoint: Path) -> None:
    """A full entrypoint run writes HOME and CODEX_HOME only inside the runner container.

    The entrypoint configures the agent homes it runs in: the global git config
    (naming the credential helper ``agent-gh-app``, which only the runner image
    ships), ``~/.claude.json`` (onboarding, bypass-permissions acceptance,
    workspace and repository trust) and Codex's ``config.toml`` (project trust).
    Run outside the container, on a workstation or in CI, all of it landed in
    the invoking user's real homes; every push there then failed with
    "'credential-agent-gh-app' is not a git command". So a run with a temp HOME
    and no container marker must leave HOME exactly as it was, whether it
    started empty or already held the owner's files. With the Containerfile's
    marker set, the same run must still write all of it: the container works
    unchanged.
    """
    seeded = {".claude.json": b'{"mine": true}\n', ".codex/config.toml": b'model = "mine"\n'}
    cases: tuple[tuple[str, bool, dict[str, bytes]], ...] = (
        ("outside-empty", False, {}),
        ("outside-seeded", False, seeded),
        ("inside", True, {}),
    )
    with tempfile.TemporaryDirectory() as tmp:
        for case, in_container, seed in cases:
            if not in_container and any(marker.exists() for marker in CONTAINER_MARKER_FILES):
                # This host is itself a container, so "outside" cannot be staged here.
                continue
            home = Path(tmp) / case / "home"
            workspace = Path(tmp) / case / "workspace"
            repo = workspace / "repo"
            (repo / ".git").mkdir(parents=True)
            home.mkdir(parents=True)
            for relative, content in seed.items():
                (home / relative).parent.mkdir(parents=True, exist_ok=True)
                (home / relative).write_bytes(content)
            before = _home_tree(home)
            env = {
                "PATH": os.environ.get("PATH", ""),
                "HOME": str(home),
                "CLAUDE_CONFIG_DIR": str(home / ".claude"),
                "CODEX_HOME": str(home / ".codex"),
                "WORKSPACE_ROOT": str(workspace),
                # No AGENT_MCP_DIR: a missing MCP profile file must not end the run.
                "AGENT_GIT_HOST": "git-host",
                # An existing checkout: the run registers its trust without cloning.
                "REPO_URL": "git@git-host:owner/repo.git",
                "GIT_CONFIG_NOSYSTEM": "1",
            }
            if in_container:
                env["AGENT_RUNNER_CONTAINER"] = "1"
            run_checked(["sh", str(entrypoint), "true"], env=env)

            if not in_container:
                after = _home_tree(home)
                changed = sorted(
                    path
                    for path in set(before) | set(after)
                    if (path in before) != (path in after) or before.get(path) != after.get(path)
                )
                if changed:
                    fail(f"entrypoint run outside the runner container changed HOME ({case}): {changed}")
                continue

            def global_values(key: str, run_env: dict[str, str] = env) -> list[str]:
                # Exit 1 means the key is unset: an empty list, not a failure.
                completed = subprocess.run(
                    ["git", "config", "--global", "--get-all", key],
                    env=run_env,
                    text=True,
                    capture_output=True,
                    check=False,
                )
                return completed.stdout.splitlines()

            if global_values("credential.helper") != ["agent-gh-app"]:
                fail("entrypoint run inside the runner container did not set credential.helper agent-gh-app")
            if global_values("credential.useHttpPath") != ["true"]:
                fail("entrypoint run inside the runner container did not set credential.useHttpPath")
            safe = global_values("safe.directory")
            if str(workspace) not in safe or str(repo) not in safe:
                fail(f"entrypoint run inside the runner container did not trust the workspace: {safe}")
            if global_values("url.https://git-host/.insteadOf") != ["git@git-host:", "ssh://git@git-host/"]:
                fail("entrypoint run inside the runner container did not rewrite both SSH URL spellings")

            claude_json = home / ".claude.json"
            if not claude_json.is_file():
                fail("entrypoint run inside the runner container did not write ~/.claude.json")
            if shutil.which("jq", path=env["PATH"]):
                claude = json.loads(claude_json.read_text())
                projects = claude.get("projects", {})
                if claude.get("bypassPermissionsModeAccepted") is not True:
                    fail("entrypoint run inside the runner container did not accept bypass permissions")
                for trusted in (workspace, repo):
                    if projects.get(str(trusted), {}).get("hasTrustDialogAccepted") is not True:
                        fail(f"entrypoint run inside the runner container did not trust {trusted} for Claude")
            codex_config = home / ".codex" / "config.toml"
            codex = tomllib.loads(codex_config.read_text()) if codex_config.is_file() else {}
            codex_projects = codex.get("projects", {})
            for trusted in (workspace, repo):
                if codex_projects.get(str(trusted), {}).get("trust_level") != "trusted":
                    fail(f"entrypoint run inside the runner container did not trust {trusted} for Codex")


def validate_runtime_shell_contracts(*, runtime_selftest: bool = False) -> None:
    runtime_dir = ROOT / "runner-manifests" / "runtime"
    entrypoint = runtime_dir / "entrypoint.sh"
    run_checked(["sh", "-n", str(entrypoint)])
    for path in sorted((runtime_dir / "bin").glob("*")):
        run_checked(["bash", "-n", str(path)])

    with tempfile.TemporaryDirectory() as tmp:
        tmp_path = Path(tmp)
        home = tmp_path / "home"
        workspace = tmp_path / "workspace"
        repo = workspace / "repo"
        repo_git = repo / ".git"
        (home / ".claude").mkdir(parents=True)
        (home / ".codex").mkdir(parents=True)
        repo_git.mkdir(parents=True)

        env = {
            "PATH": os.environ.get("PATH", ""),
            "HOME": str(home),
            "CLAUDE_CONFIG_DIR": str(home / ".claude"),
            "CODEX_HOME": str(home / ".codex"),
            "WORKSPACE_ROOT": str(workspace),
            "AGENT_GIT_HOST": "git-host",
            "REPO_URL": "git@git-host:owner/repo.git",
            "REPO_URLS": "ssh://git@git-host/owner/extra.git#dev;https://git-host/owner/second.git",
        }
        if os.environ.get("LANG"):
            env["LANG"] = os.environ["LANG"]
        env.update(
            {
                "AGENT_RUNNER_ENTRYPOINT_SELF_TEST": "repo-dir",
            },
        )

        repo_dir = run_checked(
            ["sh", str(entrypoint)],
            env=env,
        ).strip()
        if repo_dir != "repo":
            fail(f"entrypoint repo-dir self-test returned {repo_dir!r}")

        repo_allow = run_checked(
            ["sh", str(entrypoint)],
            env={**env, "AGENT_RUNNER_ENTRYPOINT_SELF_TEST": "repo-allow"},
        ).strip()
        if repo_allow != "owner/repo owner/extra owner/second":
            fail(f"entrypoint repo-allow self-test returned {repo_allow!r}")

        run_checked(["sh", str(entrypoint)], env={**env, "AGENT_RUNNER_ENTRYPOINT_SELF_TEST": "agent-kit-manifest"})

        helper = runtime_dir / "bin" / "agent-github-token"
        slug = run_checked(["bash", str(helper), "--print-slug", "git@git-host:owner/repo.git"], env=env).strip()
        if slug != "owner/repo":
            fail(f"agent-github-token --print-slug returned {slug!r}")

        if runtime_selftest:
            validate_git_credential_preseeded_token(runtime_dir, {**env, "GH_TOKEN": "preseeded-token"})

    validate_entrypoint_home_isolation(entrypoint)


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Validate the agent-kit manifest against checked-in sources.")
    parser.add_argument(
        "--runtime-selftest",
        action="store_true",
        help="also run runtime-only credential helper token self-tests",
    )
    return parser.parse_args(argv)


def runtime_selftest_enabled(args: argparse.Namespace) -> bool:
    return args.runtime_selftest or os.environ.get(RUNTIME_SELFTEST_ENV) == "1"


def main(argv: list[str] | None = None) -> int:
    args = parse_args(argv)
    runtime_selftest = runtime_selftest_enabled(args)
    try:
        manifest = load_manifest()
        validate_artifact(manifest)
        managed_paths = validate_renderer(manifest)
        validate_checksums(manifest, managed_paths)
        validate_surface_parity(manifest)
        validate_hooks(manifest)
        validate_council_command_surface()
        validate_council(manifest)
        validate_critique(manifest)
        validate_build_tracker(manifest)
        validate_runtime_package_manifest(manifest)
        validate_runtime_package_artifacts()
        validate_runtime_shell_contracts(runtime_selftest=runtime_selftest)
        validate_round3_skeleton()
    except AssertionError as exc:
        print(f"manifest validation failed: {exc}", file=sys.stderr)
        return 1

    suffix = " with runtime self-tests" if runtime_selftest else ""
    print(f"manifest validation passed{suffix}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
