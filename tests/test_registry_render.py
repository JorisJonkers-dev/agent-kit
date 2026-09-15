"""Drift guard for the generated registry artifacts."""

from __future__ import annotations

import copy
import json
import os
import re
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


def test_retired_memory_server_reaches_no_surface() -> None:
    """The mem0 plan (agent-kit#41/#42) was abandoned for Hindsight + Basic Memory.

    `memory` must stay retired the same way `knowledge` is, or a registry edit
    would silently re-register a backend that no longer exists.
    """
    data = _registry()
    memory = next(s for s in data["mcp_servers"] if s["name"] == "memory")
    assert memory["surfaces"] == []
    assert "\n  memory:\n" not in render_registry.render_hermes_mcp(data)
    assert "mem0.memory-system" not in render_registry.render_setup_script(data)


def test_workstation_http_credential_becomes_a_bearer_header() -> None:
    """The per-host service token (agent-kit#41/#42) must reach both CLIs.

    Claude bakes the resolved value into `--header` at registration time;
    Codex reads the named env var at its own runtime via
    `--bearer-token-env-var`. Neither flag existed before this credential
    shape, so assert both are actually emitted, not just that the server is
    registered.
    """
    data = _registry()
    server = next(s for s in data["mcp_servers"] if s["name"] == "memory-api")
    assert server["transport"] == "http" and "workstation" in server["surfaces"]
    script = render_registry.render_setup_script(data)
    cred = server["credential"]
    assert f'--header "Authorization: Bearer ${{{cred}}}"' in script
    assert f"--bearer-token-env-var {cred}" in script


def test_hermes_gateway_skips_the_bearer_header_when_credential_hermes_is_false() -> None:
    """In-cluster Hermes reaches memory-api/memory-mcp by NetworkPolicy alone.

    It never mints a per-host service token, so `credential_hermes: false`
    must suppress the header the workstation surface gets -- sending a
    workstation credential placeholder into a ConfigMap Hermes has no way to
    fill would be worse than sending nothing.
    """
    data = _registry()
    for name in ("memory-api", "memory-mcp"):
        server = next(s for s in data["mcp_servers"] if s["name"] == name)
        assert server.get("credential") and server.get("credential_hermes") is False
    fragment = render_registry.render_hermes_mcp(data)
    for name in ("memory-api", "memory-mcp"):
        entry = fragment.split(f"  {name}:")[1].split("\n\n")[0]
        assert "headers" not in entry
        assert "Authorization" not in entry


def test_local_hermes_http_credential_becomes_a_bearer_header() -> None:
    """Local Hermes (the laptop CLI, not the in-cluster gateway) goes through

    forward-auth like Claude and Codex, so it needs the same bearer header --
    interpolated as ``${VAR}`` at Hermes' own connect time, never baked in.
    """
    data = _registry()
    fragment = render_registry.render_hermes_local_mcp(data)
    for name in ("memory-api", "memory-mcp"):
        cred = next(s for s in data["mcp_servers"] if s["name"] == name)["credential"]
        entry = fragment.split(f"  {name}:")[1].split("\n\n")[0]
        assert f'Authorization: "Bearer ${{{cred}}}"' in entry


def test_plugin_required_env_is_checked_and_reported() -> None:
    """An unset HINDSIGHT_API_URL fails silently into a personal local daemon.

    That is exactly the "looks like it works" trap CLAUDE.md warns about, so
    the script must warn by name rather than let the plugin's own fallback
    hide it.
    """
    data = _registry()
    plugin = next(p for p in data["plugins"] if p["name"] == "hindsight-memory")
    assert plugin.get("requires_env"), "hindsight-memory must declare requires_env"
    script = render_registry.render_setup_script(data)
    for var in plugin["requires_env"]:
        assert f'if [ -z "${{{var}:-}}" ]; then' in script
        assert f'missing_plugin_env+=("hindsight-memory: export {var}")' in script
    assert 'missing_plugin_env=()' in script
    assert '${#missing_plugin_env[@]}" = 0' in script


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


# --- Claude Code profiles ---


def test_exactly_one_primary_profile() -> None:
    data = _registry()
    data["claude_profiles"][1]["primary"] = True
    with pytest.raises(render_registry.RegistryError, match="exactly one primary"):
        render_registry.validate(data)


def test_a_secondary_profile_shares_from_the_primary() -> None:
    data = _registry()
    data["claude_profiles"][1]["shares_from"] = "nowhere"
    with pytest.raises(render_registry.RegistryError, match="must share from the primary"):
        render_registry.validate(data)


def test_a_secondary_profile_must_share_something() -> None:
    """A profile that shares nothing is a second install, not a second login."""
    data = _registry()
    data["claude_profiles"][1]["shared_paths"] = []
    with pytest.raises(render_registry.RegistryError, match="must name the paths it shares"):
        render_registry.validate(data)


@pytest.mark.parametrize("private", ["projects", "history.jsonl", ".claude.json", "sessions"])
def test_history_and_credentials_may_not_be_shared(private: str) -> None:
    """Sharing either one merges the histories the profiles exist to keep apart."""
    data = _registry()
    data["claude_profiles"][1]["shared_paths"] = [private]
    with pytest.raises(render_registry.RegistryError, match="defeats the separate profile"):
        render_registry.validate(data)


def test_a_shared_path_may_not_escape_the_config_root() -> None:
    data = _registry()
    data["claude_profiles"][1]["shared_paths"] = ["../.ssh"]
    with pytest.raises(render_registry.RegistryError, match="may not escape it"):
        render_registry.validate(data)


def test_setup_script_links_every_shared_surface_of_every_secondary() -> None:
    data = _registry()
    script = render_registry.render_setup_script(data)
    for profile in render_registry.claude_profiles(data)[1:]:
        assert f'profile_dir="{profile["config_dir"]}"' in script
        for rel in profile["shared_paths"]:
            assert f'link_profile_path "${{CLAUDE_HOME}}" "${{profile_dir}}" "{rel}"' in script
        # The profile has to exist before anything registers a server into it.
        assert script.index(f'profile_dir="{profile["config_dir"]}"') < script.index('log "MCP servers"')


def test_every_mcp_registration_reaches_every_profile() -> None:
    """A server lives in a profile's own .claude.json, so one add per profile."""
    data = _registry()
    script = render_registry.render_setup_script(data)
    for server in data["mcp_servers"]:
        name = server["name"]
        retired = server.get("enabled") is False or not server.get("surfaces")
        if retired:
            assert f"claude_each_profile claude mcp remove --scope user {name}" in script
            continue
        if "workstation" not in server["surfaces"]:
            continue
        assert f"claude_each_profile claude mcp add --scope user {name}" in script
        assert f"claude_each_profile claude mcp remove --scope user {name}" in script
    # And the verification asks each profile what it actually has.
    assert 'for profile_dir in "${CLAUDE_PROFILE_DIRS[@]}"; do' in script
    assert 'CLAUDE_CONFIG_DIR="${profile_dir}" claude mcp list' in script


def test_plugins_are_installed_once_into_the_primary() -> None:
    """plugins/ is shared and enabledPlugins lives in the shared settings.json."""
    script = render_registry.render_setup_script(_registry())
    assert "claude_each_profile claude plugin" not in script


# --- link_profile_path, driven against a real filesystem ---


def _shell_function(name: str) -> str:
    script = render_registry.render_setup_script(_registry())
    start = script.index(f"{name}() {{")
    return script[start : script.index("\n}\n", start) + 3]


def _link(tmp_path: Path, shared: list[str], check_only: int = 0) -> subprocess.CompletedProcess:
    primary = tmp_path / ".claude"
    secondary = tmp_path / ".claude-personal"
    secondary.mkdir(parents=True, exist_ok=True)
    body = "\n".join(
        [
            f"CHECK_ONLY={check_only}",
            "unshared_profile_paths=()",
            "warnings=0",
            "failures=0",
            'log()  { echo "log $*"; }',
            'ok()   { echo "ok $*"; }',
            'warn() { echo "warn $*"; }',
            'fail() { echo "FAIL $*"; }',
            _shell_function("link_profile_path"),
            *[f'link_profile_path "{primary}" "{secondary}" "{rel}"' for rel in shared],
        ],
    )
    return subprocess.run(["bash", "-c", body], capture_output=True, text=True, check=False)


def test_shared_surfaces_become_symlinks_into_the_primary(tmp_path: Path) -> None:
    primary = tmp_path / ".claude"
    (primary / "skills").mkdir(parents=True)
    (primary / "settings.json").write_text("{}")

    result = _link(tmp_path, ["skills", "settings.json"])
    assert "FAIL" not in result.stdout, result.stdout + result.stderr

    for rel in ("skills", "settings.json"):
        dest = tmp_path / ".claude-personal" / rel
        assert dest.is_symlink() and dest.resolve() == (primary / rel).resolve()

    # Idempotent: a second run reports the link and changes nothing.
    again = _link(tmp_path, ["skills", "settings.json"])
    assert "warn" not in again.stdout, again.stdout


def test_a_real_file_in_the_way_is_left_alone(tmp_path: Path) -> None:
    """Replacing someone's own config with a symlink loses what it cannot give back."""
    (tmp_path / ".claude" / "skills").mkdir(parents=True)
    own = tmp_path / ".claude-personal" / "skills"
    own.mkdir(parents=True)
    (own / "mine.md").write_text("mine")

    result = _link(tmp_path, ["skills"])
    assert "not a symlink" in result.stdout, result.stdout
    assert (own / "mine.md").read_text() == "mine"
    assert not own.is_symlink()


def test_a_missing_shared_directory_is_created_then_linked(tmp_path: Path) -> None:
    """The link has to exist before whatever writes into it does."""
    (tmp_path / ".claude").mkdir()
    result = _link(tmp_path, ["skills"])
    assert "warn" not in result.stdout, result.stdout
    assert (tmp_path / ".claude" / "skills").is_dir()
    assert (tmp_path / ".claude-personal" / "skills").is_symlink()


def test_a_missing_shared_file_is_reported_not_invented(tmp_path: Path) -> None:
    """An empty settings.json would read as a real answer to a real question."""
    (tmp_path / ".claude").mkdir()
    result = _link(tmp_path, ["settings.json"])
    assert "does not exist" in result.stdout, result.stdout
    assert not (tmp_path / ".claude" / "settings.json").exists()
    assert not (tmp_path / ".claude-personal" / "settings.json").exists()


def test_a_root_is_never_linked_onto_itself(tmp_path: Path) -> None:
    """Setup run from a personal session sees CLAUDE_CONFIG_DIR as the primary."""
    root = tmp_path / ".claude-personal"
    (root / "skills").mkdir(parents=True)
    body = "\n".join(
        [
            "CHECK_ONLY=0",
            "unshared_profile_paths=()",
            'log()  { echo "log $*"; }',
            'ok()   { echo "ok $*"; }',
            'warn() { echo "warn $*"; }',
            'fail() { echo "FAIL $*"; }',
            _shell_function("link_profile_path"),
            f'link_profile_path "{root}" "{root}" skills',
        ],
    )
    result = subprocess.run(["bash", "-c", body], capture_output=True, text=True, check=False)
    assert "refusing to link it onto itself" in result.stdout, result.stdout
    assert (root / "skills").is_dir() and not (root / "skills").is_symlink()


def test_setup_script_skips_a_secondary_that_is_the_config_root() -> None:
    script = render_registry.render_setup_script(_registry())
    guard = script.index('if [ "${profile_dir}" = "${CLAUDE_HOME}" ]')
    assert guard < script.index('link_profile_path "${CLAUDE_HOME}" "${profile_dir}"')


def test_check_mode_links_nothing(tmp_path: Path) -> None:
    (tmp_path / ".claude" / "skills").mkdir(parents=True)
    (tmp_path / ".claude" / "agents").mkdir()
    result = _link(tmp_path, ["skills"], check_only=1)
    assert "would link" in result.stdout, result.stdout
    assert not (tmp_path / ".claude-personal" / "skills").exists()


def test_a_profile_name_must_be_a_command_name() -> None:
    data = _registry()
    data["claude_profiles"][1]["name"] = "my profile"
    with pytest.raises(render_registry.RegistryError, match="lowercase letters"):
        render_registry.validate(data)


def test_setup_script_installs_a_launcher_for_every_secondary() -> None:
    data = _registry()
    script = render_registry.render_setup_script(data)
    for profile in render_registry.claude_profiles(data)[1:]:
        assert f'install_profile_launcher "{profile["name"]}" "${{profile_dir}}"' in script


def _launch(tmp_path: Path, check_only: int = 0) -> subprocess.CompletedProcess:
    body = "\n".join(
        [
            f"CHECK_ONLY={check_only}",
            f'CLAUDE_LAUNCHER_DIR="{tmp_path / "bin"}"',
            f'PATH="{tmp_path / "bin"}:$PATH"',
            "unshared_profile_paths=()",
            'log()  { echo "log $*"; }',
            'ok()   { echo "ok $*"; }',
            'warn() { echo "warn $*"; }',
            'fail() { echo "FAIL $*"; }',
            _shell_function("install_profile_launcher"),
            f'install_profile_launcher personal "{tmp_path / "my root"}"',
        ],
    )
    return subprocess.run(["bash", "-c", body], capture_output=True, text=True, check=False)


def test_the_launcher_runs_claude_with_the_profile_root(tmp_path: Path) -> None:
    result = _launch(tmp_path)
    assert "FAIL" not in result.stdout and "warn" not in result.stdout, result.stdout + result.stderr
    launcher = tmp_path / "bin" / "claude-personal"
    assert os.access(launcher, os.X_OK)

    # A fake claude that reports the root it was given and its arguments.
    fake = tmp_path / "fake"
    fake.mkdir()
    (fake / "claude").write_text('#!/usr/bin/env bash\necho "root=$CLAUDE_CONFIG_DIR args=$*"\n')
    (fake / "claude").chmod(0o755)
    env = {**os.environ, "PATH": f"{fake}:{os.environ['PATH']}"}
    ran = subprocess.run([str(launcher), "--resume", "x y"], capture_output=True, text=True, env=env, check=False)
    assert ran.stdout.strip() == f"root={tmp_path / 'my root'} args=--resume x y", ran.stdout + ran.stderr

    # Idempotent: a second run leaves the launcher as it is.
    again = _launch(tmp_path)
    assert "wrote" not in again.stdout, again.stdout


def test_a_hand_written_launcher_is_left_alone(tmp_path: Path) -> None:
    (tmp_path / "bin").mkdir()
    own = tmp_path / "bin" / "claude-personal"
    own.write_text("#!/bin/sh\necho mine\n")
    result = _launch(tmp_path)
    assert "not written by setup" in result.stdout, result.stdout
    assert own.read_text() == "#!/bin/sh\necho mine\n"


def test_check_mode_writes_no_launcher(tmp_path: Path) -> None:
    result = _launch(tmp_path, check_only=1)
    assert "would write" in result.stdout, result.stdout
    assert not (tmp_path / "bin" / "claude-personal").exists()


def test_an_optional_credential_survives_being_unset(tmp_path: Path) -> None:
    """`set -u` turns a bare ${VAR} for an unset var into an aborted run."""
    data = _registry()
    script = render_registry.render_setup_script(data)
    for server in data["mcp_servers"]:
        if not server.get("credential_optional"):
            continue
        var = server["credential"]
        assert f'--env {var}="${{{var}}}"' not in script
        assert f'--env {var}="${{{var}:-}}"' in script

    # And prove it against bash: the expansion, under the script's own flags.
    probe = tmp_path / "probe.sh"
    probe.write_text("set -uo pipefail\necho \"[${OVERLEAF_SESSION:-}]\"\necho reached-the-end\n")
    result = subprocess.run(
        ["bash", str(probe)], capture_output=True, text=True, check=False,
        env={"PATH": "/usr/bin:/bin"},
    )
    assert result.returncode == 0, result.stderr
    assert "reached-the-end" in result.stdout


def test_registered_server_names_are_extracted_without_pcre(tmp_path: Path) -> None:
    """BSD grep has no lookahead: `(?=:)` errors out and inspects nothing."""
    script = render_registry.render_setup_script(_registry())
    assert 'grep -oE "\\b[a-z0-9_-]+\\b(?=:)"' not in script

    listing = (
        "Checking MCP server health…\n\n"
        "plugin:github:github: https://api.githubcopilot.com/mcp/ (HTTP) - ✘ Failed\n"
        "idea: http://127.0.0.1:64342/stream (HTTP) - ✔ Connected\n"
        "playwright: npx -y @playwright/mcp@latest --headless - ✔ Connected\n"
    )
    extract = next(
        line.strip() for line in script.splitlines() if "sort -u | while read -r found" in line
    ).split("| sort -u")[0]
    probe = tmp_path / "probe.sh"
    probe.write_text(f'registered=$(cat)\n{extract} | sort -u\n')
    result = subprocess.run(
        ["bash", str(probe)], input=listing, capture_output=True, text=True, check=False,
    )
    assert result.returncode == 0, result.stderr
    assert result.stderr == ""
    assert result.stdout.split() == ["idea", "playwright", "plugin:github:github"]


# ---------------------------------------------------------------------------
# Container surface: installer/setup-container.sh
# ---------------------------------------------------------------------------


def _raw_entry(data: dict, name: str) -> dict:
    for key in render_registry.CONTAINER_SOURCES:
        for item in data.get(key) or []:
            if item.get("name", item.get("plugin")) == name:
                return item
    raise KeyError(name)


def test_container_tools_are_declared() -> None:
    names = {t["name"] for t in render_registry.container_tools(_registry())}
    assert {"claude-code", "codex", "hermes-agent", "node", "uv"} <= names


@pytest.mark.parametrize("version", ["", "latest", None])
def test_a_container_tool_must_pin_a_version(version: str | None) -> None:
    data = _registry()
    _raw_entry(data, "codex")["container"]["version"] = version
    with pytest.raises(render_registry.RegistryError, match="pin a version"):
        render_registry.validate(data)


def test_a_container_install_must_use_the_pinned_version() -> None:
    data = _registry()
    _raw_entry(data, "codex")["container"]["install"] = "npm install -g @openai/codex@latest"
    with pytest.raises(render_registry.RegistryError, match=r"\$\{VERSION\}"):
        render_registry.validate(data)


def test_the_container_surface_and_block_go_together() -> None:
    data = _registry()
    del _raw_entry(data, "codex")["container"]
    with pytest.raises(render_registry.RegistryError, match="container surface"):
        render_registry.validate(data)

    data = _registry()
    _raw_entry(data, "codex")["surfaces"].remove("container")
    with pytest.raises(render_registry.RegistryError, match="container surface"):
        render_registry.validate(data)


def test_a_container_tool_may_only_require_another_container_tool() -> None:
    data = _registry()
    _raw_entry(data, "codex")["container"]["requires"] = ["nope"]
    with pytest.raises(render_registry.RegistryError, match="requires unknown"):
        render_registry.validate(data)


def test_container_tools_install_after_what_they_require() -> None:
    order = [t["name"] for t in render_registry.container_tools(_registry())]
    for tool in render_registry.container_tools(_registry()):
        for required in tool["requires"]:
            assert order.index(required) < order.index(tool["name"])


def test_renovate_tracks_every_container_pin() -> None:
    """A pin Renovate cannot see never moves, so prove the manager matches each one."""
    renovate = json.loads((KIT_ROOT / "renovate.json").read_text())
    manager = next(
        m for m in renovate["customManagers"]
        if "estate-tooling" in "".join(m["managerFilePatterns"])
    )
    text = (KIT_ROOT / "registry" / "estate-tooling.yaml").read_text()
    found = set()
    for pattern in manager["matchStrings"]:
        for match in re.finditer(pattern.replace("(?<", "(?P<"), text):
            found.add((match["datasource"], match["depName"], match["currentValue"]))
    expected = {
        (t["datasource"], t["package"], t["version"])
        for t in render_registry.container_tools(_registry())
    }
    assert found == expected


def test_a_container_mcp_server_is_checked_by_the_binary_it_runs() -> None:
    """An MCP server's entry name is not its binary: `drawio` runs `drawio-mcp`."""
    data = _registry()
    tools = {t["name"]: t for t in render_registry.container_tools(data)}
    for server in data["mcp_servers"]:
        tool = tools.get(server["name"])
        if tool and server["transport"] == "stdio" and "binary" not in server["container"]:
            assert tool["binary"] == server["requires_binary"]


def test_a_container_tool_name_may_not_be_declared_twice() -> None:
    data = _registry()
    clone = copy.deepcopy(_raw_entry(data, "codex"))
    clone.update(transport="stdio", command="codex")
    data["mcp_servers"].append(clone)
    with pytest.raises(render_registry.RegistryError, match="declared twice"):
        render_registry.validate(data)


def test_the_typescript_language_server_gets_a_tsserver() -> None:
    """typescript-language-server drives tsserver, which typescript 7.x no longer ships."""
    typescript = next(t for t in render_registry.container_tools(_registry()) if t["name"] == "typescript")
    assert typescript["binary"] == "tsserver"


def test_container_help_prints_only_the_header(tmp_path: Path) -> None:
    script = tmp_path / "setup-container.sh"
    script.write_text(render_registry.render_container_setup_script(_registry()))
    result = subprocess.run(["bash", str(script), "--help"], capture_output=True, text=True, check=False)
    assert result.returncode == 0
    assert all(line.startswith("#") for line in result.stdout.splitlines()), result.stdout
    assert "--check" in result.stdout


def test_container_script_carries_no_secret() -> None:
    data = _registry()
    script = render_registry.render_container_setup_script(data)
    for server in data["mcp_servers"]:
        if server.get("credential"):
            assert server["credential"] not in script
    assert "@latest" not in script


def _stub(bin_dir: Path, name: str, output: str) -> None:
    path = bin_dir / name
    path.write_text(f"#!/bin/sh\necho '{output}'\n")
    path.chmod(0o755)


def _container_check(tmp_path: Path, override: dict[str, str] | None = None) -> subprocess.CompletedProcess:
    data = _registry()
    script = tmp_path / "setup-container.sh"
    script.write_text(render_registry.render_container_setup_script(data))
    bin_dir = tmp_path / "bin"
    bin_dir.mkdir(exist_ok=True)
    # One stub can answer for several tools (`npm ls -g`), so it prints every line.
    outputs: dict[str, list[str]] = {}
    for tool in render_registry.container_tools(data):
        version = (override or {}).get(tool["name"], tool["version"])
        line = f"{tool['package']}@{version}"
        outputs.setdefault(tool["binary"], []).append(line)
        outputs.setdefault(tool["version_command"].split()[0], []).append(line)
    for name, lines in outputs.items():
        _stub(bin_dir, name, "\n".join(dict.fromkeys(lines)))
    _stub(bin_dir, "dpkg", "ok")
    return subprocess.run(
        ["bash", str(script), "--check"], capture_output=True, text=True, check=False,
        env={"PATH": f"{bin_dir}:/usr/bin:/bin"},
    )


def test_container_check_passes_when_every_tool_is_at_its_pin(tmp_path: Path) -> None:
    result = _container_check(tmp_path)
    assert result.returncode == 0, result.stdout + result.stderr


def test_container_check_fails_on_a_version_mismatch(tmp_path: Path) -> None:
    result = _container_check(tmp_path, {"codex": "0.0.1"})
    assert result.returncode != 0
    assert "codex: expected" in result.stderr


def test_container_check_fails_when_a_tool_is_missing(tmp_path: Path) -> None:
    script = tmp_path / "setup-container.sh"
    script.write_text(render_registry.render_container_setup_script(_registry()))
    result = subprocess.run(
        ["bash", str(script), "--check"], capture_output=True, text=True, check=False,
        env={"PATH": "/usr/bin:/bin"},
    )
    assert result.returncode != 0
    assert "hermes: not on PATH" in result.stderr


@pytest.mark.skipif(os.geteuid() == 0, reason="the refusal is for non-root callers")
def test_container_install_refuses_to_run_as_non_root(tmp_path: Path) -> None:
    script = tmp_path / "setup-container.sh"
    script.write_text(render_registry.render_container_setup_script(_registry()))
    result = subprocess.run(
        ["bash", str(script)], capture_output=True, text=True, check=False,
        env={"PATH": "/usr/bin:/bin"},
    )
    assert result.returncode == 77
    assert "must run as root" in result.stderr
