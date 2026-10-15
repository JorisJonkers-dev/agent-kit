"""The kit's HTTPS calls work on a python3 that ships no CA certificates.

The python.org macOS installer leaves OpenSSL with an empty trust store until
its "Install Certificates" step is run, so every HTTPS call fails with
CERTIFICATE_VERIFY_FAILED. Both stdlib scripts fall back to the system bundle.
A network failure is a one-line note, never a traceback.
"""

from __future__ import annotations

import importlib.machinery
import importlib.util
import json
import os
import subprocess
import sys
from pathlib import Path
from types import ModuleType

import pytest

KIT_ROOT = Path(__file__).resolve().parent.parent
SCRIPTS = {
    "keys": KIT_ROOT / "scripts" / "agent-kit-keys.py",
}
# An empty trust store, as on a python.org install before "Install Certificates".
NO_CA = {"SSL_CERT_FILE": "/dev/null", "SSL_CERT_DIR": "/nonexistent"}


def _load(path: Path) -> ModuleType:
    loader = importlib.machinery.SourceFileLoader(f"kit_{path.stem.replace('-', '_')}", str(path))
    spec = importlib.util.spec_from_loader(loader.name, loader)
    assert spec
    module = importlib.util.module_from_spec(spec)
    loader.exec_module(module)
    return module


@pytest.mark.parametrize("name", sorted(SCRIPTS))
def test_an_empty_trust_store_falls_back_to_the_system_bundle(
    name: str, monkeypatch: pytest.MonkeyPatch,
) -> None:
    for key, value in NO_CA.items():
        monkeypatch.setenv(key, value)
    import ssl

    assert ssl.create_default_context().cert_store_stats()["x509_ca"] == 0, "the empty store is reproduced"
    context = _load(SCRIPTS[name]).tls_context()
    assert context.cert_store_stats()["x509_ca"] > 0
    assert context.verify_mode == ssl.CERT_REQUIRED and context.check_hostname


def test_a_working_trust_store_is_left_alone(monkeypatch: pytest.MonkeyPatch) -> None:
    import ssl

    module = _load(SCRIPTS["keys"])
    default = ssl.create_default_context().cert_store_stats()["x509_ca"]
    assert module.tls_context().cert_store_stats()["x509_ca"] == default


def test_an_unreachable_auth_api_is_a_note_not_a_traceback(tmp_path: Path) -> None:
    table = tmp_path / "table.json"
    table.write_text(json.dumps([{"env": "HINDSIGHT_API_TOKEN", "grant": "MEMORY_API",
                                  "check": "https://127.0.0.1:9/check", "export": True}]))
    env = {**os.environ, "HOME": str(tmp_path), "AGENT_KIT_AUTH_URL": "https://127.0.0.1:9",
           "AGENT_KIT_SECRETS_BACKEND": "file", "SHELL": "/bin/zsh"}
    result = subprocess.run(
        [sys.executable, str(SCRIPTS["keys"]), "--table", str(table), "ensure"],
        input="me@example.com\npw\n\n", capture_output=True, text=True, check=False, env=env, timeout=60,
        start_new_session=True,
    )
    assert "Traceback" not in result.stderr
    assert "could not reach auth.jorisjonkers.dev" in result.stderr
    assert result.returncode == 0
