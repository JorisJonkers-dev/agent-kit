"""The Vibe launchers: vibe-study and vibe-private.

vibe and vibe-study share one Vibe home and Shared Memory; vibe-study signs in
with the Study account key instead of the personal login. vibe-private has its
own Vibe home and Private Memory, and only after the owner answers the
passphrase gate at a terminal.
"""

from __future__ import annotations

import os
import shutil
import subprocess
import sys
from pathlib import Path

KIT_ROOT = Path(__file__).resolve().parent.parent
STUDY = KIT_ROOT / "installer" / "vibe-study"
PRIVATE = KIT_ROOT / "installer" / "vibe-private"
GATE = KIT_ROOT / "installer" / "agent-kit-private"
TOKEN = "smt_private_Ab-1"
STUDY_KEY = "study-key-123"
PASSPHRASE = "correct horse"

# Stands in for Vibe: reports the home, login and token it was started with.
FAKE_VIBE = """#!/usr/bin/env bash
printf 'VIBE_HOME=%s\\n' "${VIBE_HOME:-unset}"
printf 'MISTRAL_API_KEY=%s\\n' "${MISTRAL_API_KEY:-unset}"
printf 'MEMORY_PRIVATE_TOKEN=%s\\n' "${MEMORY_PRIVATE_TOKEN:-unset}"
printf 'ARGS=%s\\n' "$*"
"""


class Workstation:
    def __init__(self, tmp_path: Path) -> None:
        self.tmp = tmp_path
        self.home = tmp_path / "home"
        self.bin = tmp_path / "bin"
        self.home.mkdir()
        self.bin.mkdir()
        vibe = self.bin / "vibe"
        vibe.write_text(FAKE_VIBE)
        vibe.chmod(0o755)
        # The real passphrase gate, installed the way setup installs it.
        shutil.copy(GATE, self.bin / "agent-kit-private")
        (self.bin / "agent-kit-private").chmod(0o755)

    def store_study_key(self, value: str = STUDY_KEY) -> None:
        """What agent-kit-keys' env.sh gives the shell: a reader for the secret store."""
        env_sh = self.home / ".config" / "agent-kit" / "env.sh"
        env_sh.parent.mkdir(parents=True, exist_ok=True)
        env_sh.write_text(
            f"_agent_kit_secret() {{ [ \"$1\" = MISTRAL_STUDY_API_KEY ] && printf '%s' '{value}'; }}\n"
            # An env.sh from before the split defined this; the launchers must not run it.
            "vibe() { echo 'old vibe function ran'; }\n"
            # The rest of env.sh: one keychain lookup per exported secret.
            f"touch '{self.tmp}/env-sh-ran'\n",
        )

    def seal_token(self) -> None:
        answers = self.tmp / "store-tty"
        answers.write_text(f"{PASSPHRASE}\n{PASSPHRASE}\n")
        result = subprocess.run(
            [sys.executable, str(GATE), "store"], input=TOKEN, capture_output=True, text=True, check=False,
            env={**self._env(), "AGENT_KIT_TTY": str(answers)}, timeout=60,
        )
        assert result.returncode == 0, result.stderr

    def _env(self, **extra: str) -> dict[str, str]:
        env = {k: v for k, v in os.environ.items()
               if k not in ("MEMORY_PRIVATE_TOKEN", "MISTRAL_API_KEY", "VIBE_HOME", "MISTRAL_STUDY_API_KEY")}
        env.update({"HOME": str(self.home), "PATH": f"{self.bin}:/usr/bin:/bin"}, **extra)
        return env

    def run(self, launcher: Path, *args: str, tty: str | None = None, **env: str) -> subprocess.CompletedProcess:
        run_env = self._env(**env)
        if tty is None:
            run_env["AGENT_KIT_TTY"] = str(self.tmp / "no-terminal")
        else:
            answers = self.tmp / "tty"
            answers.write_text(tty)
            run_env["AGENT_KIT_TTY"] = str(answers)
        return subprocess.run(["bash", str(launcher), *args], capture_output=True, text=True, check=False,
                              env=run_env, timeout=60, start_new_session=True)


def _seen(result: subprocess.CompletedProcess) -> dict[str, str]:
    return dict(line.split("=", 1) for line in result.stdout.splitlines() if "=" in line)


# --- vibe-study ---


def test_vibe_study_runs_vibe_in_the_shared_home_signed_in_with_the_study_key(tmp_path: Path) -> None:
    ws = Workstation(tmp_path)
    ws.store_study_key()
    result = ws.run(STUDY, "--resume", "abc")
    assert result.returncode == 0, result.stderr
    assert _seen(result) == {
        "VIBE_HOME": f"{ws.home}/.vibe",
        "MISTRAL_API_KEY": STUDY_KEY,
        "MEMORY_PRIVATE_TOKEN": "unset",
        "ARGS": "--resume abc",
    }


def test_vibe_study_drops_an_inherited_private_memory_token_and_private_home(tmp_path: Path) -> None:
    ws = Workstation(tmp_path)
    ws.store_study_key()
    result = ws.run(STUDY, MEMORY_PRIVATE_TOKEN=TOKEN, VIBE_HOME=f"{ws.home}/.vibe-private")
    assert result.returncode == 0, result.stderr
    assert _seen(result)["MEMORY_PRIVATE_TOKEN"] == "unset"
    assert _seen(result)["VIBE_HOME"] == f"{ws.home}/.vibe"


def test_vibe_study_overrides_an_inherited_personal_key(tmp_path: Path) -> None:
    ws = Workstation(tmp_path)
    ws.store_study_key()
    result = ws.run(STUDY, MISTRAL_API_KEY="personal-key")
    assert _seen(result)["MISTRAL_API_KEY"] == STUDY_KEY


def test_vibe_study_prefers_a_study_key_already_in_the_environment(tmp_path: Path) -> None:
    ws = Workstation(tmp_path)
    ws.store_study_key()
    result = ws.run(STUDY, MISTRAL_STUDY_API_KEY="from-env")
    assert _seen(result)["MISTRAL_API_KEY"] == "from-env"


def test_vibe_study_never_runs_an_old_vibe_shell_function(tmp_path: Path) -> None:
    ws = Workstation(tmp_path)
    ws.store_study_key()
    result = ws.run(STUDY)
    assert "old vibe function ran" not in result.stdout


def test_vibe_study_reads_only_the_study_key_not_the_rest_of_env_sh(tmp_path: Path) -> None:
    ws = Workstation(tmp_path)
    ws.store_study_key()
    assert ws.run(STUDY).returncode == 0
    assert not (tmp_path / "env-sh-ran").exists()


def test_vibe_study_refuses_without_the_study_key_and_never_falls_back_to_the_personal_login(
    tmp_path: Path,
) -> None:
    ws = Workstation(tmp_path)
    result = ws.run(STUDY, MISTRAL_API_KEY="personal-key")
    assert result.returncode != 0
    assert result.stdout == ""
    assert "MISTRAL_STUDY_API_KEY" in result.stderr
    assert "setup-workstation" in result.stderr


def test_vibe_study_needs_no_terminal_so_agents_can_run_it(tmp_path: Path) -> None:
    ws = Workstation(tmp_path)
    ws.store_study_key()
    result = ws.run(STUDY, "-p", "hello", tty=None)
    assert result.returncode == 0, result.stderr
    assert _seen(result)["ARGS"] == "-p hello"


# --- vibe-private ---


def test_vibe_private_runs_vibe_in_its_own_home_with_private_memory_after_the_right_passphrase(
    tmp_path: Path,
) -> None:
    ws = Workstation(tmp_path)
    ws.seal_token()
    result = ws.run(PRIVATE, "--resume", tty=f"{PASSPHRASE}\n")
    assert result.returncode == 0, result.stderr
    assert _seen(result) == {
        "VIBE_HOME": f"{ws.home}/.vibe-private",
        "MISTRAL_API_KEY": "unset",
        "MEMORY_PRIVATE_TOKEN": TOKEN,
        "ARGS": "--resume",
    }


def test_vibe_private_launches_nothing_on_a_wrong_passphrase(tmp_path: Path) -> None:
    ws = Workstation(tmp_path)
    ws.seal_token()
    result = ws.run(PRIVATE, tty="wrong horse\n")
    assert result.returncode != 0
    assert result.stdout == ""
    assert "passphrase" in result.stderr


def test_vibe_private_launches_nothing_without_a_terminal_so_no_agent_can_answer(tmp_path: Path) -> None:
    ws = Workstation(tmp_path)
    ws.seal_token()
    result = ws.run(PRIVATE, tty=None)
    assert result.returncode != 0
    assert result.stdout == ""
    assert "terminal" in result.stderr


def test_vibe_private_never_reads_the_passphrase_from_stdin(tmp_path: Path) -> None:
    ws = Workstation(tmp_path)
    ws.seal_token()
    run_env = ws._env(AGENT_KIT_TTY=str(tmp_path / "no-terminal"))
    result = subprocess.run(["bash", str(PRIVATE)], input=f"{PASSPHRASE}\n", capture_output=True, text=True,
                            check=False, env=run_env, timeout=60, start_new_session=True)
    assert result.returncode != 0
    assert result.stdout == ""


def test_vibe_private_signs_in_with_the_personal_login_even_from_a_study_session(tmp_path: Path) -> None:
    ws = Workstation(tmp_path)
    ws.seal_token()
    result = ws.run(PRIVATE, tty=f"{PASSPHRASE}\n", MISTRAL_API_KEY=STUDY_KEY)
    assert result.returncode == 0, result.stderr
    assert _seen(result)["MISTRAL_API_KEY"] == "unset"


def test_vibe_private_without_a_sealed_token_says_how_to_get_one(tmp_path: Path) -> None:
    ws = Workstation(tmp_path)
    result = ws.run(PRIVATE, tty=f"{PASSPHRASE}\n")
    assert result.returncode != 0
    assert result.stdout == ""
    assert "setup-workstation" in result.stderr
