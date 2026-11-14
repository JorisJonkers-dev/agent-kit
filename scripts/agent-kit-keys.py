#!/usr/bin/env python3
"""Get the workstation's tokens and keys, store them, and wire them into the shell.

setup-workstation.sh runs this. ``ensure`` asks only for what is missing or no
longer works: it signs in to auth-api once (password, then TOTP), grants the
missing services when you are an admin, mints each service token, checks every
value against its own host, and stores it in the one secret store, through
installer/agent-kit-secret: the macOS Keychain, or the Secret Service (libsecret)
on Linux. There is no plain-file store: a missing store stops the run.
``write-env`` renders ~/.config/agent-kit/env.sh, which reads them back in
every shell; the Private Memory token is opened only by ``vibe-private``, and the Study
account key is read only by ``vibe-study``.
The LiteLLM agent keys (``mint: litellm``) are minted on the proxy itself: given a
short-lived management key (``LITELLM_ADMIN_TOKEN``, or asked for once when a key is
missing; never stored), it makes or updates each route's team, which holds the budget,
deletes the old shared key of the route, and mints this host's own key as
``<agent>@<host>`` into the team.
``status`` reports without asking anything. ``revoke NAME`` signs in and revokes this
host's token for NAME (its recorded id), so the host refuses it from then on; the next
``ensure`` sees the stored value refused and mints a fresh one.

Secrets never reach argv or output: the store gets them on stdin. Stdlib only, so the system python3 runs it.

Usage::

    python3 scripts/agent-kit-keys.py --table <workstation-secrets.json> ensure|status|write-env
    python3 scripts/agent-kit-keys.py --table <workstation-secrets.json> revoke NAME
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
import urllib.parse
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


SECRET_TOOL = Path(__file__).resolve().parent.parent / "installer" / "agent-kit-secret"


class StoreError(Exception):
    """The secret store is missing or refused; the run stops rather than fall back to a file."""


class Store:
    """The one secret store, through agent-kit-secret: the macOS Keychain, or on Linux libsecret."""

    @property
    def name(self) -> str:
        return "keychain" if self._backend() == "keychain" else "Secret Service"

    @staticmethod
    def _backend() -> str:
        forced = os.environ.get("AGENT_KIT_SECRETS_BACKEND", "")
        return forced or ("keychain" if sys.platform == "darwin" and shutil.which("security") else "libsecret")

    def _run(self, command: str, var: str, value: str | None = None) -> subprocess.CompletedProcess[str]:
        # The value goes over stdin, never argv.
        return subprocess.run([sys.executable, str(SECRET_TOOL), command, var], input=value,
                              capture_output=True, text=True, check=False)

    def _checked(self, done: subprocess.CompletedProcess[str]) -> subprocess.CompletedProcess[str]:
        if done.returncode == 2:
            raise StoreError(done.stderr.strip().removeprefix("agent-kit-secret: ") or "the secret store is not usable")
        return done

    def get(self, var: str) -> str:
        done = self._checked(self._run("get", var))
        return done.stdout.strip() if done.returncode == 0 else ""

    def put(self, var: str, value: str) -> None:
        self._checked(self._run("put", var, value))
        if self.get(var) != value:
            raise RuntimeError(f"{var} did not read back from the {self.name}")

    def delete(self, var: str) -> None:
        self._checked(self._run("delete", var))

    def shell_reader(self) -> str:
        # Quiet on a missing key: env.sh exports only what is there.
        return '_agent_kit_secret() { "$HOME/.local/bin/agent-kit-secret" get "$1" 2>/dev/null; }'

    def migrate_legacy_file(self) -> None:
        """Move a secrets.env an earlier setup wrote into the store, then remove the plain file."""
        path = config_dir() / "secrets.env"
        if not path.is_file():
            return
        values = dict(line.split("=", 1) for line in path.read_text().splitlines() if "=" in line)
        for var, value in values.items():
            if value and not self.get(var):
                self.put(var, value)
        path.unlink()
        say(f"moved the keys in {path} into the {self.name} and removed the plain file")


# --- checks ----------------------------------------------------------------


class _NoRedirect(urllib.request.HTTPRedirectHandler):
    # Forward-auth sends a bad token to the login page, which answers 200.
    def redirect_request(self, *args: object, **kwargs: object) -> None:
        return None


_CHECK = urllib.request.build_opener(_NoRedirect, urllib.request.HTTPSHandler(context=tls_context()))


def check(entry: dict[str, Any], value: str) -> str:
    """ok, invalid, or unchecked (the host did not answer, or the entry says it cannot be asked)."""
    if not entry.get("check"):
        return "unchecked"
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
    return [e for e in table if "value" not in e and not e.get("retired")]


def retire(table: list[dict[str, Any]], keys: Store) -> None:
    """Drop values the registry no longer manages from the store."""
    for entry in table:
        if entry.get("retired") and keys.get(entry["env"]):
            keys.delete(entry["env"])
            say(f"{entry['env']}: retired, removed from the {keys.name}")


# A gated value lives behind a passphrase only you know (installer/agent-kit-private).
GATE = Path(__file__).resolve().parent.parent / "installer" / "agent-kit-private"


def gated(entry: dict[str, Any]) -> bool:
    return entry.get("gate") == "passphrase"


def gate_store(value: str) -> bool:
    """Seal a value; the gate asks for the passphrase at the terminal itself."""
    return subprocess.run([sys.executable, str(GATE), "store"], input=value, text=True, check=False).returncode == 0


def state(entry: dict[str, Any], keys: Store) -> str:
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

    def mint(self, grant: str) -> tuple[str, str, str]:
        """(token, id, why it failed)."""
        label = f"{socket.gethostname().split('.')[0]} agent-kit"
        code, reply = self.call("POST", "/auth/service-tokens", {"service": grant, "label": label})
        if code not in (200, 201) or not reply.get("token"):
            return "", "", str(reply.get("detail") or f"HTTP {code}")
        return reply["token"], str(reply.get("id") or ""), ""

    def revoke(self, ident: str) -> bool:
        code, _ = self.call("DELETE", f"/auth/service-tokens/{ident}")
        return code in (200, 204)

    def sign_out(self) -> None:
        self.call("GET", "/auth/logout")


# --- token ids -------------------------------------------------------------
# Not secrets: an id only lets a signed-in owner revoke the token it names.


def ids_path() -> Path:
    return config_dir() / "token-ids.json"


def load_ids() -> dict[str, str]:
    try:
        return dict(json.loads(ids_path().read_text()))
    except (OSError, ValueError):
        return {}


def save_ids(ids: dict[str, str]) -> None:
    ids_path().parent.mkdir(parents=True, exist_ok=True)
    ids_path().write_text(json.dumps(ids, indent=2, sort_keys=True) + "\n")


def replaced(auth: Auth, ids: dict[str, str], var: str, new_id: str) -> None:
    """Record the new token's id and revoke the one it replaces."""
    old = ids.get(var)
    if new_id:
        ids[var] = new_id
        save_ids(ids)
    if old and old != new_id:
        if auth.revoke(old):
            say(f"{var}: revoked the token it replaces")
        else:
            say(f"{var}: could not revoke the token it replaces (id {old}); revoke it on the admin page")


# --- env.sh ----------------------------------------------------------------

SOURCE_LINE = '[ -f "$HOME/.config/agent-kit/env.sh" ] && . "$HOME/.config/agent-kit/env.sh"  # agent-kit'


# --- LiteLLM ---------------------------------------------------------------


# LiteLLM's sentinel for "whatever the key's team may use".
TEAM_MODELS = "all-team-models"


class Refused(Exception):
    """The proxy refused the admin token itself: no route can be minted with it."""


def _why(reply: Any) -> str:
    """LiteLLM's own reason for a refusal, short, for the message."""
    error = reply.get("error") if isinstance(reply, dict) else None
    message = error.get("message") if isinstance(error, dict) else (error or (reply or {}).get("detail"))
    return f": {str(message)[:200]}" if message else ""


class LiteLLM:
    """The proxy's management API, called with a short-lived management key that is never stored."""

    def __init__(self, proxy: str, token: str) -> None:
        self.proxy = proxy.rstrip("/")
        self.token = token

    def call(self, method: str, path: str, body: dict[str, Any] | None = None) -> tuple[int, Any]:
        request = urllib.request.Request(
            self.proxy + path, data=None if body is None else json.dumps(body).encode(), method=method,
            headers={"Authorization": f"Bearer {self.token}", "User-Agent": AGENT, "Content-Type": "application/json"},
        )
        try:
            with _CHECK.open(request, timeout=30) as response:
                raw = response.read()
                return response.status, json.loads(raw) if raw else None
        except urllib.error.HTTPError as e:
            return e.code, _json(e.read())
        except ValueError:
            return 0, None
        except (urllib.error.URLError, OSError) as e:
            raise Unreachable(str(e)) from None

    def team(self, want: dict[str, Any]) -> str | None:
        """The id of the team ``want`` names, made or brought to ``want``'s budget and access groups."""
        alias = want["team_alias"]
        code, reply = self.call("GET", "/team/list")
        teams = reply.get("teams") if isinstance(reply, dict) else reply
        if code != 200 or not isinstance(teams, list):
            raise Refused(f"could not list the teams (HTTP {code}{_why(reply)})")
        found = next((t for t in teams if isinstance(t, dict) and t.get("team_alias") == alias), None)
        if found is None:
            code, made = self.call("POST", "/team/new", want)
            if code != 200 or not isinstance(made, dict) or not made.get("team_id"):
                say(f"litellm: team {alias} could not be made (HTTP {code})")
                return None
            say(f"litellm: made team {alias}")
            return str(made["team_id"])
        # A route without a cap is a team without one: an old cap is cleared, not kept.
        wanted = {"max_budget": None, "budget_duration": None, **want}
        drift = {k: v for k, v in wanted.items() if k != "team_alias" and found.get(k) != v}
        if drift:
            code, _ = self.call("POST", "/team/update", {"team_id": found["team_id"], **drift})
            if code != 200:
                say(f"litellm: team {alias} could not be updated (HTTP {code}); it keeps its old settings")
            else:
                say(f"litellm: set {', '.join(sorted(drift))} on team {alias}")
        return str(found["team_id"])

    def holds(self, alias: str) -> bool:
        code, reply = self.call("GET", "/key/list?" + urllib.parse.urlencode({"key_alias": alias}))
        return code == 200 and isinstance(reply, dict) and bool(reply.get("keys"))

    def delete(self, alias: str) -> bool:
        code, _ = self.call("POST", "/key/delete", {"key_aliases": [alias]})
        return code == 200

    def mint(self, team_id: str, alias: str) -> str:
        # The key takes its team's access groups (LiteLLM's all-team-models), so a team change needs no new key.
        body = {
            "key_alias": alias,
            "team_id": team_id,
            "models": [TEAM_MODELS],
            "metadata": {"minted_by": "agent-kit setup"},
        }
        code, reply = self.call("POST", "/key/generate", body)
        key = reply.get("key") if isinstance(reply, dict) else None
        return str(key) if code == 200 and isinstance(key, str) else ""


def host_name() -> str:
    return socket.gethostname().split(".")[0].lower() or "host"


def mint_litellm(entries: list[dict[str, Any]], wanted: set[str], token: str, keys: Store, host: str) -> None:
    """Converge every route's team; delete each route's old shared key; mint the keys in ``wanted``."""
    for entry in entries:
        spec = entry["litellm"]
        api = LiteLLM(spec["proxy"], token)
        team = spec["team"]
        agent = team["team_alias"]
        try:
            team_id = api.team(team)
            if team_id is None:
                continue
            # The key both machines once shared, by the route's own name: each host has its own now.
            if api.holds(agent):
                if api.delete(agent):
                    say(f"litellm: deleted the shared key {agent}; each host has its own key now")
                else:
                    say(f"litellm: the shared key {agent} could not be deleted")
            if entry["env"] not in wanted:
                continue
            alias = f"{agent}@{host}"
            # A key under this host's name that this store lost is useless: replace it.
            if api.holds(alias) and not api.delete(alias):
                say(f"{entry['env']}: the old key {alias} could not be deleted; nothing minted")
                continue
            key = api.mint(team_id, alias)
            if not key or not SAFE.match(key):
                say(f"{entry['env']}: the proxy did not mint a usable key for {alias}")
                continue
            if check(entry, key) == "invalid":
                say(f"{entry['env']}: the new key {alias} is refused by the proxy; not stored")
                api.delete(alias)
                continue
            keys.put(entry["env"], key)
            say(f"{entry['env']}: minted {alias} in team {agent}, checked and stored in the {keys.name}")
        except Unreachable as e:
            say(f"could not reach the LiteLLM proxy ({e}); the remaining keys were not minted")
            return
        except Refused as e:
            say(f"litellm: {e}; nothing minted (registry/generated/litellm/make-admin-token.sh makes the token)")
            return


def ensure_litellm(table: list[dict[str, Any]], todo: set[str], keys: Store) -> None:
    """Mint the missing LiteLLM keys, and converge the teams whenever an admin token is at hand."""
    entries = [e for e in secrets(table) if e.get("mint") == "litellm"]
    wanted = {e["env"] for e in entries if e["env"] in todo}
    token = os.environ.get("LITELLM_ADMIN_TOKEN", "").strip()
    if wanted and not token:
        token = ask(
            "Paste a LiteLLM admin token to mint " + ", ".join(sorted(wanted))
            + " (a one-hour management key: registry/generated/litellm/make-admin-token.sh makes one;"
            " it is not stored), or Enter to skip: ",
            secret=True,
        )
    if not token:
        for env in sorted(wanted):
            say(f"{env}: skipped (no LiteLLM admin token)")
        return
    if not SAFE.match(token):
        say("the LiteLLM admin token has unexpected characters; nothing minted")
        return
    mint_litellm(entries, wanted, token, keys, host_name())


def write_env(table: list[dict[str, Any]], keys: Store) -> Path:
    lines = [
        "# managed by agent-kit setup-workstation.sh; rewritten on every run.",
        f"# Reads the workstation's tokens and keys from the {keys.name}. A value already",
        "# in the environment wins. An `export: false` value is never exported: the",
        "# Private Memory token is opened only by vibe-private (through agent-kit-private,",
        "# which asks for its passphrase), and vibe-study reads the Study account key",
        "# with _agent_kit_secret itself.",
        keys.shell_reader(),
    ]
    for entry in table:
        if entry.get("retired"):
            continue
        var = entry["env"]
        if "value" in entry:
            lines.append(f'[ -n "${{{var}:-}}" ] || export {var}="{entry["value"]}"')
        elif entry.get("export", True):
            lines.append(
                f'if [ -z "${{{var}:-}}" ]; then _agent_kit_v="$(_agent_kit_secret {var})"; '
                f'if [ -n "$_agent_kit_v" ]; then export {var}="$_agent_kit_v"; fi; fi'
            )
    lines.append("unset _agent_kit_v")
    # A shell that sourced an older env.sh still has its vibe wrapper; drop that
    # one only, never a vibe function of your own.
    lines.append('case "$(typeset -f vibe 2>/dev/null)" in *"agent-kit-private exec vibe"*) unset -f vibe ;; esac')
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
    keys = Store()
    keys.migrate_legacy_file()
    retire(table, keys)
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
            ids = load_ids()
            try:
                held = auth.ensure_grants(me, {e["grant"] for e in tokens})
                for entry in tokens:
                    if entry["grant"] not in held:
                        say(f"{entry['env']}: you do not hold {entry['grant']}; ask an admin to grant it")
                        continue
                    token, ident, why = auth.mint(entry["grant"])
                    if not token:
                        say(f"{entry['env']}: mint refused: {why}"
                            + ("; ask an admin to grant it" if "grant" in why else ""))
                        continue
                    if not SAFE.match(token):
                        say(f"{entry['env']}: the minted token has unexpected characters; not stored")
                        auth.revoke(ident)
                        continue
                    if check(entry, token) == "invalid":
                        say(f"{entry['env']}: the new token is refused by its host; not stored")
                        auth.revoke(ident)
                        continue
                    if gated(entry):
                        if not gate_store(token):
                            say(f"{entry['env']}: minted but not sealed; revoking it, run setup again to retry")
                            auth.revoke(ident)
                            continue
                        say(f"{entry['env']}: minted, checked and sealed behind your passphrase")
                    else:
                        keys.put(entry["env"], token)
                        say(f"{entry['env']}: minted, checked and stored in the {keys.name}")
                    replaced(auth, ids, entry["env"], ident)
            except Unreachable as e:
                say(f"could not reach auth.jorisjonkers.dev ({e}); the remaining tokens were not minted")
            finally:
                with contextlib.suppress(Unreachable):
                    auth.sign_out()
    ensure_litellm(table, set(todo), keys)
    for entry in secrets(table):
        if entry["env"] not in todo or entry.get("grant") or entry.get("mint"):
            continue
        if entry.get("login"):
            # A sign-in this tool cannot do for you (a browser login): say which command does.
            say(f"{entry['env']}: sign in with  {entry['login']}  (in a terminal)")
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
    keys = Store()
    for entry in secrets(table):
        print(f"{entry['env']} {state(entry, keys)}")
    rc = shell_rc()
    wired = bool(rc and rc.is_file() and ".config/agent-kit/env.sh" in rc.read_text())
    print(f"shell {'ok' if wired else 'missing'}")
    return 0


def revoke(table: list[dict[str, Any]], name: str) -> int:
    """Revoke this host's minted token for NAME by its recorded id; the stored value is left to fail its check."""
    if not any(e["env"] == name and e.get("grant") for e in secrets(table)):
        say(f"{name}: not a token this tool mints")
        return 2
    ids = load_ids()
    ident = ids.get(name)
    if not ident:
        say(f"{name}: no token id recorded on this host; revoke it on the admin page")
        return 1
    auth = Auth()
    try:
        if not auth.sign_in():
            return 1
        revoked = auth.revoke(ident)
    except Unreachable as e:
        say(f"could not reach auth.jorisjonkers.dev ({e}); nothing revoked")
        return 1
    finally:
        with contextlib.suppress(Unreachable):
            auth.sign_out()
    if not revoked:
        say(f"{name}: auth-api refused to revoke token id {ident}")
        return 1
    del ids[name]
    save_ids(ids)
    say(f"{name}: revoked; `status` now reports it invalid, and the next setup run mints a new one")
    return 0


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--table", type=Path, required=True)
    parser.add_argument("command", choices=["ensure", "status", "write-env", "revoke"])
    parser.add_argument("name", nargs="?", help="revoke: the variable whose token to revoke")
    args = parser.parse_args(argv)
    table = json.loads(args.table.read_text())
    if args.command == "ensure":
        return ensure(table)
    if args.command == "status":
        return status(table)
    if args.command == "revoke":
        if not args.name:
            parser.error("revoke needs the variable name, for example HINDSIGHT_API_TOKEN")
        return revoke(table, args.name)
    keys = Store()
    keys.migrate_legacy_file()
    retire(table, keys)
    write_env(table, keys)
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
