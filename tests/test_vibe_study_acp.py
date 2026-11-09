"""The vibe-study-acp launcher: the Study account over ACP for Gold Band (#336).

Gold Band starts ``~/.local/bin/vibe-study-acp`` as its vibe-study agent's
adapter (#338 declares it in the registry). The launcher reads the Study key
exactly the way ``vibe-study`` does -- the environment first, then the
agent-kit secret store's one-line reader in ``env.sh`` -- hands it to the ACP
binary as ``MISTRAL_API_KEY`` with the Study variable and any Private Memory
unlock dropped, and uses the shared Vibe home. Without a key it starts
nothing: the key never reaches a Gold Band file, so a missing one fails
loudly instead of falling back to the personal login. Setup installs the
launcher the way it installs the other kit commands: executable, and
repaired when missing.

Each case runs the whole checked-in setup script against a temporary home
(tests/setup_harness.py), then runs the installer it installed from
``~/.local/bin`` with a stub ACP binary that reports the environment it was
started with.
"""

from __future__ import annotations

import os
import subprocess
import sys
from pathlib import Path

KIT_ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(Path(__file__).resolve().parent))

from setup_harness import setup_run  # noqa: E402

LAUNCHER = "vibe-study-acp"
STUDY_KEY = "study-key-123"
MESSAGE = "vibe-study-acp: not signed in with the Study account; run: vibe-study --login"

# Stands in for the ACP binary Gold Band starts: it writes the environment and
# the arguments it was started with to $ACP_REPORT, so the tests can see both
# what reached it and that it ran at all.
ACP_STUB = """#!/usr/bin/env bash
{
  printf 'MISTRAL_API_KEY=%s\\n' "${MISTRAL_API_KEY:-unset}"
  printf 'MISTRAL_STUDY_API_KEY=%s\\n' "${MISTRAL_STUDY_API_KEY:-unset}"
  printf 'MEMORY_PRIVATE_TOKEN=%s\\n' "${MEMORY_PRIVATE_TOKEN:-unset}"
  printf 'VIBE_HOME=%s\\n' "${VIBE_HOME:-unset}"
  printf 'ARGS=%s\\n' "$*"
} > "${ACP_REPORT}"
"""

# The stub secret store: what agent-kit-keys' env.sh gives the shell. The
# empty one answers no key; the keyed one reads the Study key back.
EMPTY_STORE = '_agent_kit_secret() { :; }\n'


def _keyed_store(key: str) -> str:
    return (
        '_agent_kit_secret() { [ "$1" = MISTRAL_STUDY_API_KEY ] '
        f"&& printf '%s' \"{key}\"; }}\n"
    )


def _stub_store(home: Path, reader: str) -> None:
    env_sh = home / ".config" / "agent-kit" / "env.sh"
    env_sh.parent.mkdir(parents=True, exist_ok=True)
    env_sh.write_text(reader)


def _acp_stub(home: Path) -> None:
    path = home / ".local" / "bin" / "vibe-acp"
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(ACP_STUB)
    path.chmod(0o755)


def _launch(
    home: Path, *args: str, env_extra: dict[str, str] | None = None
) -> subprocess.CompletedProcess:
    """Runs the launcher setup installed, with a clean environment."""
    launcher = home / ".local" / "bin" / LAUNCHER
    env = {"HOME": str(home), "PATH": "/usr/bin:/bin", **(env_extra or {})}
    return subprocess.run(
        [str(launcher), *args], capture_output=True, text=True, check=False, env=env, timeout=60
    )


def _seen(report: Path) -> dict[str, str]:
    return dict(line.split("=", 1) for line in report.read_text().splitlines() if "=" in line)


def test_without_a_key_the_launcher_starts_nothing_and_says_how_to_sign_in(tmp_path: Path) -> None:
    run = setup_run(tmp_path, "Linux")
    _acp_stub(run.home)
    _stub_store(run.home, EMPTY_STORE)
    report = tmp_path / "acp-report"
    result = _launch(run.home, env_extra={"ACP_REPORT": str(report)})
    assert result.returncode == 1
    assert result.stdout == ""
    assert MESSAGE in result.stderr
    assert not report.exists(), "vibe-acp was started without the Study key"


def test_with_a_key_the_launcher_starts_vibe_acp_with_it_in_the_shared_home(tmp_path: Path) -> None:
    run = setup_run(tmp_path, "Linux")
    _acp_stub(run.home)
    _stub_store(run.home, _keyed_store(STUDY_KEY))
    report = tmp_path / "acp-report"
    result = _launch(
        run.home, "--resume", "abc",
        env_extra={
            "ACP_REPORT": str(report),
            "MEMORY_PRIVATE_TOKEN": "smt_private_1",
            "MISTRAL_API_KEY": "personal-key",
            "VIBE_HOME": f"{run.home}/.vibe-private",
        },
    )
    assert result.returncode == 0, result.stderr
    assert _seen(report) == {
        "MISTRAL_API_KEY": STUDY_KEY,
        "MISTRAL_STUDY_API_KEY": "unset",
        "MEMORY_PRIVATE_TOKEN": "unset",
        "VIBE_HOME": f"{run.home}/.vibe",
        "ARGS": "--resume abc",
    }


def test_a_key_in_the_environment_wins_over_the_store(tmp_path: Path) -> None:
    run = setup_run(tmp_path, "Linux")
    _acp_stub(run.home)
    _stub_store(run.home, _keyed_store("from-store"))
    report = tmp_path / "acp-report"
    result = _launch(
        run.home, env_extra={"ACP_REPORT": str(report), "MISTRAL_STUDY_API_KEY": "from-env"}
    )
    assert result.returncode == 0, result.stderr
    assert _seen(report)["MISTRAL_API_KEY"] == "from-env"


def test_setup_installs_the_launcher_executable_and_reinstalls_it_when_missing(tmp_path: Path) -> None:
    run = setup_run(tmp_path, "Linux")
    launcher = run.home / ".local" / "bin" / LAUNCHER
    source = (KIT_ROOT / "installer" / LAUNCHER).read_bytes()
    assert launcher.is_file()
    assert os.access(launcher, os.X_OK)
    assert launcher.read_bytes() == source
    assert f"vibe-study-acp -> {launcher}" in run.stdout
    # Deleted: a second setup run repairs it, like every other kit command.
    launcher.unlink()
    again = setup_run(tmp_path / "again", "Linux", home=run.home)
    assert launcher.is_file()
    assert os.access(again.home / ".local" / "bin" / LAUNCHER, os.X_OK)
    assert launcher.read_bytes() == source
    assert f"vibe-study-acp -> {launcher}" in again.stdout
