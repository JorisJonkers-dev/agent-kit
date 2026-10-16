"""Setup converges Claude's marketplaces and plugins to the registry and the keep file.

Each test renders setup from the registry and runs its plugin convergence step
under bash against a fake machine: a temporary HOME and keep file, with fake
`claude` and `uv` on PATH. The fake `claude` serves `plugin list` and
`plugin marketplace list` from a state file and edits it the way the real one
does; removing a marketplace uninstalls its plugins.
"""

from __future__ import annotations

import json
import subprocess
import sys
from pathlib import Path

KIT_ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(KIT_ROOT / "scripts"))

import render_registry  # noqa: E402
import yaml  # noqa: E402

PY = sys.executable

FAKE_CLAUDE = f"""#!{PY}
import json, os, sys
args = sys.argv[1:]
state_path = os.environ["FAKE_STATE"]
state = json.load(open(state_path))
open(os.environ["FAKE_LOG"], "a").write(f"claude {{' '.join(args)}} @ {{os.environ.get('CLAUDE_CONFIG_DIR', '-')}}\\n")
stuck = os.environ.get("FAKE_STUCK", "").split()
if args[:4] == ["plugin", "marketplace", "list", "--json"]:
    print(json.dumps(state["markets"]))
elif args[:3] == ["plugin", "list", "--json"]:
    print(json.dumps(state["plugins"]))
elif args[:3] == ["plugin", "marketplace", "remove"] and args[3] not in stuck:
    state["markets"] = [m for m in state["markets"] if m["name"] != args[3]]
    state["plugins"] = [p for p in state["plugins"] if not p["id"].endswith("@" + args[3])]
elif args[:2] == ["plugin", "uninstall"] and args[2] not in stuck:
    state["plugins"] = [p for p in state["plugins"] if not (p["id"] == args[2] and p["scope"] == "user")]
json.dump(state, open(state_path, "w"))
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


def _declared_plugins() -> list[str]:
    data = _registry()
    ids = [f"{p['name']}@{render_registry.declared_name(data, p['marketplace'])}" for p in data["plugins"]]
    official = render_registry.declared_name(data, "claude-plugins-official")
    return ids + [f"{s['plugin']}@{official}" for s in data["language_servers"]]


def _folder(name: str) -> dict:
    return {"name": name, "source": "directory", "path": f"/m/{name}"}


def _github(name: str, repo: str) -> dict:
    return {"name": name, "source": "github", "repo": repo}


DECLARED_MARKETS = [_folder("agent-kit-official"), _folder("caveman"), _folder("drawio"),
                    _folder("kubernetes-skill"), _folder("ai-software-architect")]


class Machine:
    def __init__(self, tmp_path: Path, markets: list[dict], plugins: list[str],
                 keep: list[str] | None = None, project: list[str] = ()) -> None:
        self.tmp = tmp_path
        self.home = tmp_path / "home"
        self.home.mkdir()
        self.state = tmp_path / "state.json"
        self.state.write_text(json.dumps({
            "markets": markets,
            "plugins": [{"id": p, "scope": "user", "enabled": True} for p in plugins]
            + [{"id": p, "scope": "project", "enabled": True, "projectPath": "/repo"} for p in project],
        }))
        self.log = tmp_path / "calls.log"
        self.log.write_text("")
        if keep is not None:
            keep_file = self.home / ".config" / "agent-kit" / "keep.yaml"
            keep_file.parent.mkdir(parents=True)
            keep_file.write_text(yaml.safe_dump({"keep": keep}))

    def markets(self) -> set[str]:
        return {m["name"] for m in json.loads(self.state.read_text())["markets"]}

    def plugins(self) -> set[str]:
        return {p["id"] for p in json.loads(self.state.read_text())["plugins"]}

    def setup(self, *, check: bool = False, stuck: str = "") -> subprocess.CompletedProcess:
        script = render_registry.render_setup_script(_registry())
        bin_dir = self.tmp / "bin"
        bin_dir.mkdir(exist_ok=True)
        for name, body in (("claude", FAKE_CLAUDE), ("uv", FAKE_UV)):
            (bin_dir / name).write_text(body)
            (bin_dir / name).chmod(0o755)
        body = (
            f"CHECK_ONLY={int(check)}\nKIT_ROOT={KIT_ROOT}\nfailures=0\n"
            "log() { echo \"setup: $*\"; }\nok() { echo \"setup: ok $*\"; }\n"
            "warn() { echo \"setup: warn $*\" >&2; }\n"
            "fail() { echo \"setup: FAIL $*\" >&2; failures=$((failures + 1)); }\n"
            "run() { \"$@\"; }\n"
            + _fn(script, "converge_plugins")
            + "converge_plugins\necho \"failures=${failures}\"\n"
        )
        env = {"PATH": f"{bin_dir}:/usr/bin:/bin", "HOME": str(self.home), "FAKE_STATE": str(self.state),
               "FAKE_LOG": str(self.log), "FAKE_STUCK": stuck}
        return subprocess.run(["/bin/bash", "-c", body], capture_output=True, text=True, check=False, env=env)


def _clean(result: subprocess.CompletedProcess) -> None:
    assert "failures=0" in result.stdout, result.stdout + result.stderr


GITHUB_OFFICIAL = _github("claude-plugins-official", "anthropics/claude-plugins-official")


def test_check_reports_the_github_official_marketplace_as_undeclared(tmp_path: Path) -> None:
    machine = Machine(tmp_path, [*DECLARED_MARKETS, GITHUB_OFFICIAL], _declared_plugins(), keep=[])
    before = machine.state.read_text()
    result = machine.setup(check=True)
    _clean(result)
    assert "setup: would remove claude-plugins-official (claude marketplace, undeclared)" in result.stdout
    assert machine.state.read_text() == before
    assert "remove" not in machine.log.read_text() and "uninstall" not in machine.log.read_text()


def test_a_normal_run_removes_it_and_every_declared_plugin_stays(tmp_path: Path) -> None:
    machine = Machine(tmp_path, [*DECLARED_MARKETS, GITHUB_OFFICIAL], _declared_plugins(), keep=[])
    result = machine.setup()
    _clean(result)
    assert "claude-plugins-official" not in machine.markets()
    assert "agent-kit-official" in machine.markets()
    assert machine.plugins() == set(_declared_plugins())
    assert "mattpocock-skills@agent-kit-official" in machine.plugins()


def test_an_undeclared_plugin_is_uninstalled(tmp_path: Path) -> None:
    machine = Machine(tmp_path, DECLARED_MARKETS, [*_declared_plugins(), "plugin-dev@agent-kit-official"], keep=[])
    result = machine.setup()
    _clean(result)
    assert "plugin-dev@agent-kit-official" not in machine.plugins()
    assert "removed plugin-dev@agent-kit-official (claude plugin, undeclared)" in result.stdout


def test_a_kept_marketplace_and_its_plugins_stay(tmp_path: Path) -> None:
    work = _github("nedap-ai-skills", "nedap/ai-tools-plugins")
    machine = Machine(tmp_path, [*DECLARED_MARKETS, work], [*_declared_plugins(), "review@nedap-ai-skills"],
                      keep=["nedap-ai-skills"])
    _clean(machine.setup())
    assert "nedap-ai-skills" in machine.markets()
    assert "review@nedap-ai-skills" in machine.plugins()


def test_a_retired_plugin_and_its_marketplace_go(tmp_path: Path) -> None:
    machine = Machine(tmp_path, [*DECLARED_MARKETS, _github("hindsight", "vectorize-io/hindsight")],
                      [*_declared_plugins(), "hindsight-memory@hindsight"], keep=["hindsight"])
    result = machine.setup()
    _clean(result)
    assert "hindsight" not in machine.markets()
    assert "hindsight-memory@hindsight" not in machine.plugins()
    assert "removed hindsight (claude marketplace, retired)" in result.stdout


def test_a_declared_marketplace_still_on_github_is_left_to_the_pinning_step(tmp_path: Path) -> None:
    markets = [m for m in DECLARED_MARKETS if m["name"] != "caveman"] + [_github("caveman", "JuliusBrussee/caveman")]
    machine = Machine(tmp_path, markets, _declared_plugins(), keep=[])
    _clean(machine.setup())
    assert "caveman" in machine.markets()


def test_a_project_scoped_plugin_is_left_alone(tmp_path: Path) -> None:
    machine = Machine(tmp_path, DECLARED_MARKETS, _declared_plugins(), keep=[],
                      project=["plugin-dev@agent-kit-official"])
    result = machine.setup()
    _clean(result)
    assert "plugin-dev@agent-kit-official" in machine.plugins()
    assert "uninstall" not in machine.log.read_text()


def test_removal_runs_once_across_profiles(tmp_path: Path) -> None:
    machine = Machine(tmp_path, [*DECLARED_MARKETS, GITHUB_OFFICIAL], _declared_plugins(), keep=[])
    _clean(machine.setup())
    removes = [c for c in machine.log.read_text().splitlines() if "marketplace remove" in c]
    assert removes == ["claude plugin marketplace remove claude-plugins-official @ -"]


def test_a_marketplace_still_listed_after_removal_fails_the_run(tmp_path: Path) -> None:
    machine = Machine(tmp_path, [*DECLARED_MARKETS, GITHUB_OFFICIAL], _declared_plugins(), keep=[])
    result = machine.setup(stuck="claude-plugins-official")
    assert "failures=1" in result.stdout, result.stdout + result.stderr
    assert "claude-plugins-official is still present (claude marketplace)" in result.stdout


def test_setup_converges_plugins_after_installing_them() -> None:
    script = render_registry.render_setup_script(_registry())
    lsp = script.index('log "language servers skipped (--no-lsp)"')
    assert lsp < script.index("\n  converge_plugins\n", lsp) < script.index('log "MCP servers"')


def test_the_hand_written_retired_plugin_removal_is_gone() -> None:
    script = render_registry.render_setup_script(_registry())
    for retired in _registry()["retired_plugins"]:
        assert f"claude plugin uninstall {retired['ref']}" not in script
        assert f"claude plugin marketplace remove {retired['marketplace']}" not in script
