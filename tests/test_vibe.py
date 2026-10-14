"""Mistral Vibe: installed and configured from the registry like the other agents."""

from __future__ import annotations

import subprocess
import sys
import tomllib
from pathlib import Path

import yaml

KIT_ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(KIT_ROOT / "scripts"))

import render_registry  # noqa: E402

MERGE = KIT_ROOT / "scripts" / "vibe-merge-mcp.py"


def _registry() -> dict:
    return yaml.safe_load((KIT_ROOT / "registry" / "estate-tooling.yaml").read_text())


def _vibe_servers(data: dict) -> list[dict]:
    return [
        s
        for s in data["mcp_servers"]
        if s.get("enabled") is not False
        and "workstation" in (s.get("surfaces") or [])
    ]


def test_vibe_is_installed_with_uv_on_the_workstation() -> None:
    cli = next(c for c in _registry()["clis"] if c["name"] == "mistral-vibe")
    assert cli["binary"] == "vibe"
    assert "workstation" in cli["surfaces"]
    assert cli["install"].startswith("uv tool install")
    assert ">=3.12" in cli["install"], "mistral-vibe needs Python 3.12+"
    script = render_registry.render_setup_script(_registry())
    assert render_registry._q(cli["install"]) in script


def test_vibe_warns_by_name_when_mistral_key_is_unset() -> None:
    script = render_registry.render_setup_script(_registry())
    assert 'if [ -z "${MISTRAL_API_KEY:-}" ]; then' in script
    assert 'missing_plugin_env+=("mistral-vibe: export MISTRAL_API_KEY")' in script


def test_vibe_mcp_fragment_covers_every_workstation_server() -> None:
    data = _registry()
    fragment = tomllib.loads(render_registry.render_vibe_mcp(data))
    rendered = {s["name"]: s for s in fragment["mcp_servers"]}
    assert set(rendered) == {s["name"] for s in _vibe_servers(data)}
    for server in _vibe_servers(data):
        got = rendered[server["name"]]
        if server["transport"] == "http":
            assert got["transport"] == "streamable-http"
            assert got["url"] == server["url_workstation"]
            if server.get("credential"):
                assert got["auth"] == {
                    "type": "static",
                    "api_key_env": server["credential"],
                    "api_key_header": "Authorization",
                    "api_key_format": "Bearer {token}",
                }
        else:
            assert got["transport"] == "stdio"
            assert got["command"] == server["command"]
            assert got["args"] == (server.get("args") or [])
            # The MCP SDK hands a stdio server only a safe allowlist of the parent
            # environment plus this table, so static env must be rendered.
            assert got.get("env", {}) == (server.get("env") or {})
        if server.get("timeout"):
            assert got["tool_timeout_sec"] == float(server["timeout"])
        assert server.get("credential") not in (got.get("env") or {}), "no secret is written to disk"


def test_vibe_fragment_is_a_generated_artifact() -> None:
    assert render_registry.VIBE_MCP in render_registry.artifacts(_registry())


def _merge(tmp_path: Path, *extra: str) -> subprocess.CompletedProcess:
    fragment = tmp_path / "fragment.toml"
    fragment.write_text(render_registry.render_vibe_mcp(_registry()))
    return subprocess.run(
        ["uv", "run", "--no-project", "--with", "tomlkit", "python", str(MERGE), str(fragment),
         "--config", str(tmp_path / "config.toml"), *extra],
        capture_output=True, text=True, check=False,
    )


def test_merge_keeps_the_users_settings_and_servers(tmp_path: Path) -> None:
    config = tmp_path / "config.toml"
    config.write_text(
        'active_model = "mistral-medium-3.5"\n\n'
        "[[mcp_servers]]\nname = \"mine\"\ntransport = \"stdio\"\ncommand = \"my-mcp\"\n\n"
        "[[mcp_servers]]\nname = \"memory-api\"\ntransport = \"http\"\nurl = \"https://stale.example/mcp\"\n"
    )
    result = _merge(tmp_path)
    assert result.returncode == 0, result.stdout + result.stderr
    data = tomllib.loads(config.read_text())
    assert data["active_model"] == "mistral-medium-3.5"
    servers = {s["name"]: s for s in data["mcp_servers"]}
    assert servers["mine"]["command"] == "my-mcp"
    assert servers["memory-api"]["url"].endswith("/mcp/default/")
    assert [s["name"] for s in data["mcp_servers"]].count("memory-api") == 1
    assert set(servers) >= {s["name"] for s in _vibe_servers(_registry())}


def test_merge_is_idempotent_and_creates_the_config(tmp_path: Path) -> None:
    assert _merge(tmp_path).returncode == 0
    first = (tmp_path / "config.toml").read_text()
    assert _merge(tmp_path).returncode == 0
    assert (tmp_path / "config.toml").read_text() == first


def test_merge_is_idempotent_beside_the_users_own_settings(tmp_path: Path) -> None:
    config = tmp_path / "config.toml"
    config.write_text('# my vibe\nactive_model = "mistral-medium-3.5"\n\n[tools.bash]\npermission = "ask"\n')
    assert _merge(tmp_path).returncode == 0
    first = config.read_text()
    assert _merge(tmp_path).returncode == 0
    assert config.read_text() == first
    assert first.startswith("# my vibe\n")


def test_merge_keeps_the_first_backup(tmp_path: Path) -> None:
    config = tmp_path / "config.toml"
    config.write_text('active_model = "original"\n')
    assert _merge(tmp_path).returncode == 0
    assert _merge(tmp_path).returncode == 0
    assert (tmp_path / "config.toml.bak").read_text() == 'active_model = "original"\n'


def test_cloud_mode_merges_once_with_its_removals() -> None:
    script = render_registry.render_setup_script(_registry())
    assert script.count("scripts/vibe-merge-mcp.py\"") == 1
    assert 'vibe_remove=(--remove kubernetes)' in script


def test_merge_can_remove_a_managed_server(tmp_path: Path) -> None:
    assert _merge(tmp_path).returncode == 0
    assert _merge(tmp_path, "--remove", "kubernetes").returncode == 0
    names = {s["name"] for s in tomllib.loads((tmp_path / "config.toml").read_text())["mcp_servers"]}
    assert "kubernetes" not in names


def test_setup_merges_vibe_servers_when_vibe_is_on_path() -> None:
    script = render_registry.render_setup_script(_registry())
    assert "if command -v vibe >/dev/null 2>&1; then" in script
    assert 'scripts/vibe-merge-mcp.py" \\' in script
    assert f'"${{KIT_ROOT}}/{render_registry.VIBE_MCP}"' in script


def test_first_party_skills_reach_vibe() -> None:
    data = _registry()
    script = render_registry.render_setup_script(data)
    assert 'VIBE_HOME="${VIBE_HOME:-$HOME/.vibe}"' in script
    for skill in data["local_skills"]:
        if "workstation" in (skill.get("surfaces") or []):
            assert "vibe" in skill["agents"], f"{skill['name']} does not reach vibe"
            assert f'dest="${{VIBE_HOME}}/skills/{skill["name"]}"' in script


def _agents_pointer(tmp_path: Path) -> subprocess.CompletedProcess:
    script = render_registry.render_setup_script(_registry())
    start = script.index("_vibe_agents_pointer() {")
    fn = script[start : script.index("\n}\n", start) + 3]
    stubs = 'ok() { echo "ok $*"; }\nlog() { echo "log $*"; }\n'
    body = f'CHECK_ONLY=0\nVIBE_HOME="{tmp_path}"\n{stubs}{fn}\n_vibe_agents_pointer\n'
    return subprocess.run(["bash", "-c", body], capture_output=True, text=True, check=False)


def test_vibe_gets_the_estate_agents_pointer(tmp_path: Path) -> None:
    assert _agents_pointer(tmp_path).returncode == 0
    text = (tmp_path / "AGENTS.md").read_text()
    assert "github.com/JorisJonkers-dev/workspace/blob/main/CLAUDE.md" in text


def test_vibe_agents_pointer_never_overwrites_the_users_own(tmp_path: Path) -> None:
    (tmp_path / "AGENTS.md").write_text("my own instructions\n")
    assert _agents_pointer(tmp_path).returncode == 0
    assert (tmp_path / "AGENTS.md").read_text() == "my own instructions\n"


def test_cloud_environment_carries_the_mistral_key() -> None:
    assert "MISTRAL_API_KEY=" in (KIT_ROOT / "cloud" / "environment.env").read_text()
    helper = (KIT_ROOT / "installer" / "cloud-session.sh").read_text()
    assert "MISTRAL_API_KEY" in helper.split("AK_CLOUD_VARS=(")[1].split(")")[0]
