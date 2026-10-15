"""setup-workstation ends with one list of everything still to do.

Every warning and failure the run raised is listed there, failures first, so
nothing that needs a hand is buried in the scroll-back.
"""

from __future__ import annotations

import os
import subprocess
import sys
from pathlib import Path

KIT_ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(KIT_ROOT / "scripts"))

import render_registry  # noqa: E402
import yaml  # noqa: E402


def _registry() -> dict:
    return yaml.safe_load((KIT_ROOT / "registry" / "estate-tooling.yaml").read_text())


def _script() -> str:
    return render_registry.render_setup_script(_registry())


def _fn(script: str, name: str) -> str:
    return f"{name}() {{" + script.split(f"\n{name}() {{")[1].split("\n}\n")[0] + "\n}\n"


def _helpers(script: str) -> str:
    start = script.index("\nlog()  {")
    return script[start : script.index("\n\n", start)] + "\n"


def _bash(body: str, env: dict | None = None) -> subprocess.CompletedProcess:
    return subprocess.run(
        # /bin/bash is 3.2 on macOS: no associative arrays, and set -u trips on empty ones.
        ["/bin/bash", "-c", body], capture_output=True, text=True, check=False, env={**os.environ, **(env or {})},
    )


def _prelude(script: str) -> str:
    return "set -u\nfailures=0\nwarnings=0\ntodo_failures=()\ntodo_warnings=()\n" + _helpers(script)


def test_the_list_holds_every_warning_and_failure_once_failures_first() -> None:
    script = _script()
    body = _prelude(script) + _fn(script, "print_todo") + (
        'warn "export A"\nfail "B did not install"\nwarn "export A"\nwarn "export C"\nprint_todo\n'
    )
    out = _bash(body).stdout.splitlines()
    todo = out[out.index("setup: Still to do (3):") + 1 :]
    assert todo == [
        "setup:   1. FAIL B did not install",
        "setup:   2. export A",
        "setup:   3. export C",
    ]


def test_a_clean_run_says_so() -> None:
    script = _script()
    out = _bash(_prelude(script) + _fn(script, "print_todo") + "print_todo\n").stdout
    assert "setup: Nothing left to do." in out


def test_the_run_ends_with_the_list() -> None:
    tail = _script().rstrip().splitlines()[-4:]
    assert "print_todo" in tail
    assert tail[-1] == '[ "${failures}" = 0 ]'


def test_a_plugin_disabled_on_purpose_does_not_block_ready() -> None:
    script = _script()
    final = script[script.index("# Final summary") :]
    assert "disabled_plugins" not in final
