"""agent-kit-keys: setup gets the workstation's tokens and keys for you.

It signs in to auth-api once, grants the missing MEMORY_* services (as admin),
mints each token, checks it against its own host, and stores it in the
keychain. A managed env.sh reads them back for every shell; the private-bank
token only reaches Vibe and memory-private sessions.
"""

from __future__ import annotations

import json
import subprocess
import sys
import threading
from collections.abc import Iterator
from http.server import BaseHTTPRequestHandler, HTTPServer
from pathlib import Path

import pytest

KIT_ROOT = Path(__file__).resolve().parent.parent
KEYS = KIT_ROOT / "scripts" / "agent-kit-keys.py"

FAKE_SECURITY = r'''#!/usr/bin/env python3
"""A stand-in for macOS `security`: generic passwords as files in $FAKE_KEYCHAIN."""
import os, shlex, sys
store = os.environ["FAKE_KEYCHAIN"]
with open(os.path.join(store, "argv.log"), "a") as log:
    log.write(" ".join(sys.argv[1:]) + "\n")

def add(args):
    s = args[args.index("-s") + 1]
    w = args[args.index("-w") + 1]
    open(os.path.join(store, s), "w").write(w)

if sys.argv[1:2] == ["-i"]:
    for line in sys.stdin:
        args = shlex.split(line)
        if args and args[0] == "add-generic-password":
            add(args[1:])
    sys.exit(0)
if sys.argv[1] == "delete-generic-password":
    path = os.path.join(store, sys.argv[sys.argv.index("-s") + 1])
    if not os.path.exists(path):
        sys.exit(44)
    os.remove(path)
    sys.exit(0)
if sys.argv[1] == "find-generic-password":
    s = sys.argv[sys.argv.index("-s") + 1]
    path = os.path.join(store, s)
    if not os.path.exists(path):
        sys.exit(44)
    if "-w" in sys.argv:
        print(open(path).read())
    sys.exit(0)
sys.exit(1)
'''


class Auth:
    def __init__(self) -> None:
        self.grants = {"MEMORY_API"}
        self.role = "ADMIN"
        self.totp = True
        self.minted: list[str] = []
        self.valid: set[str] = set()
        self.refused_services: set[str] = set()
        self.log: list[str] = []


@pytest.fixture
def auth() -> Iterator[tuple[Auth, str]]:
    state = Auth()

    class Handler(BaseHTTPRequestHandler):
        def _send(self, code: int, body: dict | None = None, cookie: str | None = None) -> None:
            data = json.dumps(body or {}).encode()
            self.send_response(code)
            self.send_header("Content-Type", "application/json")
            if cookie:
                self.send_header("Set-Cookie", cookie)
            self.end_headers()
            self.wfile.write(data)

        def _body(self) -> dict:
            n = int(self.headers.get("Content-Length") or 0)
            return json.loads(self.rfile.read(n) or b"{}")

        def _signed_in(self) -> bool:
            return "SESSION=ok" in (self.headers.get("Cookie") or "")

        def _xsrf(self) -> bool:
            return self.headers.get("X-XSRF-TOKEN") == "x1" and "XSRF-TOKEN=x1" in (self.headers.get("Cookie") or "")

        def do_POST(self) -> None:
            state.log.append(f"POST {self.path}")
            if self.headers.get("User-Agent", "").startswith("Python-urllib"):
                return self._send(403, {"detail": "error 1010"})
            if self.path == "/api/v1/auth/session-login":
                body = self._body()
                if body.get("password") != "pw":
                    return self._send(401, {"detail": "Bad credentials"})
                if state.totp and body.get("totpCode") != "123456":
                    return self._send(200, {"totpRequired": True})
                return self._send(200, {"success": True}, "SESSION=ok; Path=/")
            if self.path == "/api/v1/auth/service-tokens":
                if not (self._signed_in() and self._xsrf()):
                    return self._send(403, {"detail": "csrf"})
                service = self._body()["service"]
                if service not in state.grants:
                    return self._send(403, {"detail": f"User does not hold the {service} service grant"})
                token = f"smt_{service.lower()}_{len(state.minted)}"
                state.minted.append(service)
                if service not in state.refused_services:
                    state.valid.add(token)
                return self._send(201, {"token": token, "id": str(len(state.minted))})
            if self.path.startswith("/check/"):
                token = (self.headers.get("Authorization") or "").removeprefix("Bearer ")
                if token in state.valid:
                    return self._send(200)
                # Like forward-auth: a bad token is sent to the login page, which answers 200.
                self.send_response(302)
                self.send_header("Location", "/login")
                self.end_headers()
                return None
            return self._send(404)

        def do_GET(self) -> None:
            state.log.append(f"GET {self.path}")
            if self.path == "/api/v1/auth/me":
                if not self._signed_in():
                    return self._send(401)
                return self._send(200, {"id": "u1", "role": state.role}, "XSRF-TOKEN=x1; Path=/")
            if self.path == "/api/v1/admin/users/u1":
                return self._send(200, {"id": "u1", "servicePermissions": sorted(state.grants)})
            if self.path == "/login":
                # An HTML page, not JSON.
                self.send_response(200)
                self.send_header("Content-Type", "text/html")
                self.end_headers()
                self.wfile.write(b"<html>login</html>")
                return None
            if self.path == "/api/v1/auth/logout":
                # Like auth-api: sign-out ends on the login page.
                self.send_response(302)
                self.send_header("Location", "/login")
                self.end_headers()
                return None
            if self.path == "/mistral":
                key = (self.headers.get("Authorization") or "").removeprefix("Bearer ")
                return self._send(200 if key == "mk_good" else 401)
            return self._send(404)

        def do_PUT(self) -> None:
            state.log.append(f"PUT {self.path}")
            if self.path == "/api/v1/admin/users/u1/services" and self._signed_in() and self._xsrf():
                state.grants = set(self._body()["services"])
                return self._send(200, {"servicePermissions": sorted(state.grants)})
            return self._send(403)

        def log_message(self, format: str, *args: object) -> None:
            pass

    server = HTTPServer(("127.0.0.1", 0), Handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    yield state, f"http://127.0.0.1:{server.server_port}"
    server.shutdown()


def _table(base: str) -> list[dict]:
    return [
        {"env": "HINDSIGHT_API_URL", "value": "https://hindsight-api.jorisjonkers.dev", "export": True},
        {"env": "HINDSIGHT_API_TOKEN", "grant": "MEMORY_API", "check": f"{base}/check/api", "export": True},
        {"env": "MEMORY_MCP_TOKEN", "grant": "MEMORY_MCP", "check": f"{base}/check/mcp", "export": True},
        {"env": "MEMORY_PRIVATE_TOKEN", "grant": "MEMORY_PRIVATE", "check": f"{base}/check/private", "export": False,
         "gate": "passphrase"},
        {"env": "MISTRAL_API_KEY", "paste": "a Mistral key for this machine", "check": f"{base}/mistral",
         "check_method": "GET", "export": True},
    ]


GATE_PASSPHRASE = "correct horse"


def _tty(tmp_path: Path) -> Path:
    tty = tmp_path / "tty"
    tty.write_text(f"{GATE_PASSPHRASE}\n{GATE_PASSPHRASE}\n")
    return tty


def _gated(home: dict) -> Path:
    return Path(home["env"]["HOME"]) / ".config" / "agent-kit" / "private" / "MEMORY_PRIVATE_TOKEN.enc"


def _unseal(home: dict) -> str:
    out = subprocess.run(
        ["openssl", "enc", "-d", "-aes-256-cbc", "-pbkdf2", "-iter", "600000", "-pass", "stdin",
         "-in", str(_gated(home))],
        input=GATE_PASSPHRASE + "\n", capture_output=True, text=True, check=False,
    )
    return out.stdout.strip()


@pytest.fixture
def home(tmp_path: Path, auth: tuple[Auth, str]) -> dict:
    _, base = auth
    bin_dir = tmp_path / "bin"
    bin_dir.mkdir()
    (bin_dir / "security").write_text(FAKE_SECURITY.replace("/usr/bin/env python3", sys.executable))
    (bin_dir / "security").chmod(0o755)
    keychain = tmp_path / "keychain"
    keychain.mkdir()
    table = tmp_path / "table.json"
    table.write_text(json.dumps(_table(base)))
    return {
        "tmp": tmp_path,
        "keychain": keychain,
        "table": table,
        "env": {
            "PATH": f"{bin_dir}:/usr/bin:/bin",
            "HOME": str(tmp_path / "home"),
            "SHELL": "/bin/zsh",
            "FAKE_KEYCHAIN": str(keychain),
            "AGENT_KIT_TTY": str(_tty(tmp_path)),
            "AGENT_KIT_AUTH_URL": base,
        },
    }


def _keys(home: dict, *args: str, answers: str = "") -> subprocess.CompletedProcess:
    (home["tmp"] / "home").mkdir(exist_ok=True)
    return subprocess.run(
        [sys.executable, str(KEYS), "--table", str(home["table"]), *args],
        input=answers, capture_output=True, text=True, check=False, env=home["env"], timeout=60,
        start_new_session=True,
    )


def _stored(home: dict) -> dict:
    return {p.name.removeprefix("agent-kit."): p.read_text() for p in home["keychain"].glob("agent-kit.*")}


ANSWERS = "me@example.com\npw\n123456\nmk_good\ny\n"


def test_ensure_mints_grants_and_stores_everything(home: dict, auth: tuple[Auth, str]) -> None:
    state, _ = auth
    result = _keys(home, "ensure", answers=ANSWERS)
    assert result.returncode == 0, result.stderr
    stored = _stored(home)
    assert set(stored) == {"HINDSIGHT_API_TOKEN", "MEMORY_MCP_TOKEN", "MISTRAL_API_KEY"}
    # The private token is behind the passphrase, not in the keychain.
    stored["MEMORY_PRIVATE_TOKEN"] = _unseal(home)
    assert stored["MISTRAL_API_KEY"] == "mk_good"
    assert {"MEMORY_MCP", "MEMORY_PRIVATE"} <= state.grants, "missing grants were added"
    assert "MEMORY_API" in state.grants, "existing grants were kept"
    assert sorted(state.minted) == ["MEMORY_API", "MEMORY_MCP", "MEMORY_PRIVATE"]
    assert all(stored[k] in state.valid for k in ("HINDSIGHT_API_TOKEN", "MEMORY_MCP_TOKEN", "MEMORY_PRIVATE_TOKEN"))
    assert "GET /api/v1/auth/logout" in state.log


def test_an_unexpected_failure_is_a_note_not_a_traceback(home: dict, auth: tuple[Auth, str]) -> None:
    home["table"].write_text("[{]")
    result = _keys(home, "status")
    assert "Traceback" not in result.stderr
    assert result.returncode == 1
    assert "keys: stopped:" in result.stderr


def test_secrets_never_reach_argv_or_output(home: dict, auth: tuple[Auth, str]) -> None:
    result = _keys(home, "ensure", answers=ANSWERS)
    secrets = list(_stored(home).values())
    argv = (home["keychain"] / "argv.log").read_text()
    for secret in secrets:
        assert secret not in argv
        assert secret not in result.stdout and secret not in result.stderr


def test_a_second_run_with_everything_valid_asks_nothing(home: dict, auth: tuple[Auth, str]) -> None:
    state, _ = auth
    _keys(home, "ensure", answers=ANSWERS)
    before = list(state.minted)
    result = _keys(home, "ensure", answers="")
    assert result.returncode == 0, result.stderr
    assert state.minted == before
    assert "POST /api/v1/auth/session-login" not in state.log[len(state.log) - 10 :]


def test_an_invalid_stored_token_is_replaced(home: dict, auth: tuple[Auth, str]) -> None:
    state, _ = auth
    _keys(home, "ensure", answers=ANSWERS)
    state.valid.discard(_stored(home)["MEMORY_MCP_TOKEN"])
    result = _keys(home, "ensure", answers="me@example.com\npw\n123456\n")
    assert result.returncode == 0, result.stderr
    assert state.minted.count("MEMORY_MCP") == 2
    assert _stored(home)["MEMORY_MCP_TOKEN"] in state.valid


def test_a_new_token_its_host_refuses_is_not_stored(home: dict, auth: tuple[Auth, str]) -> None:
    state, _ = auth
    state.refused_services = {"MEMORY_MCP"}
    result = _keys(home, "ensure", answers=ANSWERS)
    assert "MEMORY_MCP_TOKEN" not in _stored(home)
    assert "MEMORY_MCP_TOKEN: the new token is refused by its host" in result.stderr
    assert "HINDSIGHT_API_TOKEN" in _stored(home)


def test_status_reports_without_asking(home: dict, auth: tuple[Auth, str]) -> None:
    _keys(home, "ensure", answers=ANSWERS)
    (home["keychain"] / "agent-kit.MEMORY_MCP_TOKEN").unlink()
    result = _keys(home, "status")
    lines = dict(line.split(" ", 1) for line in result.stdout.splitlines())
    assert lines["HINDSIGHT_API_TOKEN"] == "ok"
    assert lines["MEMORY_MCP_TOKEN"] == "missing"
    assert lines["MISTRAL_API_KEY"] == "ok"


def test_a_wrong_mistral_key_is_asked_again(home: dict, auth: tuple[Auth, str]) -> None:
    answers = "me@example.com\npw\n123456\nmk_bad\nmk_good\ny\n"
    assert _keys(home, "ensure", answers=answers).returncode == 0
    assert _stored(home)["MISTRAL_API_KEY"] == "mk_good"


def test_a_skipped_answer_leaves_the_rest_done(home: dict, auth: tuple[Auth, str]) -> None:
    # Empty Mistral key = skip it; everything else still lands.
    result = _keys(home, "ensure", answers="me@example.com\npw\n123456\n\ny\n")
    assert result.returncode == 0, result.stderr
    assert "MISTRAL_API_KEY" not in _stored(home)
    assert _unseal(home) in state_tokens(auth)


def state_tokens(auth: tuple[Auth, str]) -> set[str]:
    return auth[0].valid


def test_a_private_token_left_in_the_keychain_is_removed_and_replaced_behind_the_gate(
    home: dict, auth: tuple[Auth, str],
) -> None:
    state, _ = auth
    (home["keychain"] / "agent-kit.MEMORY_PRIVATE_TOKEN").write_text("smt_old_exposed")
    result = _keys(home, "ensure", answers=ANSWERS)
    assert result.returncode == 0, result.stderr
    assert not (home["keychain"] / "agent-kit.MEMORY_PRIVATE_TOKEN").exists()
    assert "MEMORY_PRIVATE" in state.minted
    assert _unseal(home) in state.valid and _unseal(home) != "smt_old_exposed"


def test_status_of_the_gated_token_asks_nothing(home: dict, auth: tuple[Auth, str]) -> None:
    _keys(home, "ensure", answers=ANSWERS)
    home["env"]["AGENT_KIT_TTY"] = str(home["tmp"] / "no-terminal")
    lines = dict(line.split(" ", 1) for line in _keys(home, "status").stdout.splitlines())
    assert lines["MEMORY_PRIVATE_TOKEN"] == "ok"


def test_a_non_admin_without_a_grant_is_told_who_can_fix_it(home: dict, auth: tuple[Auth, str]) -> None:
    state, _ = auth
    state.role = "USER"
    result = _keys(home, "ensure", answers=ANSWERS)
    assert "MEMORY_MCP" in result.stderr and "admin" in result.stderr
    assert "PUT /api/v1/admin/users/u1/services" not in state.log
    assert "HINDSIGHT_API_TOKEN" in _stored(home), "the grant you hold is still minted"


def test_a_refused_password_stops_signing_in(home: dict, auth: tuple[Auth, str]) -> None:
    result = _keys(home, "ensure", answers="me@example.com\nwrong\n\n")
    assert "Bad credentials" in result.stderr
    assert not any(k.endswith("TOKEN") for k in _stored(home))


# --- env.sh: every shell reads the keychain, the private token stays scoped ---


def _env_sh(home: dict) -> Path:
    return Path(home["env"]["HOME"]) / ".config" / "agent-kit" / "env.sh"


def test_env_sh_exports_the_shared_values_and_scopes_the_private_token(home: dict, auth: tuple[Auth, str]) -> None:
    _keys(home, "ensure", answers=ANSWERS)
    stored = _stored(home)
    probe = (
        f'. "{_env_sh(home)}"; '
        'printf "%s|%s|%s|%s|%s\\n" "$HINDSIGHT_API_URL" "$HINDSIGHT_API_TOKEN" "$MEMORY_MCP_TOKEN" '
        '"$MISTRAL_API_KEY" "${MEMORY_PRIVATE_TOKEN:-unset}"'
    )
    for shell in ("/bin/bash", "/bin/zsh"):
        if not Path(shell).exists():
            continue
        out = subprocess.run([shell, "-c", probe], capture_output=True, text=True, env=home["env"], check=False)
        assert out.stdout.strip() == (
            f"https://hindsight-api.jorisjonkers.dev|{stored['HINDSIGHT_API_TOKEN']}|"
            f"{stored['MEMORY_MCP_TOKEN']}|mk_good|unset"
        ), out.stderr


def test_env_sh_launches_vibe_through_the_gate_and_exports_nothing_private(home: dict, auth: tuple[Auth, str]) -> None:
    _keys(home, "ensure", answers=ANSWERS)
    bin_dir = home["tmp"] / "bin"
    (bin_dir / "vibe").write_text('#!/bin/sh\nprintf "%s\\n" "${MEMORY_PRIVATE_TOKEN:-unset}"\n')
    (bin_dir / "vibe").chmod(0o755)
    gate = bin_dir / "agent-kit-private"
    gate.write_text((KIT_ROOT / "installer" / "agent-kit-private").read_text().replace(
        "/usr/bin/env python3", sys.executable, 1))
    gate.chmod(0o755)
    (home["tmp"] / "tty").write_text(f"{GATE_PASSPHRASE}\n")
    probe = f'. "{_env_sh(home)}"; vibe; printf "%s\\n" "${{MEMORY_PRIVATE_TOKEN:-unset}}"'
    out = subprocess.run(["/bin/bash", "-c", probe], capture_output=True, text=True, env=home["env"], check=False)
    assert out.stdout.splitlines() == [_unseal(home), "unset"], out.stderr
    assert "MEMORY_PRIVATE_TOKEN" not in _env_sh(home).read_text().split("vibe()")[0].split("unset _agent_kit_v")[-1]
    assert "_agent_kit_secret MEMORY_PRIVATE_TOKEN" not in _env_sh(home).read_text()


def test_a_value_already_in_the_environment_wins(home: dict, auth: tuple[Auth, str]) -> None:
    _keys(home, "ensure", answers=ANSWERS)
    env = {**home["env"], "MISTRAL_API_KEY": "mine"}
    out = subprocess.run(["/bin/bash", "-c", f'. "{_env_sh(home)}"; printf "%s" "$MISTRAL_API_KEY"'],
                         capture_output=True, text=True, env=env, check=False)
    assert out.stdout == "mine"


def test_the_shell_profile_sources_env_sh_once(home: dict, auth: tuple[Auth, str]) -> None:
    _keys(home, "ensure", answers=ANSWERS)
    _keys(home, "ensure", answers="y\n")
    rc = (Path(home["env"]["HOME"]) / ".zshrc").read_text()
    assert [line for line in rc.splitlines() if "agent-kit/env.sh" in line] == [
        '[ -f "$HOME/.config/agent-kit/env.sh" ] && . "$HOME/.config/agent-kit/env.sh"  # agent-kit',
    ]


def test_declining_the_profile_line_leaves_it_alone_and_says_how(home: dict, auth: tuple[Auth, str]) -> None:
    result = _keys(home, "ensure", answers="me@example.com\npw\n123456\nmk_good\nn\n")
    assert not (Path(home["env"]["HOME"]) / ".zshrc").exists()
    assert ".config/agent-kit/env.sh" in result.stderr


def test_write_env_needs_no_answers(home: dict, auth: tuple[Auth, str]) -> None:
    result = _keys(home, "write-env")
    assert result.returncode == 0, result.stderr
    assert _env_sh(home).is_file()
    assert "# managed by agent-kit" in _env_sh(home).read_text()


def test_without_a_keychain_secrets_go_to_a_private_file(home: dict, auth: tuple[Auth, str]) -> None:
    # Not by removing the stub: /usr/bin/security would then reach the real keychain.
    home["env"]["AGENT_KIT_SECRETS_BACKEND"] = "file"
    _keys(home, "ensure", answers=ANSWERS)
    secrets = Path(home["env"]["HOME"]) / ".config" / "agent-kit" / "secrets.env"
    assert secrets.stat().st_mode & 0o777 == 0o600
    out = subprocess.run(["/bin/bash", "-c", f'. "{_env_sh(home)}"; printf "%s" "$MISTRAL_API_KEY"'],
                         capture_output=True, text=True, env=home["env"], check=False)
    assert out.stdout == "mk_good"


def test_requests_do_not_use_the_python_urllib_agent(home: dict, auth: tuple[Auth, str]) -> None:
    # The fake refuses Python-urllib like Cloudflare does; a full run still succeeds.
    assert _keys(home, "ensure", answers=ANSWERS).returncode == 0
    assert "HINDSIGHT_API_TOKEN" in _stored(home)
