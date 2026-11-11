"""OpenCode: installed and configured from the registry on the workstation.

The setup assertions run the rendered script against a stubbed PATH, so they
check the tree a run leaves behind rather than the script's text. The merge,
the plugin-skill copy and the convergence run as subprocesses against
temporary files, the way setup calls them.
"""

from __future__ import annotations

import json
import subprocess
import sys
from pathlib import Path
from typing import Any

import pytest
import yaml

KIT_ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(KIT_ROOT / "scripts"))

import render_registry  # noqa: E402
from setup_harness import Run, setup_run  # noqa: E402

MERGE = KIT_ROOT / "scripts" / "opencode-merge-mcp.py"
PLUGIN_SKILLS = KIT_ROOT / "scripts" / "opencode-plugin-skills.py"
CONVERGE = KIT_ROOT / "scripts" / "converge.py"
FRAGMENT = KIT_ROOT / "registry" / "generated" / "opencode" / "opencode.json"
OPENCODE_V2 = 'echo "opencode $*" >> "$(dirname "$0")/../calls.log"\n[ "$1" = --version ] && echo "opencode v2.2.5"'


def _registry() -> dict:
    return yaml.safe_load((KIT_ROOT / "registry" / "estate-tooling.yaml").read_text())


def _servers(doc: dict[str, Any]) -> dict[str, Any]:
    return doc["mcp"]["servers"]


def _fragment() -> dict[str, Any]:
    return json.loads(render_registry.render_opencode_mcp(_registry()))


def _opencode_skills() -> list[str]:
    return [
        s["name"] for s in _registry()["local_skills"]
        if "opencode" in (s.get("agents") or []) and not s.get("pending")
    ]


def _opencode_home(run: Run) -> Path:
    return run.home / ".config" / "opencode"


# ---------------------------------------------------------------------------
# A whole setup run, with every external command stubbed
# ---------------------------------------------------------------------------


@pytest.fixture(scope="module")
def run(tmp_path_factory: pytest.TempPathFactory) -> Run:
    return setup_run(tmp_path_factory.mktemp("opencode"), "Darwin", stubs={"opencode": OPENCODE_V2})


@pytest.fixture(scope="module")
def run_over_an_existing_config(tmp_path_factory: pytest.TempPathFactory) -> Run:
    """A machine whose opencode.json holds a retired server, Private Memory, one of its own and other settings."""

    def seed(home: Path) -> None:
        config = home / ".config" / "opencode" / "opencode.json"
        config.parent.mkdir(parents=True)
        config.write_text(json.dumps({
            "theme": "dark",
            "mcp": {"timeout": 9000, "servers": {
                "memory-mcp": {"type": "remote", "url": "https://memory-mcp.example/mcp"},
                "memory-private": {"type": "remote", "url": "https://private.example/mcp"},
                "my-own-server": {"type": "local", "command": ["node", "/opt/mine.js"]},
                "drawio": {"type": "local", "command": ["old-drawio"], "enabled": False},
            }},
        }))

    return setup_run(tmp_path_factory.mktemp("opencode-existing"), "Darwin",
                     prepare_home=seed, stubs={"opencode": OPENCODE_V2})


def test_setup_installs_opencode_with_homebrew_when_it_is_missing(tmp_path: Path) -> None:
    result = setup_run(tmp_path, "Darwin")
    assert "brew install opencode" in result.calls


def test_without_homebrew_setup_says_so(tmp_path: Path) -> None:
    result = setup_run(tmp_path, "Linux", without=("brew",))
    assert "opencode: Homebrew is not on PATH; skipped (install Homebrew or OpenCode 2, then re-run)" in result.stderr


def test_setup_upgrades_only_a_homebrew_opencode(run: Run) -> None:
    """The update is guarded by `brew list --formula`, so a V1 from npm is not handed to brew."""
    assert "brew list --formula opencode" in run.calls
    assert "brew install opencode" not in run.calls
    assert "ok    opencode opencode v2.2.5" in run.stdout


def test_setup_writes_the_opencode_surface(run: Run) -> None:
    home = _opencode_home(run)
    doc = json.loads((home / "opencode.json").read_text())
    assert _servers(doc) == _servers(_fragment())
    assert doc["$schema"] == "https://opencode.ai/config.json"
    for skill in _opencode_skills():
        assert (home / "skills" / skill / "SKILL.md").is_file(), skill


@pytest.mark.parametrize("platform", ["Darwin", "Linux"])
def test_the_listings_equal_what_the_registry_declares(tmp_path: Path, platform: str) -> None:
    """After a run, OpenCode holds exactly the declared MCP servers and the declared kit skills, on both OSes.

    No plugin is listed here and the vendored skills are cloned from the network,
    so the skills folder holds the kit's skills and nothing else.
    """
    result = setup_run(tmp_path, platform, stubs={"opencode": OPENCODE_V2})
    home = _opencode_home(result)
    held_servers = set(_servers(json.loads((home / "opencode.json").read_text())))
    assert held_servers == set(render_registry.declared_servers(_registry(), "opencode"))
    held_skills = {folder.name for folder in (home / "skills").iterdir()}
    assert held_skills == set(_opencode_skills())


def test_opencode_gets_every_skill_the_other_agents_get() -> None:
    """The gateway, delegation and research skills are not left to the other agents."""
    claude = {s["name"] for s in _registry()["local_skills"] if "claude" in (s.get("agents") or [])}
    assert claude == set(_opencode_skills())


def test_the_pointer_names_only_shared_memory(run: Run) -> None:
    text = (_opencode_home(run) / "AGENTS.md").read_text()
    assert "<!-- managed by agent-kit setup-workstation.sh -->" in text
    assert "memory-api" in text
    assert "memory-private" not in text


def test_a_retired_server_and_private_memory_go_and_the_rest_of_the_file_stays(
    run_over_an_existing_config: Run,
) -> None:
    doc = json.loads((_opencode_home(run_over_an_existing_config) / "opencode.json").read_text())
    servers = _servers(doc)
    for name in [*render_registry.retired_servers(_registry()), "memory-private"]:
        assert name not in servers, name
    assert servers["my-own-server"] == {"type": "local", "command": ["node", "/opt/mine.js"]}
    assert doc["theme"] == "dark"
    assert doc["mcp"]["timeout"] == 9000


def test_a_server_the_user_disabled_stays_disabled(run_over_an_existing_config: Run) -> None:
    servers = _servers(json.loads((_opencode_home(run_over_an_existing_config) / "opencode.json").read_text()))
    assert servers["drawio"]["enabled"] is False
    assert servers["drawio"]["command"] == _servers(_fragment())["drawio"]["command"]


@pytest.mark.parametrize("printed", ["1.19.5", "v1.19.5", "opencode 1.19.5", "opencode v1.19.5"])
def test_an_opencode_1_binary_is_named(tmp_path: Path, printed: str) -> None:
    v1 = f'[ "$1" = --version ] && echo "{printed}"'
    result = setup_run(tmp_path, "Darwin", stubs={"opencode": v1})
    assert f"opencode: {printed} is OpenCode 1; its MCP config is left alone" in result.stderr


@pytest.mark.parametrize("printed", ["opencode v2.2.5", "2.2.5", "opencode v11.0.0"])
def test_an_opencode_2_binary_is_not_named(tmp_path: Path, printed: str) -> None:
    v2 = f'[ "$1" = --version ] && echo "{printed}"'
    result = setup_run(tmp_path, "Darwin", stubs={"opencode": v2})
    assert "is OpenCode 1" not in result.stderr


def test_the_running_service_is_restarted(tmp_path: Path) -> None:
    running = OPENCODE_V2 + '\n[ "$1 $2" = "service status" ] && echo "http://127.0.0.1:4096"\nexit 0'
    result = setup_run(tmp_path, "Darwin", stubs={"opencode": running})
    assert "opencode service restart" in result.calls
    assert "OpenCode service restarted" in result.stdout


def test_a_stopped_service_is_left_stopped(run: Run) -> None:
    assert "opencode service restart" not in run.calls


def test_a_check_run_writes_nothing(tmp_path: Path) -> None:
    result = setup_run(tmp_path, "Darwin", args=("--check",), stubs={"opencode": OPENCODE_V2})
    assert not (result.home / ".config" / "opencode").exists()
    assert "would merge OpenCode MCP servers" in result.stdout


# ---------------------------------------------------------------------------
# Plugin skills, through a whole setup run
# ---------------------------------------------------------------------------


def _plugin(root: Path, name: str, skills: dict[str, str], manifest: dict | None = None) -> Path:
    """A plugin install folder: ``skills`` maps a path under it to the skill's name."""
    plugin = root / name
    for rel, skill in skills.items():
        folder = plugin / rel
        folder.mkdir(parents=True)
        (folder / "SKILL.md").write_text(f"---\nname: {skill}\ndescription: {skill} skill\n---\nbody\n")
        (folder / "notes.md").write_text("more\n")
    if manifest is not None:
        (plugin / ".claude-plugin").mkdir(parents=True, exist_ok=True)
        (plugin / ".claude-plugin" / "plugin.json").write_text(json.dumps(manifest))
    return plugin


def _fake_claude(listing: list[dict[str, Any]]) -> str:
    out = json.dumps(listing)
    return f"if [ \"$1 $2 $3\" = 'plugin list --json' ]; then cat <<'EOF'\n{out}\nEOF\nfi"


def test_setup_copies_the_enabled_plugins_skills(tmp_path: Path) -> None:
    caveman = _plugin(tmp_path / "cache", "caveman", {"skills/caveman": "caveman"})
    listing = [{"id": "caveman@caveman", "scope": "user", "enabled": True, "installPath": str(caveman)}]
    result = setup_run(tmp_path / "run", "Darwin", stubs={"opencode": OPENCODE_V2, "claude": _fake_claude(listing)})
    skills = _opencode_home(result) / "skills"
    assert (skills / "caveman" / "SKILL.md").is_file()
    assert (skills / "caveman" / ".agent-kit-plugin-skill").read_text() == "caveman@caveman\n"
    assert "frontend-design@agent-kit-official: not installed in Claude Code" in result.stdout


def test_the_setup_step_names_only_enabled_plugins() -> None:
    data = _registry()
    ids = render_registry.opencode_plugin_ids(data)
    assert "caveman@caveman" in ids
    assert "mattpocock-skills@agent-kit-official" in ids
    for plugin in data["plugins"]:
        if plugin.get("enabled") is False:
            assert not [i for i in ids if i.startswith(f"{plugin['name']}@")], plugin["name"]


# ---------------------------------------------------------------------------
# The plugin-skill copy on its own
# ---------------------------------------------------------------------------


def _copy(tmp_path: Path, listing: list[dict[str, Any]], ids: str, *extra: str) -> subprocess.CompletedProcess[str]:
    bin_dir = tmp_path / "bin"
    bin_dir.mkdir(exist_ok=True)
    claude = bin_dir / "claude"
    claude.write_text("#!/usr/bin/env bash\n" + _fake_claude(listing) + "\n")
    claude.chmod(0o755)
    return subprocess.run(
        [sys.executable, str(PLUGIN_SKILLS), "--skills-dir", str(tmp_path / "skills"), "--plugins", ids, *extra],
        capture_output=True, text=True, check=False,
        env={"PATH": f"{bin_dir}:/usr/bin:/bin", "HOME": str(tmp_path)},
    )


def test_skills_are_found_the_way_claude_finds_them(tmp_path: Path) -> None:
    cache = tmp_path / "cache"
    listed = _plugin(cache, "matt", {"skills/engineering/tdd": "tdd", "skills/in-progress/draft": "draft"},
                     manifest={"skills": ["./skills/engineering/tdd"]})
    default = _plugin(cache, "cave", {"skills/caveman": "caveman", "skills/cavecrew": "cavecrew"})
    root = _plugin(cache, "k8s", {".": "kubernetes-skill"})
    (root / ".claude-plugin").mkdir()
    (root / ".claude-plugin" / "marketplace.json").write_text("{}")
    listing = [
        {"id": f"{p.name}@m", "scope": "user", "enabled": True, "installPath": str(p)}
        for p in (listed, default, root)
    ]
    result = _copy(tmp_path, listing, "matt@m,cave@m,k8s@m")
    assert result.returncode == 0, result.stdout + result.stderr
    skills = tmp_path / "skills"
    assert sorted(p.name for p in skills.iterdir() if p.is_dir()) == ["cavecrew", "caveman", "kubernetes-skill", "tdd"]
    assert (skills / "tdd" / "notes.md").is_file()
    assert not (skills / "kubernetes-skill" / ".claude-plugin").exists()


def test_a_skill_from_another_source_is_left_alone(tmp_path: Path) -> None:
    plugin = _plugin(tmp_path / "cache", "p", {"skills/council": "council"})
    own = tmp_path / "skills" / "council"
    own.mkdir(parents=True)
    (own / "SKILL.md").write_text("the kit's own\n")
    result = _copy(tmp_path, [{"id": "p@m", "scope": "user", "installPath": str(plugin)}], "p@m")
    assert result.returncode == 0
    assert (own / "SKILL.md").read_text() == "the kit's own\n"
    assert "came from another source; left alone" in result.stdout


def test_a_skill_no_plugin_ships_any_more_is_removed(tmp_path: Path) -> None:
    plugin = _plugin(tmp_path / "cache", "p", {"skills/one": "one", "skills/two": "two"})
    entry = {"id": "p@m", "scope": "user", "installPath": str(plugin)}
    assert _copy(tmp_path, [entry], "p@m").returncode == 0
    assert (tmp_path / "skills" / "two").is_dir()
    import shutil
    shutil.rmtree(plugin / "skills" / "two")
    result = _copy(tmp_path, [entry], "p@m")
    assert result.returncode == 0
    assert not (tmp_path / "skills" / "two").exists()
    assert (tmp_path / "skills" / "one" / "SKILL.md").is_file()


def test_a_plugin_dropped_from_the_list_takes_its_skills_with_it(tmp_path: Path) -> None:
    plugin = _plugin(tmp_path / "cache", "p", {"skills/one": "one"})
    entry = {"id": "p@m", "scope": "user", "installPath": str(plugin)}
    _copy(tmp_path, [entry], "p@m")
    _copy(tmp_path, [entry], "")
    assert not (tmp_path / "skills" / "one").exists()


def test_a_plugin_disabled_in_claude_ships_no_skills(tmp_path: Path) -> None:
    plugin = _plugin(tmp_path / "cache", "p", {"skills/one": "one"})
    entry = {"id": "p@m", "scope": "user", "enabled": True, "installPath": str(plugin)}
    _copy(tmp_path, [entry], "p@m")
    result = _copy(tmp_path, [{**entry, "enabled": False}], "p@m")
    assert result.returncode == 0
    assert "p@m: disabled in Claude Code" in result.stdout
    assert not (tmp_path / "skills" / "one").exists()


def test_a_marked_folder_behind_a_symlink_is_never_removed(tmp_path: Path) -> None:
    """Only real folders directly in the skills folder count as copies this script made."""
    victim = tmp_path / "victim"
    victim.mkdir()
    (victim / "keep.txt").write_text("mine\n")
    (victim / ".agent-kit-plugin-skill").write_text("p@m\n")
    skills = tmp_path / "skills"
    skills.mkdir()
    (skills / "evil").symlink_to(victim)
    result = _copy(tmp_path, [], "")
    assert result.returncode == 0
    assert (victim / "keep.txt").read_text() == "mine\n"
    assert (skills / "evil").is_symlink()


def test_an_unmarked_skill_folder_is_never_removed(tmp_path: Path) -> None:
    own = tmp_path / "skills" / "mine"
    own.mkdir(parents=True)
    (own / "SKILL.md").write_text("mine\n")
    assert _copy(tmp_path, [], "").returncode == 0
    assert (own / "SKILL.md").read_text() == "mine\n"


def test_a_plugin_json_path_outside_the_plugin_is_ignored(tmp_path: Path) -> None:
    outside = _plugin(tmp_path, "elsewhere", {"secret": "secret"})
    plugin = _plugin(tmp_path / "cache", "p", {"skills/one": "one"},
                     manifest={"skills": [str(outside), "../../elsewhere"]})
    result = _copy(tmp_path, [{"id": "p@m", "scope": "user", "installPath": str(plugin)}], "p@m")
    assert result.returncode == 0
    assert (tmp_path / "skills" / "one" / "SKILL.md").is_file()
    assert not (tmp_path / "skills" / "secret").exists()


def test_check_changes_nothing(tmp_path: Path) -> None:
    plugin = _plugin(tmp_path / "cache", "p", {"skills/one": "one"})
    result = _copy(tmp_path, [{"id": "p@m", "scope": "user", "installPath": str(plugin)}], "p@m", "--check")
    assert result.returncode == 0
    assert "would copy one" in result.stdout
    assert not (tmp_path / "skills").exists()


def test_a_failing_claude_changes_nothing_and_says_so(tmp_path: Path) -> None:
    bin_dir = tmp_path / "bin"
    bin_dir.mkdir()
    (bin_dir / "claude").write_text("#!/usr/bin/env bash\nexit 3\n")
    (bin_dir / "claude").chmod(0o755)
    result = subprocess.run(
        [sys.executable, str(PLUGIN_SKILLS), "--skills-dir", str(tmp_path / "skills"), "--plugins", "p@m"],
        capture_output=True, text=True, check=False, env={"PATH": f"{bin_dir}:/usr/bin:/bin"},
    )
    assert result.returncode == 1
    assert "claude plugin list failed" in result.stderr
    assert not (tmp_path / "skills").exists()


# ---------------------------------------------------------------------------
# The merge on its own
# ---------------------------------------------------------------------------


def _merge(config: Path, *extra: str) -> subprocess.CompletedProcess[str]:
    return subprocess.run([sys.executable, str(MERGE), str(FRAGMENT), "--config", str(config), *extra],
                          capture_output=True, text=True, check=False)


def test_merge_is_idempotent_and_backs_up_the_first_version_only(tmp_path: Path) -> None:
    config = tmp_path / "opencode.json"
    config.write_text(json.dumps({"model": "x"}))
    assert _merge(config).returncode == 0
    first = config.read_text()
    assert _merge(config).returncode == 0
    assert config.read_text() == first
    assert json.loads((tmp_path / "opencode.json.bak").read_text()) == {"model": "x"}


def test_merge_writes_a_fresh_config(tmp_path: Path) -> None:
    config = tmp_path / "new" / "opencode.json"
    assert _merge(config).returncode == 0
    assert _servers(json.loads(config.read_text())) == _servers(_fragment())


def test_merge_refuses_a_config_with_comments(tmp_path: Path) -> None:
    config = tmp_path / "opencode.json"
    config.write_text('{\n  // mine\n  "model": "x"\n}\n')
    result = _merge(config)
    assert result.returncode == 1
    assert "is not plain JSON" in result.stderr
    assert config.read_text() == '{\n  // mine\n  "model": "x"\n}\n'


# ---------------------------------------------------------------------------
# Convergence: undeclared servers leave, the keep file spares them
# ---------------------------------------------------------------------------


def _converge(tmp_path: Path, servers: list[str], keep: list[str], *extra: str) -> tuple[Path, str]:
    config = tmp_path / "opencode.json"
    config.write_text(json.dumps({"mcp": {"servers": {s: {"type": "remote", "url": "https://x"} for s in servers}}}))
    keep_file = tmp_path / "keep.yaml"
    keep_file.write_text(yaml.safe_dump({"keep": keep}))
    declared = ",".join(render_registry.declared_servers(_registry(), "opencode"))
    result = subprocess.run(
        [sys.executable, str(CONVERGE), "mcp", "--keep", str(keep_file), "--claude-home", str(tmp_path),
         "--opencode", str(config), "--declared", f"opencode={declared}",
         "--retired", ",".join(render_registry.retired_servers(_registry())), "--private", "memory-private", *extra],
        capture_output=True, text=True, check=False,
    )
    assert result.returncode == 0, result.stdout + result.stderr
    return config, result.stdout


def test_converge_removes_undeclared_retired_and_private_servers(tmp_path: Path) -> None:
    config, out = _converge(tmp_path, ["memory-api", "pycharm", "vuetify", "memory-private", "context7"], ["context7"])
    assert set(_servers(json.loads(config.read_text()))) == {"memory-api", "context7"}
    assert "removed pycharm (opencode, undeclared)" in out
    assert "removed memory-private (opencode, private)" in out


def test_converge_check_changes_nothing(tmp_path: Path) -> None:
    config, out = _converge(tmp_path, ["memory-api", "pycharm"], [], "--check")
    assert set(_servers(json.loads(config.read_text()))) == {"memory-api", "pycharm"}
    assert "would remove pycharm (opencode, undeclared)" in out


def test_a_commented_opencode_json_does_not_stop_the_other_surfaces(tmp_path: Path) -> None:
    """The file is left as it is and reported; Vibe still loses its undeclared server."""
    config = tmp_path / "opencode.json"
    commented = '{\n  // mine\n  "mcp": {"servers": {"pycharm": {"type": "remote", "url": "https://x"}}}\n}\n'
    config.write_text(commented)
    vibe = tmp_path / "config.toml"
    vibe.write_text('[[mcp_servers]]\nname = "pycharm"\ntransport = "stdio"\ncommand = "x"\n')
    result = subprocess.run(
        [sys.executable, str(CONVERGE), "mcp", "--keep", str(tmp_path / "keep.yaml"), "--claude-home", str(tmp_path),
         "--opencode", str(config), "--vibe", str(vibe), "--declared", "opencode=memory-api",
         "--declared", "vibe=memory-api", "--retired", "", "--private", ""],
        capture_output=True, text=True, check=False,
    )
    assert result.returncode == 1, result.stdout + result.stderr
    assert "is not plain JSON; opencode not converged" in result.stdout
    assert "removed pycharm (vibe, undeclared)" in result.stdout
    assert config.read_text() == commented


def test_opencode_declares_no_hindsight_server() -> None:
    """Hindsight Coding Agents installs for Claude Code and Codex only."""
    assert "hindsight" not in render_registry.declared_servers(_registry(), "opencode")


# ---------------------------------------------------------------------------
# The fragment
# ---------------------------------------------------------------------------


def test_the_fragment_covers_every_workstation_server_except_private_memory() -> None:
    data = _registry()
    workstation = {
        s["name"] for s in data["mcp_servers"]
        if "workstation" in (s.get("surfaces") or []) and s.get("enabled") is not False
    }
    assert set(_servers(_fragment())) == workstation - {"memory-private"}
    assert render_registry.OPENCODE_MCP in render_registry.artifacts(data)


def test_no_fragment_entry_carries_a_secret(monkeypatch: pytest.MonkeyPatch) -> None:
    data = _registry()
    for server in data["mcp_servers"]:
        if server.get("credential"):
            monkeypatch.setenv(server["credential"], "sentinel-token-value")
    text = render_registry.render_opencode_mcp(data)
    assert "sentinel-token-value" not in text
    servers = _servers(json.loads(text))
    for server in data["mcp_servers"]:
        credential = server.get("credential")
        if not credential or server["name"] not in servers or server["transport"] != "http":
            continue
        entry = servers[server["name"]]
        assert entry["headers"] == {"Authorization": f"Bearer {{env:{credential}}}"}
        assert entry["oauth"] is False


def test_an_oauth_server_is_left_to_opencodes_own_sign_in() -> None:
    entry = _servers(_fragment())["n8n"]
    assert entry == {"type": "remote", "url": "https://n8n.jorisjonkers.dev/mcp-server/http", "timeout": 60000}


def test_timeouts_are_milliseconds() -> None:
    data = _registry()
    servers = _servers(_fragment())
    for server in render_registry.opencode_servers(data):
        if server.get("timeout"):
            want = max(server["timeout"], server.get("startup_timeout") or 0) * 1000
            assert servers[server["name"]]["timeout"] == want, server["name"]


def test_a_stdio_server_runs_its_command_and_args() -> None:
    entry = _servers(_fragment())["playwright"]
    assert entry["type"] == "local"
    assert entry["command"] == ["playwright-mcp", "--headless", "--browser", "chromium"]


def test_validation_rejects_private_memory_for_opencode() -> None:
    data = _registry()
    private = next(s for s in data["mcp_servers"] if s["name"] == "memory-private")
    private["enabled_for"] = ["vibe-private", "opencode"]
    with pytest.raises(render_registry.RegistryError, match="OpenCode never gets Private Memory"):
        render_registry.validate(data)


def test_opencode_is_off_the_cloud_and_off_every_other_surface() -> None:
    data = _registry()
    cli = next(c for c in data["clis"] if c["name"] == "opencode")
    assert cli["surfaces"] == ["workstation"]
    assert cli["cloud"] is False
    for rendered in (
        render_registry.render_hermes_local_mcp(data),
        render_registry.render_container_setup_script(data),
    ):
        assert "opencode" not in rendered.lower()
    script = render_registry.render_setup_script(data)
    assert '[ "${CLOUD}" = 1 ] && OPENCODE_READY=0' in script
