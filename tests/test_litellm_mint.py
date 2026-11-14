"""agent-kit-keys mints this host's LiteLLM agent keys on the proxy itself.

A stub proxy stands in for LiteLLM's management API. What is asserted is what the
proxy and the secret store hold afterwards: which teams exist with which budget,
which keys exist under which alias, and what the store holds.
"""

from __future__ import annotations

import importlib.util
import json
import threading
import urllib.parse
from collections.abc import Iterator
from http.server import BaseHTTPRequestHandler, HTTPServer
from pathlib import Path
from typing import Any

import pytest

KIT_ROOT = Path(__file__).resolve().parent.parent
ADMIN = "sk-admin-1h"


def _module() -> Any:
    spec = importlib.util.spec_from_file_location("agent_kit_keys", KIT_ROOT / "scripts" / "agent-kit-keys.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


keys_tool = _module()


class Proxy:
    def __init__(self) -> None:
        self.teams: list[dict[str, Any]] = []
        self.keys: dict[str, dict[str, Any]] = {}  # key value -> its record
        self.refuse: set[str] = set()  # paths answered 403
        self.calls: list[str] = []

    def by_alias(self, alias: str) -> list[str]:
        return [k for k, v in self.keys.items() if v["key_alias"] == alias]


@pytest.fixture
def proxy() -> Iterator[tuple[Proxy, str]]:
    state = Proxy()

    class Handler(BaseHTTPRequestHandler):
        def _send(self, code: int, body: Any) -> None:
            data = json.dumps(body).encode()
            self.send_response(code)
            self.send_header("Content-Type", "application/json")
            self.end_headers()
            self.wfile.write(data)

        def _body(self) -> dict:
            n = int(self.headers.get("Content-Length") or 0)
            return json.loads(self.rfile.read(n) or b"{}")

        def _bearer(self) -> str:
            return (self.headers.get("Authorization") or "").removeprefix("Bearer ")

        def _route(self, method: str) -> None:
            url = urllib.parse.urlsplit(self.path)
            state.calls.append(f"{method} {url.path}")
            if url.path == "/key/info":
                return self._send(200 if self._bearer() in state.keys else 401, {})
            if self._bearer() != ADMIN or url.path in state.refuse:
                return self._send(403, {"error": "not allowed"})
            if url.path == "/team/list":
                return self._send(200, state.teams)
            if url.path == "/team/new":
                team = {**self._body(), "team_id": f"team-{len(state.teams)}"}
                state.teams.append(team)
                return self._send(200, team)
            if url.path == "/team/update":
                body = self._body()
                team = next(t for t in state.teams if t["team_id"] == body["team_id"])
                team.update(body)
                return self._send(200, team)
            if url.path == "/key/list":
                alias = urllib.parse.parse_qs(url.query)["key_alias"][0]
                return self._send(200, {"keys": state.by_alias(alias)})
            if url.path == "/key/delete":
                for alias in self._body()["key_aliases"]:
                    for key in state.by_alias(alias):
                        del state.keys[key]
                return self._send(200, {"deleted_keys": []})
            if url.path == "/key/generate":
                body = self._body()
                key = f"sk-{body['key_alias'].replace('@', '-at-')}-{len(state.keys)}"
                state.keys[key] = body
                return self._send(200, {"key": key, **body})
            return self._send(404, {})

        def do_GET(self) -> None:
            self._route("GET")

        def do_POST(self) -> None:
            self._route("POST")

        def log_message(self, *args: object) -> None:
            pass

    server = HTTPServer(("127.0.0.1", 0), Handler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    yield state, f"http://127.0.0.1:{server.server_port}"
    server.shutdown()


class Store:
    name = "test store"

    def __init__(self) -> None:
        self.values: dict[str, str] = {}

    def get(self, var: str) -> str:
        return self.values.get(var, "")

    def put(self, var: str, value: str) -> None:
        self.values[var] = value


def entry(url: str, env: str, agent: str, budget: float | None) -> dict[str, Any]:
    team: dict[str, Any] = {"team_alias": agent, "models": ["canonical"]}
    if budget is not None:
        team.update(max_budget=budget, budget_duration="30d")
    return {
        "env": env,
        "mint": "litellm",
        "check": f"{url}/key/info",
        "check_method": "GET",
        "export": False,
        "litellm": {"proxy": url, "team": team},
    }


def table(url: str) -> list[dict[str, Any]]:
    return [entry(url, "LITELLM_VIBE_KEY", "vibe", 255), entry(url, "LITELLM_LOCAL_KEY", "local", None)]


def test_missing_keys_get_a_team_each_and_a_key_named_for_this_host(proxy) -> None:
    state, url = proxy
    store = Store()
    keys_tool.mint_litellm(table(url), {"LITELLM_VIBE_KEY", "LITELLM_LOCAL_KEY"}, ADMIN, store, "box")
    teams = {t["team_alias"]: t for t in state.teams}
    assert teams["vibe"]["max_budget"] == 255 and teams["vibe"]["budget_duration"] == "30d"
    assert "max_budget" not in teams["local"]
    vibe = store.get("LITELLM_VIBE_KEY")
    assert state.keys[vibe]["key_alias"] == "vibe@box"
    assert state.keys[vibe]["team_id"] == teams["vibe"]["team_id"]
    # A key follows its team's access groups, so a change of the team's models needs no new key.
    assert state.keys[vibe]["models"] == ["all-team-models"]
    assert state.keys[store.get("LITELLM_LOCAL_KEY")]["key_alias"] == "local@box"


def test_the_old_shared_key_is_deleted_and_a_drifted_budget_is_put_back(proxy) -> None:
    state, url = proxy
    state.teams.append(
        {"team_alias": "vibe", "team_id": "t-old", "models": ["canonical"], "max_budget": 100, "budget_duration": "30d"}
    )
    state.keys["sk-shared"] = {"key_alias": "vibe"}
    store = Store()
    keys_tool.mint_litellm(table(url)[:1], {"LITELLM_VIBE_KEY"}, ADMIN, store, "box")
    assert "sk-shared" not in state.keys
    assert state.teams[0]["max_budget"] == 255 and len(state.teams) == 1
    assert state.keys[store.get("LITELLM_VIBE_KEY")]["team_id"] == "t-old"


def test_a_key_this_host_lost_is_replaced_not_duplicated(proxy) -> None:
    state, url = proxy
    state.keys["sk-lost"] = {"key_alias": "vibe@box"}
    store = Store()
    keys_tool.mint_litellm(table(url)[:1], {"LITELLM_VIBE_KEY"}, ADMIN, store, "box")
    assert state.by_alias("vibe@box") == [store.get("LITELLM_VIBE_KEY")]


def test_a_present_key_is_kept_while_the_team_still_converges(proxy) -> None:
    state, url = proxy
    store = Store()
    keys_tool.mint_litellm(table(url)[:1], set(), ADMIN, store, "box")
    assert [t["team_alias"] for t in state.teams] == ["vibe"]
    assert not state.keys and not store.values


def test_a_token_without_team_access_mints_nothing_and_stops_at_once(proxy, capsys) -> None:
    state, url = proxy
    state.refuse.add("/team/list")
    store = Store()
    keys_tool.mint_litellm(table(url), {"LITELLM_VIBE_KEY", "LITELLM_LOCAL_KEY"}, ADMIN, store, "box")
    assert not state.keys and not store.values
    assert state.calls == ["GET /team/list"]
    assert "HTTP 403: not allowed" in capsys.readouterr().err


def test_without_a_token_nothing_is_asked_when_no_key_is_missing(proxy, monkeypatch) -> None:
    state, url = proxy
    monkeypatch.delenv("LITELLM_ADMIN_TOKEN", raising=False)
    monkeypatch.setattr(keys_tool, "ask", lambda *a, **k: pytest.fail("asked for a token"))
    keys_tool.ensure_litellm(table(url), set(), Store())
    assert state.calls == []


def test_a_skipped_token_mints_nothing(proxy, monkeypatch) -> None:
    state, url = proxy
    monkeypatch.delenv("LITELLM_ADMIN_TOKEN", raising=False)
    monkeypatch.setattr(keys_tool, "ask", lambda *a, **k: "")
    store = Store()
    keys_tool.ensure_litellm(table(url), {"LITELLM_VIBE_KEY"}, store)
    assert state.calls == [] and not store.values


def test_a_token_in_the_environment_mints_without_asking(proxy, monkeypatch) -> None:
    state, url = proxy
    monkeypatch.setenv("LITELLM_ADMIN_TOKEN", ADMIN)
    monkeypatch.setattr(keys_tool, "ask", lambda *a, **k: pytest.fail("asked for a token"))
    monkeypatch.setattr(keys_tool, "host_name", lambda: "box")
    store = Store()
    keys_tool.ensure_litellm(table(url), {"LITELLM_VIBE_KEY"}, store)
    assert state.keys[store.get("LITELLM_VIBE_KEY")]["key_alias"] == "vibe@box"
