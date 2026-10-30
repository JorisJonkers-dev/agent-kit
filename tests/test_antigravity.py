"""Antigravity: installed and configured from the registry, on macOS only.

The setup assertions run the rendered script against a stubbed PATH, so they
check the tree a run leaves behind rather than the script's text. The proxy's
own behaviour is tests/test_antigravity_proxy.py; the merge's is
tests/test_antigravity_merge.py.
"""

from __future__ import annotations

import json
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path

import pytest
import yaml

KIT_ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(KIT_ROOT / "scripts"))

import render_registry  # noqa: E402

SETUP = KIT_ROOT / "installer" / "setup-workstation.sh"
SIGN_IN = "antigravity: needs sign-in: run agy and sign in with your Google account"
SKIPPED = (
    "antigravity-cli: macOS only; skipped on Linux",
    "antigravity: macOS only; skipped on Linux",
)


def _registry() -> dict:
    return yaml.safe_load((KIT_ROOT / "registry" / "estate-tooling.yaml").read_text())


def _script() -> str:
    return render_registry.render_setup_script(_registry())


def _antigravity_skills() -> list[str]:
    return [s["name"] for s in _registry()["local_skills"] if "antigravity" in (s.get("agents") or [])]


# ---------------------------------------------------------------------------
# A whole setup run, with every external command stubbed
# ---------------------------------------------------------------------------


@dataclass(frozen=True)
class Run:
    home: Path
    stdout: str
    stderr: str
    calls: list[str]


def _stub(bin_dir: Path, name: str, body: str) -> None:
    path = bin_dir / name
    path.write_text("#!/usr/bin/env bash\n" + body + "\n")
    path.chmod(0o755)


def _setup_run(root: Path, platform: str, seed: dict | None = None) -> Run:
    """Run the checked-in setup script with HOME in a temp dir and nothing real on PATH.

    ``agy`` is on PATH on both platforms: the skip has to come from the platform
    gate, not from a binary that happens to be missing. ``seed`` is written to
    Antigravity's MCP config before the run, standing in for a config that was
    already there.
    """
    bin_dir = root / "bin"
    bin_dir.mkdir(parents=True)
    home = root / "home"
    home.mkdir()
    if seed is not None:
        config = home / ".gemini" / "config" / "mcp_config.json"
        config.parent.mkdir(parents=True)
        config.write_text(json.dumps(seed, indent=2) + "\n")
    calls = root / "calls.log"
    for name in ("uv", "npm", "node", "npx", "curl", "git", "claude", "codex", "vibe", "gh", "jq", "docker"):
        _stub(bin_dir, name, f'echo "{name} $*" >> "{calls}"')
    _stub(bin_dir, "uname", f"echo {platform}")
    _stub(bin_dir, "security", f'echo "security $*" >> "{calls}"')
    _stub(bin_dir, "agy", 'echo "agy 1.0.0"')
    # Homebrew answers `list --cask` for what `install --cask` recorded, so a
    # fresh machine takes the install branch and a second run the upgrade one.
    _stub(
        bin_dir,
        "brew",
        f'echo "brew $*" >> "{calls}"\n'
        'case "$1 $2" in\n'
        f'  "list --cask") [ -f "{root}/casks/$3" ] ;;\n'
        f'  "install --cask") mkdir -p "{root}/casks" && : > "{root}/casks/$3" ;;\n'
        "esac",
    )
    result = subprocess.run(
        ["bash", str(SETUP)],
        capture_output=True,
        text=True,
        check=False,
        env={"HOME": str(home), "PATH": f"{bin_dir}:/usr/bin:/bin"},
    )
    return Run(
        home=home,
        stdout=result.stdout,
        stderr=result.stderr,
        calls=calls.read_text().splitlines() if calls.exists() else [],
    )


@pytest.fixture(scope="module")
def macos_run(tmp_path_factory: pytest.TempPathFactory) -> Run:
    return _setup_run(tmp_path_factory.mktemp("macos"), "Darwin")


@pytest.fixture(scope="module")
def linux_run(tmp_path_factory: pytest.TempPathFactory) -> Run:
    return _setup_run(tmp_path_factory.mktemp("linux"), "Linux")


@pytest.fixture(scope="module")
def macos_run_over_an_existing_config(tmp_path_factory: pytest.TempPathFactory) -> Run:
    """A machine whose mcp_config.json already holds a retired server and one of its own."""
    return _setup_run(
        tmp_path_factory.mktemp("macos-existing"),
        "Darwin",
        seed={
            "mcpServers": {
                "memory-mcp": {"command": "npx", "args": ["-y", "memory-mcp"]},
                "my-own-server": {"command": "node", "args": ["/opt/mine.js"]},
            },
            "somethingElse": {"kept": True},
        },
    )


# ---------------------------------------------------------------------------
# The casks
# ---------------------------------------------------------------------------


def test_setup_installs_both_casks_on_macos(macos_run: Run) -> None:
    assert "brew install --cask antigravity-cli" in macos_run.calls
    assert "brew install --cask antigravity" in macos_run.calls
    assert "antigravity-cli installed" in macos_run.stdout
    assert "antigravity installed" in macos_run.stdout


def test_setup_skips_the_whole_surface_on_linux(linux_run: Run) -> None:
    """Two notices and nothing else: no cask, no proxy, no ~/.gemini."""
    lines = [line for line in (linux_run.stdout + linux_run.stderr).splitlines() if "antigravity" in line.lower()]
    assert lines, "the skip is silent"
    for line in lines:
        assert line.endswith(SKIPPED), line
    for message in SKIPPED:
        assert f"warn  {message}" in linux_run.stderr
    assert not [call for call in linux_run.calls if call.startswith("brew")]
    assert not (linux_run.home / ".gemini").exists()
    assert not (linux_run.home / ".local" / "bin" / "antigravity-mcp-proxy").exists()


# ---------------------------------------------------------------------------
# The configuration a macOS run converges
# ---------------------------------------------------------------------------


def test_setup_writes_the_antigravity_surface_on_macos(macos_run: Run) -> None:
    config = macos_run.home / ".gemini" / "config"
    assert (macos_run.home / ".local" / "bin" / "antigravity-mcp-proxy").is_file()
    assert (config / "AGENTS.md").is_file()
    merged = json.loads((config / "mcp_config.json").read_text())["mcpServers"]
    assert merged == json.loads(render_registry.render_antigravity_mcp(_registry()))["mcpServers"]
    for skill in _antigravity_skills():
        assert (config / "skills" / skill / "SKILL.md").is_file(), skill
        assert (macos_run.home / ".gemini" / "antigravity-cli" / "skills" / skill / "SKILL.md").is_file(), skill


def test_the_pointer_and_skills_stay_out_of_the_shared_gemini_root(macos_run: Run) -> None:
    """~/.gemini/AGENTS.md and GEMINI.md belong to Gemini CLI, which reads them too."""
    gemini = macos_run.home / ".gemini"
    assert not (gemini / "AGENTS.md").exists()
    assert not (gemini / "GEMINI.md").exists()
    script = _script()
    assert "${HOME}/.gemini/AGENTS.md" not in script
    assert "${HOME}/.gemini/GEMINI.md" not in script
    assert '_agents_pointer "${ANTIGRAVITY_HOME}/AGENTS.md" shared' in script


def test_the_pointer_names_only_shared_memory(macos_run: Run) -> None:
    text = (macos_run.home / ".gemini" / "config" / "AGENTS.md").read_text()
    assert "<!-- managed by agent-kit setup-workstation.sh -->" in text
    assert "memory-api" in text
    assert "memory-private" not in text


def test_a_retired_server_is_removed_and_the_rest_of_the_file_kept(
    macos_run_over_an_existing_config: Run,
) -> None:
    """The merge only adds and replaces, so a retired server needs --remove to leave."""
    config = macos_run_over_an_existing_config.home / ".gemini" / "config" / "mcp_config.json"
    doc = json.loads(config.read_text())
    retired = render_registry.retired_servers(_registry())
    assert retired, "the registry retires nothing, so this test proves nothing"
    for name in retired:
        assert name not in doc["mcpServers"], name
    assert doc["mcpServers"]["my-own-server"] == {"command": "node", "args": ["/opt/mine.js"]}
    assert doc["somethingElse"] == {"kept": True}


# ---------------------------------------------------------------------------
# The MCP fragment
# ---------------------------------------------------------------------------


def test_the_fragment_covers_every_workstation_server_except_private_memory() -> None:
    data = _registry()
    servers = json.loads(render_registry.render_antigravity_mcp(data))["mcpServers"]
    workstation = {
        s["name"]
        for s in data["mcp_servers"]
        if "workstation" in (s.get("surfaces") or []) and s.get("enabled") is not False
    }
    assert set(servers) == workstation - {"memory-private"}
    assert render_registry.ANTIGRAVITY_MCP in render_registry.artifacts(data)


def test_no_fragment_entry_carries_a_secret(monkeypatch: pytest.MonkeyPatch) -> None:
    """Antigravity expands no environment variables, so a header would be a token on disk."""
    data = _registry()
    for server in data["mcp_servers"]:
        if server.get("credential"):
            monkeypatch.setenv(server["credential"], "sentinel-token-value")
    fragment = render_registry.render_antigravity_mcp(data)
    assert "sentinel-token-value" not in fragment
    servers = json.loads(fragment)["mcpServers"]
    for name, entry in servers.items():
        assert "headers" not in entry, f"{name} would put a literal token on disk"
        assert "Authorization" not in json.dumps(entry), name
    for server in data["mcp_servers"]:
        credential = server.get("credential")
        if not credential or server["name"] not in servers:
            continue
        entry = servers[server["name"]]
        if server["transport"] == "http":
            # The stdio proxy reads the token from the secret store at start-up.
            assert entry["command"] == "/bin/sh"
            assert "$HOME/.local/bin/antigravity-mcp-proxy" in entry["args"][1]
            assert f"--secret {credential}" in entry["args"][1]
        else:
            assert credential not in (entry.get("env") or {})


def test_every_fragment_entry_carries_the_registry_timeout() -> None:
    """The credential branch renders memory-api, and must not skip its timeout on the way."""
    data = _registry()
    servers = json.loads(render_registry.render_antigravity_mcp(data))["mcpServers"]
    timed = [s for s in render_registry.antigravity_servers(data) if s.get("timeout")]
    assert timed, "no server sets a timeout, so this test proves nothing"
    for server in timed:
        assert servers[server["name"]]["timeoutSeconds"] == server["timeout"], server["name"]


def test_validation_rejects_private_memory_for_antigravity() -> None:
    data = _registry()
    private = next(s for s in data["mcp_servers"] if s["name"] == "memory-private")
    private["enabled_for"] = ["vibe-private", "antigravity"]
    with pytest.raises(render_registry.RegistryError, match="never gets Private Memory"):
        render_registry.validate(data)


def test_validation_rejects_an_unknown_skill_agent() -> None:
    """A typo in `agents:` is a registry error, not a KeyError from the homes table."""
    data = _registry()
    skill = next(s for s in data["local_skills"] if "antigravity" in (s.get("agents") or []))
    skill["agents"] = ["claude", "antigravty"]
    with pytest.raises(render_registry.RegistryError, match="unknown agents"):
        render_registry.validate(data)


# ---------------------------------------------------------------------------
# Surfaces Antigravity is not on
# ---------------------------------------------------------------------------


def test_the_runner_hermes_and_container_outputs_have_no_antigravity() -> None:
    data = _registry()
    for rendered in (
        render_registry.render_hermes_mcp(data),
        render_registry.render_hermes_local_mcp(data),
        render_registry.render_container_setup_script(data),
    ):
        assert "antigravity" not in rendered.lower()
    for path in (KIT_ROOT / "runner-manifests").rglob("*"):
        if path.is_file():
            assert "antigravity" not in path.read_text(errors="ignore").lower(), path


# ---------------------------------------------------------------------------
# The doctor
# ---------------------------------------------------------------------------


def _signin(tmp_path: Path, *, agy: bool, keychain: bool, cask: bool = False) -> subprocess.CompletedProcess[str]:
    script = _script()
    start = script.index("report_antigravity_signin() {")
    fn = script[start : script.index("\n}\n", start) + 3]
    bin_dir = tmp_path / "bin"
    bin_dir.mkdir()
    if agy:
        _stub(bin_dir, "agy", "exit 0")
    _stub(bin_dir, "security", "exit 0" if keychain else "exit 1")
    if cask:
        _stub(
            bin_dir,
            "brew",
            'case "$1 $2 $3" in\n  "list --cask antigravity") exit 0 ;;\nesac\nexit 1',
        )
    body = 'warn() { echo "warn $*"; }\n' + fn + "\nreport_antigravity_signin\n"
    return subprocess.run(
        ["bash", "-c", body],
        capture_output=True,
        text=True,
        check=False,
        env={"HOME": str(tmp_path), "PATH": f"{bin_dir}:/usr/bin:/bin"},
    )


def test_the_doctor_names_a_missing_sign_in(tmp_path: Path) -> None:
    result = _signin(tmp_path, agy=True, keychain=False)
    assert result.returncode == 0, result.stderr
    assert f"warn {SIGN_IN}" in result.stdout


def test_the_doctor_reports_for_the_desktop_app_alone(tmp_path: Path) -> None:
    """No `agy`, but the app's cask is installed: still a machine that needs a sign-in."""
    result = _signin(tmp_path, agy=False, keychain=False, cask=True)
    assert result.returncode == 0, result.stderr
    assert f"warn {SIGN_IN}" in result.stdout


def test_the_doctor_is_quiet_for_the_desktop_app_once_signed_in(tmp_path: Path) -> None:
    result = _signin(tmp_path, agy=False, keychain=True, cask=True)
    assert result.returncode == 0, result.stderr
    assert result.stdout == ""


def test_the_doctor_is_quiet_once_the_keychain_holds_the_credential(tmp_path: Path) -> None:
    result = _signin(tmp_path, agy=True, keychain=True)
    assert result.returncode == 0, result.stderr
    assert result.stdout == ""


def test_the_doctor_says_nothing_without_antigravity(tmp_path: Path) -> None:
    """Neither `agy` nor the app's cask: nothing to sign in to."""
    result = _signin(tmp_path, agy=False, keychain=False)
    assert result.returncode == 0, result.stderr
    assert result.stdout == ""


def test_the_doctor_runs_under_check_too() -> None:
    """A --check run reports the missing sign-in; the call is not inside a CHECK_ONLY guard."""
    assert "\nreport_antigravity_signin\n" in _script()
