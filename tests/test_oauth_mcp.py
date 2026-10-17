"""An `oauth` MCP server is registered by URL only, and each client signs in itself.

The setup tests render setup from the registry and run the n8n step under bash
against a fake machine: a temporary HOME with two Claude profiles and Codex,
and fake `claude`, `codex`, `vibe`, `security` and `uv` on PATH. FAKE_SIGNED_IN names
the clients that have signed in (`claude:<profile dir>`, `codex`, `vibe`).
"""

from __future__ import annotations

import copy
import json
import subprocess
import sys
import tomllib
from pathlib import Path

import pytest

KIT_ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(KIT_ROOT / "scripts"))

import render_registry  # noqa: E402
import yaml  # noqa: E402

PY = sys.executable
MERGE = KIT_ROOT / "scripts" / "vibe-merge-mcp.py"
URL = "https://n8n.jorisjonkers.dev/mcp-server/http"

FAKE_CLAUDE = f"""#!{PY}
import json, os, sys
args = sys.argv[1:]
config_dir = os.environ.get("CLAUDE_CONFIG_DIR")
path = os.path.join(config_dir, ".claude.json") if config_dir else os.path.join(os.environ["HOME"], ".claude.json")
data = json.load(open(path))
servers = data.setdefault("mcpServers", {{}})
profile = config_dir or os.path.join(os.environ["HOME"], ".claude")
if args[:4] == ["mcp", "remove", "--scope", "user"]:
    servers.pop(args[4], None)
elif args[:4] == ["mcp", "add", "--scope", "user"]:
    name, rest = args[4], args[5:]
    entry = {{"type": rest[rest.index("--transport") + 1], "url": rest[rest.index("--transport") + 2]}}
    if "--header" in rest:
        entry["headers"] = rest[rest.index("--header") + 1]
    servers[name] = entry
elif args[:2] == ["mcp", "get"]:
    if args[2] not in servers:
        print(f"No MCP server found with name: {{args[2]}}")
        sys.exit(1)
    signed = f"claude:{{profile}}" in os.environ.get("FAKE_SIGNED_IN", "").split()
    print(f"{{args[2]}}:\\n  Status: " + ("\\u2714 Connected" if signed else "\\u26a0 Needs authentication"))
json.dump(data, open(path, "w"))
"""

FAKE_CODEX = f"""#!{PY}
import json, os, sys
args = sys.argv[1:]
path = os.path.join(os.environ["HOME"], "codex.json")
servers = json.load(open(path)) if os.path.exists(path) else {{}}
if args[:2] == ["mcp", "remove"]:
    servers.pop(args[2], None)
elif args[:2] == ["mcp", "add"]:
    servers[args[2]] = args[3:]
elif args[:3] == ["mcp", "list", "--json"]:
    signed = "codex" in os.environ.get("FAKE_SIGNED_IN", "").split()
    print(json.dumps([{{"name": n, "auth_status": "o_auth" if signed else "not_logged_in"}} for n in servers]))
json.dump(servers, open(path, "w"))
"""

FAKE_SECURITY = """#!/bin/sh
echo "security $*" >> "$FAKE_LOG"
case "$1" in
  find-generic-password)
    case " $FAKE_SIGNED_IN " in *" vibe "*) exit 0 ;; esac
    exit 44 ;;
esac
exit 0
"""

FAKE_UV = f"""#!{PY}
import os, sys
args = sys.argv[1:]
os.execv({PY!r}, [{PY!r}, *args[args.index("python") + 1:]])
"""


def _registry() -> dict:
    return yaml.safe_load((KIT_ROOT / "registry" / "estate-tooling.yaml").read_text())


def _n8n() -> dict:
    return next(s for s in _registry()["mcp_servers"] if s["name"] == "n8n")


def _fn(script: str, name: str) -> str:
    start = script.index(f"\n{name}() {{") + 1
    end = script.index("\n}\n", start)
    return script[start : end + 3]


def _server_block(script: str, purpose: str) -> str:
    start = script.index(f"    # {purpose}\n")
    end = script.index("\n\n", start)
    return script[start:end] + "\n"


class Machine:
    def __init__(self, tmp_path: Path) -> None:
        self.home = tmp_path / "home"
        self.work = self.home / ".claude"
        self.personal = self.home / ".claude-personal"
        for d in (self.work, self.personal):
            d.mkdir(parents=True)
        for path in (self.home / ".claude.json", self.personal / ".claude.json"):
            path.write_text(json.dumps({"mcpServers": {}}))
        self.bin = tmp_path / "bin"
        self.bin.mkdir()
        for name, body in (("claude", FAKE_CLAUDE), ("codex", FAKE_CODEX), ("security", FAKE_SECURITY),
                           ("uv", FAKE_UV), ("vibe", "#!/bin/sh\n")):
            (self.bin / name).write_text(body)
            (self.bin / name).chmod(0o755)
        self.log = tmp_path / "calls.log"
        self.log.write_text("")

    def claude(self, profile: Path) -> dict:
        path = self.home / ".claude.json" if profile == self.work else profile / ".claude.json"
        return json.loads(path.read_text())["mcpServers"]

    def codex(self) -> dict:
        return json.loads((self.home / "codex.json").read_text())

    def setup(self, *, check: bool = False, signed_in: str = "") -> subprocess.CompletedProcess:
        script = render_registry.render_setup_script(_registry())
        body = (
            f"CHECK_ONLY={int(check)}\nCLOUD=0\nKIT_ROOT={KIT_ROOT}\nCLAUDE_HOME={self.work}\nfailures=0\n"
            f"CLAUDE_PROFILE_DIRS=({self.work} {self.personal})\n"
            "log() { echo \"setup: $*\"; }\nok() { echo \"setup: ok $*\"; }\n"
            "warn() { echo \"setup: warn $*\"; }\n"
            "fail() { echo \"setup: FAIL $*\"; failures=$((failures + 1)); }\n"
            + "".join(_fn(script, f) for f in ("run", "run_sh", "run_redacted", "claude_in_profile",
                                               "claude_each_profile", "oauth_status"))
            + _server_block(script, _n8n()["purpose"])
            + "echo \"failures=${failures}\"\n"
        )
        env = {
            "PATH": f"{self.bin}:/usr/bin:/bin",
            "HOME": str(self.home),
            "FAKE_LOG": str(self.log),
            "FAKE_SIGNED_IN": signed_in,
        }
        return subprocess.run(["/bin/bash", "-c", body], capture_output=True, text=True, check=False, env=env)


def test_both_claude_profiles_register_n8n_by_url_only(tmp_path: Path) -> None:
    machine = Machine(tmp_path)
    machine.setup()
    assert machine.claude(machine.work)["n8n"] == {"type": "http", "url": URL}
    assert machine.claude(machine.personal)["n8n"] == {"type": "http", "url": URL}


def test_codex_registers_n8n_by_url_only(tmp_path: Path) -> None:
    machine = Machine(tmp_path)
    machine.setup()
    assert machine.codex()["n8n"] == ["--url", URL]


def test_vibe_registers_n8n_with_oauth_and_no_token(tmp_path: Path) -> None:
    fragment = tmp_path / "fragment.toml"
    fragment.write_text(render_registry.render_vibe_mcp(_registry()))
    config = tmp_path / "config.toml"
    subprocess.run(
        ["uv", "run", "--no-project", "--with", "tomlkit", "python", str(MERGE), str(fragment),
         "--config", str(config)],
        capture_output=True, text=True, check=True,
    )
    server = next(s for s in tomllib.loads(config.read_text())["mcp_servers"] if s["name"] == "n8n")
    assert server["url"] == URL
    assert server["auth"] == {"type": "oauth", "scopes": []}


def test_setup_stores_no_n8n_secret_in_the_keychain(tmp_path: Path) -> None:
    machine = Machine(tmp_path)
    machine.setup()
    writes = [c for c in machine.log.read_text().splitlines() if "find-generic-password" not in c]
    assert writes == []
    assert not [s for s in render_registry._entries(_registry(), "workstation_secrets") if "n8n" in json.dumps(s)]


def test_check_reports_needs_sign_in_for_every_client_that_has_not_signed_in(tmp_path: Path) -> None:
    machine = Machine(tmp_path)
    machine.setup()
    result = machine.setup(check=True)
    for client in (f"claude {machine.work}", f"claude {machine.personal}", "codex", "vibe"):
        assert f"warn n8n: needs sign-in ({client})" in result.stdout, result.stdout


def test_check_is_quiet_about_a_client_once_it_has_signed_in(tmp_path: Path) -> None:
    machine = Machine(tmp_path)
    machine.setup()
    result = machine.setup(check=True, signed_in=f"claude:{machine.work} claude:{machine.personal} codex vibe")
    assert "needs sign-in" not in result.stdout


def test_convergence_keeps_n8n_on_every_client() -> None:
    for surface in ("claude", "codex", "vibe"):
        assert "n8n" in render_registry.declared_servers(_registry(), surface), surface


def test_local_hermes_does_not_get_an_oauth_server() -> None:
    assert "n8n" not in yaml.safe_load(render_registry.render_hermes_local_mcp(_registry()))["mcp_servers"]


def test_an_oauth_server_with_a_credential_is_rejected() -> None:
    data = copy.deepcopy(_registry())
    next(s for s in data["mcp_servers"] if s["name"] == "n8n")["credential"] = "N8N_TOKEN"
    with pytest.raises(render_registry.RegistryError, match="oauth"):
        render_registry.validate(data)
