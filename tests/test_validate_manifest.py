from __future__ import annotations

import importlib.util
import io
import os
import sys
import tempfile
import unittest
import unittest.mock
from contextlib import redirect_stdout
from importlib.machinery import ModuleSpec
from pathlib import Path
from types import ModuleType
from typing import ClassVar

ROOT = Path(__file__).resolve().parents[1]
VALIDATOR_PATH = ROOT / "scripts" / "validate_manifest.py"
RENDERER_PATH = ROOT / "render-agent-kit.py"


def load_validator() -> ModuleType:
    spec = importlib.util.spec_from_file_location("validate_manifest", VALIDATOR_PATH)
    if spec is None or spec.loader is None:
        raise AssertionError("cannot load validate_manifest.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def load_renderer() -> ModuleType:
    spec = importlib.util.spec_from_file_location("render_agent_kit", RENDERER_PATH)
    if spec is None or spec.loader is None:
        raise AssertionError("cannot load render-agent-kit.py")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


def valid_attachment_runtime() -> dict[str, object]:
    return {
        "attachmentProfiles": {
            "active": {
                "profiles": ["default"],
                "skillCards": ["routing-card"],
                "fullSkills": ["full-skill"],
            },
            "profiles": [
                {
                    "name": "default",
                    "mcpProfile": "minimal",
                    "skillCards": [
                        {
                            "name": "routing-card",
                            "purpose": "Route simple work",
                            "positiveTriggers": ["simple", "small"],
                            "negativeTriggers": ["cluster", "frontend"],
                            "requiredMcpProfile": "minimal",
                            "risk": "low",
                            "expectedOutputs": ["plan", "summary"],
                        },
                    ],
                    "fullSkills": ["full-skill"],
                },
            ],
        },
    }


class ValidateManifestRuntimeSelfTest(unittest.TestCase):
    validator: ClassVar[ModuleType]

    @classmethod
    def setUpClass(cls) -> None:
        cls.validator = load_validator()

    def test_runtime_selftest_is_off_by_default(self) -> None:
        args = self.validator.parse_args([])
        with unittest.mock.patch.dict(os.environ, {self.validator.RUNTIME_SELFTEST_ENV: ""}, clear=False):
            self.assertFalse(self.validator.runtime_selftest_enabled(args))

    def test_runtime_selftest_flag_or_env_enable_probe(self) -> None:
        self.assertTrue(self.validator.runtime_selftest_enabled(self.validator.parse_args(["--runtime-selftest"])))
        args = self.validator.parse_args([])
        with unittest.mock.patch.dict(os.environ, {self.validator.RUNTIME_SELFTEST_ENV: "1"}, clear=False):
            self.assertTrue(self.validator.runtime_selftest_enabled(args))

    def test_credential_probe_accepts_preseeded_mock_helper(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            runtime_dir = Path(tmp) / "runtime"
            bin_dir = runtime_dir / "bin"
            bin_dir.mkdir(parents=True)

            credential = bin_dir / "git-credential-agent-gh-app"
            credential.write_text((ROOT / "runner-manifests/runtime/bin/git-credential-agent-gh-app").read_text())
            credential.chmod(0o755)

            token_helper = bin_dir / "mock-token-helper"
            token_helper.write_text(
                "#!/usr/bin/env bash\n"
                "set -eu\n"
                'if [ "${AGENT_GITHUB_REPO_URL:-}" = "https://git-host/owner/repo" ]; then\n'
                "  printf 'preseeded-token'\n"
                "  exit 0\n"
                "fi\n"
                "exit 1\n",
            )
            token_helper.chmod(0o755)

            env = {
                "PATH": os.environ.get("PATH", ""),
                "AGENT_GIT_HOST": "git-host",
                "AGENT_GITHUB_TOKEN_HELPER": str(token_helper),
            }
            self.validator.validate_git_credential_preseeded_token(runtime_dir, env)


class ValidateManifestCouncilCommandSurface(unittest.TestCase):
    validator: ClassVar[ModuleType]

    @classmethod
    def setUpClass(cls) -> None:
        cls.validator = load_validator()

    def test_current_cli_source_contains_required_council_command_surface(self) -> None:
        self.validator.validate_council_command_surface()

    def test_cli_surface_requires_source_file(self) -> None:
        with (
            unittest.mock.patch.object(
                self.validator,
                "COUNCIL_CLI_INDEX_PATH",
                ROOT / "council" / "ts" / "src" / "cli" / "missing.ts",
            ),
            self.assertRaisesRegex(AssertionError, "council command surface source is missing"),
        ):
            self.validator.validate_council_command_surface()

    def test_cli_surface_requires_registry_specs_for_required_commands(self) -> None:
        source = """
const COMMANDS: readonly CommandSpec[] = [
  { help: 'score a run with the eval workflow', name: 'eval' },
]
export async function runCli(command: string) {
  switch (command) {
    case 'eval':
      return okJson(await app.eval(parseEval(rest)))
    case 'status':
      return await runStatusCommand(app, parseStatus(rest))
    case 'tail':
      return await runTailCommand(app, parseTail(rest))
    case 'triage':
      return okJson(await app.triage(parseTriage(rest)))
  }
}
"""

        with self.assertRaisesRegex(
            AssertionError,
            r"required commands \(eval, status, tail, triage\) missing command registry specs: status, tail, triage",
        ):
            self.validator.validate_council_command_surface_source(source)

    def test_cli_surface_accepts_direct_and_handler_dispatch_branches(self) -> None:
        source = """
const COMMANDS: readonly CommandSpec[] = [
  { help: 'score a run with the eval workflow', name: 'eval' },
  { help: 'summarize a run directory', name: 'status' },
  { help: 'tail one task log', name: 'tail' },
  { help: 'run the triage gate and emit routing payload', name: 'triage' },
]
export async function runCli(command: string) {
  switch (command) {
    case 'eval':
      return okJson(await app.eval(parseEval(rest)))
    case 'status':
      return await runStatusCommand(app, parseStatus(rest))
    case 'tail':
      return await runTailCommand(app, parseTail(rest))
    case 'triage':
      return okJson(await app.triage(parseTriage(rest)))
  }
}
"""

        self.validator.validate_council_command_surface_source(source)

    def test_cli_surface_requires_dispatch_branches_for_required_commands(self) -> None:
        source = """
const COMMANDS: readonly CommandSpec[] = [
  { help: 'score a run with the eval workflow', name: 'eval' },
  { help: 'summarize a run directory', name: 'status' },
  { help: 'tail one task log', name: 'tail' },
  { help: 'run the triage gate and emit routing payload', name: 'triage' },
]
export async function runCli(command: string) {
  switch (command) {
    case 'eval':
      return okJson(await app.eval(parseEval(rest)))
    case 'status':
      return okJson({ command: 'status', compiled: true })
    case 'tail':
      return okJson({ command: 'tail', compiled: true })
    case 'triage':
      return okJson({ command: 'triage', compiled: true })
  }
}
"""

        with self.assertRaisesRegex(
            AssertionError,
            r"required commands \(eval, status, tail, triage\) missing command dispatch branches: status, tail, triage",
        ):
            self.validator.validate_council_command_surface_source(source)


class ValidateManifestAttachmentProfiles(unittest.TestCase):
    validator: ClassVar[ModuleType]

    @classmethod
    def setUpClass(cls) -> None:
        cls.validator = load_validator()

    def test_attachment_profiles_reject_unknown_profile_mcp_profile(self) -> None:
        runtime = valid_attachment_runtime()
        runtime["attachmentProfiles"]["profiles"][0]["mcpProfile"] = "missing"

        with self.assertRaisesRegex(AssertionError, "mcpProfile references unknown MCP profile 'missing'"):
            self.validator._validate_runtime_attachment_profiles(runtime, {"minimal"})

    def test_attachment_profiles_reject_unknown_routing_card_mcp_profile(self) -> None:
        runtime = valid_attachment_runtime()
        card = runtime["attachmentProfiles"]["profiles"][0]["skillCards"][0]
        card["requiredMcpProfile"] = "missing"

        with self.assertRaisesRegex(AssertionError, "requiredMcpProfile references unknown MCP profile 'missing'"):
            self.validator._validate_runtime_attachment_profiles(runtime, {"minimal"})

    def test_attachment_profiles_reject_duplicate_routing_card_references(self) -> None:
        runtime = valid_attachment_runtime()
        runtime["attachmentProfiles"]["active"]["skillCards"] = ["routing-card", "routing-card"]

        with self.assertRaisesRegex(AssertionError, "active.skillCards contains duplicate active references"):
            self.validator._validate_runtime_attachment_profiles(runtime, {"minimal"})

    def test_attachment_profiles_reject_active_set_over_cap(self) -> None:
        runtime = valid_attachment_runtime()
        full_skills = [f"skill-{index}" for index in range(self.validator.ATTACHMENT_ACTIVE_LIMIT + 1)]
        runtime["attachmentProfiles"]["profiles"][0]["fullSkills"] = full_skills
        runtime["attachmentProfiles"]["active"]["fullSkills"] = full_skills

        with self.assertRaisesRegex(AssertionError, "active.fullSkills must contain at most 12 item"):
            self.validator._validate_runtime_attachment_profiles(runtime, {"minimal"})


class RenderAgentKitDoctor(unittest.TestCase):
    renderer: ClassVar[ModuleType]

    @classmethod
    def setUpClass(cls) -> None:
        cls.renderer = load_renderer()

    def test_manifest_check_reports_successful_command_surface_validation(self) -> None:
        result = self.renderer.manifest_check()

        self.assertEqual(result.name, "manifest")
        self.assertEqual(result.status, "ok")
        self.assertIn("council command surface validated: eval, status, tail, triage", result.detail)

    def test_manifest_check_reports_missing_manifest(self) -> None:
        with unittest.mock.patch.object(self.renderer, "MANIFEST_PATH", ROOT / "missing-manifest.yaml"):
            result = self.renderer.manifest_check()

        self.assertEqual(result, self.renderer.DoctorCheck("manifest", "fail", "manifest.yaml is missing"))

    def test_manifest_check_reports_unloadable_validator(self) -> None:
        with unittest.mock.patch.object(self.renderer.importlib.util, "spec_from_file_location", return_value=None):
            result = self.renderer.manifest_check()

        self.assertEqual(
            result,
            self.renderer.DoctorCheck("manifest", "fail", "cannot load scripts/validate_manifest.py"),
        )

    def test_manifest_check_reports_council_command_surface_assertion(self) -> None:
        spec = ModuleSpec("validate_manifest", AssertionLoader())

        with unittest.mock.patch.object(self.renderer.importlib.util, "spec_from_file_location", return_value=spec):
            result = self.renderer.manifest_check()

        self.assertEqual(result, self.renderer.DoctorCheck("council-command-surface", "fail", "surface drift"))

    def test_manifest_check_reports_validator_load_failure(self) -> None:
        spec = ModuleSpec("validate_manifest", SystemExitLoader())

        with unittest.mock.patch.object(self.renderer.importlib.util, "spec_from_file_location", return_value=spec):
            result = self.renderer.manifest_check()

        self.assertEqual(result, self.renderer.DoctorCheck("manifest", "fail", "missing dependency"))

    def test_doctor_reports_council_command_surface_failure(self) -> None:
        args = self.renderer.parse_args(["--doctor"])
        ok_findings = self.renderer.RenderFindings(missing=[], drifted=[])

        with (
            unittest.mock.patch.object(self.renderer, "render_findings", return_value=ok_findings),
            unittest.mock.patch.object(
                self.renderer,
                "manifest_check",
                return_value=self.renderer.DoctorCheck(
                    name="council-command-surface",
                    status="fail",
                    detail=(
                        "required commands (eval, status, tail, triage) "
                        "missing command dispatch branches: status, tail"
                    ),
                ),
            ),
            unittest.mock.patch.object(
                self.renderer,
                "kb_reachability_check",
                return_value=self.renderer.DoctorCheck(name="kb-live", status="ok", detail="skipped"),
            ),
        ):
            output = io.StringIO()
            with redirect_stdout(output):
                exit_code = self.renderer.doctor(args)

        self.assertEqual(exit_code, 1)
        self.assertIn(
            "fail council-command-surface: required commands (eval, status, tail, triage) "
            "missing command dispatch branches: status, tail",
            output.getvalue(),
        )


class AssertionLoader:
    def create_module(self, _spec: ModuleSpec) -> None:
        return None

    def exec_module(self, module: ModuleType) -> None:
        def fail_validation() -> None:
            raise AssertionError("surface drift")

        module.validate_council_command_surface = fail_validation


class SystemExitLoader:
    def create_module(self, _spec: ModuleSpec) -> None:
        return None

    def exec_module(self, _module: ModuleType) -> None:
        raise SystemExit("missing dependency")


if __name__ == "__main__":
    unittest.main()
