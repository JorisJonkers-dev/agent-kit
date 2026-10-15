"""vibe-retain-private: Vibe's automatic memory goes to the private Hindsight bank only.

A Vibe post_agent hook retains the turn that just ended into `private`, appended
to one document per session. It has no path to the shared bank: no other host,
no other bank, no other token. Any failure leaves the session alone.
"""

from __future__ import annotations

import json
import os
import subprocess
import sys
import threading
import tomllib
from collections.abc import Iterator
from http.server import BaseHTTPRequestHandler, HTTPServer
from pathlib import Path

import pytest

KIT_ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(KIT_ROOT / "scripts"))

import render_registry  # noqa: E402
import yaml  # noqa: E402

HOOK = KIT_ROOT / "installer" / "vibe-retain-private"
HOOKS_TOML = KIT_ROOT / "installer" / "vibe-hooks.toml"
MERGE = KIT_ROOT / "scripts" / "vibe-merge-mcp.py"


class _Bank:
    def __init__(self, status: int = 200) -> None:
        self.status = status
        self.requests: list[dict] = []


@pytest.fixture
def bank() -> Iterator[tuple[_Bank, str]]:
    state = _Bank()

    class Handler(BaseHTTPRequestHandler):
        def do_POST(self) -> None:
            body = self.rfile.read(int(self.headers["Content-Length"]))
            state.requests.append({"path": self.path, "headers": dict(self.headers), "body": json.loads(body)})
            self.send_response(state.status)
            self.send_header("Content-Type", "application/json")
            self.end_headers()
            self.wfile.write(b'{"success": true}')

        def log_message(self, format: str, *args: object) -> None:
            pass

    server = HTTPServer(("127.0.0.1", 0), Handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    yield state, f"http://127.0.0.1:{server.server_port}"
    server.shutdown()


def _transcript(tmp_path: Path, messages: list[dict]) -> Path:
    path = tmp_path / "messages.jsonl"
    path.write_text("".join(json.dumps(m) + "\n" for m in messages))
    return path


TWO_TURNS = [
    {"role": "user", "content": "Lunch with Sam was moved to Friday."},
    {"role": "assistant", "content": "Noted: lunch with Sam is on Friday."},
    {"role": "user", "content": "Anna starts at the new job on the 3rd."},
    {"role": "assistant", "content": "", "tool_calls": [{"id": "c1", "function": {"name": "bash"}}]},
    {"role": "tool", "content": "SECRET FILE CONTENTS", "tool_call_id": "c1"},
    {"role": "user", "content": "Try again", "injected": True},
    {"role": "assistant", "content": "Noted: Anna starts on the 3rd."},
]


def _run(
    tmp_path: Path,
    url: str | None,
    messages: list[dict] | None = None,
    *,
    token: str | None = "smt_private",
    extra: dict | None = None,
    env: dict | None = None,
) -> subprocess.CompletedProcess:
    transcript = _transcript(tmp_path, TWO_TURNS if messages is None else messages)
    hook_input = {
        "hook_event_name": "post_agent",
        "session_id": "sess-1",
        "parent_session_id": None,
        "transcript_path": str(transcript),
        "cwd": str(tmp_path),
        **(extra or {}),
    }
    run_env = {k: v for k, v in os.environ.items() if k not in {"MEMORY_PRIVATE_TOKEN", "HINDSIGHT_API_TOKEN"}}
    if url is not None:
        run_env["AGENT_KIT_PRIVATE_BANK_URL"] = url
    if token is not None:
        run_env["MEMORY_PRIVATE_TOKEN"] = token
    run_env.update(env or {})
    return subprocess.run(
        [sys.executable, str(HOOK)], input=json.dumps(hook_input), capture_output=True, text=True,
        check=False, env=run_env, timeout=30,
    )


def test_the_last_turn_is_appended_to_the_session_document_in_the_private_bank(
    tmp_path: Path, bank: tuple[_Bank, str],
) -> None:
    state, url = bank
    result = _run(tmp_path, url)
    assert result.returncode == 0, result.stderr
    assert result.stdout == ""
    [request] = state.requests
    assert request["path"] == "/v1/default/banks/private/memories"
    assert request["headers"]["Authorization"] == "Bearer smt_private"
    assert not request["headers"]["User-Agent"].startswith("Python-urllib")
    body = request["body"]
    assert body["async"] is True
    [item] = body["items"]
    assert item["document_id"] == "vibe-sess-1"
    assert item["update_mode"] == "append"
    assert item["context"] == "vibe"
    assert "Anna starts at the new job on the 3rd." in item["content"]
    assert "Noted: Anna starts on the 3rd." in item["content"]
    assert "Sam" not in item["content"], "an earlier turn was already retained"
    assert "SECRET FILE CONTENTS" not in item["content"], "tool output is not retained"
    assert "Try again" not in item["content"], "injected messages are not the user's"


def test_a_repeat_of_the_same_turn_is_the_same_operation(tmp_path: Path, bank: tuple[_Bank, str]) -> None:
    state, url = bank
    _run(tmp_path, url)
    _run(tmp_path, url)
    _run(tmp_path, url, TWO_TURNS[:2])
    first, again, other = (r["body"]["operation_id"] for r in state.requests)
    assert first == again
    assert first != other


def test_without_the_private_token_nothing_is_sent_anywhere(tmp_path: Path, bank: tuple[_Bank, str]) -> None:
    state, url = bank
    result = _run(tmp_path, url, token=None, env={"HINDSIGHT_API_TOKEN": "smt_shared"})
    assert result.returncode == 0
    assert state.requests == []
    assert "MEMORY_PRIVATE_TOKEN" in json.loads(result.stdout)["system_message"]


@pytest.mark.parametrize("status", [403, 500])
def test_a_refused_retain_is_reported_and_the_turn_goes_on(
    tmp_path: Path, bank: tuple[_Bank, str], status: int,
) -> None:
    state, url = bank
    state.status = status
    result = _run(tmp_path, url)
    assert result.returncode == 0
    assert len(state.requests) == 1
    note = json.loads(result.stdout)["system_message"]
    assert "not retained" in note and str(status) in note


def test_an_unreachable_private_bank_is_reported_and_the_turn_goes_on(tmp_path: Path) -> None:
    result = _run(tmp_path, "http://127.0.0.1:9")
    assert result.returncode == 0
    assert "not retained" in json.loads(result.stdout)["system_message"]


def test_a_subagent_turn_is_left_to_its_parent(tmp_path: Path, bank: tuple[_Bank, str]) -> None:
    state, url = bank
    assert _run(tmp_path, url, extra={"parent_session_id": "sess-0"}).returncode == 0
    assert state.requests == []


def test_no_transcript_means_nothing_to_retain(tmp_path: Path, bank: tuple[_Bank, str]) -> None:
    state, url = bank
    result = _run(tmp_path, url, extra={"transcript_path": ""})
    assert (result.returncode, result.stdout, state.requests) == (0, "", [])


def test_the_hook_has_no_route_to_the_shared_bank() -> None:
    source = HOOK.read_text()
    assert "https://hindsight-private.jorisjonkers.dev" in source
    for shared in ("hindsight-api", "banks/default", "HINDSIGHT_API_TOKEN", "HINDSIGHT_API_URL"):
        assert shared not in source


def test_the_bank_url_override_is_never_used_for_another_bank(tmp_path: Path, bank: tuple[_Bank, str]) -> None:
    state, url = bank
    _run(tmp_path, url + "/v1/default/banks/default")
    assert all(r["path"].endswith("/banks/private/memories") for r in state.requests)
    assert all("/banks/default/" not in r["path"] for r in state.requests)


# --- installation ---


def _setup_script() -> str:
    registry = yaml.safe_load((KIT_ROOT / "registry" / "estate-tooling.yaml").read_text())
    return render_registry.render_setup_script(registry)


def _hooks() -> list[dict]:
    return tomllib.loads(HOOKS_TOML.read_text())["hooks"]


def test_vibe_runs_it_after_every_turn() -> None:
    [hook] = _hooks()
    assert hook["type"] == "post_agent"
    assert hook["command"] == '"$HOME/.local/bin/vibe-retain-private"'
    assert "match" not in hook and "strict" not in hook, "Vibe refuses both on post_agent"
    assert hook["timeout"] <= 30


def test_setup_installs_the_hook_and_merges_it_into_vibe() -> None:
    script = _setup_script()
    assert "install_kit_command vibe-retain-private" in script
    assert '"${KIT_ROOT}/installer/vibe-hooks.toml" --key hooks --config "${VIBE_HOME}/hooks.toml"' in script
    name = _hooks()[0]["name"]
    assert f'[ "${{CLOUD}}" = 1 ] && vibe_hooks_remove=(--remove {name})' in script


def test_the_bundle_ships_the_hook() -> None:
    publish = (KIT_ROOT / "scripts" / "publish-installer-artifacts.sh").read_text()
    paths = publish.split("KIT_PATHS=(")[1].split(")")[0].split()
    assert "installer/vibe-retain-private" in paths
    assert "installer/vibe-hooks.toml" in paths


def test_the_hook_file_carries_the_install_marker() -> None:
    assert "# managed by agent-kit setup-workstation.sh" in HOOK.read_text().splitlines()


def test_merging_hooks_keeps_the_users_own(tmp_path: Path) -> None:
    config = tmp_path / "hooks.toml"
    config.write_text('[[hooks]]\nname = "lint"\ntype = "post_agent"\ncommand = "make lint"\n')
    args = [sys.executable, str(MERGE), str(HOOKS_TOML), "--key", "hooks", "--config", str(config)]
    assert subprocess.run(args, capture_output=True, check=False).returncode == 0
    assert subprocess.run(args, capture_output=True, check=False).returncode == 0
    names = [h["name"] for h in tomllib.loads(config.read_text())["hooks"]]
    assert names == ["lint", _hooks()[0]["name"]]


def test_vibe_is_told_to_recall_from_both_banks(tmp_path: Path) -> None:
    # Vibe has no prompt hook, so recall is the model's call; AGENTS.md asks for it.
    script = _setup_script()
    fn = "_vibe_agents_pointer() {" + script.split("_vibe_agents_pointer() {")[1].split("\n}\n")[0] + "\n}\n"
    bin_dir = tmp_path / "bin"
    bin_dir.mkdir()
    for name in ("memory-private", "memory-api"):
        (bin_dir / name).write_text(f"#!/bin/sh\ntouch {tmp_path}/ran-{name}\n")
        (bin_dir / name).chmod(0o755)
    body = f'CHECK_ONLY=0\nVIBE_HOME="{tmp_path}/vibe"\nok() {{ :; }}\nlog() {{ :; }}\n{fn}\n_vibe_agents_pointer\n'
    env = {**os.environ, "PATH": f"{bin_dir}:{os.environ['PATH']}"}
    assert subprocess.run(["bash", "-c", body], check=False, env=env).returncode == 0
    text = (tmp_path / "vibe" / "AGENTS.md").read_text()
    assert "memory-private" in text and "memory-api" in text
    assert "Never retain personal notes into memory-api" in " ".join(text.split())
    assert not list(tmp_path.glob("ran-*")), "the heredoc ran a command"
