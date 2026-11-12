"""measure_litellm_budget: the budget window and reset of a key, read from /key/info."""

from __future__ import annotations

import importlib.util
import json
import threading
from collections.abc import Iterator
from http.server import BaseHTTPRequestHandler, HTTPServer
from pathlib import Path

import pytest

KIT_ROOT = Path(__file__).resolve().parent.parent
spec = importlib.util.spec_from_file_location("measure", KIT_ROOT / "scripts" / "measure_litellm_budget.py")
assert spec and spec.loader
measure = importlib.util.module_from_spec(spec)
spec.loader.exec_module(measure)

KEY = "sk-virtual-key-1234"
EDGE = "smt_edge_token_5678"
INFO = {"info": {"max_budget": 255.0, "budget_duration": "30d", "budget_reset_at": "2026-12-01T00:00:00Z",
                 "spend": 1.5, "key_name": "sk-...1234", "token": "hashed-secret-field"}}


def stub(status: int, body: bytes) -> Iterator[tuple[str, list[dict[str, str]]]]:
    seen: list[dict[str, str]] = []

    class Handler(BaseHTTPRequestHandler):
        def do_GET(self) -> None:  # noqa: N802
            seen.append({"path": self.path, **{k.lower(): v for k, v in self.headers.items()}})
            self.send_response(status)
            self.end_headers()
            self.wfile.write(body)

        def log_message(self, *args: object) -> None:
            pass

    server = HTTPServer(("127.0.0.1", 0), Handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    try:
        yield f"http://127.0.0.1:{server.server_port}/v1", seen
    finally:
        server.shutdown()
        server.server_close()


@pytest.fixture
def proxy() -> Iterator[tuple[str, list[dict[str, str]]]]:
    yield from stub(200, json.dumps(INFO).encode())


ENV = {"LITELLM_VIBE_KEY": KEY, "LITELLM_EDGE_TOKEN": EDGE}


def test_prints_only_the_budget_fields_and_a_redacted_request(proxy, capsys) -> None:
    url, seen = proxy
    assert measure.main(["--url", url], ENV) == 0
    out = capsys.readouterr()
    assert seen[0]["path"] == "/key/info"
    assert seen[0]["authorization"] == f"Bearer {EDGE}"
    assert seen[0]["x-litellm-api-key"] == f"Bearer {KEY}"
    assert '"30d"' in out.out and "255.0" in out.out and "2026-12-01T00:00:00Z" in out.out and "1.5" in out.out
    assert "GET " in out.out and "/key/info" in out.out and "redacted" in out.out
    for secret in (KEY, EDGE, "hashed-secret-field", "key_name"):
        assert secret not in out.out + out.err


def test_an_unset_budget_prints_null(capsys) -> None:
    for url, _ in stub(200, b'{"info": {"spend": 0}}'):
        assert measure.main(["--url", url], ENV) == 0
    assert "budget_duration: null" in capsys.readouterr().out


def test_a_refusal_fails_with_the_status_and_no_secret(capsys) -> None:
    for url, _ in stub(401, b'{"error": "bad key sk-virtual-key-1234"}'):
        assert measure.main(["--url", url], ENV) == 1
    err = capsys.readouterr().err
    assert "HTTP 401" in err and KEY not in err and EDGE not in err


@pytest.mark.parametrize("body", [b"not json", b"[]", b'{"info": 3}'])
def test_an_unusable_reply_fails(body, capsys) -> None:
    for url, _ in stub(200, body):
        assert measure.main(["--url", url], ENV) == 1
    assert "proxy" in capsys.readouterr().err


def test_an_unreachable_proxy_fails(capsys) -> None:
    assert measure.main(["--url", "http://127.0.0.1:1"], ENV) == 1
    assert "no usable answer" in capsys.readouterr().err


def test_plain_http_off_loopback_is_refused(capsys) -> None:
    assert measure.main(["--url", "http://litellm.example.com"], ENV) == 2
    assert "https" in capsys.readouterr().err


def test_the_default_url_is_the_route_tables(monkeypatch, capsys) -> None:
    seen: list[str] = []

    def fake(self, request, timeout=None):  # noqa: ANN001
        seen.append(request.full_url)
        raise OSError("down")

    monkeypatch.setattr("urllib.request.OpenerDirector.open", fake)
    assert measure.main([], ENV) == 1
    assert seen == ["https://litellm.jorisjonkers.dev/key/info"]


def test_a_url_with_credentials_is_refused(capsys) -> None:
    assert measure.main(["--url", "https://u:p@litellm.example.com"], ENV) == 2
    assert "credentials" in capsys.readouterr().err


def test_a_redirect_is_not_followed(capsys) -> None:
    for url, seen in stub(302, b""):
        assert measure.main(["--url", url], ENV) == 1
        assert len(seen) == 1
    assert "HTTP 302" in capsys.readouterr().err


def test_an_unknown_agent_is_a_usage_error(capsys) -> None:
    assert measure.main(["--agent", "nope"], ENV) == 2
    assert "known: vibe" in capsys.readouterr().err


def test_a_missing_credential_is_named_not_printed(tmp_path, capsys) -> None:
    refuse = tmp_path / "secret"
    refuse.write_text("#!/usr/bin/env python3\nimport sys\nsys.exit(1)\n")
    with pytest.raises(measure.Failure) as raised:
        measure.credential("LITELLM_VIBE_KEY", {}, refuse)
    assert raised.value.code == 2 and "LITELLM_VIBE_KEY" in str(raised.value)


def test_a_hung_secret_store_counts_as_missing(tmp_path, monkeypatch) -> None:
    def hang(*args, **kwargs):  # noqa: ANN002, ANN003
        raise measure.subprocess.TimeoutExpired("x", 1)

    monkeypatch.setattr(measure.subprocess, "run", hang)
    with pytest.raises(measure.Failure) as raised:
        measure.credential("NAME", {}, tmp_path / "x")
    assert raised.value.code == 2


def test_the_secret_store_is_the_fallback(tmp_path) -> None:
    store = tmp_path / "secret"
    store.write_text("#!/usr/bin/env python3\nimport sys\nprint('from-store-' + sys.argv[2])\n")
    assert measure.credential("NAME", {}, store) == "from-store-NAME"


def test_an_unsafe_value_is_refused_without_echo() -> None:
    with pytest.raises(measure.Failure) as raised:
        measure.credential("NAME", {"NAME": "bad value\r\nx: y"})
    assert "bad value" not in str(raised.value)


def test_a_missing_route_table_is_a_usage_failure_not_a_traceback(monkeypatch, capsys) -> None:
    def missing(agent, routes=None):  # noqa: ANN001
        raise FileNotFoundError

    monkeypatch.setattr(measure, "route_for", missing)
    assert measure.main([], ENV) == 2
    assert "FileNotFoundError" in capsys.readouterr().err


def test_a_trailing_newline_is_not_safe() -> None:
    with pytest.raises(measure.Failure):
        measure.credential("NAME", {"NAME": "abc\n"})
