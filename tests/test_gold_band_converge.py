"""Gold Band's settings.json agents map is converged onto the registry and the keep file.

Each test drives ``converge.main(["gold-band", ...])`` against a temporary
registry, settings file and keep file, with a fake ``opencode`` on PATH. The
running guard is driven through ``AGENT_KIT_GOLD_BAND_RUNNING``, or through a
fake ``pgrep`` when that variable is unset.
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

import pytest
import yaml

KIT_ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(KIT_ROOT / "scripts"))

import converge  # noqa: E402

OPENCODE = {
    "id": "opencode",
    "binary": "opencode",
    "adapter": {"displayName": "OpenCode", "env": {}},
    "icon": "opencode",
    "primaryAgentDir": ".opencode",
    "compatibleAgentDirs": [".agents"],
    "systemPromptDelivery": "none",
    "externalSessionSyncSupported": False,
    "externalSessionSyncEnabled": False,
}

VIBE = {
    "id": "vibe",
    "adapter": {"displayName": "Vibe", "command": "vibe", "args": ["--acp"], "env": {"VIBE_TOKEN": "$VIBE_TOKEN"}},
    "icon": "vibe",
    "systemPromptDelivery": "meta-append",
}

# What the two declared agents above become in settings.json: no id, no binary,
# and no adapter.command or adapter.args for the built-in opencode.
OPENCODE_ENTRY = {
    "adapter": {"displayName": "OpenCode", "env": {}},
    "icon": "opencode",
    "primaryAgentDir": ".opencode",
    "compatibleAgentDirs": [".agents"],
    "systemPromptDelivery": "none",
    "externalSessionSyncSupported": False,
    "externalSessionSyncEnabled": False,
}

VIBE_ENTRY = {
    "adapter": {"displayName": "Vibe", "command": "vibe", "args": ["--acp"], "env": {"VIBE_TOKEN": "$VIBE_TOKEN"}},
    "icon": "vibe",
    "systemPromptDelivery": "meta-append",
}


def _registry(
    agents: list[dict] | None = None,
    retired: list[str] | None = None,
    allowed: tuple[str, ...] = ("opencode", "vibe", "claude"),
) -> dict:
    section: dict = {"agents": [OPENCODE, VIBE] if agents is None else agents}
    if retired is not None:
        section["retired_agents"] = retired
    return {"gold_band": section, "gold_band_instances": {"gold-band-personal": {"agents": list(allowed)}}}


class Machine:
    """A temporary registry, settings file, keep file and a PATH of fake binaries."""

    def __init__(
        self,
        tmp_path: Path,
        monkeypatch: pytest.MonkeyPatch,
        *,
        settings: dict | None = None,
        keep: list[str] | None = None,
        registry: dict | None = None,
        binaries: tuple[str, ...] = ("opencode",),
    ) -> None:
        self.bin = tmp_path / "bin"
        self.bin.mkdir()
        for name in binaries:
            self._fake(name, "#!/bin/sh\nexit 0\n")
        # Only the fakes: nothing else on the machine decides what is installed.
        monkeypatch.setenv("PATH", str(self.bin))
        self.settings = tmp_path / "gold-band" / "settings.json"
        if settings is not None:
            self.settings.parent.mkdir(parents=True, exist_ok=True)
            # Compact, so a run that rewrites it is visible in the bytes.
            self.settings.write_text(json.dumps(settings))
        self.keep = tmp_path / "keep.yaml"
        if keep is not None:
            self.keep.write_text(yaml.safe_dump({"keep": keep}))
        self.registry = tmp_path / "registry.yaml"
        self.registry.write_text(yaml.safe_dump(_registry() if registry is None else registry))

    def _fake(self, name: str, body: str) -> Path:
        path = self.bin / name
        path.write_text(body)
        path.chmod(0o755)
        return path

    @property
    def backup(self) -> Path:
        return self.settings.with_name(self.settings.name + ".bak")

    def run(self, *, check: bool = False, settings: bool = True, registry: bool = True) -> int:
        argv = ["gold-band", "--keep", str(self.keep)]
        if settings:
            argv += ["--settings", str(self.settings)]
        if registry:
            argv += ["--registry", str(self.registry)]
        if check:
            argv.append("--check")
        return converge.main(argv)

    def agents(self) -> dict:
        return json.loads(self.settings.read_text())["agents"]


@pytest.fixture(autouse=True)
def _not_running(monkeypatch: pytest.MonkeyPatch) -> None:
    # The guard reads this first, so no test depends on the machine's processes.
    monkeypatch.setenv("AGENT_KIT_GOLD_BAND_RUNNING", "0")
    monkeypatch.delenv("GOLD_BAND_HOME", raising=False)


def test_the_first_run_adds_the_declared_agents_and_keeps_the_other_keys_in_order(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    machine = Machine(
        tmp_path, monkeypatch, settings={"editor": {"theme": "dark"}, "agents": {}, "window": {"width": 900}}
    )
    assert machine.run() == 0
    out = capsys.readouterr().out
    assert "added opencode" in out
    assert "added vibe" in out
    doc = json.loads(machine.settings.read_text())
    assert list(doc) == ["editor", "agents", "window"]
    assert doc["editor"] == {"theme": "dark"}
    assert doc["window"] == {"width": 900}
    assert doc["agents"] == {"opencode": OPENCODE_ENTRY, "vibe": VIBE_ENTRY}
    assert "binary" not in doc["agents"]["opencode"]
    assert "command" not in doc["agents"]["opencode"]["adapter"]
    assert "args" not in doc["agents"]["opencode"]["adapter"]


def test_a_settings_file_without_an_agents_key_gains_one_at_the_end(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    machine = Machine(tmp_path, monkeypatch, settings={"editor": {"theme": "dark"}})
    assert machine.run() == 0
    doc = json.loads(machine.settings.read_text())
    assert list(doc) == ["editor", "agents"]
    assert doc["agents"] == {"opencode": OPENCODE_ENTRY, "vibe": VIBE_ENTRY}


def test_a_second_run_changes_nothing_and_makes_no_new_backup(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    machine = Machine(tmp_path, monkeypatch, settings={"agents": {}})
    assert machine.run() == 0
    capsys.readouterr()
    written = machine.settings.read_bytes()
    backup = machine.backup.read_bytes()
    stamp = machine.backup.stat().st_mtime_ns
    assert machine.run() == 0
    assert "settings unchanged" in capsys.readouterr().out
    assert machine.settings.read_bytes() == written
    assert machine.backup.read_bytes() == backup
    assert machine.backup.stat().st_mtime_ns == stamp


def test_an_undeclared_agent_is_removed(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    machine = Machine(tmp_path, monkeypatch, settings={"agents": {"pycharm": {"adapter": {"displayName": "PyCharm"}}}})
    assert machine.run() == 0
    assert "removed pycharm" in capsys.readouterr().out
    assert "pycharm" not in machine.agents()


def test_an_undeclared_agent_the_keep_file_names_stays(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    machine = Machine(
        tmp_path,
        monkeypatch,
        settings={"agents": {"pycharm": {"adapter": {"displayName": "PyCharm"}}}},
        keep=["pycharm"],
    )
    assert machine.run() == 0
    assert "removed" not in capsys.readouterr().out
    assert machine.agents()["pycharm"] == {"adapter": {"displayName": "PyCharm"}}


def test_a_retired_agent_is_removed_even_when_the_keep_file_names_it(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    machine = Machine(
        tmp_path,
        monkeypatch,
        settings={"agents": {"pycharm": {"adapter": {}}}},
        keep=["pycharm"],
        registry=_registry(retired=["pycharm"]),
    )
    assert machine.run() == 0
    assert "removed pycharm" in capsys.readouterr().out
    assert "pycharm" not in machine.agents()


def test_a_codex_named_agent_is_removed_even_when_the_keep_file_names_it(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    machine = Machine(tmp_path, monkeypatch, settings={"agents": {"codex-acp": {"adapter": {}}}}, keep=["codex-acp"])
    assert machine.run() == 0
    assert "removed codex-acp" in capsys.readouterr().out
    assert "codex-acp" not in machine.agents()


def test_a_declared_agent_whose_binary_is_missing_is_skipped_and_left_alone(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    machine = Machine(
        tmp_path, monkeypatch, binaries=(), settings={"agents": {"opencode": {"adapter": {"displayName": "Mine"}}}}
    )
    assert machine.run() == 0
    out = capsys.readouterr().out
    assert "skipped: opencode (opencode is not installed)" in out
    assert "added opencode" not in out
    assert machine.agents()["opencode"] == {"adapter": {"displayName": "Mine"}}
    assert machine.agents()["vibe"] == VIBE_ENTRY


def test_check_prints_the_change_and_writes_nothing(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    machine = Machine(tmp_path, monkeypatch, settings={"agents": {}})
    before = machine.settings.read_bytes()
    assert machine.run(check=True) == 0
    assert "added opencode" in capsys.readouterr().out
    assert machine.settings.read_bytes() == before
    assert not machine.backup.exists()


def test_a_missing_settings_file_starts_empty_and_its_directory_appears_only_on_a_write(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    machine = Machine(tmp_path, monkeypatch)
    assert machine.run(check=True) == 0
    assert "added opencode" in capsys.readouterr().out
    assert not machine.settings.parent.exists()
    assert machine.run() == 0
    assert machine.agents() == {"opencode": OPENCODE_ENTRY, "vibe": VIBE_ENTRY}


def test_a_changed_entry_is_reported(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    machine = Machine(tmp_path, monkeypatch, settings={"agents": {"opencode": {"adapter": {"displayName": "Old"}}}})
    assert machine.run() == 0
    assert "changed opencode" in capsys.readouterr().out
    assert machine.agents()["opencode"] == OPENCODE_ENTRY


def test_a_declared_agent_with_no_adapter_or_binary_is_written_as_an_empty_entry(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    machine = Machine(tmp_path, monkeypatch, settings={"agents": {}}, registry=_registry(agents=[{"id": "claude"}]))
    assert machine.run() == 0
    assert machine.agents() == {"claude": {}}


def test_a_leading_tilde_in_the_adapter_expands_to_the_home_directory(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    agent = {
        "id": "vibe",
        "adapter": {
            "displayName": "Vibe",
            "command": "~/bin/vibe",
            "args": ["~/a", "b"],
            "env": {"VIBE_TOKEN": "$VIBE_TOKEN"},
        },
    }
    machine = Machine(tmp_path, monkeypatch, settings={"agents": {}}, registry=_registry(agents=[agent]))
    assert machine.run() == 0
    adapter = machine.agents()["vibe"]["adapter"]
    home = str(Path.home())
    assert adapter["command"] == f"{home}/bin/vibe"
    assert adapter["args"] == [f"{home}/a", "b"]
    assert adapter["env"] == {"VIBE_TOKEN": "$VIBE_TOKEN"}


def test_a_change_copies_the_settings_file_to_a_backup(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    machine = Machine(tmp_path, monkeypatch, settings={"agents": {}})
    original = machine.settings.read_bytes()
    assert machine.run() == 0
    assert machine.backup.read_bytes() == original
    assert machine.settings.stat().st_mode & 0o777 == 0o600


def test_the_settings_path_defaults_to_gold_band_home(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    home = tmp_path / "gb"
    monkeypatch.setenv("GOLD_BAND_HOME", str(home))
    machine = Machine(tmp_path, monkeypatch)
    assert machine.run(settings=False) == 0
    assert json.loads((home / "settings.json").read_text())["agents"]["opencode"] == OPENCODE_ENTRY


def test_the_registry_path_defaults_to_the_kits_own(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    machine = Machine(tmp_path, monkeypatch, settings={"agents": {}})
    assert machine.run(registry=False) == 0
    assert "opencode" in machine.agents()


def test_the_running_guard_refuses_to_write(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    monkeypatch.setenv("AGENT_KIT_GOLD_BAND_RUNNING", "1")
    machine = Machine(tmp_path, monkeypatch, settings={"agents": {}})
    before = machine.settings.read_bytes()
    assert machine.run() == 3
    assert (
        "gold-band-agents: Gold Band is running; quit it and re-run setup (nothing was written)"
        in capsys.readouterr().out
    )
    assert machine.settings.read_bytes() == before
    assert not machine.backup.exists()


def test_the_running_guard_reads_pgrep_when_the_variable_is_unset(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    monkeypatch.delenv("AGENT_KIT_GOLD_BAND_RUNNING")
    machine = Machine(tmp_path, monkeypatch, settings={"agents": {}})
    machine._fake("pgrep", "#!/bin/sh\nexit 0\n")
    assert machine.run() == 3
    assert "Gold Band is running" in capsys.readouterr().out


def test_a_machine_without_pgrep_is_not_running(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.delenv("AGENT_KIT_GOLD_BAND_RUNNING")
    machine = Machine(tmp_path, monkeypatch, settings={"agents": {}})
    assert machine.run() == 0
    assert "opencode" in machine.agents()


def test_settings_that_are_not_json_exit_3(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    machine = Machine(tmp_path, monkeypatch, settings={"agents": {}})
    machine.settings.write_text("{not json")
    assert machine.run() == 3
    assert f"gold-band-agents: cannot converge: {machine.settings} is not a JSON object" in capsys.readouterr().out
    assert machine.settings.read_text() == "{not json"


@pytest.mark.parametrize("body", ["[1, 2]", '{"agents": []}'])
def test_settings_that_are_not_a_json_object_exit_3(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str], body: str
) -> None:
    machine = Machine(tmp_path, monkeypatch, settings={"agents": {}})
    machine.settings.write_text(body)
    assert machine.run() == 3
    assert f"gold-band-agents: cannot converge: {machine.settings} is not a JSON object" in capsys.readouterr().out
    assert machine.settings.read_text() == body


def test_an_unreadable_registry_exits_3(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    machine = Machine(tmp_path, monkeypatch, settings={"agents": {}})
    machine.registry.unlink()
    before = machine.settings.read_bytes()
    assert machine.run() == 3
    out = capsys.readouterr().out
    assert "gold-band-agents: cannot converge:" in out
    assert "cannot be read" in out
    assert machine.settings.read_bytes() == before


@pytest.mark.parametrize(
    ("body", "suffix"),
    [
        ("- 1\n", " is not a mapping"),
        ("gold_band_instances: {}\n", " has no gold_band mapping"),
        ("gold_band:\n  agents: [1]\n", ": gold_band.agents must be a list of mappings"),
        ("gold_band:\n  agents: []\n  retired_agents: x\n", ": gold_band.retired_agents must be a list of ids"),
        ("gold_band:\n  agents: []\n", ": gold_band_instances.gold-band-personal.agents must be a list of ids"),
        (
            "gold_band:\n  agents: []\ngold_band_instances:\n  gold-band-personal:\n    agents: x\n",
            ": gold_band_instances.gold-band-personal.agents must be a list of ids",
        ),
    ],
)
def test_a_malformed_registry_exits_3(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str], body: str, suffix: str
) -> None:
    machine = Machine(tmp_path, monkeypatch, settings={"agents": {}})
    machine.registry.write_text(body)
    before = machine.settings.read_bytes()
    assert machine.run() == 3
    assert f"gold-band-agents: cannot converge: {machine.registry}{suffix}" in capsys.readouterr().out
    assert machine.settings.read_bytes() == before


@pytest.mark.parametrize(
    ("agents", "allowed", "line"),
    [
        ([{"id": "OpenCode"}], ("opencode",), "OpenCode: id must match ^[a-z0-9]+(-[a-z0-9]+)*$"),
        ([{"binary": "x"}], ("opencode",), "?: id must match ^[a-z0-9]+(-[a-z0-9]+)*$"),
        ([OPENCODE, OPENCODE], ("opencode",), "opencode: duplicate id"),
        ([{"id": "codex-acp"}], ("codex-acp",), "codex-acp: Codex is refused on the personal instance"),
        ([{"id": "vibe"}], ("opencode",), "vibe: not in gold_band_instances.gold-band-personal.agents"),
        ([{"id": "opencode", "colour": "red"}], ("opencode",), "opencode: unknown field colour"),
        ([{"id": "opencode", "adapter": "x"}], ("opencode",), "opencode: adapter must be a mapping"),
        ([{"id": "opencode", "adapter": {"shell": "x"}}], ("opencode",), "opencode: unknown adapter field shell"),
        ([{"id": "opencode", "adapter": {"command": 3}}], ("opencode",), "opencode: adapter.command must be a string"),
        (
            [{"id": "opencode", "adapter": {"args": "x"}}],
            ("opencode",),
            "opencode: adapter.args must be a list of strings",
        ),
        (
            [{"id": "opencode", "adapter": {"env": {"A": 1}}}],
            ("opencode",),
            "opencode: adapter.env must be a mapping of string to string",
        ),
        (
            [{"id": "opencode", "adapter": {"env": {"A": "secret"}}}],
            ("opencode",),
            "opencode: adapter.env A must be an environment reference, not a literal",
        ),
        (
            [{"id": "opencode", "adapter": {"command": "codex"}}],
            ("opencode",),
            "opencode: Codex is refused in adapter.command or adapter.args",
        ),
        (
            [{"id": "opencode", "adapter": {"args": ["--agent", "Codex"]}}],
            ("opencode",),
            "opencode: Codex is refused in adapter.command or adapter.args",
        ),
        (
            [{"id": "opencode", "systemPromptDelivery": "append"}],
            ("opencode",),
            "opencode: systemPromptDelivery must be one of none, meta-append",
        ),
    ],
)
def test_a_declared_agent_that_breaks_a_rule_is_refused(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
    capsys: pytest.CaptureFixture[str],
    agents: list[dict],
    allowed: tuple[str, ...],
    line: str,
) -> None:
    machine = Machine(
        tmp_path,
        monkeypatch,
        settings={"agents": {"pycharm": {"adapter": {}}}},
        registry=_registry(agents=agents, allowed=allowed),
    )
    before = machine.settings.read_bytes()
    assert machine.run() == 1
    assert f"gold-band-agents: {line}" in capsys.readouterr().out
    assert machine.settings.read_bytes() == before
    assert not machine.backup.exists()


def test_an_agent_declared_and_retired_at_once_is_refused(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    machine = Machine(
        tmp_path,
        monkeypatch,
        settings={"agents": {"opencode": {}}},
        registry=_registry(agents=[OPENCODE], retired=["opencode"]),
    )
    before = machine.settings.read_bytes()
    assert machine.run() == 1
    assert "opencode: declared and retired at once" in capsys.readouterr().out
    assert machine.settings.read_bytes() == before


def test_an_unreadable_settings_file_cannot_converge(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    machine = Machine(tmp_path, monkeypatch, settings={"agents": {}}, registry=_registry())
    machine.settings.chmod(0)
    try:
        if machine.settings.open().readable():
            pytest.skip("permissions are not enforced for this user")
    except PermissionError:
        pass
    assert machine.run() == 3
    assert "cannot converge" in capsys.readouterr().out
    machine.settings.chmod(0o600)
