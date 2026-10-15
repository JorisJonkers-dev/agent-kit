"""setup-workstation ends with one list of everything still to do.

Every warning and failure the run raised is listed there, failures first, so
nothing that needs a hand is buried in the scroll-back. Steps setup cannot do
itself, such as minting the private-bank token, are checked and listed too.
"""

from __future__ import annotations

import os
import subprocess
import sys
from pathlib import Path

import pytest

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


def test_a_skipped_server_says_which_grant_and_token_to_mint() -> None:
    script = _script()
    assert (
        'warn "memory-api: HINDSIGHT_API_TOKEN is not set; skipping. Grant yourself MEMORY_API on the '
        'admin page, mint a MEMORY_API service token, export it as HINDSIGHT_API_TOKEN and re-run"'
    ) in script
    assert "mint a MEMORY_MCP service token, export it as MEMORY_MCP_TOKEN" in script


# --- the private-bank token: setup cannot mint it, so it checks and lists it ---


def _security_stub(bin_dir: Path, found: bool) -> None:
    stub = bin_dir / "security"
    stub.write_text(f"#!/bin/sh\nexit {0 if found else 44}\n")
    stub.chmod(0o755)


def _vibe_stub(bin_dir: Path) -> None:
    stub = bin_dir / "vibe"
    stub.write_text("#!/bin/sh\nexit 0\n")
    stub.chmod(0o755)


def _check(tmp_path: Path, *, keychain: bool, env_token: bool = False, vibe: bool = True,
           rc: str | None = None, cloud: bool = False) -> list[str]:
    script = _script()
    bin_dir = tmp_path / "bin"
    bin_dir.mkdir(exist_ok=True)
    _security_stub(bin_dir, keychain)
    if vibe:
        _vibe_stub(bin_dir)
    home = tmp_path / "home"
    home.mkdir(exist_ok=True)
    if rc is not None:
        (home / ".zshrc").write_text(rc)
    env = {"PATH": f"{bin_dir}:/usr/bin:/bin", "HOME": str(home), "CLOUD": "0"}
    body = f"CLOUD={int(cloud)}\n" + _prelude(script) + _fn(script, "check_memory_private_token") + (
        'check_memory_private_token\nprintf "%s\\n" "${todo_warnings[@]+"${todo_warnings[@]}"}"\n'
    )
    if not env_token:
        body = "unset MEMORY_PRIVATE_TOKEN\n" + body
    else:
        env["MEMORY_PRIVATE_TOKEN"] = "smt_x"
    result = _bash(body, env)
    assert result.returncode == 0, result.stderr
    return [line for line in result.stdout.splitlines() if line]


ALIAS = 'alias vibe=\'MEMORY_PRIVATE_TOKEN="$(security find-generic-password -s memory-private -w)" command vibe\'\n'


def test_no_private_token_anywhere_is_listed_with_how_to_get_one(tmp_path: Path) -> None:
    [todo, *_] = _check(tmp_path, keychain=False, rc=ALIAS)
    assert "MEMORY_PRIVATE" in todo
    assert "admin page" in todo
    assert 'security add-generic-password -a "$USER" -s memory-private -w' in todo


@pytest.mark.parametrize(("keychain", "env_token"), [(True, False), (False, True)])
def test_a_token_in_the_keychain_or_the_environment_is_enough(
    tmp_path: Path, keychain: bool, env_token: bool,
) -> None:
    assert _check(tmp_path, keychain=keychain, env_token=env_token, rc=ALIAS) == []


def test_vibe_without_the_launch_alias_is_listed(tmp_path: Path) -> None:
    [todo] = _check(tmp_path, keychain=True, rc="export EDITOR=vim\n")
    assert "vibe" in todo and "docs/SETUP.md" in todo


def test_no_vibe_means_no_alias_to_add(tmp_path: Path) -> None:
    assert _check(tmp_path, keychain=True, vibe=False) == []


def test_a_cloud_session_gets_its_token_with_the_session(tmp_path: Path) -> None:
    assert _check(tmp_path, keychain=False, rc="", cloud=True) == []


def test_the_run_calls_the_check() -> None:
    script = _script()
    assert "\ncheck_memory_private_token\n" in script.split(_fn(script, "check_memory_private_token"))[1]
