"""Setup converges every surface's MCP servers to the registry and the keep file.

Each test renders setup from the registry and runs its convergence step under
bash against a fake machine: a temporary HOME holding two Claude profiles, a
Codex config.toml, a Vibe config.toml and a keep file, with fake `claude`,
`codex` and `uv` on PATH that edit those files the way the real tools do.
"""

from __future__ import annotations

import copy
import json
import subprocess
import sys
import tomllib
from pathlib import Path

KIT_ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(KIT_ROOT / "scripts"))

import render_registry  # noqa: E402
import yaml  # noqa: E402

PY = sys.executable

FAKE_CLAUDE = f"""#!{PY}
import json, os, sys
args = sys.argv[1:]
config_dir = os.environ.get("CLAUDE_CONFIG_DIR")
path = os.path.join(config_dir, ".claude.json") if config_dir else os.path.join(os.environ["HOME"], ".claude.json")
open(os.environ["FAKE_LOG"], "a").write(f"claude {{' '.join(args)}} @ {{config_dir or '-'}}\\n")
if args[:4] == ["mcp", "remove", "--scope", "user"]:
    if args[4] in os.environ.get("FAKE_STUCK", "").split():
        sys.exit(0)
    data = json.load(open(path))
    data.get("mcpServers", {{}}).pop(args[4], None)
    json.dump(data, open(path, "w"))
"""

FAKE_CODEX = f"""#!{PY}
import os, sys
args = sys.argv[1:]
open(os.environ["FAKE_LOG"], "a").write(f"codex {{' '.join(args)}}\\n")
if args[:2] == ["mcp", "remove"] and args[2] not in os.environ.get("FAKE_STUCK", "").split():
    home = os.environ.get("CODEX_HOME") or os.path.join(os.environ["HOME"], ".codex")
    path = os.path.join(home, "config.toml")
    out, drop = [], False
    for line in open(path).read().splitlines(keepends=True):
        if line.startswith("["):
            head = line.strip().strip("[]").replace('"', "")
            drop = head == f"mcp_servers.{{args[2]}}" or head.startswith(f"mcp_servers.{{args[2]}}.")
        if not drop:
            out.append(line)
    open(path, "w").write("".join(out))
"""

FAKE_UV = f"""#!{PY}
import os, sys
args = sys.argv[1:]
os.execv({PY!r}, [{PY!r}, *args[args.index("python") + 1:]])
"""


def _registry() -> dict:
    return yaml.safe_load((KIT_ROOT / "registry" / "estate-tooling.yaml").read_text())


def _fn(script: str, name: str) -> str:
    start = script.index(f"\n{name}() {{") + 1
    end = script.index("\n}\n", start)
    return script[start : end + 3]


def _bin(tmp_path: Path) -> Path:
    bin_dir = tmp_path / "bin"
    bin_dir.mkdir(exist_ok=True)
    for name, body in (("claude", FAKE_CLAUDE), ("codex", FAKE_CODEX), ("uv", FAKE_UV)):
        (bin_dir / name).write_text(body)
        (bin_dir / name).chmod(0o755)
    return bin_dir


class Machine:
    """A fake workstation: two Claude profiles, Codex, Vibe and a keep file."""

    def __init__(self, tmp_path: Path, servers: list[str], keep: list[str] | None = None) -> None:
        self.tmp = tmp_path
        self.home = tmp_path / "home"
        self.work = self.home / ".claude"
        self.personal = self.home / ".claude-personal"
        self.codex = self.home / ".codex"
        self.vibe = self.home / ".vibe"
        for d in (self.work, self.personal, self.codex, self.vibe):
            d.mkdir(parents=True)
        entry = {"type": "http", "url": "https://x/mcp"}
        for path in (self.home / ".claude.json", self.personal / ".claude.json"):
            path.write_text(json.dumps({"mcpServers": {s: entry for s in servers}, "other": 1}))
        self.codex_config.write_text(
            'model = "gpt"\n\n'
            + "".join(f'[mcp_servers."{s}"]\nurl = "https://x/mcp"\n\n[mcp_servers."{s}".env]\nA = "b"\n\n'
                      for s in servers)
        )
        self.vibe_config.write_text(
            'active_model = "mistral"\n\n'
            + "".join(f'[[mcp_servers]]\nname = "{s}"\ntransport = "stdio"\ncommand = "x"\n\n' for s in servers)
        )
        if keep is not None:
            self.keep_file.parent.mkdir(parents=True)
            self.keep_file.write_text(yaml.safe_dump({"keep": keep}))
        self.log = tmp_path / "calls.log"
        self.log.write_text("")

    @property
    def codex_config(self) -> Path:
        return self.codex / "config.toml"

    @property
    def vibe_config(self) -> Path:
        return self.vibe / "config.toml"

    @property
    def keep_file(self) -> Path:
        return self.home / ".config" / "agent-kit" / "keep.yaml"

    def surfaces(self) -> dict[str, set[str]]:
        return {
            "claude work": set(json.loads((self.home / ".claude.json").read_text())["mcpServers"]),
            "claude personal": set(json.loads((self.personal / ".claude.json").read_text())["mcpServers"]),
            "codex": set(tomllib.loads(self.codex_config.read_text()).get("mcp_servers", {})),
            "vibe": {s["name"] for s in tomllib.loads(self.vibe_config.read_text()).get("mcp_servers", [])},
        }

    def files(self) -> dict[Path, str]:
        paths = [self.home / ".claude.json", self.personal / ".claude.json", self.codex_config, self.vibe_config]
        return {p: p.read_text() for p in paths}

    def setup(self, *, check: bool = False, stuck: str = "", data: dict | None = None,
              ) -> subprocess.CompletedProcess:
        script = render_registry.render_setup_script(data or _registry())
        body = (
            f"CHECK_ONLY={int(check)}\nKIT_ROOT={KIT_ROOT}\nCLAUDE_HOME={self.work}\n"
            f"CODEX_HOME={self.codex}\nVIBE_HOME={self.vibe}\nfailures=0\n"
            f"CLAUDE_PROFILE_DIRS=({self.work} {self.personal})\n"
            "log() { echo \"setup: $*\"; }\nok() { echo \"setup: ok $*\"; }\n"
            "warn() { echo \"setup: warn $*\" >&2; }\n"
            "fail() { echo \"setup: FAIL $*\" >&2; failures=$((failures + 1)); }\n"
            "run() { \"$@\"; }\n"
            + _fn(script, "converge_mcp")
            + "converge_mcp\necho \"failures=${failures}\"\n"
        )
        env = {
            "PATH": f"{_bin(self.tmp)}:/usr/bin:/bin",
            "HOME": str(self.home),
            "FAKE_LOG": str(self.log),
            "FAKE_STUCK": stuck,
        }
        return subprocess.run(["/bin/bash", "-c", body], capture_output=True, text=True, check=False, env=env)


def _clean(result: subprocess.CompletedProcess) -> None:
    assert "failures=0" in result.stdout, result.stdout + result.stderr


def test_an_undeclared_server_is_removed_from_every_surface(tmp_path: Path) -> None:
    machine = Machine(tmp_path, ["memory-api", "pycharm"], keep=[])
    result = machine.setup()
    _clean(result)
    for surface, names in machine.surfaces().items():
        assert "pycharm" not in names, surface
        assert "memory-api" in names, surface
    assert "removed pycharm (codex, undeclared)" in result.stdout


def test_a_server_the_keep_file_names_survives(tmp_path: Path) -> None:
    machine = Machine(tmp_path, ["context7", "idea", "pycharm"], keep=["context7", "idea"])
    _clean(machine.setup())
    for surface, names in machine.surfaces().items():
        assert {"context7", "idea"} <= names, surface
        assert "pycharm" not in names, surface


def test_a_missing_keep_file_keeps_nothing(tmp_path: Path) -> None:
    machine = Machine(tmp_path, ["context7"])
    _clean(machine.setup())
    assert all("context7" not in names for names in machine.surfaces().values())


def test_a_retired_record_is_removed_even_when_the_keep_file_names_it(tmp_path: Path) -> None:
    # In the registry, knowledge has `surfaces: []` and vuetify has `enabled: false`.
    machine = Machine(tmp_path, ["knowledge", "vuetify", "memory-mcp", "memory"], keep=["vuetify"])
    result = machine.setup()
    _clean(result)
    for surface, names in machine.surfaces().items():
        assert not names & {"knowledge", "vuetify", "memory-mcp", "memory"}, surface
    assert "removed knowledge (codex, retired)" in result.stdout
    assert "removed vuetify (codex, retired)" in result.stdout


def test_a_server_retired_by_enabled_false_is_removed(tmp_path: Path) -> None:
    data = copy.deepcopy(_registry())
    next(s for s in data["mcp_servers"] if s["name"] == "drawio")["enabled"] = False
    machine = Machine(tmp_path, ["drawio", "playwright"], keep=[])
    result = machine.setup(data=data)
    _clean(result)
    for surface, names in machine.surfaces().items():
        assert "drawio" not in names and "playwright" in names, surface
    assert "removed drawio (vibe, retired)" in result.stdout


def test_plugin_and_coding_agents_servers_stay(tmp_path: Path) -> None:
    machine = Machine(tmp_path, ["memory-api", "hindsight", "plugin:github:github"], keep=[])
    _clean(machine.setup())
    for surface, names in machine.surfaces().items():
        assert {"memory-api", "hindsight", "plugin:github:github"} <= names, surface


def test_every_declared_workstation_server_stays(tmp_path: Path) -> None:
    declared = [s["name"] for s in render_registry.vibe_servers(_registry())]
    machine = Machine(tmp_path, declared, keep=[])
    result = machine.setup()
    _clean(result)
    assert "removed" not in result.stdout
    for surface, names in machine.surfaces().items():
        assert set(declared) <= names, surface


def test_removal_runs_in_each_claude_profile(tmp_path: Path) -> None:
    machine = Machine(tmp_path, ["pycharm"], keep=[])
    result = machine.setup()
    _clean(result)
    calls = machine.log.read_text().splitlines()
    assert "claude mcp remove --scope user pycharm @ -" in calls
    assert f"claude mcp remove --scope user pycharm @ {machine.personal}" in calls
    assert "removed pycharm (claude ~/.claude, undeclared)" in result.stdout
    assert "removed pycharm (claude ~/.claude-personal, undeclared)" in result.stdout


def test_check_previews_each_removal_and_changes_nothing(tmp_path: Path) -> None:
    machine = Machine(tmp_path, ["memory-api", "pycharm", "knowledge", "context7"], keep=["context7"])
    before = machine.files()
    result = machine.setup(check=True)
    _clean(result)
    assert machine.files() == before
    assert "remove" not in machine.log.read_text()
    lines = [line for line in result.stdout.splitlines() if "would remove" in line]
    assert sorted(lines) == sorted(
        f"setup: would remove {name} ({surface}, {reason})"
        for name, reason in (("pycharm", "undeclared"), ("knowledge", "retired"))
        for surface in ("claude ~/.claude", "claude ~/.claude-personal", "codex", "vibe")
    )


def test_a_run_fails_when_a_removed_server_is_still_listed(tmp_path: Path) -> None:
    machine = Machine(tmp_path, ["pycharm"], keep=[])
    result = machine.setup(stuck="pycharm")
    assert "failures=1" in result.stdout, result.stdout + result.stderr
    assert "pycharm is still present (codex)" in result.stdout + result.stderr


def test_a_malformed_keep_file_removes_nothing(tmp_path: Path) -> None:
    machine = Machine(tmp_path, ["pycharm"])
    machine.keep_file.parent.mkdir(parents=True)
    machine.keep_file.write_text("keep: pycharm\n")
    before = machine.files()
    result = machine.setup()
    assert "failures=1" in result.stdout, result.stdout + result.stderr
    assert machine.files() == before


def test_setup_converges_after_the_servers_are_registered() -> None:
    script = render_registry.render_setup_script(_registry())
    mcp = script.index('log "MCP servers"')
    assert mcp < script.index("\n    converge_mcp\n", mcp) < script.index('log "MCP registration skipped')


def test_the_hand_written_retirement_lists_are_gone() -> None:
    script = render_registry.render_setup_script(_registry())
    for retired in render_registry.retired_servers(_registry()):
        assert f"claude mcp remove --scope user {retired} " not in script
        assert f"codex mcp remove {retired} " not in script
        vibe = [line for line in script.splitlines() if "vibe_remove" in line]
        assert not [line for line in vibe if f"--remove {retired} " in f"{line} "]
    assert "codex mcp remove context7 " not in script
    assert "hand-added" not in script
