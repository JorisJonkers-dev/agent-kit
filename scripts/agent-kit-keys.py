#!/usr/bin/env python3
"""Get the workstation's tokens and keys, store them, and wire them into the shell.

setup-workstation.sh runs this. ``ensure`` asks only for what is missing or no
longer works: it signs in to auth-api once (password, then TOTP), grants the
missing services when you are an admin, mints each service token, checks every
value against its own host, and stores it in the macOS keychain (a 0600 file
where there is none). ``write-env`` renders ~/.config/agent-kit/env.sh, which
reads them back in every shell; the private-bank token is handed only to
``vibe`` and ``memory-private``. ``status`` reports without asking anything.

Secrets never reach argv or output: the keychain gets them over ``security -i``
on stdin. Stdlib only, so the system python3 runs it.

Usage::

    python3 scripts/agent-kit-keys.py --table <workstation-secrets.json> ensure|status|write-env
"""

from __future__ import annotations

import argparse
import contextlib
import getpass
import http.cookiejar
import json
import os
import re
import shutil
import socket
import ssl
import subprocess
import sys
import urllib.error
import urllib.request
from pathlib import Path
from typing import Any

AUTH = os.environ.get("AGENT_KIT_AUTH_URL", "https://auth.jorisjonkers.dev").rstrip("/") + "/api/v1"
# Cloudflare refuses the default Python-urllib agent.
AGENT = "agent-kit-keys/1"
PREFIX = "agent-kit."
SAFE = re.compile(r"^[A-Za-z0-9._~+/=-]+$")
MCP_INIT = json.dumps({
    "jsonrpc": "2.0", "id": 1, "method": "initialize",
    "params": {
        "protocolVersion": "2025-03-26", "capabilities": {},
        "clientInfo": {"name": "agent-kit", "version": "1"},
    },
}).encode()


# The python.org macOS python3 trusts no CA until its "Install Certificates"
# step runs; fall back to the system bundle then.
SYSTEM_CA = ("/etc/ssl/cert.pem", "/etc/ssl/certs/ca-certificates.crt", "/etc/pki/tls/certs/ca-bundle.crt")


def tls_context() -> ssl.SSLContext:
    context = ssl.create_default_context()
    if not context.cert_store_stats()["x509_ca"]:
        for path in SYSTEM_CA:
            if os.path.isfile(path):
                context.load_verify_locations(cafile=path)
                break
    return context


class Unreachable(Exception):
    """auth-api did not answer at all."""


def say(message: str) -> None:
    print(f"keys: {message}", file=sys.stderr)


def ask(prompt: str, secret: bool = False) -> str:
    """One answer from the terminal; EOF counts as no answer."""
    if sys.stdin.isatty():
        try:
            return (getpass.getpass(prompt) if secret else input(prompt)).strip()
        except EOFError:
            return ""
    print(prompt, end="", file=sys.stderr, flush=True)
    return sys.stdin.readline().strip()


def config_dir() -> Path:
    return Path.home() / ".config" / "agent-kit"


# --- storage ---------------------------------------------------------------


class Keychain:
    """macOS login keychain, one generic password per variable."""

    name = "keychain"

    def get(self, var: str) -> str:
        out = subprocess.run(
            ["security", "find-generic-password", "-s", PREFIX + var, "-w"],
            capture_output=True, text=True, check=False,
        )
        return out.stdout.strip() if out.returncode == 0 else ""

    def put(self, var: str, value: str) -> None:
        command = f'add-generic-password -U -a "{getpass.getuser()}" -s "{PREFIX + var}" -w "{value}"\n'
        subprocess.run(["security", "-i"], input=command, capture_output=True, text=True, check=True)
        if self.get(var) != value:
            raise RuntimeError(f"{var} did not read back from the keychain")

    def delete(self, var: str) -> None:
        subprocess.run(["security", "delete-generic-password", "-s", PREFIX + var],
                       capture_output=True, check=False)

    def shell_reader(self) -> str:
        return f'_agent_kit_secret() {{ security find-generic-password -s "{PREFIX}$1" -w 2>/dev/null; }}'


class SecretsFile:
    """~/.config/agent-kit/secrets.env, mode 0600, where there is no keychain."""

    name = "secrets file"

    def __init__(self) -> None:
        self.path = config_dir() / "secrets.env"

    def _read(self) -> dict[str, str]:
        if not self.path.is_file():
            return {}
        return dict(line.split("=", 1) for line in self.path.read_text().splitlines() if "=" in line)

    def get(self, var: str) -> str:
        return self._read().get(var, "")

    def put(self, var: str, value: str) -> None:
        values = {**self._read(), var: value}
        self.path.parent.mkdir(parents=True, exist_ok=True)
        fd = os.open(self.path, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
        with os.fdopen(fd, "w") as f:
            f.write("".join(f"{k}={v}\n" for k, v in values.items()))
        os.chmod(self.path, 0o600)

    def delete(self, var: str) -> None:
        values = self._read()
        if values.pop(var, None) is not None:
            self.path.write_text("".join(f"{k}={v}\n" for k, v in values.items()))

    def shell_reader(self) -> str:
        return '_agent_kit_secret() { sed -n "s/^$1=//p" "$HOME/.config/agent-kit/secrets.env" 2>/dev/null; }'


def store() -> Keychain | SecretsFile:
    backend = os.environ.get("AGENT_KIT_SECRETS_BACKEND") or ("keychain" if shutil.which("security") else "file")
    return Keychain() if backend == "keychain" else SecretsFile()


# --- checks ----------------------------------------------------------------


class _NoRedirect(urllib.request.HTTPRedirectHandler):
    # Forward-auth sends a bad token to the login page, which answers 200.
    def redirect_request(self, *args: object, **kwargs: object) -> None:
        return None


_CHECK = urllib.request.build_opener(_NoRedirect, urllib.request.HTTPSHandler(context=tls_context()))


def check(entry: dict[str, Any], value: str) -> str:
    """ok, invalid, or unchecked (the host did not answer)."""
    get = entry.get("check_method") == "GET"
    request = urllib.request.Request(
        entry["check"], data=None if get else MCP_INIT, method="GET" if get else "POST",
        headers={
            "Authorization": f"Bearer {value}", "User-Agent": AGENT,
            "Content-Type": "application/json", "Accept": "application/json, text/event-stream",
        },
    )
    try:
        with _CHECK.open(request, timeout=20) as response:
            return "ok" if response.status == 200 else "invalid"
    except urllib.error.HTTPError:
        return "invalid"
    except (urllib.error.URLError, OSError):
        return "unchecked"


def secrets(table: list[dict[str, Any]]) -> list[dict[str, Any]]:
    return [e for e in table if "value" not in e]


# A gated value lives behind a passphrase only you know (installer/agent-kit-private).
GATE = Path(__file__).resolve().parent.parent / "installer" / "agent-kit-private"


def gated(entry: dict[str, Any]) -> bool:
    return entry.get("gate") == "passphrase"


def gate_store(value: str) -> bool:
    """Seal a value; the gate asks for the passphrase at the terminal itself."""
    return subprocess.run([sys.executable, str(GATE), "store"], input=value, text=True, check=False).returncode == 0


def state(entry: dict[str, Any], keys: Keychain | SecretsFile) -> str:
    if gated(entry):
        out = subprocess.run([sys.executable, str(GATE), "status"], capture_output=True, text=True, check=False)
        return "ok" if out.stdout.strip() == "present" else "missing"
    value = keys.get(entry["env"])
    return check(entry, value) if value else "missing"


# --- auth-api --------------------------------------------------------------


def _json(raw: bytes) -> dict[str, Any]:
    """A JSON object, or nothing: sign-out, for one, ends on an HTML page."""
    try:
        value = json.loads(raw) if raw else {}
    except ValueError:
        return {}
    return value if isinstance(value, dict) else {}


class Auth:
    def __init__(self) -> None:
        self.jar = http.cookiejar.CookieJar()
        self.opener = urllib.request.build_opener(
            urllib.request.HTTPCookieProcessor(self.jar), urllib.request.HTTPSHandler(context=tls_context()),
        )

    def call(self, method: str, path: str, body: dict[str, Any] | None = None) -> tuple[int, dict[str, Any]]:
        headers = {"User-Agent": AGENT, "Content-Type": "application/json"}
        xsrf = next((c.value for c in self.jar if c.name == "XSRF-TOKEN"), None)
        if xsrf:
            headers["X-XSRF-TOKEN"] = xsrf
        data = json.dumps(body).encode() if body is not None else None
        request = urllib.request.Request(AUTH + path, data=data, method=method, headers=headers)
        try:
            with self.opener.open(request, timeout=20) as response:
                return response.status, _json(response.read())
        except urllib.error.HTTPError as e:
            return e.code, _json(e.read())
        except (urllib.error.URLError, OSError) as e:
            raise Unreachable(str(getattr(e, "reason", e))) from None

    def sign_in(self) -> dict[str, Any] | None:
        default = subprocess.run(["git", "config", "user.email"], capture_output=True, text=True,
                                 check=False).stdout.strip()
        user = ask(f"auth.jorisjonkers.dev username or email{f' [{default}]' if default else ''}: ") or default
        password = ask("password: ", secret=True)
        body = {"username": user, "password": password}
        code, reply = self.call("POST", "/auth/session-login", body)
        if code == 200 and reply.get("totpRequired"):
            body["totpCode"] = ask("TOTP code: ")
            code, reply = self.call("POST", "/auth/session-login", body)
        if code != 200 or not reply.get("success"):
            say(f"sign-in refused: {reply.get('detail') or f'HTTP {code}'}")
            return None
        code, me = self.call("GET", "/auth/me")
        if code != 200:
            say(f"signed in, but /auth/me answered HTTP {code}")
            return None
        return me

    def ensure_grants(self, me: dict[str, Any], wanted: set[str]) -> set[str]:
        """The grants held after granting what is missing; only an admin can grant."""
        if me.get("role") != "ADMIN":
            return wanted  # minting tells us what is missing, with auth-api's own reason
        code, user = self.call("GET", f"/admin/users/{me['id']}")
        held = set(user.get("servicePermissions") or []) if code == 200 else set()
        missing = wanted - held
        if missing:
            code, user = self.call("PUT", f"/admin/users/{me['id']}/services",
                                   {"services": sorted(held | missing)})
            held = set(user.get("servicePermissions") or []) if code == 200 else held
            granted = missing & held
            if granted:
                say(f"granted yourself {', '.join(sorted(granted))}")
        return held

    def mint(self, grant: str) -> tuple[str, str]:
        label = f"{socket.gethostname().split('.')[0]} agent-kit"
        code, reply = self.call("POST", "/auth/service-tokens", {"service": grant, "label": label})
        if code not in (200, 201) or not reply.get("token"):
            return "", str(reply.get("detail") or f"HTTP {code}")
        return reply["token"], ""

    def sign_out(self) -> None:
        self.call("GET", "/auth/logout")


# --- env.sh ----------------------------------------------------------------

SOURCE_LINE = '[ -f "$HOME/.config/agent-kit/env.sh" ] && . "$HOME/.config/agent-kit/env.sh"  # agent-kit'


def write_env(table: list[dict[str, Any]], keys: Keychain | SecretsFile) -> Path:
    lines = [
        "# managed by agent-kit setup-workstation.sh; rewritten on every run.",
        f"# Reads the workstation's tokens and keys from the {keys.name}. A value already",
        "# in the environment wins. The private-bank token is never read here: vibe",
        "# asks agent-kit-private for it, which asks you for its passphrase.",
        keys.shell_reader(),
    ]
    scoped = []
    for entry in table:
        var = entry["env"]
        if "value" in entry:
            lines.append(f'[ -n "${{{var}:-}}" ] || export {var}="{entry["value"]}"')
        elif entry.get("export", True):
            lines.append(
                f'if [ -z "${{{var}:-}}" ]; then _agent_kit_v="$(_agent_kit_secret {var})"; '
                f'if [ -n "$_agent_kit_v" ]; then export {var}="$_agent_kit_v"; fi; fi'
            )
        else:
            scoped.append(var)
    lines.append("unset _agent_kit_v")
    if scoped:
        lines.append('vibe() { command agent-kit-private exec vibe "$@"; }')
    path = config_dir() / "env.sh"
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text("\n".join(lines) + "\n")
    return path


def shell_rc() -> Path | None:
    shell = Path(os.environ.get("SHELL", "")).name
    if shell == "zsh":
        return Path(os.environ.get("ZDOTDIR") or Path.home()) / ".zshrc"
    if shell == "bash":
        return Path.home() / ".bashrc"
    return None


def wire_shell() -> None:
    rc = shell_rc()
    if rc and rc.is_file() and ".config/agent-kit/env.sh" in rc.read_text():
        return
    if rc is None:
        say(f"add this line to your shell's startup file: {SOURCE_LINE}")
        return
    answer = ask(f"Load the tokens in every new shell by adding one line to {rc}? [Y/n] ").lower()
    if answer in ("", "y", "yes"):
        with rc.open("a") as f:
            f.write(f"\n{SOURCE_LINE}\n")
        say(f"added the agent-kit line to {rc}; open a new shell to pick it up")
    else:
        say(f"left {rc} alone; to load the tokens, add: {SOURCE_LINE}")


# --- commands --------------------------------------------------------------


def ensure(table: list[dict[str, Any]]) -> int:
    keys = store()
    for entry in secrets(table):
        # A copy outside the gate was readable without the passphrase: drop it and mint anew.
        if gated(entry) and keys.get(entry["env"]):
            keys.delete(entry["env"])
            say(f"{entry['env']}: removed from the {keys.name}; a new one goes behind your passphrase")
    todo = {e["env"]: s for e in secrets(table) if (s := state(e, keys)) in ("missing", "invalid")}
    tokens = [e for e in secrets(table) if e["env"] in todo and e.get("grant")]
    if tokens:
        say("signing in to auth.jorisjonkers.dev to mint " + ", ".join(e["env"] for e in tokens))
        auth = Auth()
        try:
            me = auth.sign_in()
        except Unreachable as e:
            say(f"could not reach auth.jorisjonkers.dev ({e}); no tokens minted this run")
            me = None
        if me:
            try:
                held = auth.ensure_grants(me, {e["grant"] for e in tokens})
                for entry in tokens:
                    if entry["grant"] not in held:
                        say(f"{entry['env']}: you do not hold {entry['grant']}; ask an admin to grant it")
                        continue
                    token, why = auth.mint(entry["grant"])
                    if not token:
                        say(f"{entry['env']}: mint refused: {why}"
                            + ("; ask an admin to grant it" if "grant" in why else ""))
                        continue
                    if not SAFE.match(token):
                        say(f"{entry['env']}: the minted token has unexpected characters; not stored")
                        continue
                    if check(entry, token) == "invalid":
                        say(f"{entry['env']}: the new token is refused by its host; not stored")
                        continue
                    if gated(entry):
                        if gate_store(token):
                            say(f"{entry['env']}: minted, checked and sealed behind your passphrase")
                        else:
                            say(f"{entry['env']}: minted but not sealed; run setup again to retry")
                        continue
                    keys.put(entry["env"], token)
                    say(f"{entry['env']}: minted, checked and stored in the {keys.name}")
            except Unreachable as e:
                say(f"could not reach auth.jorisjonkers.dev ({e}); the remaining tokens were not minted")
            finally:
                with contextlib.suppress(Unreachable):
                    auth.sign_out()
    for entry in secrets(table):
        if entry["env"] not in todo or entry.get("grant"):
            continue
        while True:
            value = ask(f"Paste {entry['env']} ({entry['paste']}), or Enter to skip: ", secret=True)
            if not value:
                say(f"{entry['env']}: skipped")
                break
            if not SAFE.match(value) or check(entry, value) == "invalid":
                say(f"{entry['env']}: refused by its host; try again")
                continue
            keys.put(entry["env"], value)
            say(f"{entry['env']}: checked and stored in the {keys.name}")
            break
    write_env(table, keys)
    wire_shell()
    return 0


def status(table: list[dict[str, Any]]) -> int:
    keys = store()
    for entry in secrets(table):
        print(f"{entry['env']} {state(entry, keys)}")
    rc = shell_rc()
    wired = bool(rc and rc.is_file() and ".config/agent-kit/env.sh" in rc.read_text())
    print(f"shell {'ok' if wired else 'missing'}")
    return 0


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--table", type=Path, required=True)
    parser.add_argument("command", choices=["ensure", "status", "write-env"])
    args = parser.parse_args(argv)
    table = json.loads(args.table.read_text())
    if args.command == "ensure":
        return ensure(table)
    if args.command == "status":
        return status(table)
    write_env(table, store())
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except KeyboardInterrupt:
        say("stopped")
        raise SystemExit(130) from None
    except Exception as e:  # what is stored stays stored; say what stopped the rest
        say(f"stopped: {type(e).__name__}: {e}")
        raise SystemExit(1) from None
