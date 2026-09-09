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
