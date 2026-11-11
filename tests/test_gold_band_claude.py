"""Each instance's Claude agent starts with an explicit configuration directory (issue #427).

The registry's ``config_dir`` becomes the entry's ``adapter.env.CLAUDE_CONFIG_DIR``,
so the Claude process the launcher starts has the instance's directory whatever
``CLAUDE_CONFIG_DIR`` the launcher's own environment holds. The converge tests
drive ``converge.main(["gold-band", ...])`` against temporary files; the
launcher test runs the real headless ``gold-band run`` (skipped, with the
reason, when ``GOLD_BAND_BIN`` is unset) with a stub standing in for the
adapter package on PATH. No real Claude directory is read or written.
"""

from __future__ import annotations

import json
import os
import subprocess
import sys
from pathlib import Path
from typing import Any

import pytest
import yaml

KIT_ROOT = Path(__file__).resolve().parent.parent
STUB = KIT_ROOT / "tests" / "fixtures" / "gold_band_stub_acp.py"
sys.path.insert(0, str(KIT_ROOT / "scripts"))

import converge  # noqa: E402

GOLD_BAND_BIN = os.environ.get("GOLD_BAND_BIN")
PERSONAL_DIR = "~/.claude-personal"
WORK_DIR = "~/.claude"
ADAPTER = {"displayName": "Claude", "command": "npx", "args": ["-y", "pkg@1"], "env": {}}


def _claude(name: str, config_dir: Any = PERSONAL_DIR, **more: Any) -> dict[str, Any]:
    agent: dict[str, Any] = {"id": name, "binary": "npx", "adapter": dict(ADAPTER, env={})}
    if config_dir is not None:
        agent["config_dir"] = config_dir
    return {**agent, **more}


def _registry(agents: list[dict[str, Any]]) -> dict[str, Any]:
    return {
        "gold_band": {"agents": agents},
        "gold_band_instances": {
            "gold-band": {"agents": ["claude-work", "codex"]},
            "gold-band-personal": {"agents": ["claude", "claude-other", "opencode"]},
        },
    }


class Machine:
    def __init__(self, tmp_path: Path, monkeypatch: pytest.MonkeyPatch, agents: list[dict[str, Any]]) -> None:
        bin_dir = tmp_path / "bin"
        bin_dir.mkdir()
        (bin_dir / "npx").write_text("#!/bin/sh\nexit 0\n")
        (bin_dir / "npx").chmod(0o755)
        monkeypatch.setenv("PATH", str(bin_dir))
        monkeypatch.setenv("AGENT_KIT_GOLD_BAND_RUNNING", "0")
        self.settings = tmp_path / "gb" / "settings.json"
        self.keep = tmp_path / "keep.yaml"
        self.registry = tmp_path / "registry.yaml"
        self.registry.write_text(yaml.safe_dump(_registry(agents)))

    def run(self, instance: str | None = None) -> int:
        argv = ["gold-band", "--keep", str(self.keep), "--settings", str(self.settings)]
        argv += ["--registry", str(self.registry)]
        if instance:
            argv += ["--instance", instance]
        return converge.main(argv)

    def agents(self) -> dict[str, Any]:
        agents: dict[str, Any] = json.loads(self.settings.read_text())["agents"]
        return agents


def test_each_instance_writes_its_claude_agent_with_its_own_directory(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    agents = [_claude("claude"), _claude("claude-work", WORK_DIR, instance="gold-band")]
    machine = Machine(tmp_path, monkeypatch, agents)
    home = str(Path.home())
    assert machine.run() == 0
    assert list(machine.agents()) == ["claude"]
    adapter = machine.agents()["claude"]["adapter"]
    assert adapter["env"] == {"CLAUDE_CONFIG_DIR": f"{home}/.claude-personal"}
    assert adapter["command"] == "npx"
    assert not {"config_dir", "instance", "binary", "id"} & set(machine.agents()["claude"])

    machine.settings.unlink()
    assert machine.run("gold-band") == 0
    assert list(machine.agents()) == ["claude-work"]
    assert machine.agents()["claude-work"]["adapter"]["env"] == {"CLAUDE_CONFIG_DIR": f"{home}/.claude"}


def test_an_absolute_directory_is_written_as_it_is(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    machine = Machine(tmp_path, monkeypatch, [_claude("claude", "/srv/profile/")])
    assert machine.run() == 0
    assert machine.agents()["claude"]["adapter"]["env"] == {"CLAUDE_CONFIG_DIR": "/srv/profile/"}


def test_a_claude_agent_without_an_adapter_gets_one_for_the_directory(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    agent = {"id": "claude", "config_dir": "/srv/profile"}
    machine = Machine(tmp_path, monkeypatch, [agent])
    assert machine.run() == 0
    assert machine.agents()["claude"] == {"adapter": {"env": {"CLAUDE_CONFIG_DIR": "/srv/profile"}}}


def test_the_work_instance_keeps_codex_and_the_personal_one_removes_it(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    machine = Machine(tmp_path, monkeypatch, [_claude("claude-work", WORK_DIR, instance="gold-band")])
    machine.settings.parent.mkdir()
    machine.settings.write_text(json.dumps({"agents": {"codex": {"adapter": {}}}}))
    machine.keep.write_text(yaml.safe_dump({"keep": ["codex"]}))
    assert machine.run("gold-band") == 0
    assert "codex" in machine.agents()
    # A kept Codex entry still goes on the personal instance.
    assert machine.run() == 0
    assert "codex" not in machine.agents()


@pytest.mark.parametrize(
    ("agents", "message"),
    [
        ([_claude("claude", None)], "claude: a Claude agent must declare config_dir"),
        ([_claude("claude", "")], "claude: config_dir must be a non-empty string"),
        ([_claude("claude", 7)], "claude: config_dir must be a non-empty string"),
        ([_claude("claude", "relative/dir")], "claude: config_dir must be an absolute path or start with ~/"),
        ([_claude("claude", "~/../etc")], "claude: config_dir must be an absolute path or start with ~/"),
        (
            [
                _claude("claude", "~/.claude-personal"),
                _claude("claude-work", "~/.claude-personal/", instance="gold-band"),
            ],
            "claude-work: config_dir ~/.claude-personal/ is already claude's",
        ),
        (
            [{**_claude("opencode", None), "config_dir": "~/.x"}],
            "opencode: config_dir is only for a Claude agent",
        ),
        (
            [{**_claude("claude"), "adapter": {**ADAPTER, "env": {"CLAUDE_CONFIG_DIR": "$X"}}}],
            "claude: adapter.env CLAUDE_CONFIG_DIR is set by config_dir, not by hand",
        ),
        ([_claude("claude", instance="elsewhere")], "claude: instance must be one of gold-band, gold-band-personal"),
        ([{"id": 5, "config_dir": "/x"}], "?: id must match"),
    ],
)
def test_a_bad_claude_declaration_is_refused_and_nothing_is_written(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
    capsys: pytest.CaptureFixture[str],
    agents: list[dict[str, Any]],
    message: str,
) -> None:
    machine = Machine(tmp_path, monkeypatch, agents)
    assert machine.run() == 1
    assert f"gold-band-agents: {message}" in capsys.readouterr().out
    assert not machine.settings.exists()


def test_an_agent_the_instance_does_not_allow_is_refused(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    machine = Machine(tmp_path, monkeypatch, [_claude("claude-work", WORK_DIR)])
    assert machine.run() == 1
    assert "claude-work: not in gold_band_instances.gold-band-personal.agents" in capsys.readouterr().out


def test_the_running_guard_names_the_instance(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    machine = Machine(tmp_path, monkeypatch, [_claude("claude-work", WORK_DIR, instance="gold-band")])
    monkeypatch.setenv("AGENT_KIT_GOLD_BAND_RUNNING", "1")
    assert machine.run("gold-band") == converge.GOLD_BAND_RUNNING_EXIT
    assert "Gold Band instance gold-band (set by" in capsys.readouterr().out


def test_the_kits_registry_declares_both_claude_agents_on_their_own_directories(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    machine = Machine(tmp_path, monkeypatch, [])
    machine.registry = converge.REGISTRY_PATH
    home = str(Path.home())
    assert machine.run() == 0
    personal = machine.agents()["claude"]
    assert personal["adapter"]["env"] == {"CLAUDE_CONFIG_DIR": f"{home}/.claude-personal"}
    machine.settings.unlink()
    assert machine.run("gold-band") == 0
    assert list(machine.agents()) == ["claude-work"]
    work = machine.agents()["claude-work"]
    assert work["adapter"]["env"] == {"CLAUDE_CONFIG_DIR": f"{home}/.claude"}
    # The adapter command is the pinned package the app's catalog ships.
    assert work["adapter"]["args"] == ["-y", "@agentclientprotocol/claude-agent-acp@0.87.0"]


# ---- the launcher: the process it starts gets the instance's directory ---------


@pytest.mark.skipif(GOLD_BAND_BIN is None, reason="GOLD_BAND_BIN not set: point it at a headless gold-band binary")
@pytest.mark.parametrize(
    ("agent", "instance", "directory"), [("claude", None, ".claude-personal"), ("claude-work", "gold-band", ".claude")]
)
def test_the_launched_claude_process_has_the_instances_directory_despite_a_conflicting_variable(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, agent: str, instance: str | None, directory: str
) -> None:
    home = tmp_path / "home"
    home.mkdir()
    real_path = os.environ["PATH"]
    machine = Machine(tmp_path, monkeypatch, [])
    machine.registry = converge.REGISTRY_PATH
    monkeypatch.setenv("HOME", str(home))
    machine.settings = home / ".gold-band" / "settings.json"
    machine.settings.parent.mkdir()
    machine.settings.write_text(json.dumps({"settingsSchemaVersion": 12}))
    assert machine.run(instance) == 0

    # The stub stands in for `npx -y <adapter package>`: it records the
    # variable it was started with, then speaks ACP like the other stubs.
    seen = tmp_path / "seen"
    bin_dir = tmp_path / "bin"
    (bin_dir / "npx").write_text(
        f'#!/bin/sh\nprintf %s "$CLAUDE_CONFIG_DIR" > "{seen}"\n'
        f'STUB_ACP_MODELS=stub-model exec "{sys.executable}" "{STUB}"\n'
    )
    repo = tmp_path / "repo"
    repo.mkdir()
    subprocess.run(["git", "-C", str(repo), "init", "-q"], check=True, env=dict(os.environ, PATH=real_path))
    workflow = {
        "workflow": {
            "description": "One worker on the Claude agent.",
            "version": "0.1",
            "id": "config-dir",
            "entry": "work",
            "control": {"max_attempts": 1, "max_rounds": 1},
            "nodes": [
                {
                    "type": "worker",
                    "id": "work",
                    "executionSlotId": "slot-work",
                    "provider": agent,
                    "model": "stub-model",
                    "profile": "pf-builtin-dev",
                    "goal": "Answer with one fenced json block holding ok.",
                    "output": {"kind": "json", "artifact": "result", "schema": {"ok": True}},
                }
            ],
            "edges": [{"from": "work", "to": "$end", "on": "success"}],
        },
        "modelBindings": {"bindings": [{"executionSlotId": "slot-work", "agentId": agent, "modelId": "stub-model"}]},
    }
    path = tmp_path / "workflow.json"
    path.write_text(json.dumps(workflow))
    requirement = repo / "requirement.md"
    requirement.write_text("# Requirement\n")
    assert GOLD_BAND_BIN is not None
    env = dict(os.environ, HOME=str(home), GOLD_BAND_HOME=str(home), CLAUDE_CONFIG_DIR="/conflicting/ambient/dir")
    env["PATH"] = f"{bin_dir}{os.pathsep}{real_path}"
    run = subprocess.run(
        [GOLD_BAND_BIN, "run", "--requirement-file", str(requirement), "--workflow", str(path)],
        capture_output=True,
        text=True,
        timeout=600,
        env=env,
        cwd=str(repo),
        check=False,
    )
    assert seen.exists(), f"the adapter was never started\n{run.stdout[-2000:]}\n{run.stderr[-2000:]}"
    assert seen.read_text() == str(home / directory)


def test_the_work_instance_is_never_converged_onto_the_default_personal_path(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    machine = Machine(tmp_path, monkeypatch, [_claude("claude-work", WORK_DIR, instance="gold-band")])
    monkeypatch.delenv("GOLD_BAND_HOME", raising=False)
    argv = ["gold-band", "--keep", str(machine.keep), "--registry", str(machine.registry), "--instance", "gold-band"]
    assert converge.main(argv) == 3
    assert "gold-band needs --settings or GOLD_BAND_HOME" in capsys.readouterr().out
    monkeypatch.setenv("GOLD_BAND_HOME", str(tmp_path / "work-home"))
    assert converge.main(argv) == 0
    assert (tmp_path / "work-home" / ".gold-band" / "settings.json").is_file()


def test_two_spellings_of_one_directory_are_one_directory(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    same = str(Path.home() / ".claude")
    machine = Machine(
        tmp_path, monkeypatch, [_claude("claude", same), _claude("claude-work", "~/.claude/.", instance="gold-band")]
    )
    assert machine.run() == 1
    assert "claude-work: config_dir ~/.claude/. is already claude's" in capsys.readouterr().out
