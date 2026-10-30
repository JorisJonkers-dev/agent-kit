"""The Antigravity MCP proxy: stdio JSON-RPC to a streamable-HTTP MCP server.

Antigravity's MCP configuration cannot reference environment variables, so
Shared Memory reaches it through this local stdio command. The token comes from
the agent-kit secret store at start-up and stays in memory: never on disk, in
argv, on stdout or on stderr.
"""

from __future__ import annotations

import json
import os
import subprocess
import sys
import threading
from collections.abc import Callable, Iterator
from http.server import BaseHTTPRequestHandler, HTTPServer
from pathlib import Path

import pytest

KIT_ROOT = Path(__file__).resolve().parent.parent
PROXY = KIT_ROOT / "installer" / "antigravity-mcp-proxy"
VAR = "HINDSIGHT_API_TOKEN"
TOKEN = "hindsight_token_Ab-1"
URL = "https://hindsight-api.jorisjonkers.dev/mcp/default/"

INIT = {
    "jsonrpc": "2.0", "id": 1, "method": "initialize",
    "params": {"protocolVersion": "2025-06-18", "capabilities": {},
               "clientInfo": {"name": "antigravity", "version": "1"}},
}
CALL = {"jsonrpc": "2.0", "id": 2, "method": "tools/list"}
NOTIFICATION = {"jsonrpc": "2.0", "method": "notifications/initialized"}

# Stands in for macOS `security`: one generic password per file, keyed by service.
FAKE_SECURITY = r'''#!/usr/bin/env python3
import os, sys
store = os.environ["FAKE_KEYCHAIN"]
args = sys.argv[1:]
with open(os.path.join(store, "argv.log"), "a") as log:
    log.write(" ".join(args) + "\n")
if args[:1] == ["find-generic-password"]:
    path = os.path.join(store, args[args.index("-s") + 1])
    if not os.path.exists(path):
        sys.exit(44)
    print(open(path).read())
    sys.exit(0)
sys.exit(1)
'''

Reply = Callable[[dict], "tuple[int, str, str]"]


class Upstream:
    """A stub streamable-HTTP MCP server: records what it was sent, answers from `reply`."""

    def __init__(self) -> None:
        self.url = ""
        self.session = "session-1"
        self.requests: list[dict] = []

    def reply(self, message: dict) -> tuple[int, str, str]:
        if "id" not in message:
            return 202, "", ""
        return 200, "application/json", json.dumps(
            {"jsonrpc": "2.0", "id": message["id"], "result": {"ok": True}})


@pytest.fixture
def upstream() -> Iterator[Upstream]:
    state = Upstream()

    class Handler(BaseHTTPRequestHandler):
        def do_POST(self) -> None:
            body = self.rfile.read(int(self.headers.get("Content-Length", 0))).decode()
            message = json.loads(body)
            state.requests.append({
                "message": message,
                "authorization": self.headers.get("Authorization", ""),
                "accept": self.headers.get("Accept", ""),
                "session": self.headers.get("Mcp-Session-Id", ""),
                "protocol": self.headers.get("MCP-Protocol-Version", ""),
            })
            status, content_type, reply = state.reply(message)
            self.send_response(status)
            if content_type:
                self.send_header("Content-Type", content_type)
            if state.session:
                self.send_header("Mcp-Session-Id", state.session)
            self.end_headers()
            if reply:
                self.wfile.write(reply.encode())

        def log_message(self, format: str, *args: object) -> None:
            pass

    server = HTTPServer(("127.0.0.1", 0), Handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    state.url = f"http://127.0.0.1:{server.server_port}/mcp/default/"
    yield state
    server.shutdown()


class Workstation:
    def __init__(self, tmp_path: Path) -> None:
        self.tmp = tmp_path
        self.home = tmp_path / "home"
        self.bin = tmp_path / "bin"
        self.keychain = tmp_path / "keychain"
        for path in (self.home, self.bin, self.keychain):
            path.mkdir()
        security = self.bin / "security"
        security.write_text(FAKE_SECURITY.replace("/usr/bin/env python3", sys.executable))
        security.chmod(0o755)

    def store(self, value: str = TOKEN, var: str = VAR) -> None:
        """What agent-kit-keys puts in the keychain: one generic password per variable."""
        (self.keychain / f"agent-kit.{var}").write_text(value)

    def secrets_file(self, text: str) -> None:
        path = self.home / ".config" / "agent-kit" / "secrets.env"
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text)

    def run(self, url: str, *messages: dict, var: str = VAR, raw: str | None = None,
            **env: str) -> subprocess.CompletedProcess:
        run_env = {k: v for k, v in os.environ.items() if k != "AGENT_KIT_SECRETS_BACKEND"}
        run_env.update({"HOME": str(self.home), "PATH": f"{self.bin}:/usr/bin:/bin",
                        "FAKE_KEYCHAIN": str(self.keychain)}, **env)
        stdin = raw if raw is not None else "".join(json.dumps(m) + "\n" for m in messages)
        return subprocess.run([sys.executable, str(PROXY), "--url", url, "--secret", var],
                              input=stdin, capture_output=True, text=True, check=False,
                              env=run_env, timeout=60)


def _lines(result: subprocess.CompletedProcess) -> list[dict]:
    return [json.loads(line) for line in result.stdout.splitlines()]


# --- the installed helper --------------------------------------------------


def test_the_helper_is_executable_and_carries_the_marker_setup_checks() -> None:
    assert PROXY.stat().st_mode & 0o111
    assert "# managed by agent-kit setup-workstation.sh" in PROXY.read_text().splitlines()


# --- the token -------------------------------------------------------------


def test_it_reads_the_token_from_the_keychain_and_attaches_it_to_every_request(
    tmp_path: Path, upstream: Upstream,
) -> None:
    ws = Workstation(tmp_path)
    ws.store()
    result = ws.run(upstream.url, INIT, CALL)
    assert result.returncode == 0, result.stderr
    assert [r["authorization"] for r in upstream.requests] == [f"Bearer {TOKEN}"] * 2


def test_the_file_backend_reads_the_same_variable(tmp_path: Path, upstream: Upstream) -> None:
    ws = Workstation(tmp_path)
    ws.secrets_file(f"OTHER=kept\n{VAR}={TOKEN}\n")
    result = ws.run(upstream.url, INIT, AGENT_KIT_SECRETS_BACKEND="file")
    assert result.returncode == 0, result.stderr
    assert upstream.requests[0]["authorization"] == f"Bearer {TOKEN}"


def test_a_missing_token_fails_before_reading_stdin(tmp_path: Path, upstream: Upstream) -> None:
    ws = Workstation(tmp_path)
    result = ws.run(upstream.url, INIT)
    assert result.returncode == 1
    assert result.stdout == ""
    assert VAR in result.stderr
    assert "keychain" in result.stderr
    assert upstream.requests == [], "nothing was proxied"


def test_an_empty_token_in_the_secrets_file_is_missing_too(tmp_path: Path, upstream: Upstream) -> None:
    ws = Workstation(tmp_path)
    ws.secrets_file(f"{VAR}=\n")
    result = ws.run(upstream.url, INIT, AGENT_KIT_SECRETS_BACKEND="file")
    assert result.returncode == 1
    assert result.stdout == ""
    assert "secrets file" in result.stderr
    assert upstream.requests == []


def test_the_token_reaches_neither_the_helpers_output_nor_any_file_it_writes(
    tmp_path: Path, upstream: Upstream,
) -> None:
    ws = Workstation(tmp_path)
    ws.store()
    result = ws.run(upstream.url, INIT, CALL)
    assert result.returncode == 0, result.stderr
    assert TOKEN not in result.stdout + result.stderr
    assert [p for p in ws.home.rglob("*") if p.is_file()] == [], "the helper wrote nothing"
    assert TOKEN not in (ws.keychain / "argv.log").read_text(), "the store lookup keeps it out of argv"


# --- the transport ---------------------------------------------------------


def test_it_writes_each_upstream_response_back_as_one_json_rpc_line(
    tmp_path: Path, upstream: Upstream,
) -> None:
    ws = Workstation(tmp_path)
    ws.store()
    result = ws.run(upstream.url, INIT, CALL)
    assert result.returncode == 0, result.stderr
    assert [line["id"] for line in _lines(result)] == [1, 2]


def test_it_asks_for_both_response_shapes_the_streamable_transport_allows(
    tmp_path: Path, upstream: Upstream,
) -> None:
    ws = Workstation(tmp_path)
    ws.store()
    assert ws.run(upstream.url, INIT).returncode == 0
    assert upstream.requests[0]["accept"] == "application/json, text/event-stream"


def test_it_forwards_the_session_id_the_upstream_hands_back(tmp_path: Path, upstream: Upstream) -> None:
    ws = Workstation(tmp_path)
    ws.store()
    upstream.session = "abc123"
    result = ws.run(upstream.url, INIT, CALL)
    assert result.returncode == 0, result.stderr
    assert [r["session"] for r in upstream.requests] == ["", "abc123"]


def test_it_sends_the_protocol_version_the_server_negotiated(tmp_path: Path, upstream: Upstream) -> None:
    ws = Workstation(tmp_path)
    ws.store()

    def negotiate(message: dict) -> tuple[int, str, str]:
        return 200, "application/json", json.dumps(
            {"jsonrpc": "2.0", "id": message["id"],
             "result": {"protocolVersion": "2025-06-18", "capabilities": {}}})

    upstream.reply = negotiate
    result = ws.run(upstream.url, INIT, CALL)
    assert result.returncode == 0, result.stderr
    assert [r["protocol"] for r in upstream.requests] == ["", "2025-06-18"]


def test_an_sse_response_body_is_written_back_as_json_rpc_lines(tmp_path: Path, upstream: Upstream) -> None:
    ws = Workstation(tmp_path)
    ws.store()

    def stream(message: dict) -> tuple[int, str, str]:
        progress = {"jsonrpc": "2.0", "method": "notifications/progress", "params": {"progress": 1}}
        answer = {"jsonrpc": "2.0", "id": message["id"], "result": {"ok": True}}
        return 200, "text/event-stream", (
            f"event: message\ndata: {json.dumps(progress)}\n\n"
            f"event: message\ndata: {json.dumps(answer)}\n\n")

    upstream.reply = stream
    result = ws.run(upstream.url, INIT)
    assert result.returncode == 0, result.stderr
    assert [line.get("method") or line["id"] for line in _lines(result)] == ["notifications/progress", 1]


def test_a_notification_gets_no_response_line(tmp_path: Path, upstream: Upstream) -> None:
    ws = Workstation(tmp_path)
    ws.store()
    result = ws.run(upstream.url, NOTIFICATION)
    assert result.returncode == 0, result.stderr
    assert result.stdout == ""


def test_a_malformed_line_is_answered_and_the_proxy_keeps_going(tmp_path: Path, upstream: Upstream) -> None:
    ws = Workstation(tmp_path)
    ws.store()
    result = ws.run(upstream.url, raw="not json\n" + json.dumps(CALL) + "\n")
    assert result.returncode == 0, result.stderr
    assert [line.get("error", {}).get("code") or line["id"] for line in _lines(result)] == [-32700, 2]


def test_an_upstream_refusal_is_reported_without_the_token(tmp_path: Path, upstream: Upstream) -> None:
    ws = Workstation(tmp_path)
    ws.store()
    upstream.reply = lambda message: (401, "text/html", "<html>sign in</html>")
    result = ws.run(upstream.url, INIT)
    assert result.returncode == 0
    assert "401" in result.stderr
    assert "sign in" not in result.stdout, "an HTML login page is not a JSON-RPC message"
    assert _lines(result)[0]["error"]["code"] == -32603
    assert TOKEN not in result.stdout + result.stderr


def test_a_json_rpc_error_body_on_an_error_status_is_forwarded(tmp_path: Path, upstream: Upstream) -> None:
    ws = Workstation(tmp_path)
    ws.store()
    upstream.reply = lambda message: (400, "application/json", json.dumps(
        {"jsonrpc": "2.0", "id": message["id"], "error": {"code": -32602, "message": "bad params"}}))
    result = ws.run(upstream.url, CALL)
    assert result.returncode == 0, result.stderr
    assert _lines(result) == [{"jsonrpc": "2.0", "id": 2, "error": {"code": -32602, "message": "bad params"}}]


def test_a_request_the_upstream_leaves_unanswered_still_gets_an_answer(
    tmp_path: Path, upstream: Upstream,
) -> None:
    ws = Workstation(tmp_path)
    ws.store()
    # Forward-auth that serves its login page with 200 instead of redirecting.
    upstream.reply = lambda message: (200, "text/html", "<html>sign in</html>")
    result = ws.run(upstream.url, CALL)
    assert result.returncode == 0, result.stderr
    assert [(line["id"], line["error"]["code"]) for line in _lines(result)] == [(2, -32603)]
    assert "sign in" not in result.stdout


def test_a_dropped_session_is_reopened_and_the_request_retried(tmp_path: Path, upstream: Upstream) -> None:
    ws = Workstation(tmp_path)
    ws.store()
    upstream.session = "s1"
    answer = upstream.reply

    def redeployed(message: dict) -> tuple[int, str, str]:
        # The first tools/list on s1 finds the server restarted: s1 is gone.
        if message.get("method") == "tools/list" and upstream.requests[-1]["session"] == "s1":
            upstream.session = "s2"
            return 404, "application/json", ""
        return answer(message)

    upstream.reply = redeployed
    result = ws.run(upstream.url, INIT, NOTIFICATION, CALL)
    assert result.returncode == 0, result.stderr
    sent = [(r["message"].get("method"), r["session"]) for r in upstream.requests]
    assert sent == [
        ("initialize", ""), ("notifications/initialized", "s1"), ("tools/list", "s1"),
        ("initialize", ""), ("notifications/initialized", "s2"), ("tools/list", "s2"),
    ]
    assert [line["id"] for line in _lines(result)] == [1, 2], "the replayed handshake is not answered twice"
