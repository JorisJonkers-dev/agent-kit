"""Drift guard for the generated registry artifacts."""

from __future__ import annotations

import copy
import json
import os
import re
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


def test_no_generated_artifact_registers_the_retired_knowledge_base() -> None:
    """The knowledge base is retired, so nothing setup writes may point at it."""
    data = _registry()
    # Match the server KEY, not the word: the memory server's comment
    # legitimately mentions the retired knowledge base.
    assert "\n  knowledge:\n" not in render_registry.render_hermes_local_mcp(data)
    assert "kb.jorisjonkers.dev" not in render_registry.render_setup_script(data)


def test_the_registry_no_longer_records_the_knowledge_server() -> None:
    """Convergence removes an undeclared `knowledge`, and its token is revoked, so the tombstone is gone."""
    assert "knowledge" not in {s["name"] for s in _registry()["mcp_servers"]}


def test_the_registry_no_longer_records_the_mem0_memory_server() -> None:
    """Convergence removes an undeclared `memory`, so the mem0 tombstone is gone."""
    assert "memory" not in {s["name"] for s in _registry()["mcp_servers"]}


def test_workstation_http_credential_becomes_a_bearer_header() -> None:
    """Claude gets --header, Codex gets --bearer-token-env-var, for the same credential."""
    data = _registry()
    server = next(s for s in data["mcp_servers"] if s["name"] == "memory-api")
    assert server["transport"] == "http" and "workstation" in server["surfaces"]
    script = render_registry.render_setup_script(data)
    cred = server["credential"]
    assert f'--header "Authorization: Bearer $(secret_ref {cred})"' in script
    assert f"--bearer-token-env-var {cred}" in script


def test_local_hermes_http_credential_becomes_a_bearer_header() -> None:
    """Local Hermes gets a ${VAR}-interpolated bearer header, like Claude and Codex."""
    data = _registry()
    fragment = render_registry.render_hermes_local_mcp(data)
    for name in ("memory-api",):
        cred = next(s for s in data["mcp_servers"] if s["name"] == name)["credential"]
        entry = fragment.split(f"  {name}:")[1].split("\n\n")[0]
        assert f'Authorization: "Bearer ${{{cred}}}"' in entry


def test_plugin_required_env_is_checked_and_reported() -> None:
    """An unset requires_env var is warned about by name, not silently ignored."""
    data = _registry()
    plugin = data["plugins"][0]
    plugin["requires_env"] = {"EXAMPLE_PLUGIN_TOKEN": "what it is for"}
    script = render_registry.render_setup_script(data)
    name = plugin["name"]
    for var in plugin["requires_env"]:
        assert f'if [ -z "${{{var}:-}}" ]; then' in script
        assert f'missing_plugin_env+=("{name}: export {var}")' in script
        # The warning is what puts it on the closing to-do list.
        assert f'warn "{name}: {var} is not set' in script
    assert 'missing_plugin_env=()' in script


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
        render_registry.render_hermes_local_mcp(data),
    ):
        assert "overleaf_session2" not in surface
        assert "overleaf.sid" in surface


def test_overleaf_writes_do_not_ask_per_write() -> None:
    """olcli-mcp asks through MCP elicitation before every write by default.

    Since olcli 0.15.1 each push_file, rename or comment opens its own
    confirmation dialog on top of the agent client's tool approval. The
    registry turns that off with OLCLI_MCP_UNCONFIRMED_WRITES=1 so the client's
    approval is the only gate, and every surface that registers overleaf must
    carry it, or re-running setup brings the per-write dialog back.
    """
    data = _registry()
    script = render_registry.render_setup_script(data)
    overleaf_adds = [line for line in script.splitlines() if "mcp add" in line and " overleaf " in line]
    assert overleaf_adds, "setup script no longer registers overleaf"
    for line in overleaf_adds:
        assert '--env OLCLI_MCP_UNCONFIRMED_WRITES="1"' in line, line
    for surface in (
        render_registry.render_hermes_local_mcp(data),
    ):
        overleaf_entry = surface.split("  overleaf:")[1].split("\n\n")[0]
        assert 'OLCLI_MCP_UNCONFIRMED_WRITES: "1"' in overleaf_entry


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
        if server.get("enabled") is False or server.get("session_only"):
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


LAUNCHER = KIT_ROOT / "installer" / "kubernetes-mcp-readonly"


def test_kubernetes_is_a_local_read_only_stdio_server() -> None:
    server = next(s for s in _registry()["mcp_servers"] if s["name"] == "kubernetes")
    assert server["transport"] == "stdio"
    assert server["command"] == "kubernetes-mcp-readonly"
    assert server["surfaces"] == ["workstation"]
    assert server["cloud"] is False
    assert server["requires_binary"] == "kubernetes-mcp-server" and server["install_pinned"] is True
    assert not any(key in server for key in ("url_workstation", "workstation_connect"))


def test_kubernetes_registers_the_launcher_for_every_agent_and_no_port_forward_remains() -> None:
    script = render_registry.render_setup_script(_registry())
    assert "claude mcp add --scope user kubernetes -- kubernetes-mcp-readonly" in script
    assert "codex mcp add kubernetes -- kubernetes-mcp-readonly" in script
    assert "install_kit_command kubernetes-mcp-readonly" in script
    assert "127.0.0.1:18080" not in script
    assert "ensure_port_forward" not in script
    for generated in (
        "registry/generated/vibe/mcp-servers.toml",
        "registry/generated/opencode/opencode.json",
        "registry/generated/antigravity/mcp_config.json",
        "registry/generated/hermes/mcp-servers.local.yaml",
    ):
        text = (KIT_ROOT / generated).read_text()
        assert "kubernetes-mcp-readonly" in text and "18080" not in text, generated


CODE_NAV_ROOT = "~/Projects/JorisJonkers-dev"


def test_code_nav_is_a_read_only_stdio_server_over_the_personal_roots() -> None:
    servers = [s for s in _registry()["mcp_servers"] if s["name"] == "code-nav"]
    assert len(servers) == 1
    server = servers[0]
    assert server["transport"] == "stdio"
    assert server["command"] == "code-nav"
    assert server["surfaces"] == ["workstation"]
    assert server["cloud"] is False
    assert server["requires_binary"] == "code-nav"
    # The args are the allowlist: one --root, one personal path.
    assert server["args"][0] == "--root"
    assert len(server["args"]) == 2 and server["args"][1] == CODE_NAV_ROOT
    assert "nedap" not in json.dumps(server).lower()


def test_code_nav_registers_the_personal_root_for_every_agent() -> None:
    script = render_registry.render_setup_script(_registry())
    assert f"claude mcp add --scope user code-nav -- code-nav --root {CODE_NAV_ROOT}" in script
    assert f"codex mcp add code-nav -- code-nav --root {CODE_NAV_ROOT}" in script
    for generated in (
        "registry/generated/vibe/mcp-servers.toml",
        "registry/generated/opencode/opencode.json",
        "registry/generated/antigravity/mcp_config.json",
        "registry/generated/hermes/mcp-servers.local.yaml",
    ):
        text = (KIT_ROOT / generated).read_text()
        assert "code-nav" in text and CODE_NAV_ROOT in text, generated


def test_a_work_root_in_the_registry_is_refused() -> None:
    data = _registry()
    server = next(s for s in data["mcp_servers"] if s["name"] == "code-nav")
    server["args"] = ["--root", "/Users/x/Projects/Nedap"]
    with pytest.raises(render_registry.RegistryError, match="work roots belong in the work machine overlay"):
        render_registry.validate(data)


def test_no_generated_artifact_names_a_work_root() -> None:
    """A work root in the public registry would ship to every personal machine."""
    paths = [KIT_ROOT / "installer" / "setup-workstation.sh"]
    paths += [p for p in (KIT_ROOT / "registry" / "generated").rglob("*") if p.is_file()]
    for path in paths:
        for line in path.read_text().splitlines():
            if "--root" in line or "code-nav" in line:
                assert "nedap" not in line.lower(), f"{path}: {line}"


def _retire(tmp_path: Path, check_only: int = 0) -> subprocess.CompletedProcess:
    script = render_registry.render_setup_script(_registry())
    start = script.index("retire_port_forward_agents() {")
    body = script[start : script.index("\n}\n", start) + 3]
    bin_dir = tmp_path / "bin"
    bin_dir.mkdir(exist_ok=True)
    for name in ("launchctl", "systemctl"):
        stub = bin_dir / name
        stub.write_text(f'#!/usr/bin/env bash\necho "{name} $*" >> "{tmp_path}/calls.log"\n')
        stub.chmod(0o755)
    run = f"""
CHECK_ONLY={check_only}
log()  {{ echo "log $*"; }}
ok()   {{ echo "ok $*"; }}
{body}
retire_port_forward_agents
"""
    env = {
        "HOME": str(tmp_path / "home"),
        "PATH": f"{bin_dir}:{os.environ['PATH']}",
        "AK_LAUNCH_AGENTS_DIR": str(tmp_path / "agents"),
        "AK_SYSTEMD_USER_DIR": str(tmp_path / "units"),
        "AK_LOG_DIR": str(tmp_path / "logs"),
    }
    return subprocess.run(["bash", "-c", run], env=env, capture_output=True, text=True, check=False)


def _old_agents(tmp_path: Path) -> list[Path]:
    prefix = "dev.jorisjonkers.agent-kit.port-forward.kubernetes"
    files = [tmp_path / "agents" / f"{prefix}.plist", tmp_path / "units" / f"{prefix}.service",
             tmp_path / "logs" / "port-forward-kubernetes.log"]
    for f in files:
        f.parent.mkdir(parents=True, exist_ok=True)
        f.write_text("x")
    return files


def test_setup_removes_the_old_launchd_agent_systemd_unit_and_log(tmp_path: Path) -> None:
    files = _old_agents(tmp_path)
    result = _retire(tmp_path)
    assert result.returncode == 0, result.stderr
    assert not any(f.exists() for f in files)
    calls = (tmp_path / "calls.log").read_text()
    assert "bootout gui/" in calls and "disable --now" in calls and "daemon-reload" in calls


def test_setup_check_mode_only_reports_the_old_agents(tmp_path: Path) -> None:
    files = _old_agents(tmp_path)
    result = _retire(tmp_path, check_only=1)
    assert all(f.exists() for f in files)
    assert result.stdout.count("would remove retired") == 2


def test_setup_with_no_old_agent_does_nothing(tmp_path: Path) -> None:
    result = _retire(tmp_path)
    assert result.returncode == 0 and result.stdout == ""


def _launcher(
    tmp_path: Path, context: str, *args: str, user: str = "c-dev-ro", **env: str
) -> subprocess.CompletedProcess:
    """Run the launcher with a stub kubectl and a stub server that records its config."""
    bin_dir = tmp_path / "bin"
    bin_dir.mkdir(exist_ok=True)
    (bin_dir / "kubectl").write_text(
        f"#!/usr/bin/env bash\n"
        f'if [ "$2" = current-context ]; then echo {context}; exit 0; fi\n'
        f'case "$*" in *jsonpath*) echo "{user}"; exit 0;; esac\n'
        f'echo "kubeconfig for $*"\n'
    )
    (bin_dir / "kubernetes-mcp-server").write_text(
        f'#!/usr/bin/env bash\ncp "$2" "{tmp_path}/config.toml"\n'
        f'cp "$(sed -n \'s/^kubeconfig = "\\(.*\\)"/\\1/p\' "$2")" "{tmp_path}/kubeconfig"\n'
        f'echo "$*" > "{tmp_path}/server-args"\n'
    )
    for stub in bin_dir.iterdir():
        stub.chmod(0o755)
    return subprocess.run(
        [str(LAUNCHER), *args],
        env={"PATH": f"{bin_dir}:{os.environ['PATH']}", "TMPDIR": str(tmp_path), **env},
        capture_output=True, text=True, check=False,
    )


def test_the_launcher_pins_the_read_only_context(tmp_path: Path) -> None:
    result = _launcher(tmp_path, "personal", "--check")
    assert result.stdout == "personal\n"
    assert _launcher(tmp_path, "personal", "--check", AK_KUBE_CONTEXT="work").stdout == "work\n"


@pytest.mark.parametrize("context", ["personal-admin", "personal-pf"])
def test_the_launcher_refuses_an_admin_or_port_forward_context(tmp_path: Path, context: str) -> None:
    for result in (_launcher(tmp_path, context), _launcher(tmp_path, "personal", AK_KUBE_CONTEXT=context)):
        assert result.returncode == 1
        assert "refusing context" in result.stderr


def test_the_launcher_starts_the_server_read_only_on_a_single_context_kubeconfig(tmp_path: Path) -> None:
    result = _launcher(tmp_path, "personal")
    assert result.returncode == 0, result.stderr
    config = (tmp_path / "config.toml").read_text()
    for line in ("read_only = true", "disable_destructive = true", 'toolsets = ["core"]', 'kind = "Secret"'):
        assert line in config
    assert (tmp_path / "kubeconfig").read_text() == (
        "kubeconfig for config view --raw --minify --flatten --context personal\n"
    )
    assert not list(tmp_path.glob("kubernetes-mcp.*")), "the throwaway kubeconfig is removed on exit"


def test_the_launcher_refuses_a_context_whose_user_is_not_read_only(tmp_path: Path) -> None:
    result = _launcher(tmp_path, "work", user="cluster-admin-user")
    assert result.returncode == 1 and "not a read-only" in result.stderr


def test_the_launcher_needs_a_context(tmp_path: Path) -> None:
    assert "no current kube context" in _launcher(tmp_path, "").stderr



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
        if retired or "workstation" not in server["surfaces"] or server.get("session_only"):
            continue
        assert f"claude_each_profile claude mcp add --scope user {name}" in script
        assert f"claude_each_profile claude mcp remove --scope user {name}" in script
    # And the verification asks each profile what it actually has.
    assert 'for profile_dir in "${CLAUDE_PROFILE_DIRS[@]}"; do' in script
    assert 'claude_in_profile "${profile_dir}" claude mcp list' in script


@pytest.mark.parametrize(
    ("user_dir", "profile", "expected"),
    [
        ("", "primary", "unset"),
        ("", "secondary", "secondary"),
        ("primary", "primary", "primary"),
    ],
)
def test_the_primary_profile_keeps_claudes_own_config_file(
    tmp_path: Path, user_dir: str, profile: str, expected: str,
) -> None:
    """CLAUDE_CONFIG_DIR=~/.claude writes ~/.claude/.claude.json, which a bare `claude` never reads."""
    dirs = {"primary": str(tmp_path / ".claude"), "secondary": str(tmp_path / ".claude-personal")}
    body = "\n".join(
        [
            f'CLAUDE_HOME="{dirs["primary"]}"',
            'claude() { echo "${CLAUDE_CONFIG_DIR:-unset}"; }',
            _shell_function("claude_in_profile"),
            f'claude_in_profile "{dirs[profile]}" claude',
        ],
    )
    env = {"PATH": "/usr/bin:/bin"}
    if user_dir:
        env["CLAUDE_CONFIG_DIR"] = dirs[user_dir]
    result = subprocess.run(["bash", "-c", body], capture_output=True, text=True, check=False, env=env)
    assert result.stdout.strip() == dirs.get(expected, expected)


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


def test_the_image_has_both_java_language_servers_and_oras_on_path() -> None:
    tools = {t["binary"]: t for t in render_registry.container_tools(_registry())}
    assert {"kotlin-lsp", "jdtls", "oras"} <= set(tools)
    # Each one is installed from a version it reports, so --check proves the server and not just a JDK.
    for binary in ("kotlin-lsp", "jdtls", "oras"):
        assert "${VERSION}" in tools[binary]["install"]
    script = render_registry.render_container_setup_script(_registry())
    assert "ln -sf /opt/kotlin-lsp/kotlin-lsp.sh /usr/local/bin/kotlin-lsp" in script
    assert "ln -sf /opt/jdtls/bin/jdtls /usr/local/bin/jdtls" in script
    assert "temurin-21-jdk" in script  # the runtime jdtls needs
    for path in ("/opt/kotlin-lsp", "/opt/jdtls"):
        assert f"chmod -R a+rX '{path}'" in script


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
    renovate = json.loads((KIT_ROOT / ".github" / "renovate.json").read_text())
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
        for t in [*render_registry.container_tools(_registry()), *render_registry.workstation_pins(_registry())]
    }
    # A pinned_npm CLI (Pi) carries its own `datasource: npm` lines.
    for cli in _registry()["clis"]:
        for pin in (cli.get("pinned_npm") or {}).get("packages", []):
            expected.add((pin["datasource"], pin["package"], pin["version"]))
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


# --- agent-kit#40: setup-workstation.sh as a superset, install.sh retired ---


def test_estate_skills_are_local_skills_for_every_agent() -> None:
    data = _registry()
    by_name = {s["name"]: s for s in data["local_skills"]}
    for name in ("council", "kb-first", "token-economy", "agent-session-bootstrap", "pr-composer"):
        assert name in by_name, name
        skill = by_name[name]
        assert skill["surfaces"] == ["workstation"]
        assert set(skill["agents"]) == {"claude", "codex", "vibe", "antigravity", "opencode"}
        assert (KIT_ROOT / skill["path"] / "SKILL.md").is_file()


def test_retired_kb_skills_are_not_local_skills() -> None:
    """topics, audit and grill-me have no memory-api/memory-mcp equivalent (or,
    for grill-me, are no longer installed by this repo at all) and are
    deliberately left out rather than shipped against a dead service."""
    data = _registry()
    names = {s["name"] for s in data["local_skills"]}
    assert names.isdisjoint({"topics", "audit", "grill-me"})


def test_kb_first_and_token_economy_do_not_reference_the_retired_kb() -> None:
    """Mentioning that the knowledge service is retired is fine; instructing the agent
    to actually call one of its dead tools or env vars is not."""
    dead_tool_calls = ("knowledge.recall", "knowledge.relations", "KB_BEARER_TOKEN")
    for name in ("kb-first", "token-economy"):
        text = (KIT_ROOT / "skills" / "workstation" / name / "SKILL.md").read_text()
        for token in dead_tool_calls:
            assert token not in text, f"{name} still references retired {token}"
        assert "memory-api" in text
        # memory-mcp is retired as well: at most the one sentence saying so.
        assert text.count("memory-mcp") <= 1, f"{name} still points agents at memory-mcp"


def test_superpowers_reaches_the_workstation() -> None:
    data = _registry()
    source = next(s for s in data["skill_sources"] if s["name"] == "superpowers")
    assert "workstation" in source["surfaces"]
    script = render_registry.render_setup_script(data)
    assert "github.com/obra/superpowers" in script
    assert source["commit"] in script


def test_workstation_skill_source_must_use_the_star_selector() -> None:
    data = _registry()
    source = next(s for s in data["skill_sources"] if s["name"] == "superpowers")
    source["selector"] = "engineering/*"
    with pytest.raises(render_registry.RegistryError, match="only selector"):
        render_registry.validate(data)


def test_setup_script_never_installs_speckit() -> None:
    """Spec Kit stays excluded from the registry (install.sh shipped it).

    The only `speckit` references left in the rendered script are the
    --uninstall cleanup of a machine set up by that retired installer.
    """
    data = _registry()
    script = render_registry.render_setup_script(data)
    for line in script.splitlines():
        if "speckit" in line.lower():
            assert "legacy_remove" in line, line


def test_setup_script_fetches_a_published_bundle_when_not_a_checkout() -> None:
    data = _registry()
    script = render_registry.render_setup_script(data)
    assert "AGENT_KIT_SKILLS_BUNDLE_URL" in script
    # The default comes from the script's own release, never from `latest`,
    # so installer and bundle always match.
    assert (
        "https://github.com/JorisJonkers-dev/agent-kit/releases/download/"
        "${AGENT_KIT_RELEASE_TAG}/agent-kit-skills.tar.gz"
    ) in script
    assert "assets.jorisjonkers.dev" not in script
    assert "releases/latest" not in script
    assert '"${url}.sha256"' in script
    # A checksum mismatch refuses to run the bundle rather than silently
    # trusting an unverified download (tests/test_cloud_setup.py runs it).
    assert '[ "${expected}" != "${actual}" ]' in script


def test_uninstall_removes_a_legacy_kb_install_and_purges_hooks(tmp_path: Path) -> None:
    home = tmp_path / "home"
    claude_home = home / ".claude"
    codex_home = home / ".codex"
    (claude_home / "commands").mkdir(parents=True)
    (claude_home / "commands" / "speckit.plan.md").write_text("legacy")
    (claude_home / "skills" / "kb-first").mkdir(parents=True)
    (claude_home / "skills" / "kb-first" / "SKILL.md").write_text("legacy")
    council_dir = claude_home / "skills" / "council"
    council_dir.mkdir(parents=True)
    (council_dir / "council.mjs").write_text("legacy")
    (codex_home / "skills" / "speckit-plan").mkdir(parents=True)
    (codex_home / "skills" / "speckit-plan" / "SKILL.md").write_text("legacy")
    settings = claude_home / "settings.json"
    settings.write_text(
        json.dumps(
            {
                "hooks": {
                    "PreToolUse": [
                        {"hooks": [{"command": "~/.claude/hooks/pre-tool-use-edit-recall.sh"}]},
                    ],
                },
            },
        ),
    )

    script = tmp_path / "setup-workstation.sh"
    script.write_text(render_registry.render_setup_script(_registry()))
    result = subprocess.run(
        ["bash", str(script), "--uninstall"],
        capture_output=True,
        text=True,
        check=False,
        env={
            "HOME": str(home),
            "CLAUDE_CONFIG_DIR": str(claude_home),
            "CODEX_HOME": str(codex_home),
            "PATH": "/usr/bin:/bin",
        },
    )
    assert result.returncode == 0, result.stdout + result.stderr
    assert not (claude_home / "commands" / "speckit.plan.md").exists()
    assert not (claude_home / "skills" / "kb-first" / "SKILL.md").exists()
    assert not (council_dir / "council.mjs").exists()
    assert not (codex_home / "skills" / "speckit-plan" / "SKILL.md").exists()
    remaining = json.loads(settings.read_text())
    assert "hooks" not in remaining


def test_check_mode_uninstall_removes_nothing(tmp_path: Path) -> None:
    home = tmp_path / "home"
    claude_home = home / ".claude"
    codex_home = home / ".codex"
    (claude_home / "skills" / "kb-first").mkdir(parents=True)
    (claude_home / "skills" / "kb-first" / "SKILL.md").write_text("legacy")

    script = tmp_path / "setup-workstation.sh"
    script.write_text(render_registry.render_setup_script(_registry()))
    result = subprocess.run(
        ["bash", str(script), "--uninstall", "--check"],
        capture_output=True,
        text=True,
        check=False,
        env={
            "HOME": str(home),
            "CLAUDE_CONFIG_DIR": str(claude_home),
            "CODEX_HOME": str(codex_home),
            "PATH": "/usr/bin:/bin",
        },
    )
    assert result.returncode == 0, result.stdout + result.stderr
    assert (claude_home / "skills" / "kb-first" / "SKILL.md").exists()
    assert "would remove" in result.stdout


def test_memory_api_uses_the_shared_bank_endpoint() -> None:
    """hindsight-api.* routes only /mcp/default/; the multi-bank /mcp root 404s there."""
    server = next(s for s in _registry()["mcp_servers"] if s["name"] == "memory-api")
    assert server["url_workstation"].endswith("/mcp/default/")
    script = render_registry.render_setup_script(_registry())
    assert "hindsight-api.jorisjonkers.dev/mcp/default/" in script
    assert "hindsight-api.jorisjonkers.dev/mcp " not in script


@pytest.mark.parametrize("bad", ["/etc/hindsight.json", "~/.hindsight/x.json", "../outside.json", ""])
def test_plugin_user_config_path_stays_under_home(bad: str) -> None:
    data = _registry()
    plugin = data["plugins"][0]
    plugin["user_config"] = {"path": bad, "set": {"key": "value"}}
    with pytest.raises(render_registry.RegistryError, match="relative to \\$HOME"):
        render_registry.validate(data)


@pytest.mark.parametrize("bad", [{"bankId": "it's"}, {"it's": "x"}])
def test_plugin_user_config_set_may_not_break_shell_quoting(bad: dict) -> None:
    data = _registry()
    plugin = data["plugins"][0]
    plugin["user_config"] = {"path": ".example/config.json", "set": bad}
    with pytest.raises(render_registry.RegistryError, match="single quote"):
        render_registry.validate(data)


def test_user_config_merge_fails_when_the_value_does_not_read_back(tmp_path: Path) -> None:
    """Verify the value, not the exit code: an unwritable target must fail."""
    script = render_registry.render_setup_script(_registry())
    fn_start = script.index("_merge_user_config() {")
    fn = script[fn_start : script.index("\n}\n", fn_start) + 3]
    target = tmp_path / "ro" / "claude-code.json"
    target.parent.mkdir()
    target.write_text("{}")
    target.chmod(0o444)
    body = f'CHECK_ONLY=0\nlog() {{ :; }}\n{fn}\n_merge_user_config "{target}" \'{{"bankId": "default"}}\'\n'
    result = subprocess.run(["bash", "-c", body], capture_output=True, text=True, check=False)
    assert result.returncode != 0


def test_a_retired_server_is_removed_from_every_local_agent() -> None:
    """Retiring a server must unregister it from Claude, Codex, Vibe and local Hermes.

    Removing it only from Claude left `knowledge` connected in Codex long after
    it was retired; memory-mcp is the case that exposed it. converge_mcp covers
    Claude, Codex and Vibe (tests/test_converge_mcp.py); Hermes keeps its own pass.
    """
    data = _registry()
    retired = render_registry.retired_servers(data)
    assert "memory-mcp" in retired
    script = render_registry.render_setup_script(data)
    hermes_line = next(line for line in script.splitlines() if "mcp-servers.local.yaml\" --remove memory" in line)
    assert f"--retired {','.join(retired)} " in script
    for name in retired:
        assert f"--remove {name}" in hermes_line
    assert "memory-mcp" not in render_registry.render_hermes_local_mcp(data)


# --- B2-07: LiteLLM routes and their budget caps ---------------------------


def _litellm_table() -> dict:
    return json.loads((KIT_ROOT / "registry" / "generated" / "litellm" / "routes.json").read_text())


def test_litellm_routes_carry_the_owner_caps_and_no_key() -> None:
    """One route per agent id with its cap, rendered without any key value."""
    table = _litellm_table()
    caps = {route["agent"]: route["budget_eur"] for route in table["routes"]}
    # openrouter is capped at 5, local (the GPU workstation's server) is free: no cap; public is small.
    assert caps == {
        "vibe": 255, "vibe-study": 127.5, "opencode-go": 60, "openrouter": 5, "local": None, "public": 5,
    }
    # A key holds access groups of canonical models, not a provider wildcard each.
    assert {route["agent"]: route["models"] for route in table["routes"]} == {
        "vibe": ["canonical"],
        "vibe-study": ["canonical"],
        "opencode-go": ["canonical"],
        "openrouter": ["openrouter"],
        "local": ["local"],
        "public": ["google-free-public", "canonical"],
    }
    assert not any("*" in model or "/" in model for route in table["routes"] for model in route["models"])
    assert table["base_url"].startswith("https://") and table["key_header"] == "x-litellm-api-key"
    # Names of variables only: the keys live in the secret store.
    assert all(route["key_env"].startswith("LITELLM_") for route in table["routes"])


def test_every_capped_agent_key_resets_its_budget_every_30_days() -> None:
    routes = _litellm_table()["routes"]
    assert all(route["budget_duration"] == "30d" for route in routes if route["budget_eur"] is not None)
    assert all(route["budget_duration"] is None for route in routes if route["budget_eur"] is None)
    script = (KIT_ROOT / "registry" / "generated" / "litellm" / "make-keys.sh").read_text()
    for route in routes:
        request = next(line for line in script.splitlines() if f'"key_alias":"{route["agent"]}"' in line)
        body = json.loads(request.removeprefix("make_key '").removesuffix("'"))
        assert body.get("budget_duration") == route["budget_duration"] or route["budget_eur"] is None
        assert ("max_budget" in body) == (route["budget_eur"] is not None)


def _can_reach(routes: list[dict], agent: str, groups_of_deployment: set[str]) -> bool:
    """LiteLLM lets a key use a deployment when one of the key's `models` is one of its access groups."""
    route = next(r for r in routes if r["agent"] == agent)
    return bool(set(route["models"]) & groups_of_deployment)


def test_only_the_public_key_reaches_the_google_free_tier_deployments() -> None:
    """A Google free-tier deployment sits in `google-free-public` alone; no other key lists that group."""
    routes = _litellm_table()["routes"]
    google = {"google-free-public"}
    assert [r["agent"] for r in routes if _can_reach(routes, r["agent"], google)] == ["public"]
    for agent in ("vibe", "vibe-study", "opencode-go", "openrouter", "local"):
        assert not _can_reach(routes, agent, google), agent
        assert _can_reach(routes, agent, {"canonical"}) or agent in ("openrouter", "local")
    script = (KIT_ROOT / "registry" / "generated" / "litellm" / "make-keys.sh").read_text()
    assert [line for line in script.splitlines() if "google-free-public" in line and "make_key" in line] == [
        line for line in script.splitlines() if '"key_alias":"public"' in line
    ]


def test_the_public_key_is_stored_unexported_and_says_what_it_is_for() -> None:
    entry = next(e for e in _registry()["workstation_secrets"] if e["env"] == "LITELLM_PUBLIC_KEY")
    assert entry["export"] is False and "PUBLIC, NON-SENSITIVE" in entry["paste"]


def test_litellm_keys_are_stored_unexported_and_unchecked() -> None:
    secrets = {e["env"]: e for e in _registry()["workstation_secrets"]}
    for route in _registry()["litellm"]["routes"]:
        entry = secrets[route["key_env"]]
        assert entry["export"] is False and "check" not in entry and entry["unchecked"]


@pytest.mark.parametrize(
    ("mutate", "message"),
    [
        (lambda d: d["litellm"]["routes"][0].update(budget_eur=0), "positive number"),
        (lambda d: d["litellm"]["routes"][0].update(budget_eur="255"), "positive number"),
        (lambda d: d["litellm"]["routes"][0].update(budget_eur=1.234), "positive number"),
        (lambda d: d["litellm"]["routes"][0].update(budget_duration=None), "budget_duration must be 30d"),
        (lambda d: d["litellm"]["routes"][0].update(budget_duration="7d"), "budget_duration must be 30d"),
        (lambda d: d["litellm"]["routes"][4].update(budget_duration="30d"), "budget_duration must be 30d"),
        (lambda d: d["litellm"]["routes"][0].update(models=["vibe/*"]), "access group names"),
        (lambda d: d["litellm"]["routes"][0].update(models=["*"]), "access group names"),
        (lambda d: d["litellm"]["routes"][0].update(models=[]), "access group names"),
        (lambda d: d["litellm"]["routes"][0].update(models=["canonical", "canonical"]), "must not repeat"),
        (lambda d: d["litellm"]["routes"][0].update(data_scope="private"), "data_scope must be"),
        (lambda d: d["litellm"]["routes"][0].update(models=["google-free-public"]), "only a data_scope: public key"),
        (lambda d: d["litellm"]["routes"][5].update(models=["canonical"]), "only a data_scope: public key"),
        (lambda d: d["litellm"]["routes"][0].update(data_scope="public", models=["google-free-public"]), "only one public"),
        (lambda d: d["litellm"]["routes"][4].update(budget_eur=-1), "positive number"),
        (lambda d: d["litellm"]["routes"][1].update(agent="vibe"), "unique lower-case agent id"),
        (lambda d: d["litellm"]["routes"][1].update(key_env="LITELLM_VIBE_KEY"), "unique variable name"),
        (lambda d: d["litellm"]["routes"][0].update(key_env="NOT_A_SECRET"), "workstation secret"),
        (lambda d: d["litellm"]["routes"][0].pop("models"), "takes exactly"),
        (lambda d: d["litellm"].update(base_url="http://litellm.example/v1"), "https URL"),
        (lambda d: d["litellm"].update(routes=[]), "at least one agent id"),
        (lambda d: d["litellm"].update(key_header="X_Key"), "lower-case header"),
        (lambda d: d["litellm"].update(edge_token_env="bad name"), "environment variable name"),
        (lambda d: d["litellm"].update(edge_token_env="NOT_A_SECRET"), "must be a workstation secret"),
        (lambda d: d.update(litellm="x"), "must be a mapping"),
        (lambda d: d["litellm"]["routes"].append("x"), "must be a mapping"),
    ],
)
def test_a_bad_litellm_section_is_refused(mutate, message: str) -> None:
    data = copy.deepcopy(_registry())
    mutate(data)
    with pytest.raises(render_registry.RegistryError, match=message):
        render_registry.validate(data)


def test_an_unchecked_secret_may_not_also_name_a_check() -> None:
    data = copy.deepcopy(_registry())
    entry = next(e for e in data["workstation_secrets"] if e["env"] == "LITELLM_VIBE_KEY")
    entry["check"] = "https://litellm.jorisjonkers.dev/key/info"
    with pytest.raises(render_registry.RegistryError, match="either checked or unchecked"):
        render_registry.validate(data)


# --- NixOS: registry/generated/nixos/packages.nix --------------------------


def _nixos_group_block(rendered: str, group: str) -> str:
    start = rendered.index(f"  {group} = [")
    return rendered[start:rendered.index("\n  ];", start)]


def test_a_bad_nixos_attribute_path_is_rejected() -> None:
    data = _registry()
    data["nixos_packages"][0]["nixos"] = "no such attr"
    with pytest.raises(render_registry.RegistryError, match="nixpkgs attribute path"):
        render_registry.validate(data)
    data["nixos_packages"][0]["nixos"] = "texlive..combined"
    with pytest.raises(render_registry.RegistryError, match="nixpkgs attribute path"):
        render_registry.validate(data)


def test_a_bad_language_server_nixos_value_is_rejected() -> None:
    """A language server's nixos value is written into Nix source too, so it is held to the same rule."""
    data = _registry()
    server = next(s for s in data["language_servers"] if s.get("nixos"))
    server["nixos"] = 'gopls"; evil = "'
    with pytest.raises(render_registry.RegistryError, match="nixpkgs attribute path"):
        render_registry.validate(data)


def test_an_unknown_nixos_group_is_rejected() -> None:
    data = _registry()
    data["nixos_packages"][0]["group"] = "editor"
    with pytest.raises(render_registry.RegistryError, match="group must be one of"):
        render_registry.validate(data)


def test_a_duplicate_nixos_attribute_is_rejected() -> None:
    data = _registry()
    data["nixos_packages"].append(copy.deepcopy(data["nixos_packages"][0]))
    with pytest.raises(render_registry.RegistryError, match="declared twice"):
        render_registry.validate(data)


def test_a_cli_may_not_be_installed_by_nix() -> None:
    """Agent CLIs are agent-kit's to install; Nix never touches them."""
    data = _registry()
    data["clis"][0]["nixos"] = "claude-code"
    with pytest.raises(render_registry.RegistryError, match="never by Nix"):
        render_registry.validate(data)


def test_the_nixos_package_list_covers_the_language_servers() -> None:
    """packages.nix is the union of nixos_packages and the language servers' nixos keys."""
    rendered = render_registry.render_nixos_packages(_registry())
    core = _nixos_group_block(rendered, "core")
    assert '{ attr = "kotlin-lsp"; source = "nixos-modules"; }' in core
    assert '{ attr = "gopls"; source = "nixpkgs"; }' in core
    assert '{ attr = "ruby-lsp"; source = "nixpkgs"; }' in _nixos_group_block(rendered, "polyglot")
    assert re.findall(r"^  (\w+) = \[$", rendered, re.MULTILINE) == [
        "core", "kubernetes", "latex", "polyglot", "android",
    ]
