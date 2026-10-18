"""Setting up a machine, and checking it with the doctor, needs no knowledge base.

The knowledge service is retired: no `KB_URL` or `KB_BEARER_TOKEN` exists any
more, so nothing on the install path may ask for one.
"""

from __future__ import annotations

import os
import subprocess
import sys
from pathlib import Path

KIT_ROOT = Path(__file__).resolve().parent.parent
DOCS = [KIT_ROOT / "README.md", KIT_ROOT / "PORTABILITY.md", *sorted((KIT_ROOT / "docs").glob("*.md"))]


def _doctor(*flags: str, **env: str) -> subprocess.CompletedProcess:
    clean = {k: v for k, v in os.environ.items() if k not in ("KB_URL", "KB_BEARER_TOKEN")}
    return subprocess.run(
        [sys.executable, str(KIT_ROOT / "render-agent-kit.py"), "--doctor", *flags],
        capture_output=True, text=True, check=False, cwd=KIT_ROOT, env={**clean, **env},
    )


def test_the_strict_doctor_passes_on_a_machine_with_no_knowledge_base() -> None:
    result = _doctor("--strict")
    assert result.returncode == 0, result.stdout + result.stderr


def test_the_doctor_probes_no_knowledge_base_even_when_one_is_configured() -> None:
    result = _doctor(KB_URL="http://127.0.0.1:9", KB_BEARER_TOKEN="stale")
    assert "kb-live" not in result.stdout, result.stdout


def test_the_doctor_no_longer_takes_require_live_kb() -> None:
    result = _doctor("--require-live-kb")
    assert result.returncode == 2
    assert "unrecognized arguments: --require-live-kb" in result.stderr


def test_no_document_tells_the_reader_to_set_a_knowledge_base_token() -> None:
    for doc in DOCS:
        text = doc.read_text()
        assert "KB_BEARER_TOKEN" not in text, doc.name
        assert "--require-live-kb" not in text, doc.name
