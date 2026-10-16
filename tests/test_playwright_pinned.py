"""Playwright starts in every agent from one globally installed, pinned binary.

The setup tests render setup from the registry and run its Playwright step
under bash against a fake machine: a temporary HOME holding two Claude
profiles and a Codex config, with fake `claude`, `codex`, `npm` and `node` on
PATH that record what they were asked to do.
"""

from __future__ import annotations

import copy
import json
import re
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
    rest = args[args.index("--") + 1:]
    servers[args[4]] = {{"command": rest[0], "args": rest[1:]}}
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
    rest = args[args.index("--") + 1:]
    servers[args[2]] = {{"command": rest[0], "args": rest[1:]}}
json.dump(servers, open(path, "w"))
"""

# `npm ls -g --depth=0 <pkg>@<version>` succeeds only for what is installed.
# `npm install -g <pkg>@<version>` installs it and puts its binary on PATH,
# unless FAKE_NPM_BROKEN says the registry is unreachable.
FAKE_NPM = f"""#!{PY}
import os, sys
args = sys.argv[1:]
state = os.path.join(os.environ["HOME"], "npm-installed")
installed = open(state).read().split() if os.path.exists(state) else []
open(os.environ["FAKE_LOG"], "a").write("npm " + " ".join(args) + "\\n")
if args[:2] == ["ls", "-g"]:
    sys.exit(0 if args[-1] in installed else 1)
if args[:2] == ["install", "-g"]:
    if os.environ.get("FAKE_NPM_BROKEN"):
        sys.exit(1)
    open(state, "a").write(args[2] + "\\n")
    binary = os.path.join(os.environ["FAKE_BIN"], "playwright-mcp")
    open(binary, "w").write("#!/bin/sh\\n")
    os.chmod(binary, 0o755)
    sys.exit(0)
if args[:2] == ["root", "-g"]:
    print(os.path.join(os.environ["HOME"], "npm-root"))
"""

FAKE_NODE = """#!/bin/sh
echo "node $*" >> "$FAKE_LOG"
"""


def _registry() -> dict:
    return yaml.safe_load((KIT_ROOT / "registry" / "estate-tooling.yaml").read_text())


def _playwright() -> dict:
    return next(s for s in _registry()["mcp_servers"] if s["name"] == "playwright")


def _pin() -> str:
    tool = next(t for t in render_registry.container_tools(_registry()) if t["name"] == "playwright")
    return f"{tool['package']}@{tool['version']}"


def _fn(script: str, name: str) -> str:
    start = script.index(f"\n{name}() {{") + 1
    end = script.index("\n}\n", start)
    return script[start : end + 3]


def _server_block(script: str, purpose: str) -> str:
    start = script.index(f"    # {purpose}\n")
    end = script.index("\n\n", start)
    return script[start:end] + "\n"


class Machine:
    """A fake workstation: two Claude profiles and Codex, with npm and node."""

    def __init__(self, tmp_path: Path, installed: list[str] | None = None) -> None:
        self.tmp = tmp_path
        self.home = tmp_path / "home"
        self.work = self.home / ".claude"
        self.personal = self.home / ".claude-personal"
        for d in (self.work, self.personal):
            d.mkdir(parents=True)
        for path in (self.home / ".claude.json", self.personal / ".claude.json"):
            path.write_text(json.dumps({"mcpServers": {}}))
        if installed:
            (self.home / "npm-installed").write_text("\n".join(installed) + "\n")
        self.bin = tmp_path / "bin"
        self.bin.mkdir()
        for name, body in (("claude", FAKE_CLAUDE), ("codex", FAKE_CODEX), ("npm", FAKE_NPM), ("node", FAKE_NODE)):
            (self.bin / name).write_text(body)
            (self.bin / name).chmod(0o755)
        if installed:
            (self.bin / "playwright-mcp").write_text("#!/bin/sh\n")
            (self.bin / "playwright-mcp").chmod(0o755)
        self.log = tmp_path / "calls.log"
        self.log.write_text("")

    def calls(self) -> list[str]:
        return self.log.read_text().splitlines()

    def claude(self, profile: Path) -> dict:
        path = self.home / ".claude.json" if profile == self.work else profile / ".claude.json"
        return json.loads(path.read_text())["mcpServers"]

    def codex(self) -> dict:
        return json.loads((self.home / "codex.json").read_text())

    def setup(self, *, npm_broken: bool = False) -> subprocess.CompletedProcess:
        script = render_registry.render_setup_script(_registry())
        body = (
            f"CHECK_ONLY=0\nCLOUD=0\nKIT_ROOT={KIT_ROOT}\nCLAUDE_HOME={self.work}\nfailures=0\n"
            f"CLAUDE_PROFILE_DIRS=({self.work} {self.personal})\n"
            "log() { echo \"setup: $*\"; }\nok() { echo \"setup: ok $*\"; }\n"
            "warn() { echo \"setup: warn $*\" >&2; }\n"
            "fail() { echo \"setup: FAIL $*\" >&2; failures=$((failures + 1)); }\n"
            + "".join(_fn(script, f) for f in ("run", "run_sh", "run_redacted", "claude_in_profile",
                                               "claude_each_profile"))
            + _server_block(script, _playwright()["purpose"])
            + "echo \"failures=${failures}\"\n"
        )
        env = {
            "PATH": f"{self.bin}:/usr/bin:/bin",
            "HOME": str(self.home),
            "FAKE_LOG": str(self.log),
            "FAKE_BIN": str(self.bin),
            **({"FAKE_NPM_BROKEN": "1"} if npm_broken else {}),
        }
        return subprocess.run(["/bin/bash", "-c", body], capture_output=True, text=True, check=False, env=env)


EXPECTED = {"command": "playwright-mcp", "args": ["--headless", "--browser", "chromium"]}


def test_setup_installs_playwright_mcp_globally_at_the_registry_pin(tmp_path: Path) -> None:
    machine = Machine(tmp_path)
    result = machine.setup()
    assert "failures=0" in result.stdout, result.stdout + result.stderr
    assert f"npm install -g {_pin()}" in machine.calls()
    assert (machine.bin / "playwright-mcp").exists()


def test_setup_installs_the_browser_that_the_pinned_playwright_matches(tmp_path: Path) -> None:
    machine = Machine(tmp_path)
    machine.setup()
    installs = [c for c in machine.calls() if c.startswith("node ")]
    assert installs == [f"node {machine.home}/npm-root/@playwright/mcp/node_modules/playwright/cli.js install chromium"]


def test_setup_leaves_playwright_alone_when_the_pin_is_already_installed(tmp_path: Path) -> None:
    machine = Machine(tmp_path, installed=[_pin()])
    machine.setup()
    assert not [c for c in machine.calls() if c.startswith("npm install")]


def test_both_claude_profiles_register_the_pinned_binary(tmp_path: Path) -> None:
    machine = Machine(tmp_path)
    machine.setup()
    assert machine.claude(machine.work)["playwright"] == EXPECTED
    assert machine.claude(machine.personal)["playwright"] == EXPECTED


def test_codex_registers_the_pinned_binary(tmp_path: Path) -> None:
    machine = Machine(tmp_path)
    machine.setup()
    assert machine.codex()["playwright"] == EXPECTED


def test_local_hermes_starts_the_pinned_binary_too() -> None:
    local = yaml.safe_load(render_registry.render_hermes_local_mcp(_registry()))["mcp_servers"]["playwright"]
    assert {"command": local["command"], "args": local["args"]} == EXPECTED


def test_setup_warns_by_name_when_the_playwright_mcp_binary_is_missing(tmp_path: Path) -> None:
    machine = Machine(tmp_path)
    result = machine.setup(npm_broken=True)
    assert "warn playwright: playwright-mcp is not on PATH" in result.stderr


def test_vibe_starts_playwright_from_the_pinned_binary_with_a_startup_timeout(tmp_path: Path) -> None:
    fragment = tmp_path / "fragment.toml"
    fragment.write_text(render_registry.render_vibe_mcp(_registry()))
    config = tmp_path / "config.toml"
    subprocess.run(
        ["uv", "run", "--no-project", "--with", "tomlkit", "python", str(MERGE), str(fragment),
         "--config", str(config)],
        capture_output=True, text=True, check=True,
    )
    server = next(s for s in tomllib.loads(config.read_text())["mcp_servers"] if s["name"] == "playwright")
    assert {"command": server["command"], "args": server["args"]} == EXPECTED
    assert server["startup_timeout_sec"] == 60.0


def test_the_in_cluster_hermes_keeps_its_own_playwright(tmp_path: Path) -> None:
    hermes = yaml.safe_load(render_registry.render_hermes_mcp(_registry()))["mcp_servers"]["playwright"]
    assert hermes["command"] == "npx"
    assert hermes["args"] == ["-y", "@playwright/mcp@latest", "--headless", "--browser", "chromium"]


def test_one_renovate_match_moves_the_container_and_the_workstation_pin() -> None:
    renovate = json.loads((KIT_ROOT / "renovate.json").read_text())
    manager = next(m for m in renovate["customManagers"] if "container tool pins" in m["description"])
    text = (KIT_ROOT / "registry" / "estate-tooling.yaml").read_text()
    matches = [
        f"{m['depName']}@{m['currentValue']}"
        for pattern in manager["matchStrings"]
        for m in re.finditer(pattern.replace("(?<", "(?P<"), text)
        if m["depName"] == "@playwright/mcp"
    ]
    assert matches == [_pin()]
    assert f'npm install -g "{_pin()}"' in render_registry.render_setup_script(_registry())


def test_the_render_check_fails_when_the_vibe_fragment_lacks_a_startup_timeout(tmp_path: Path) -> None:
    data = _registry()
    for relative, (content, _mode) in render_registry.artifacts(data).items():
        target = tmp_path / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(content)
    vibe = tmp_path / render_registry.VIBE_MCP
    vibe.write_text("".join(line for line in vibe.read_text().splitlines(keepends=True)
                            if not line.startswith("startup_timeout_sec")))
    assert render_registry.stale_artifacts(data, tmp_path) == [str(render_registry.VIBE_MCP)]


def test_a_pinned_install_without_an_npm_pin_is_rejected() -> None:
    data = copy.deepcopy(_registry())
    del next(s for s in data["mcp_servers"] if s["name"] == "playwright")["container"]
    with pytest.raises(render_registry.RegistryError, match="install_pinned needs an npm container pin"):
        render_registry.validate(data)
