"""agent-kit-secret: the one secret store, read by name.

macOS Keychain through `security`, Linux Secret Service through `gdbus` (an
existence check) and Python's `keyring` (reads and writes). Only fakes stand in
for them here: no real credential is ever touched. There is no plain-file store.
"""

from __future__ import annotations

import os
import subprocess
import sys
from pathlib import Path

import pytest

KIT_ROOT = Path(__file__).resolve().parent.parent
SECRET = KIT_ROOT / "installer" / "agent-kit-secret"
VALUE = "tok_Fake-123"
NAME = "HINDSIGHT_API_TOKEN"

FAKE_SECURITY = r'''#!/usr/bin/env python3
import os, shlex, sys
store = os.environ["FAKE_KEYCHAIN"]
with open(os.path.join(store, "argv.log"), "a") as log:
    log.write(" ".join(sys.argv[1:]) + "\n")
if os.environ.get("FAKE_SECURITY_BROKEN"):
    sys.exit(51)
if sys.argv[1:2] == ["-i"]:
    for line in sys.stdin:
        args = shlex.split(line)
        if args and args[0] == "add-generic-password":
            open(os.path.join(store, args[args.index("-s") + 1]), "w").write(args[args.index("-w") + 1])
    sys.exit(int(os.environ.get("FAKE_SECURITY_I_EXIT", "0")))
path = os.path.join(store, sys.argv[sys.argv.index("-s") + 1])
if not os.path.exists(path):
    sys.exit(44)
if sys.argv[1] == "delete-generic-password":
    os.remove(path)
elif "-w" in sys.argv:
    print(open(path).read())
'''

FAKE_GDBUS = r'''#!/usr/bin/env python3
import os, sys
open(os.path.join(os.environ["FAKE_KEYRING_DIR"], "gdbus.log"), "a").write(" ".join(sys.argv[1:]) + "\n")
if os.environ.get("FAKE_GDBUS_FAIL"):
    sys.exit(1)
if os.environ.get("FAKE_GDBUS_HANG"):
    import time; time.sleep(30)
name = sys.argv[-1].split("'username': '")[1].split("'")[0]
if os.path.exists(os.path.join(os.environ["FAKE_KEYRING_DIR"], "agent-kit." + name)):
    print("(['/org/freedesktop/secrets/collection/login/3'], @ao [])")
else:
    print("(@ao [], @ao [])")
'''

# A stand-in `keyring` module on PYTHONPATH: items are files in $FAKE_KEYRING_DIR.
FAKE_KEYRING = '''
import os
class SecretService:
    pass
class PlaintextKeyring:
    pass
def get_keyring():
    return PlaintextKeyring() if os.environ.get("FAKE_KEYRING_NONE") else SecretService()
def _path(service, name):
    return os.path.join(os.environ["FAKE_KEYRING_DIR"], service + "." + name)
def set_password(service, name, value):
    if os.environ.get("FAKE_KEYRING_REFUSE"):
        raise RuntimeError("locked")
    open(_path(service, name), "w").write(value)
def get_password(service, name):
    if os.environ.get("FAKE_KEYRING_REFUSE"):
        raise RuntimeError("locked")
    p = _path(service, name)
    return open(p).read() if os.path.exists(p) else None
def delete_password(service, name):
    os.remove(_path(service, name))
'''

# `uv run --no-project --exclude-newer "7 days" --quiet --with keyring python SCRIPT ARGS...`:
# run the script with the fake module.
FAKE_UV = '''#!/bin/sh
echo "$*" >> "$FAKE_KEYRING_DIR/uv.log"
shift 8
PYTHONPATH="$FAKE_KEYRING_DIR/mod" exec ''' + sys.executable + ''' "$@"
'''


class Machine:
    def __init__(self, tmp_path: Path) -> None:
        self.bin = tmp_path / "bin"
        self.backend = ""
        self.keychain = tmp_path / "keychain"
        self.ring = tmp_path / "ring"
        (self.ring / "mod").mkdir(parents=True)
        for path in (self.bin, self.keychain):
            path.mkdir()
        (self.ring / "mod" / "keyring.py").write_text(FAKE_KEYRING)
        # The system python has no keyring module; shadow one the test environment may carry.
        (self.ring / "none").mkdir()
        (self.ring / "none" / "keyring.py").write_text("raise ImportError('no keyring here')\n")

    def tool(self, name: str, text: str) -> None:
        (self.bin / name).write_text(text.replace("/usr/bin/env python3", sys.executable))
        (self.bin / name).chmod(0o755)

    def run(self, *args: str, stdin: str = "", path: str | None = None, **env: str) -> subprocess.CompletedProcess:
        base = {"PATH": path if path is not None else str(self.bin), "HOME": str(self.bin.parent / "home"),
                "FAKE_KEYCHAIN": str(self.keychain), "FAKE_KEYRING_DIR": str(self.ring),
                "PYTHONPATH": str(self.ring / "none")}
        if self.backend:
            base["AGENT_KIT_SECRETS_BACKEND"] = self.backend
        return subprocess.run([sys.executable, str(SECRET), *args], input=stdin, capture_output=True, text=True,
                              check=False, env={**base, **env}, timeout=60)


@pytest.fixture
def mac(tmp_path: Path) -> Machine:
    machine = Machine(tmp_path)
    machine.tool("security", FAKE_SECURITY)
    machine.backend = "keychain"  # the default is by platform, and this suite may run on Linux
    return machine


@pytest.fixture
def linux(tmp_path: Path) -> Machine:
    """libsecret with a Secret Service, gdbus and the keyring module on hand."""
    machine = Machine(tmp_path)
    machine.tool("gdbus", FAKE_GDBUS)
    return machine


def _on_linux(machine: Machine, *args: str, **kw: object) -> subprocess.CompletedProcess:
    return machine.run(*args, AGENT_KIT_SECRETS_BACKEND="libsecret", PYTHONPATH=str(machine.ring / "mod"), **kw)  # type: ignore[arg-type]


# --- macOS Keychain ---


def test_the_keychain_round_trips_a_secret_by_name(mac: Machine) -> None:
    assert mac.run("put", NAME, stdin=VALUE + "\n").returncode == 0
    assert (mac.keychain / f"agent-kit.{NAME}").read_text() == VALUE
    got = mac.run("get", NAME)
    assert (got.returncode, got.stdout) == (0, VALUE + "\n")
    assert mac.run("has", NAME).returncode == 0
    assert mac.run("delete", NAME).returncode == 0
    assert mac.run("has", NAME).returncode == 1


def test_a_missing_key_exits_1_and_says_how_to_store_it(mac: Machine) -> None:
    result = mac.run("get", NAME)
    assert (result.returncode, result.stdout) == (1, "")
    assert NAME in result.stderr and "setup-workstation" in result.stderr
    assert mac.run("delete", NAME).returncode == 1


def test_the_keychain_never_gets_the_value_in_argv_and_has_never_reads_it(mac: Machine) -> None:
    mac.run("put", NAME, stdin=VALUE)
    mac.run("has", NAME)
    log = (mac.keychain / "argv.log").read_text()
    assert VALUE not in log
    assert "find-generic-password -s agent-kit.HINDSIGHT_API_TOKEN\n" in log, "has asks without -w"


def test_a_value_the_keychain_write_cannot_carry_safely_is_refused(mac: Machine) -> None:
    result = mac.run("put", NAME, stdin='bad"value')
    assert result.returncode == 2
    assert not (mac.keychain / f"agent-kit.{NAME}").exists()
    assert 'bad"value' not in result.stderr


def test_a_failing_keychain_is_unavailable_not_missing(mac: Machine) -> None:
    for command in ("get", "has"):
        assert mac.run(command, NAME, FAKE_SECURITY_BROKEN="1").returncode == 2
    assert mac.run("put", NAME, stdin=VALUE, FAKE_SECURITY_I_EXIT="1").returncode == 2


def test_the_keychain_is_required_when_asked_for_and_absent(tmp_path: Path) -> None:
    result = Machine(tmp_path).run("get", NAME, AGENT_KIT_SECRETS_BACKEND="keychain")
    assert result.returncode == 2
    assert "security" in result.stderr


def test_the_account_comes_from_the_login_name_or_a_default(mac: Machine) -> None:
    assert mac.run("put", NAME, stdin=VALUE, LOGNAME="alice").returncode == 0
    assert (mac.keychain / f"agent-kit.{NAME}").exists()
    (mac.keychain / f"agent-kit.{NAME}").unlink()
    assert mac.run("put", NAME, stdin=VALUE, LOGNAME="", USER="").returncode == 0, "no name still stores"


# --- Linux Secret Service ---


def test_has_asks_the_secret_service_over_dbus_and_reads_no_value(linux: Machine) -> None:
    assert _on_linux(linux, "has", NAME).returncode == 1
    (linux.ring / f"agent-kit.{NAME}").write_text(VALUE)
    assert _on_linux(linux, "has", NAME).returncode == 0
    log = (linux.ring / "gdbus.log").read_text()
    assert "org.freedesktop.Secret.Service.SearchItems" in log
    assert "GetSecret" not in log and VALUE not in log


def test_has_without_gdbus_or_a_secret_service_is_unavailable(linux: Machine) -> None:
    assert _on_linux(linux, "has", NAME, FAKE_GDBUS_FAIL="1").returncode == 2
    assert _on_linux(linux, "has", NAME, path=str(linux.bin / "nowhere")).returncode == 2
    assert _on_linux(linux, "has", NAME, FAKE_GDBUS_HANG="1").returncode == 2


def test_libsecret_round_trips_a_secret_through_keyring(linux: Machine) -> None:
    assert _on_linux(linux, "put", NAME, stdin=VALUE + "\n").returncode == 0
    assert (linux.ring / f"agent-kit.{NAME}").read_text() == VALUE
    got = _on_linux(linux, "get", NAME)
    assert (got.returncode, got.stdout) == (0, VALUE + "\n")
    assert _on_linux(linux, "delete", NAME).returncode == 0
    assert _on_linux(linux, "get", NAME).returncode == 1
    assert _on_linux(linux, "delete", NAME).returncode == 1


def test_a_missing_keyring_module_is_supplied_by_uv_and_the_value_still_round_trips(linux: Machine) -> None:
    linux.tool("uv", FAKE_UV)
    plain = linux.run("put", NAME, stdin=VALUE, AGENT_KIT_SECRETS_BACKEND="libsecret")
    assert plain.returncode == 0, plain.stderr
    assert "--with keyring" in (linux.ring / "uv.log").read_text()
    got = linux.run("get", NAME, AGENT_KIT_SECRETS_BACKEND="libsecret")
    assert (got.returncode, got.stdout) == (0, VALUE + "\n")
    assert VALUE not in (linux.ring / "uv.log").read_text(), "the value never reaches uv's argv"


def test_without_keyring_or_uv_libsecret_fails_closed(linux: Machine) -> None:
    result = linux.run("get", NAME, AGENT_KIT_SECRETS_BACKEND="libsecret")
    assert result.returncode == 2
    assert "uv" in result.stderr
    assert list(linux.ring.glob("agent-kit.*")) == []


def test_the_uv_run_does_not_recurse_when_keyring_is_still_missing(linux: Machine) -> None:
    linux.tool("uv", "#!/bin/sh\nexec " + sys.executable + " \"$7\" \"$8\" \"$9\"\n")  # keyring not supplied
    result = linux.run("get", NAME, AGENT_KIT_SECRETS_BACKEND="libsecret")
    assert result.returncode == 2


def test_a_keyring_that_is_not_the_secret_service_fails_closed(linux: Machine) -> None:
    result = _on_linux(linux, "put", NAME, stdin=VALUE, FAKE_KEYRING_NONE="1")
    assert result.returncode == 2
    assert not (linux.ring / f"agent-kit.{NAME}").exists()


def test_a_locked_secret_service_is_unavailable_and_names_no_value(linux: Machine) -> None:
    (linux.ring / f"agent-kit.{NAME}").write_text(VALUE)
    for command in ("get", "put"):
        result = _on_linux(linux, command, NAME, stdin=VALUE, FAKE_KEYRING_REFUSE="1")
        assert result.returncode == 2
        assert VALUE not in result.stdout + result.stderr


# --- no file store, no wrong use ---


def test_there_is_no_plain_file_backend(linux: Machine, tmp_path: Path) -> None:
    secrets = tmp_path / "home" / ".config" / "agent-kit" / "secrets.env"
    secrets.parent.mkdir(parents=True)
    secrets.write_text(f"{NAME}={VALUE}\n")
    result = linux.run("get", NAME, AGENT_KIT_SECRETS_BACKEND="file")
    assert (result.returncode, result.stdout) == (2, "")
    assert "plain-file" in result.stderr
    fallback = _on_linux(linux, "get", NAME)
    assert fallback.stdout != VALUE + "\n", "a secrets.env is never read"


@pytest.mark.parametrize("argv", [(), ("get",), ("peek", NAME), ("get", "bad name"), ("get", NAME, "extra")])
def test_bad_usage_is_refused(mac: Machine, argv: tuple[str, ...]) -> None:
    result = mac.run(*argv)
    assert result.returncode == 2
    assert "usage" in result.stderr


def test_put_with_nothing_on_stdin_stores_nothing(mac: Machine) -> None:
    assert mac.run("put", NAME, stdin="\n").returncode == 2
    assert not (mac.keychain / f"agent-kit.{NAME}").exists()


def test_the_helper_is_executable_and_carries_the_marker_setup_checks() -> None:
    assert SECRET.stat().st_mode & 0o111
    assert "# managed by agent-kit setup-workstation.sh" in SECRET.read_text().splitlines()
    assert os.access(SECRET, os.X_OK)
