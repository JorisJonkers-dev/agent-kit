#!/usr/bin/env python3
"""Validate the agent-kit manifest against checked-in sources."""

from __future__ import annotations

import hashlib
import importlib.util
import json
import re
import sys
from collections.abc import Iterable
from pathlib import Path
from typing import Any

try:
    import yaml
except ModuleNotFoundError as exc:  # pragma: no cover - exercised in CI setup failures
    raise SystemExit("PyYAML is required: python3 -m pip install -r requirements-dev.txt") from exc


ROOT = Path(__file__).resolve().parents[1]
MANIFEST_PATH = ROOT / "manifest.yaml"
ROUND3_SKELETON_DIR = ROOT / "runner-manifests"
ROUND3_SPEC_DIR = ROOT / "specs" / "002-round3-agent-runner-manifests"
ROUND3_VALIDATED_DIRS = (ROUND3_SKELETON_DIR, ROUND3_SPEC_DIR)

ROUND3_FORBIDDEN_PATTERNS = {
    "personal domain or hostname": re.compile(
        r"\b(?:jorisjonkers|esa-blueshell|blueshell|enschede|frankfurt|contabo)\b",
        re.IGNORECASE,
    ),
    "concrete namespace": re.compile(r"\b(?:agents-system|assistant-system|knowledge-system)\b"),
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


def fail(message: str) -> None:
    raise AssertionError(message)


def as_list(value: Any, name: str) -> list[Any]:
    if not isinstance(value, list):
        fail(f"{name} must be a list")
    return value


def as_mapping(value: Any, name: str) -> dict[str, Any]:
    if not isinstance(value, dict):
        fail(f"{name} must be a mapping")
    return value


def all_mappings(value: Any) -> Iterable[dict[str, Any]]:
    if isinstance(value, dict):
        yield value
        for child in value.values():
            yield from all_mappings(child)
    elif isinstance(value, list):
        for item in value:
            yield from all_mappings(item)


def pinned_path_entries(manifest: dict[str, Any]) -> list[dict[str, str]]:
    entries: list[dict[str, str]] = []
    for item in all_mappings(manifest):
        path = item.get("path")
        digest = item.get("sha256")
        if isinstance(path, str) and isinstance(digest, str):
            entries.append({"path": path, "sha256": digest})
    return entries


def validate_artifact(manifest: dict[str, Any]) -> None:
    artifact = as_mapping(manifest.get("artifact"), "artifact")
    coordinate = artifact.get("short_coordinate")
    if coordinate != "github:ExtraToast/agent-kit":
        fail("artifact.short_coordinate must be github:ExtraToast/agent-kit")
    if re.search(r"agent-kit[-_/]agent-kit", coordinate, re.IGNORECASE):
        fail("artifact.short_coordinate repeats the agent-kit segment")
    if artifact.get("package_publish") != "none":
        fail("artifact.package_publish must be none for the initial tool repo release")


def validate_renderer(manifest: dict[str, Any]) -> set[str]:
    renderer = load_renderer()
    renderer_manifest = as_mapping(manifest.get("renderer"), "renderer")
    if renderer_manifest.get("script_path") != "render-agent-kit.py":
        fail("renderer.script_path must be render-agent-kit.py")
    if renderer_manifest.get("template_root") != "templates/repo":
        fail("renderer.template_root must be templates/repo")

    managed_paths = set(as_list(renderer_manifest.get("managed_paths"), "renderer.managed_paths"))
    expected = {item.relative_path.as_posix() for item in renderer.template_files(ROOT)}
    if managed_paths != expected:
        missing = sorted(expected - managed_paths)
        extra = sorted(managed_paths - expected)
        fail(f"renderer.managed_paths mismatch; missing={missing}; extra={extra}")

    for path in as_list(renderer_manifest.get("include_templates"), "renderer.include_templates"):
        if not isinstance(path, str):
            fail("renderer.include_templates entries must be strings")
        if not (ROOT / path).is_file():
            fail(f"include template does not exist: {path}")

    extra_templates = as_list(renderer_manifest.get("extra_templates"), "renderer.extra_templates")
    expected_extra_templates = [
        {"source_path": "templates/installer/install.sh.tpl", "destination_path": "installer/install.sh"},
    ]
    if extra_templates != expected_extra_templates:
        fail("renderer.extra_templates must map templates/installer/install.sh.tpl to installer/install.sh")

    return managed_paths


def validate_checksums(manifest: dict[str, Any], managed_paths: set[str]) -> None:
    pinned_entries = pinned_path_entries(manifest)
    seen: dict[str, str] = {}
    for entry in pinned_entries:
        path = entry["path"]
        digest = entry["sha256"]
        full_path = ROOT / path
        if not full_path.is_file():
            fail(f"pinned path does not exist: {path}")
        actual = sha256(full_path)
        if actual != digest:
            fail(f"sha256 mismatch for {path}: manifest={digest} actual={actual}")
        if path in seen and seen[path] != digest:
            fail(f"pinned path has conflicting sha256 values: {path}")
        seen[path] = digest

    missing = sorted(managed_paths - set(seen))
    if missing:
        fail(f"managed paths are not pinned with sha256: {missing}")


def validate_surface_parity(manifest: dict[str, Any]) -> None:
    skills = as_list(manifest.get("skills"), "skills")
    commands = as_list(manifest.get("commands"), "commands")

    command_names = {str(item["name"]).removeprefix("speckit.") for item in commands}
    speckit_skill_names = {
        str(item["name"]).removeprefix("speckit-")
        for item in skills
        if str(item.get("name", "")).startswith("speckit-")
    }
    if command_names != speckit_skill_names:
        fail(
            "Spec Kit command/skill parity mismatch: "
            f"commands={sorted(command_names)} skills={sorted(speckit_skill_names)}",
        )

    for command in commands:
        supported = command.get("supported_agents")
        if supported != ["claude"]:
            fail(f"command {command.get('name')} must support only claude")
        if not as_mapping(command.get("unsupported"), f"command {command.get('name')}.unsupported").get("codex"):
            fail(f"command {command.get('name')} must record a codex unsupported reason")

    for skill in skills:
        name = str(skill.get("name"))
        supported = skill.get("supported_agents")
        targets_value = skill.get("targets", [])
        targets = {target.get("agent") for target in as_list(targets_value, f"skill {name}.targets")}
        if name.startswith("speckit-"):
            if supported != ["codex"] or targets != {"codex"}:
                fail(f"skill {name} must support only codex")
            if not as_mapping(skill.get("unsupported"), f"skill {name}.unsupported").get("claude"):
                fail(f"skill {name} must record a claude unsupported reason")
        else:
            installer = skill.get("installer")
            installer_targets = set()
            if isinstance(installer, dict):
                if installer.get("target_path"):
                    installer_targets.add("claude")
                if installer.get("codex_target_path"):
                    installer_targets.add("codex")
            if targets and (sorted(supported or []) != ["claude", "codex"] or targets != {"claude", "codex"}):
                fail(f"shared skill {name} must target claude and codex")
            if not targets and (
                sorted(supported or []) != ["claude", "codex"] or installer_targets != {"claude", "codex"}
            ):
                fail(f"installer-only skill {name} must install to claude and codex")

    settings = as_list(manifest.get("settings"), "settings")
    setting_agents = {item.get("agent") for item in settings if isinstance(item, dict)}
    if setting_agents != {"claude", "codex"}:
        fail(f"settings must include claude and codex entries: {sorted(setting_agents)}")

    for section in ("hooks", "installer"):
        value = manifest.get(section)
        items = value if isinstance(value, list) else [value]
        for item in items:
            if not isinstance(item, dict):
                fail(f"{section} entries must be mappings")
            supported = sorted(item.get("supported_agents") or [])
            unsupported = item.get("unsupported")
            if supported != ["claude", "codex"] and not unsupported:
                label = item.get("name", item.get("path", section))
                fail(f"{section} entry {label} needs parity or unsupported reason")


def validate_council(manifest: dict[str, Any]) -> None:
    council = as_mapping(manifest.get("council"), "council")
    files = as_list(council.get("files"), "council.files")
    manifest_files = {item.get("path") for item in files}
    actual_files = {
        path.relative_to(ROOT).as_posix()
        for path in (ROOT / "council").rglob("*")
        if path.is_file() and path.name != "README.md" and "__pycache__" not in path.parts and path.suffix != ".pyc"
    }
    if manifest_files != actual_files:
        fail(f"council.files mismatch: manifest={sorted(manifest_files)} actual={sorted(actual_files)}")


def validate_installer(manifest: dict[str, Any]) -> None:
    installer = as_mapping(manifest.get("installer"), "installer")
    if installer.get("path") != "installer/install.sh":
        fail("installer.path must be installer/install.sh")

    body = (ROOT / "installer" / "install.sh").read_text(errors="replace")
    for token in ("@VERSION@", "@KB_URL@"):
        if token not in body:
            fail(f"installer missing placeholder {token}")

    secret_patterns = {
        "bearer token": re.compile(r"Bearer\s+[A-Za-z0-9._~+/-]{16,}=*"),
        "private key": re.compile(r"-----BEGIN [A-Z ]*PRIVATE KEY-----"),
        "transcript block": re.compile(r"BEGIN RAW TRANSCRIPT|END RAW TRANSCRIPT", re.IGNORECASE),
    }
    matches = [name for name, pattern in secret_patterns.items() if pattern.search(body)]
    if matches:
        fail("installer contains secret-like markers: " + ",".join(matches))


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


def main() -> int:
    try:
        manifest = load_manifest()
        validate_artifact(manifest)
        managed_paths = validate_renderer(manifest)
        validate_checksums(manifest, managed_paths)
        validate_surface_parity(manifest)
        validate_council(manifest)
        validate_installer(manifest)
        validate_round3_skeleton()
    except AssertionError as exc:
        print(f"manifest validation failed: {exc}", file=sys.stderr)
        return 1

    print("manifest validation passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
