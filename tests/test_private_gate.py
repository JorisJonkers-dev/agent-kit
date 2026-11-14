"""agent-kit-private: the private-bank token, behind a passphrase only you know.

The token is stored encrypted (AES-256-CBC, PBKDF2 600k) and decrypted only
for a launch you type the passphrase for, at a terminal. Claude and Codex can
read the file but get ciphertext; without a terminal they get no prompt at all.
"""

from __future__ import annotations

import os
import subprocess
import sys
from pathlib import Path

import pytest

KIT_ROOT = Path(__file__).resolve().parent.parent
GATE = KIT_ROOT / "installer" / "agent-kit-private"
TOKEN = "smt_private_Ab-1"
SHOW = ["sh", "-c", 'printf "%s" "${MEMORY_PRIVATE_TOKEN:-unset}"']


def _gate(tmp_path: Path, *args: str, tty: str | None = "", stdin: str = "",
          env: dict | None = None) -> subprocess.CompletedProcess:
    home = tmp_path / "home"
    home.mkdir(exist_ok=True)
    run_env = {k: v for k, v in os.environ.items() if k != "MEMORY_PRIVATE_TOKEN"}
    run_env["HOME"] = str(home)
    if tty is None:
        run_env["AGENT_KIT_TTY"] = str(tmp_path / "no-terminal")
    else:
        answers = tmp_path / "tty"
        answers.write_text(tty)
        run_env["AGENT_KIT_TTY"] = str(answers)
    run_env.update(env or {})
    return subprocess.run([sys.executable, str(GATE), *args], input=stdin, capture_output=True, text=True,
                          check=False, env=run_env, timeout=60, start_new_session=True)


def _sealed(tmp_path: Path) -> Path:
    return tmp_path / "home" / ".config" / "agent-kit" / "private" / "MEMORY_PRIVATE_TOKEN.enc"


def _store(tmp_path: Path, token: str = TOKEN, tty: str = "correct horse\ncorrect horse\n") -> None:
    result = _gate(tmp_path, "store", tty=tty, stdin=token)
    assert result.returncode == 0, result.stderr


def test_a_stored_token_is_sealed_and_only_the_owner_can_open_the_folder(tmp_path: Path) -> None:
    _store(tmp_path)
    sealed = _sealed(tmp_path)
    assert TOKEN.encode() not in sealed.read_bytes()
    assert sealed.read_bytes().startswith(b"Salted__")
    assert sealed.stat().st_mode & 0o777 == 0o600
    assert sealed.parent.stat().st_mode & 0o777 == 0o700


def test_the_right_passphrase_launches_with_the_token(tmp_path: Path) -> None:
    _store(tmp_path)
    result = _gate(tmp_path, "exec", *SHOW, tty="correct horse\n")
    assert result.returncode == 0, result.stderr
    assert result.stdout == TOKEN


def test_a_wrong_passphrase_launches_nothing(tmp_path: Path) -> None:
    _store(tmp_path)
    result = _gate(tmp_path, "exec", *SHOW, tty="wrong horse\n")
    assert result.returncode != 0
    assert result.stdout == ""
    assert "passphrase" in result.stderr


def test_without_a_terminal_there_is_no_prompt_and_no_token(tmp_path: Path) -> None:
    _store(tmp_path)
    result = _gate(tmp_path, "exec", *SHOW, tty=None)
    assert result.returncode != 0
    assert result.stdout == ""
    assert "terminal" in result.stderr


def test_a_token_already_in_the_environment_needs_no_passphrase(tmp_path: Path) -> None:
    result = _gate(tmp_path, "exec", *SHOW, tty=None, env={"MEMORY_PRIVATE_TOKEN": "smt_given"})
    assert result.stdout == "smt_given"


def test_a_new_passphrase_must_be_typed_twice_and_be_long_enough(tmp_path: Path) -> None:
    assert _gate(tmp_path, "store", tty="correct horse\ncorrect h0rse\n", stdin=TOKEN).returncode != 0
    assert _gate(tmp_path, "store", tty="short\nshort\n", stdin=TOKEN).returncode != 0
    assert not _sealed(tmp_path).exists()


def test_replacing_the_token_needs_the_current_passphrase(tmp_path: Path) -> None:
    _store(tmp_path)
    refused = _gate(tmp_path, "store", tty="wrong horse\n", stdin="smt_new")
    assert refused.returncode != 0
    _store(tmp_path, token="smt_new", tty="correct horse\n")
    assert _gate(tmp_path, "exec", *SHOW, tty="correct horse\n").stdout == "smt_new"


def test_neither_the_passphrase_nor_the_token_reaches_argv(tmp_path: Path) -> None:
    bin_dir = tmp_path / "bin"
    bin_dir.mkdir()
    log = tmp_path / "argv.log"
    real = subprocess.run(["sh", "-c", "command -v openssl"], capture_output=True, text=True).stdout.strip()
    (bin_dir / "openssl").write_text(f'#!/bin/sh\nprintf "%s\\n" "$*" >> {log}\nexec {real} "$@"\n')
    (bin_dir / "openssl").chmod(0o755)
    env = {"PATH": f"{bin_dir}:{os.environ['PATH']}"}
    assert _gate(tmp_path, "store", tty="correct horse\ncorrect horse\n", stdin=TOKEN, env=env).returncode == 0
    assert _gate(tmp_path, "exec", *SHOW, tty="correct horse\n", env=env).stdout == TOKEN
    argv = log.read_text()
    assert "pbkdf2" in argv
    assert "correct horse" not in argv and TOKEN not in argv


def test_status_says_whether_a_token_is_stored_without_asking(tmp_path: Path) -> None:
    assert _gate(tmp_path, "status", tty=None).stdout.strip() == "missing"
    _store(tmp_path)
    assert _gate(tmp_path, "status", tty=None).stdout.strip() == "present"


@pytest.mark.parametrize("line", ["# managed by agent-kit setup-workstation.sh"])
def test_the_gate_installs_like_the_other_kit_commands(line: str) -> None:
    assert line in GATE.read_text().splitlines()


# Opens the terminal named in argv[1] as the controlling terminal of the new session
# subprocess started it in, puts it on stdin, stdout and stderr, and execs the rest of argv.
_ON_TERMINAL = (
    "import os, sys\n"
    "fd = os.open(sys.argv[1], os.O_RDWR)\n"
    "for n in (0, 1, 2):\n"
    "    os.dup2(fd, n)\n"
    "os.execv(sys.argv[2], sys.argv[2:])\n"
)


def test_a_real_terminal_is_asked_with_the_passphrase_hidden(tmp_path: Path) -> None:
    import pty
    import select
    import termios
    import time

    _store(tmp_path)
    env = {k: v for k, v in os.environ.items() if k not in ("MEMORY_PRIVATE_TOKEN", "AGENT_KIT_TTY")}
    env["HOME"] = str(tmp_path / "home")
    # Not pty.fork(): a pytest-xdist worker runs other threads, and forking a threaded
    # process can deadlock the child before it execs. subprocess starts it without that risk.
    fd, terminal = pty.openpty()
    child = subprocess.Popen(
        [sys.executable, "-c", _ON_TERMINAL, os.ttyname(terminal), sys.executable, str(GATE), "exec", *SHOW],
        env=env, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
        start_new_session=True,
    )
    out = b""

    def pump(marker: bytes) -> None:
        nonlocal out
        end = time.monotonic() + 15
        while marker not in out and (left := end - time.monotonic()) > 0:
            if not select.select([fd], [], [], left)[0]:
                return
            try:
                chunk = os.read(fd, 1024)
            except OSError:
                return
            if not chunk:
                return
            out += chunk

    try:
        pump(b"Passphrase")
        assert b"Passphrase for the private bank" in out, out
        # The gate prints the prompt, then turns echo off with TCSAFLUSH, which drops
        # input typed before it. Type only once echo is off, as a person would after the prompt.
        end = time.monotonic() + 15
        while termios.tcgetattr(terminal)[3] & termios.ECHO and time.monotonic() < end:
            time.sleep(0.01)
        assert not termios.tcgetattr(terminal)[3] & termios.ECHO, "the gate never turned echo off"
        os.write(fd, b"correct horse\n")
        pump(TOKEN.encode())
        assert child.wait(timeout=15) == 0, out
    finally:
        if child.poll() is None:
            child.kill()
            child.wait()
        os.close(terminal)
        os.close(fd)
    assert TOKEN.encode() in out
    assert b"correct horse" not in out, "the passphrase was echoed"
