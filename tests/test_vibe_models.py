"""Vibe offers every Mistral chat model the key can use, kept current by setup.

Vibe only lists the models under [[models]] in its config; its default is one
(Mistral Medium 3.5). scripts/vibe-sync-models.py reads Mistral's /v1/models
with MISTRAL_API_KEY and writes one entry per underlying model, preferring the
-latest id so upgrades arrive without a change here. Your own entries and
everything else in the config are kept.
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
SYNC = KIT_ROOT / "scripts" / "vibe-sync-models.py"
sys.path.insert(0, str(KIT_ROOT / "scripts"))

import render_registry  # noqa: E402
import yaml  # noqa: E402


def _model(mid: str, name: str, aliases: list[str], *, fc: bool = True, vision: bool = False,
           reasoning: bool = False, transcription: bool = False, ctx: int = 262144) -> dict:
    return {
        "id": mid, "name": name, "aliases": aliases, "max_context_length": ctx, "deprecation": None,
        "capabilities": {"completion_chat": True, "function_calling": fc, "vision": vision,
                         "reasoning": reasoning, "audio_transcription": transcription},
    }


MODELS = [
    _model("mistral-vibe-cli-latest", "mistral-medium-latest", ["mistral-medium-latest"], vision=True, reasoning=True),
    _model("mistral-medium-latest", "mistral-medium-latest", ["mistral-vibe-cli-latest"], vision=True, reasoning=True),
    _model("mistral-large-latest", "mistral-large-2512", ["mistral-large-2512"], vision=True),
    _model("mistral-large-2512", "mistral-large-2512", ["mistral-large-latest"], vision=True),
    _model("codestral-latest", "codestral-2508", ["codestral-2508", "mistral-code-latest"], ctx=256000),
    _model("mistral-code-latest", "codestral-2508", ["codestral-2508", "codestral-latest"], ctx=256000),
    _model("glm-5-2", "glm-5-2", ["zai-glm-5-2"], reasoning=True, ctx=1048576),
    _model("zai-glm-5-2", "glm-5-2", ["glm-5-2"], reasoning=True, ctx=1048576),
    _model("voxtral-small-latest", "Voxtral Small Transcribe", [], transcription=True, ctx=32768),
    _model("mistral-embed", "mistral-embed", [], fc=False),
]


class _Api:
    def __init__(self) -> None:
        self.status = 200
        self.auth: list[str] = []


@pytest.fixture
def api() -> Iterator[tuple[_Api, str]]:
    state = _Api()

    class Handler(BaseHTTPRequestHandler):
        def do_GET(self) -> None:
            state.auth.append(self.headers.get("Authorization", ""))
            if self.headers.get("User-Agent", "").startswith("Python-urllib"):
                self.send_response(403)
                self.end_headers()
                return
            self.send_response(state.status)
            self.send_header("Content-Type", "application/json")
            self.end_headers()
            if state.status == 200:
                self.wfile.write(json.dumps({"object": "list", "data": MODELS}).encode())

        def log_message(self, format: str, *args: object) -> None:
            pass

    server = HTTPServer(("127.0.0.1", 0), Handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    yield state, f"http://127.0.0.1:{server.server_port}/v1/models"
    server.shutdown()


def _sync(tmp_path: Path, url: str, *, key: str | None = "mk_test") -> subprocess.CompletedProcess:
    env = {k: v for k, v in os.environ.items() if k != "MISTRAL_API_KEY"}
    env.update({"HOME": str(tmp_path / "home"), "AGENT_KIT_MISTRAL_MODELS_URL": url})
    if key:
        env["MISTRAL_API_KEY"] = key
    return subprocess.run([sys.executable, str(SYNC), "--config", str(tmp_path / "config.toml")],
                          capture_output=True, text=True, check=False, env=env, timeout=60)


def _models(tmp_path: Path) -> dict[str, dict]:
    return {m["alias"]: m for m in tomllib.loads((tmp_path / "config.toml").read_text()).get("models", [])}


def test_one_entry_per_underlying_model_preferring_latest(tmp_path: Path, api: tuple[_Api, str]) -> None:
    state, url = api
    result = _sync(tmp_path, url)
    assert result.returncode == 0, result.stderr
    assert set(_models(tmp_path)) == {"mistral-large-latest", "codestral-latest", "glm-5-2"}
    assert state.auth == ["Bearer mk_test"]


def test_vibe_default_tools_free_and_transcription_models_are_left_out(tmp_path: Path, api: tuple[_Api, str]) -> None:
    _, url = api
    _sync(tmp_path, url)
    names = {m["name"] for m in _models(tmp_path).values()}
    assert not names & {"mistral-vibe-cli-latest", "mistral-medium-latest", "voxtral-small-latest", "mistral-embed"}


def test_entries_carry_what_vibe_needs(tmp_path: Path, api: tuple[_Api, str]) -> None:
    _, url = api
    _sync(tmp_path, url)
    large, glm = _models(tmp_path)["mistral-large-latest"], _models(tmp_path)["glm-5-2"]
    assert large["provider"] == "mistral" and large["name"] == "mistral-large-latest"
    assert large["supports_images"] is True and large["max_context_length"] == 262144
    assert large["thinking_levels"] == ["off"]
    assert glm["thinking_levels"] == ["off", "high"] and glm["supports_images"] is False


def test_your_own_models_and_settings_are_kept_and_a_rerun_changes_nothing(
    tmp_path: Path, api: tuple[_Api, str],
) -> None:
    _, url = api
    (tmp_path / "config.toml").write_text(
        'active_model = "mine"\n\n[[models]]\nname = "my-local"\nprovider = "llamacpp"\nalias = "mine"\n\n'
        '[[models]]\nname = "mistral-large-latest"\nprovider = "mistral"\nalias = "mistral-large-latest"\n'
        "temperature = 0.7\n",
    )
    _sync(tmp_path, url)
    once = (tmp_path / "config.toml").read_text()
    models = _models(tmp_path)
    assert models["mine"]["provider"] == "llamacpp"
    assert models["mistral-large-latest"]["temperature"] == 0.7, "a model you defined yourself wins"
    assert tomllib.loads(once)["active_model"] == "mine"
    _sync(tmp_path, url)
    assert (tmp_path / "config.toml").read_text() == once


def test_a_model_that_disappears_upstream_is_removed(tmp_path: Path, api: tuple[_Api, str]) -> None:
    _, url = api
    _sync(tmp_path, url)
    gone = MODELS.pop(4), MODELS.pop(4)  # both codestral ids
    try:
        _sync(tmp_path, url)
    finally:
        MODELS[4:4] = list(gone)
    assert "codestral-latest" not in _models(tmp_path)


def test_an_unreachable_api_changes_nothing(tmp_path: Path, api: tuple[_Api, str]) -> None:
    state, url = api
    _sync(tmp_path, url)
    before = (tmp_path / "config.toml").read_text()
    state.status = 500
    result = _sync(tmp_path, url)
    assert result.returncode == 0
    assert "not synced" in result.stderr
    assert (tmp_path / "config.toml").read_text() == before


def test_without_a_key_nothing_is_fetched(tmp_path: Path, api: tuple[_Api, str]) -> None:
    state, url = api
    result = _sync(tmp_path, url, key=None)
    assert result.returncode == 0
    assert state.auth == []
    assert "MISTRAL_API_KEY" in result.stderr


def test_the_key_never_reaches_output(tmp_path: Path, api: tuple[_Api, str]) -> None:
    _, url = api
    result = _sync(tmp_path, url, key="mk_secret_value")
    assert "mk_secret_value" not in result.stdout + result.stderr
    assert "mk_secret_value" not in (tmp_path / "config.toml").read_text()


def test_setup_syncs_after_merging_the_mcp_servers() -> None:
    script = render_registry.render_setup_script(
        yaml.safe_load((KIT_ROOT / "registry" / "estate-tooling.yaml").read_text()),
    )
    shared = script[script.index('ok "Vibe MCP servers merged"') :]
    assert shared.index('_vibe_models "${VIBE_HOME}"') < shared.index("\n    fi\n")
    private = script[script.index('ok "vibe-private MCP servers merged"') :]
    assert private.index('_vibe_models "${VIBE_PRIVATE_HOME}"') < private.index("\n}\n")
    fn = script[script.index("_vibe_models() {") :]
    fn = fn[: fn.index("\n}\n")]
    sync = 'uv run --no-project --with tomlkit python "${KIT_ROOT}/scripts/vibe-sync-models.py"'
    assert f'VIBE_HOME="${{home}}" {sync}' in fn
    assert '--config "${home}/config.toml"' in fn
    publish = (KIT_ROOT / "scripts" / "publish-installer-artifacts.sh").read_text()
    assert "scripts/vibe-sync-models.py" in publish.split("KIT_PATHS=(")[1].split(")")[0].split()


# --- the key is Vibe's own login, not one setup asked for ---


def _vibe_login(tmp_path: Path, *, keychain: str | None = None, env_file: str | None = None) -> dict:
    bin_dir = tmp_path / "bin"
    bin_dir.mkdir(exist_ok=True)
    found = f'printf "%s\\n" "{keychain}"; exit 0' if keychain else "exit 44"
    (bin_dir / "security").write_text(
        "#!/bin/sh\n"
        'case "$*" in *"-s ai.mistral.vibe -a MISTRAL_API_KEY -w"*) ' + found + " ;; esac\nexit 44\n",
    )
    (bin_dir / "security").chmod(0o755)
    home = tmp_path / "home"
    (home / ".vibe").mkdir(parents=True, exist_ok=True)
    if env_file:
        (home / ".vibe" / ".env").write_text(f"MISTRAL_API_KEY='{env_file}'\n")
    return {"PATH": f"{bin_dir}:{os.environ['PATH']}"}


def _sync_env(tmp_path: Path, url: str, extra: dict, *args: str) -> subprocess.CompletedProcess:
    env = {k: v for k, v in os.environ.items() if k != "MISTRAL_API_KEY"}
    env.update({"HOME": str(tmp_path / "home"), "AGENT_KIT_MISTRAL_MODELS_URL": url, **extra})
    return subprocess.run([sys.executable, str(SYNC), "--config", str(tmp_path / "config.toml"), *args],
                          capture_output=True, text=True, check=False, env=env, timeout=60)


def test_the_key_vibe_signed_in_with_is_used(tmp_path: Path, api: tuple[_Api, str]) -> None:
    state, url = api
    result = _sync_env(tmp_path, url, _vibe_login(tmp_path, keychain="mk_login"))
    assert result.returncode == 0, result.stderr
    assert state.auth == ["Bearer mk_login"]


def test_vibes_env_file_is_the_fallback(tmp_path: Path, api: tuple[_Api, str]) -> None:
    state, url = api
    _sync_env(tmp_path, url, _vibe_login(tmp_path, env_file="mk_envfile"))
    assert state.auth == ["Bearer mk_envfile"]


def test_has_key_answers_without_fetching(tmp_path: Path, api: tuple[_Api, str]) -> None:
    state, url = api
    assert _sync_env(tmp_path, url, _vibe_login(tmp_path), "--has-key").returncode == 1
    assert _sync_env(tmp_path, url, _vibe_login(tmp_path, keychain="mk_login"), "--has-key").returncode == 0
    assert state.auth == []


def test_setup_checks_vibe_is_signed_in_instead_of_asking_for_a_key() -> None:
    script = render_registry.render_setup_script(
        yaml.safe_load((KIT_ROOT / "registry" / "estate-tooling.yaml").read_text()),
    )
    assert 'missing_plugin_env+=("mistral-vibe: export MISTRAL_API_KEY")' not in script
    assert '--config "${home}/config.toml" --has-key; then' in script
    assert "sign in with your Mistral account" in script
