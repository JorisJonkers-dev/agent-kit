"""Drift guard for the generated registry artifacts."""

from __future__ import annotations

import copy
import shutil
import subprocess
import sys
from pathlib import Path

import pytest
import yaml

KIT_ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(KIT_ROOT / "scripts"))

import render_registry  # noqa: E402


def test_generated_artifacts_match_the_registry() -> None:
    """A hand edit to a generated file, or an unrendered registry change, fails here."""
    result = subprocess.run(
        [sys.executable, str(KIT_ROOT / "scripts" / "render_registry.py"), "--check"],
        capture_output=True,
        text=True,
        check=False,
    )
    assert result.returncode == 0, result.stdout + result.stderr


def test_registry_validates() -> None:
    render_registry.load_registry()


def _registry() -> dict:
    return yaml.safe_load((KIT_ROOT / "registry" / "estate-tooling.yaml").read_text())


def test_tag_object_sha_is_rejected() -> None:
    """The pin must be the COMMIT sha.

    Hermes clones `--branch <ref>` and compares `git rev-parse HEAD`, so an
    annotated tag's own object sha never matches and every such source is
    rejected at sync time. Catching it here instead means the mistake is a
    failed test rather than a Hermes pod that silently vendors nothing.
    """
    data = _registry()
    data["skill_sources"][0]["commit"] = "not-a-sha"
    with pytest.raises(render_registry.RegistryError, match="40-char commit sha"):
        render_registry.validate(data)


def test_unknown_surface_is_rejected() -> None:
    data = _registry()
    data["mcp_servers"][0]["surfaces"] = ["workstaton"]
    with pytest.raises(render_registry.RegistryError, match="unknown surfaces"):
        render_registry.validate(data)


def test_plugin_must_name_a_declared_marketplace() -> None:
    data = _registry()
    data["plugins"][0]["marketplace"] = "nope"
    with pytest.raises(render_registry.RegistryError, match="unknown marketplace"):
        render_registry.validate(data)


def test_lsp_plugin_may_not_be_listed_twice() -> None:
    data = _registry()
    data["plugins"].append({"name": "typescript-lsp", "marketplace": "claude-plugins-official"})
    with pytest.raises(render_registry.RegistryError, match="both plugins:"):
        render_registry.validate(data)


def test_hermes_http_server_needs_a_hermes_url() -> None:
    data = _registry()
    server = next(s for s in data["mcp_servers"] if "hermes" in s["surfaces"] and s["transport"] == "http")
    del server["url_hermes"]
    with pytest.raises(render_registry.RegistryError, match="no url_hermes"):
        render_registry.render_hermes_mcp(data)


def test_retired_knowledge_server_reaches_no_surface() -> None:
    """`surfaces: []` is the whole mechanism for retiring a server.

    If it grows a surface again the generated artifacts start registering the
    dead knowledge base, so assert the absence rather than trusting the edit.
    """
    data = _registry()
    knowledge = next(s for s in data["mcp_servers"] if s["name"] == "knowledge")
    assert knowledge["surfaces"] == []
    # Match the server KEY, not the word: the memory server's comment
    # legitimately mentions the retired knowledge base.
    assert "\n  knowledge:\n" not in render_registry.render_hermes_mcp(data)
    assert "kb.jorisjonkers.dev" not in render_registry.render_setup_script(data)


def test_spec_kit_is_not_in_the_registry() -> None:
    """Spec Kit is deliberately not installed on any surface."""
    text = (KIT_ROOT / "registry" / "estate-tooling.yaml").read_text()
    data = _registry()
    names = [
        item.get("name", "")
        for key in ("clis", "plugins", "skill_sources", "mcp_servers")
        for item in (data.get(key) or [])
    ]
    assert not any("speckit" in name or "spec-kit" in name for name in names)
    assert "Spec Kit is deliberately absent" in text


def test_setup_script_declares_every_credential_it_needs() -> None:
    data = copy.deepcopy(_registry())
    script = render_registry.render_setup_script(data)
    for server in data["mcp_servers"]:
        if "workstation" not in server["surfaces"]:
            continue
        credential = server.get("credential")
        if credential:
            assert credential in script


def test_overleaf_cookie_name_is_the_self_hosted_one() -> None:
    """The overleaf.com cookie name breaks auth on the self-hosted instance.

    olcli-mcp reads OVERLEAF_COOKIE_NAME from its env over its own stored
    config: `overleaf_session2` makes `olcli whoami` report "Session invalid"
    while `overleaf.sid` authenticates (verified live). Every generated surface
    must carry the self-hosted name, never the overleaf.com one.
    """
    data = _registry()
    for surface in (
        render_registry.render_setup_script(data),
        render_registry.render_hermes_mcp(data),
        render_registry.render_hermes_local_mcp(data),
    ):
        assert "overleaf_session2" not in surface
        assert "overleaf.sid" in surface


def test_local_hermes_mcp_uses_workstation_command_not_npx() -> None:
    """Local Hermes has olcli on PATH, so it uses the command directly.

    The gateway artifact routes npx because the image has no global packages;
    the LOCAL fragment must use the database `command` for overleaf (already on
    PATH locally), not npx.
    """
    data = _registry()
    fragment = render_registry.render_hermes_local_mcp(data)
    assert 'command: "olcli-mcp"' in fragment
    overleaf_entry = fragment.split("  overleaf:")[1].split("\n\n")[0]
    assert 'command: "olcli-mcp"' in overleaf_entry
    assert "npx" not in overleaf_entry


def test_setup_script_wires_every_workstation_server_into_all_three_agents() -> None:
    """The workstation surface reaches Claude Code, Codex and local Hermes.

    REGISTRY.md documents `workstation` = "Claude Code and Codex"; the MCP
    registration must therefore emit a command for each agent, not claude only.
    """
    data = _registry()
    script = render_registry.render_setup_script(data)
    for server in data["mcp_servers"]:
        if "workstation" not in server["surfaces"]:
            continue
        if server.get("enabled") is False:
            continue
        name = server["name"]
        assert f"claude mcp add --scope user {name}" in script
        assert f"codex mcp add {name}" in script
    # local-hermes merge is emitted once
    assert "hermes-merge-mcp.py" in script


def test_claude_env_flags_come_after_the_server_name() -> None:
    """claude 2.x rejects `--env K=V <name>` with 'missing required argument'.

    The name must precede the env flags (verified against claude 2.1.267), so a
    stdio server with env must be emitted `mcp add ... --scope user <name>
    --env K=V -- cmd`, never with the env flags first.
    """
    data = _registry()
    script = render_registry.render_setup_script(data)
    for server in data["mcp_servers"]:
        if (
            "workstation" not in server["surfaces"]
            or server["transport"] == "http"
            or server.get("enabled") is False
            or not (server.get("env") or server.get("credential"))
        ):
            continue
        name = server["name"]
        line = next(
            cand for cand in script.splitlines() if f"claude mcp add --scope user {name}" in cand
        )
        name_pos = line.index(f"--scope user {name}")
        env_pos = line.index("--env")
        # The --env flags that belong to THIS server come after the name.
        assert name_pos < env_pos < line.index(f"-- {server['command']}")


def test_port_forward_is_ensured_before_the_server_is_registered() -> None:
    """A loopback URL registered before its forward exists points at a dead port."""
    data = _registry()
    script = render_registry.render_setup_script(data)
    assert f'. "${{KIT_ROOT}}/{render_registry.PORT_FORWARD_HELPER}"' in script
    for server in data["mcp_servers"]:
        connect = server.get("workstation_connect")
        if not connect or "workstation" not in server["surfaces"]:
            continue
        name = server["name"]
        call = (
            f"ensure_port_forward {name} {connect['namespace']} {connect['svc']} "
            f"{connect['local_port']} {connect['remote_port']}"
        )
        assert call in script
        assert script.index(call) < script.index(f"claude mcp add --scope user {name}")
    # The helper is sourced before anything calls it.
    assert script.index("port-forward-agent.sh") < script.index("ensure_port_forward kubernetes")


# --- installer/port-forward-agent.sh, driven with stubbed system commands ---

_STUBS = {
    # launchctl keeps "loaded" state in a file; bootstrap also brings the port up.
    "launchctl": """#!/bin/bash
echo "$*" >> "$STATE/launchctl.log"
case "$1" in
  print) [ -f "$STATE/loaded" ] ;;
  bootstrap) touch "$STATE/loaded" "$STATE/up" ;;
  bootout) rm -f "$STATE/loaded" "$STATE/up" ;;
esac
""",
    "curl": '#!/bin/bash\n[ -f "$STATE/up" ]\n',
    "kubectl": """#!/bin/bash
if [ "$1 $2" = "config current-context" ]; then echo prod-ctx; exit 0; fi
echo "$*" >> "$STATE/kubectl.log"
""",
    "sleep": "#!/bin/bash\nexit 0\n",
}


def _run_helper(tmp_path: Path, os_name: str, check_only: int = 0) -> subprocess.CompletedProcess:
    bin_dir = tmp_path / "bin"
    state = tmp_path / "state"
    bin_dir.mkdir(exist_ok=True)
    state.mkdir(exist_ok=True)
    stubs = dict(_STUBS, uname=f"#!/bin/bash\necho {os_name}\n")
    for name, body in stubs.items():
        path = bin_dir / name
        path.write_text(body)
        path.chmod(0o755)
    script = f"""
CHECK_ONLY={check_only}
log()  {{ echo "log $*"; }}
ok()   {{ echo "ok $*"; }}
warn() {{ echo "warn $*"; }}
fail() {{ echo "FAIL $*"; }}
. "{KIT_ROOT / render_registry.PORT_FORWARD_HELPER}"
ensure_port_forward kubernetes agents-system kubernetes-mcp-server 18080 8080
"""
    env = {
        "HOME": str(tmp_path / "home"),
        "PATH": f"{bin_dir}:/usr/bin:/bin",
        "STATE": str(state),
        "KUBECONFIG": str(tmp_path / "kubeconfig"),
    }
    return subprocess.run(["bash", "-c", script], env=env, capture_output=True, text=True, check=False)


def _plist(tmp_path: Path) -> Path:
    return (
        tmp_path / "home" / "Library" / "LaunchAgents"
        / "dev.jorisjonkers.agent-kit.port-forward.kubernetes.plist"
    )


def test_macos_installs_a_keepalive_launchd_agent(tmp_path: Path) -> None:
    result = _run_helper(tmp_path, "Darwin")
    assert "FAIL" not in result.stdout and "warn" not in result.stdout, result.stdout + result.stderr
    assert "ok kubernetes: launchd agent" in result.stdout

    plist = _plist(tmp_path)
    text = plist.read_text()
    assert "<key>KeepAlive</key>\n  <true/>" in text
    # Pinned to the setup-time context, loopback only, the declared ports.
    for arg in ("port-forward", "--context", "prod-ctx", "svc/kubernetes-mcp-server",
                "--address", "127.0.0.1", "18080:8080"):
        assert f"<string>{arg}</string>" in text
    assert f"<string>{tmp_path / 'kubeconfig'}</string>" in text
    if shutil.which("plutil"):
        lint = subprocess.run(["plutil", "-lint", str(plist)], capture_output=True, text=True, check=False)
        assert lint.returncode == 0, lint.stdout + lint.stderr

    calls = (tmp_path / "state" / "launchctl.log").read_text()
    assert "bootstrap gui/" in calls and str(plist) in calls


def test_macos_rerun_leaves_a_loaded_agent_alone(tmp_path: Path) -> None:
    _run_helper(tmp_path, "Darwin")
    log = tmp_path / "state" / "launchctl.log"
    log.write_text("")
    result = _run_helper(tmp_path, "Darwin")
    assert "already loaded" in result.stdout, result.stdout
    assert "bootstrap" not in log.read_text() and "bootout" not in log.read_text()


def test_check_mode_installs_nothing(tmp_path: Path) -> None:
    result = _run_helper(tmp_path, "Darwin", check_only=1)
    assert "would install launchd agent" in result.stdout, result.stdout
    assert not _plist(tmp_path).exists()
    assert "bootstrap" not in (tmp_path / "state" / "launchctl.log").read_text()


def test_other_os_falls_back_to_a_one_shot_forward(tmp_path: Path) -> None:
    result = _run_helper(tmp_path, "Linux")
    assert "one-shot kubectl port-forward" in result.stdout, result.stdout
    assert not _plist(tmp_path).exists()
    assert not (tmp_path / "state" / "launchctl.log").exists()
