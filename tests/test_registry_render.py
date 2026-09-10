"""Drift guard for the generated registry artifacts."""

from __future__ import annotations

import copy
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
