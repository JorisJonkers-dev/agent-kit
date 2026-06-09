from __future__ import annotations

import importlib.util
import os
import tempfile
import unittest
import unittest.mock
from pathlib import Path
from types import ModuleType

ROOT = Path(__file__).resolve().parents[1]
VALIDATOR_PATH = ROOT / "scripts" / "validate_manifest.py"


def load_validator() -> ModuleType:
    spec = importlib.util.spec_from_file_location("validate_manifest", VALIDATOR_PATH)
    if spec is None or spec.loader is None:
        raise AssertionError("cannot load validate_manifest.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class ValidateManifestRuntimeSelfTest(unittest.TestCase):
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


if __name__ == "__main__":
    unittest.main()
