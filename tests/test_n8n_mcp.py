"""n8n-mcp runs docs-only from one pinned global binary on every workstation agent.

The setup tests render setup from the registry and run the n8n-mcp step under
bash against a fake machine: a temporary HOME with two Claude profiles and
Codex, with fake `claude`, `codex` and `npm` on PATH.
"""

from __future__ import annotations

import json
import re
import subprocess
import sys
import tomllib
from pathlib import Path

KIT_ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(KIT_ROOT / "scripts"))

import render_registry  # noqa: E402
import yaml  # noqa: E402

PY = sys.executable
MERGE = KIT_ROOT / "scripts" / "vibe-merge-mcp.py"

FAKE_CLAUDE = f"""#!{PY}
import json, os, sys
args = sys.argv[1:]
config_dir = os.environ.get("CLAUDE_CONFIG_DIR")
path = os.path.join(config_dir, ".claude.json") if config_dir else os.path.join(os.environ["HOME"], ".claude.json")
data = json.load(open(path))
servers = data.setdefault("mcpServers", {{}})
if args[:4] == ["mcp", "remove", "--scope", "user"]:
    servers.pop(args[4], None)
elif args[:4] == ["mcp", "add", "--scope", "user"]:
    servers[args[4]] = args[5:]
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
json.dump(servers, open(path, "w"))
"""

FAKE_NPM = f"""#!{PY}
import os, sys
args = sys.argv[1:]
state = os.path.join(os.environ["HOME"], "npm-installed")
installed = open(state).read().split() if os.path.exists(state) else []
open(os.environ["FAKE_LOG"], "a").write("npm " + " ".join(args) + "\\n")
if args[:2] == ["ls", "-g"]:
    sys.exit(0 if args[-1] in installed else 1)
if args[:2] == ["install", "-g"]:
    open(state, "a").write(args[2] + "\\n")
    binary = os.path.join(os.environ["FAKE_BIN"], "n8n-mcp")
    open(binary, "w").write("#!/bin/sh\\n")
    os.chmod(binary, 0o755)
"""


def _registry() -> dict:
    return yaml.safe_load((KIT_ROOT / "registry" / "estate-tooling.yaml").read_text())


def _server() -> dict:
    return next(s for s in _registry()["mcp_servers"] if s["name"] == "n8n-mcp")


def _pin() -> str:
    pin = _server()["pin"]
    return f"{pin['package']}@{pin['version']}"


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
        for name, body in (("claude", FAKE_CLAUDE), ("codex", FAKE_CODEX), ("npm", FAKE_NPM)):
            (self.bin / name).write_text(body)
            (self.bin / name).chmod(0o755)
        self.log = tmp_path / "calls.log"
        self.log.write_text("")

    def claude(self, profile: Path) -> list[str]:
        path = self.home / ".claude.json" if profile == self.work else profile / ".claude.json"
        return json.loads(path.read_text())["mcpServers"]["n8n-mcp"]

    def codex(self) -> list[str]:
        return json.loads((self.home / "codex.json").read_text())["n8n-mcp"]

    def setup(self) -> subprocess.CompletedProcess:
        script = render_registry.render_setup_script(_registry())
        body = (
            f"CHECK_ONLY=0\nCLOUD=0\nKIT_ROOT={KIT_ROOT}\nCLAUDE_HOME={self.work}\nfailures=0\n"
            f"CLAUDE_PROFILE_DIRS=({self.work} {self.personal})\n"
            "log() { echo \"setup: $*\"; }\nok() { echo \"setup: ok $*\"; }\n"
            "warn() { echo \"setup: warn $*\" >&2; }\n"
            "fail() { echo \"setup: FAIL $*\" >&2; failures=$((failures + 1)); }\n"
            + "".join(_fn(script, f) for f in ("run", "run_sh", "run_redacted", "claude_in_profile",
                                               "claude_each_profile"))
            + _server_block(script, _server()["purpose"])
            + "echo \"failures=${failures}\"\n"
        )
        env = {
            "PATH": f"{self.bin}:/usr/bin:/bin",
            "HOME": str(self.home),
            "FAKE_LOG": str(self.log),
            "FAKE_BIN": str(self.bin),
        }
        return subprocess.run(["/bin/bash", "-c", body], capture_output=True, text=True, check=False, env=env)


def test_setup_installs_n8n_mcp_globally_at_the_registry_pin(tmp_path: Path) -> None:
    machine = Machine(tmp_path)
    result = machine.setup()
    assert "failures=0" in result.stdout, result.stdout + result.stderr
    assert f"npm install -g {_pin()}" in machine.log.read_text().splitlines()


def test_both_claude_profiles_register_n8n_mcp_with_no_api_key(tmp_path: Path) -> None:
    machine = Machine(tmp_path)
    machine.setup()
    assert machine.claude(machine.work) == ["--", "n8n-mcp"]
    assert machine.claude(machine.personal) == ["--", "n8n-mcp"]


def test_codex_registers_n8n_mcp_with_no_api_key(tmp_path: Path) -> None:
    machine = Machine(tmp_path)
    machine.setup()
    assert machine.codex() == ["--", "n8n-mcp"]


def test_vibe_starts_n8n_mcp_with_no_api_key(tmp_path: Path) -> None:
    fragment = tmp_path / "fragment.toml"
    fragment.write_text(render_registry.render_vibe_mcp(_registry()))
    config = tmp_path / "config.toml"
    subprocess.run(
        ["uv", "run", "--no-project", "--with", "tomlkit", "python", str(MERGE), str(fragment),
         "--config", str(config)],
        capture_output=True, text=True, check=True,
    )
    server = next(s for s in tomllib.loads(config.read_text())["mcp_servers"] if s["name"] == "n8n-mcp")
    assert (server["command"], server["args"]) == ("n8n-mcp", [])
    assert "env" not in server


def test_convergence_keeps_n8n_mcp_on_every_client() -> None:
    for surface in ("claude", "codex", "vibe"):
        assert "n8n-mcp" in render_registry.declared_servers(_registry(), surface), surface


def test_one_renovate_match_moves_the_n8n_mcp_pin() -> None:
    renovate = json.loads((KIT_ROOT / "renovate.json").read_text())
    manager = next(m for m in renovate["customManagers"] if "container tool pins" in m["description"])
    text = (KIT_ROOT / "registry" / "estate-tooling.yaml").read_text()
    matches = [
        f"{m['depName']}@{m['currentValue']}"
        for pattern in manager["matchStrings"]
        for m in re.finditer(pattern.replace("(?<", "(?P<"), text)
        if m["depName"] == "n8n-mcp"
    ]
    assert matches == [_pin()]
    assert f'npm install -g "{_pin()}"' in render_registry.render_setup_script(_registry())
