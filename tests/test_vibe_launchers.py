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
    assert "vibe-study --login" in result.stderr


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


# --- vibe-study --login: Vibe's own browser sign-in, kept apart from the personal login ---

# Stands in for macOS `security`: generic passwords as files. Vibe's entry is
# keyed by service and account, agent-kit's by service alone.
FAKE_SECURITY = r'''#!/usr/bin/env python3
import os, shlex, sys
store = os.environ["FAKE_KEYCHAIN"]
with open(os.path.join(store, "argv.log"), "a") as log:
    log.write(" ".join(sys.argv[1:]) + "\n")

def key(args):
    s = args[args.index("-s") + 1]
    if s == "ai.mistral.vibe":
        s += "@" + args[args.index("-a") + 1]
    return os.path.join(store, s)

def add(args):
    if os.environ.get("FAKE_SECURITY_DROP_ADD"):
        return  # `security -i` exits 0 even when a command inside it fails
    open(key(args), "w").write(args[args.index("-w") + 1])
    if "-a" in args:
        with open(os.path.join(store, "accounts.log"), "a") as log:
            log.write(args[args.index("-s") + 1] + " " + args[args.index("-a") + 1] + "\n")

if sys.argv[1:2] == ["-i"]:
    for line in sys.stdin:
        args = shlex.split(line)
        if args and args[0] == "add-generic-password":
            add(args[1:])
        if args and args[0] == "delete-generic-password" and os.path.exists(key(args[1:])):
            os.remove(key(args[1:]))
    sys.exit(0)
args = sys.argv[2:]
if sys.argv[1] == "add-generic-password":
    add(args)
    sys.exit(0)
if sys.argv[1] == "delete-generic-password":
    if not os.path.exists(key(args)):
        sys.exit(44)
    os.remove(key(args))
    sys.exit(0)
if sys.argv[1] == "find-generic-password":
    if os.environ.get("FAKE_SECURITY_FAIL_FIND") and "ai.mistral.vibe" in args:
        sys.exit(51)  # not "could not be found" (44): the read itself failed
    if not os.path.exists(key(args)):
        sys.exit(44)
    print(open(key(args)).read())
    sys.exit(0)
sys.exit(1)
'''

# Stands in for Vibe's `--setup`: the browser sign-in mints a key and saves it
# to the keychain, or to $VIBE_HOME/.env when the keychain is switched off.
# FAKE_VIBE_IGNORES_SWITCH plays a future Vibe that no longer honours it.
SIGN_IN_VIBE = """#!/usr/bin/env bash
if [ "$1" = --setup ]; then
  printf '%s' "${VIBE_HOME}" > "${FAKE_KEYCHAIN}/setup-home"
  if [ "${VIBE_TEST_DISABLE_KEYRING:-}" = 1 ] && [ -z "${FAKE_VIBE_IGNORES_SWITCH:-}" ]; then
    printf "MISTRAL_API_KEY='%s'\\n" "${FAKE_SIGN_IN_KEY}" > "${VIBE_HOME}/.env"
  else
    security add-generic-password -s ai.mistral.vibe -a MISTRAL_API_KEY -w "${FAKE_SIGN_IN_KEY}" -A
  fi
  exit 0
fi
""" + FAKE_VIBE.split("\n", 1)[1]

SIGNED_IN = "study_from_sign_in_1"
PERSONAL = "personal_login_9"


# Stands in for curl against Mistral's model list: reads the header from stdin.
FAKE_CURL = r'''#!/usr/bin/env bash
printf '%s\n' "$*" >> "${FAKE_KEYCHAIN}/curl-argv.log"
[ -n "${FAKE_CURL_OFFLINE:-}" ] && exit 7
header="$(cat)"
case "${header}" in *"Bearer refused_key"*) exit 22 ;; esac
exit 0
'''


class Keychain(Workstation):
    def __init__(self, tmp_path: Path) -> None:
        super().__init__(tmp_path)
        self.keychain = tmp_path / "keychain"
        self.keychain.mkdir()
        (self.bin / "curl").write_text(FAKE_CURL)
        (self.bin / "curl").chmod(0o755)
        (self.bin / "vibe").write_text(SIGN_IN_VIBE)
        (self.bin / "security").write_text(FAKE_SECURITY.replace("/usr/bin/env python3", sys.executable))
        (self.bin / "security").chmod(0o755)
        # The reader agent-kit-keys writes into env.sh for the keychain backend.
        env_sh = self.home / ".config" / "agent-kit" / "env.sh"
        env_sh.parent.mkdir(parents=True)
        env_sh.write_text('_agent_kit_secret() { security find-generic-password -s "agent-kit.$1" -w 2>/dev/null; }\n')
        (self.keychain / "ai.mistral.vibe@MISTRAL_API_KEY").write_text(PERSONAL)

    def login(self, **env: str) -> subprocess.CompletedProcess:
        return self.run(STUDY, "--login", FAKE_KEYCHAIN=str(self.keychain), FAKE_SIGN_IN_KEY=SIGNED_IN, **env)

    def study(self) -> subprocess.CompletedProcess:
        return self.run(STUDY, FAKE_KEYCHAIN=str(self.keychain))


def test_vibe_study_login_keeps_the_key_vibes_own_sign_in_mints_and_vibe_study_then_uses_it(
    tmp_path: Path,
) -> None:
    kc = Keychain(tmp_path)
    result = kc.login()
    assert result.returncode == 0, result.stderr
    assert (kc.keychain / "agent-kit.MISTRAL_STUDY_API_KEY").read_text() == SIGNED_IN
    assert _seen(kc.study())["MISTRAL_API_KEY"] == SIGNED_IN


def test_vibe_study_login_never_touches_the_personal_login(tmp_path: Path) -> None:
    kc = Keychain(tmp_path)
    assert kc.login().returncode == 0
    assert (kc.keychain / "ai.mistral.vibe@MISTRAL_API_KEY").read_text() == PERSONAL


def test_vibe_study_login_signs_in_in_a_throwaway_home_it_then_removes(tmp_path: Path) -> None:
    kc = Keychain(tmp_path)
    assert kc.login().returncode == 0
    setup_home = Path((kc.keychain / "setup-home").read_text())
    assert setup_home not in (kc.home / ".vibe", kc.home / ".vibe-private")
    assert not setup_home.exists()
    assert not (kc.home / ".vibe" / ".env").exists()


def test_vibe_study_login_ignores_a_personal_key_in_the_environment(tmp_path: Path) -> None:
    kc = Keychain(tmp_path)
    assert kc.login(MISTRAL_API_KEY="personal-from-env").returncode == 0
    assert (kc.keychain / "agent-kit.MISTRAL_STUDY_API_KEY").read_text() == SIGNED_IN


def test_vibe_study_login_never_puts_a_key_in_argv(tmp_path: Path) -> None:
    kc = Keychain(tmp_path)
    assert kc.login().returncode == 0
    argv = (kc.keychain / "argv.log").read_text()
    assert SIGNED_IN not in argv and PERSONAL not in argv


def test_a_vibe_that_ignores_the_keychain_switch_gets_the_personal_login_restored(tmp_path: Path) -> None:
    kc = Keychain(tmp_path)
    result = kc.login(FAKE_VIBE_IGNORES_SWITCH="1")
    assert result.returncode != 0
    assert (kc.keychain / "ai.mistral.vibe@MISTRAL_API_KEY").read_text() == PERSONAL
    assert not (kc.keychain / "agent-kit.MISTRAL_STUDY_API_KEY").exists()
    assert "personal login" in result.stderr


def test_without_a_personal_login_an_ignored_switch_leaves_none_behind(tmp_path: Path) -> None:
    kc = Keychain(tmp_path)
    (kc.keychain / "ai.mistral.vibe@MISTRAL_API_KEY").unlink()
    assert kc.login(FAKE_VIBE_IGNORES_SWITCH="1").returncode != 0
    assert not (kc.keychain / "ai.mistral.vibe@MISTRAL_API_KEY").exists()


def test_without_a_keychain_the_key_goes_to_the_private_secrets_file(tmp_path: Path) -> None:
    kc = Keychain(tmp_path)
    (kc.home / ".config" / "agent-kit" / "env.sh").write_text(
        '_agent_kit_secret() { sed -n "s/^$1=//p" "$HOME/.config/agent-kit/secrets.env" 2>/dev/null; }\n')
    secrets = kc.home / ".config" / "agent-kit" / "secrets.env"
    secrets.write_text("OTHER=kept\n")
    assert kc.login(AGENT_KIT_SECRETS_BACKEND="file").returncode == 0
    assert secrets.read_text() == f"OTHER=kept\nMISTRAL_STUDY_API_KEY={SIGNED_IN}\n"
    assert secrets.stat().st_mode & 0o777 == 0o600
    assert _seen(kc.study())["MISTRAL_API_KEY"] == SIGNED_IN


def test_signing_in_again_replaces_the_study_key(tmp_path: Path) -> None:
    kc = Keychain(tmp_path)
    assert kc.login().returncode == 0
    assert kc.run(STUDY, "--login", FAKE_KEYCHAIN=str(kc.keychain), FAKE_SIGN_IN_KEY="study_new_2").returncode == 0
    assert (kc.keychain / "agent-kit.MISTRAL_STUDY_API_KEY").read_text() == "study_new_2"


def test_a_failed_read_of_the_personal_login_stops_before_signing_in(tmp_path: Path) -> None:
    kc = Keychain(tmp_path)
    result = kc.login(FAKE_SECURITY_FAIL_FIND="1")
    assert result.returncode != 0
    assert not (kc.keychain / "setup-home").exists(), "no sign-in was started"
    assert (kc.keychain / "ai.mistral.vibe@MISTRAL_API_KEY").read_text() == PERSONAL
    assert "personal login" in result.stderr


def test_a_key_that_does_not_read_back_is_not_reported_as_stored(tmp_path: Path) -> None:
    kc = Keychain(tmp_path)
    result = kc.login(FAKE_SECURITY_DROP_ADD="1")
    assert result.returncode != 0
    assert "signed in" not in result.stderr
    assert "read back" in result.stderr


def test_login_needs_setups_env_sh_so_vibe_study_can_read_the_key_back(tmp_path: Path) -> None:
    kc = Keychain(tmp_path)
    (kc.home / ".config" / "agent-kit" / "env.sh").unlink()
    result = kc.login()
    assert result.returncode != 0
    assert not (kc.keychain / "setup-home").exists(), "no sign-in was started"
    assert "setup-workstation" in result.stderr


def test_a_key_mistral_refuses_is_not_stored(tmp_path: Path) -> None:
    kc = Keychain(tmp_path)
    result = kc.run(STUDY, "--login", FAKE_KEYCHAIN=str(kc.keychain), FAKE_SIGN_IN_KEY="refused_key")
    assert result.returncode != 0
    assert not (kc.keychain / "agent-kit.MISTRAL_STUDY_API_KEY").exists()
    assert "refused" in result.stderr


def test_offline_the_key_is_stored_unchecked(tmp_path: Path) -> None:
    kc = Keychain(tmp_path)
    result = kc.login(FAKE_CURL_OFFLINE="1")
    assert result.returncode == 0, result.stderr
    assert (kc.keychain / "agent-kit.MISTRAL_STUDY_API_KEY").read_text() == SIGNED_IN
    assert "not checked" in result.stderr


def test_the_check_never_puts_the_key_in_argv(tmp_path: Path) -> None:
    kc = Keychain(tmp_path)
    assert kc.login().returncode == 0
    assert SIGNED_IN not in (kc.keychain / "curl-argv.log").read_text()


def test_the_key_is_stored_under_the_same_account_agent_kit_keys_uses(tmp_path: Path) -> None:
    kc = Keychain(tmp_path)
    assert kc.login(LOGNAME="alice", USER="bob").returncode == 0
    assert "agent-kit.MISTRAL_STUDY_API_KEY alice" in (kc.keychain / "accounts.log").read_text()
