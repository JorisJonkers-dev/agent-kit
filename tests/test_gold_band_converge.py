"""Gold Band's settings.json agents map is converged onto the registry and the keep file.

Each test drives ``converge.main(["gold-band", ...])`` against a temporary
registry, settings file and keep file, with a fake ``opencode`` on PATH. The
running guard is driven through ``AGENT_KIT_GOLD_BAND_RUNNING``; process
detection itself is covered in ``test_gold_band_instance_guard.py``.
"""

from __future__ import annotations

import json
import os
import shutil
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
    machine = Machine(tmp_path, monkeypatch, settings={"agents": {}}, registry=_registry(agents=[{"id": "vibe"}]))
    assert machine.run() == 0
    assert machine.agents() == {"vibe": {}}


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


def test_the_settings_path_without_gold_band_home_is_under_the_home(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setenv("HOME", str(tmp_path))
    assert converge._gold_band_settings_path() == tmp_path / ".gold-band" / "settings.json"
    monkeypatch.setenv("GOLD_BAND_HOME", "  ")
    assert converge._gold_band_settings_path() == tmp_path / ".gold-band" / "settings.json"


def test_the_settings_path_defaults_to_gold_band_home(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    home = tmp_path / "gb"
    monkeypatch.setenv("GOLD_BAND_HOME", str(home))
    machine = Machine(tmp_path, monkeypatch)
    assert machine.run(settings=False) == 0
    assert not (home / "settings.json").exists()
    assert json.loads((home / ".gold-band" / "settings.json").read_text())["agents"]["opencode"] == OPENCODE_ENTRY


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
    assert machine.run() == converge.GOLD_BAND_RUNNING_EXIT
    out = capsys.readouterr().out
    assert "gold-band-agents: Gold Band instance gold-band-personal" in out
    assert "quit it and re-run setup (nothing was written)" in out
    assert machine.settings.read_bytes() == before
    assert not machine.backup.exists()


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


def test_the_kits_registry_declares_vibe_and_vibe_study_without_a_key(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    machine = Machine(tmp_path, monkeypatch, binaries=("opencode", "vibe-acp", "vibe-study"))
    monkeypatch.setenv("MISTRAL_STUDY_API_KEY", "sentinel-study-key")
    monkeypatch.setenv("MISTRAL_API_KEY", "sentinel-personal-key")
    assert machine.run(registry=False) == 0
    agents = machine.agents()
    home = str(Path.home())
    assert agents["vibe"]["adapter"] == {
        "displayName": "Vibe",
        "command": f"{home}/.local/bin/vibe-acp",
        "args": [],
        "env": {},
    }
    assert agents["vibe-study"]["adapter"] == {
        "displayName": "Vibe (Study)",
        "command": f"{home}/.local/bin/vibe-study",
        "args": ["--acp"],
        "env": {},
    }
    text = machine.settings.read_text()
    assert "sentinel" not in text
    assert "sentinel" not in converge.REGISTRY_PATH.read_text()


QUIET = {
    "updater": {"enabled": False},
    "metrics": {"enabled": False},
    "feedback": {"enabled": False},
    "connectors": {"slack": False},
    "taskBoard": {"enabled": False},
    "editor": {"theme": "dark", "nested": {"a": [1, 2]}},
}


def test_a_failed_replace_leaves_the_old_settings_and_one_backup_and_is_reported(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    machine = Machine(tmp_path, monkeypatch, settings={"agents": {}, **QUIET})
    original = machine.settings.read_bytes()
    real_replace = os.replace

    def refuse(src: object, dst: object) -> None:
        if Path(str(dst)) == machine.settings:
            raise PermissionError("replace refused")
        real_replace(src, dst)

    monkeypatch.setattr(os, "replace", refuse)
    assert machine.run() == 1
    assert "old file is in place" in capsys.readouterr().out
    assert machine.settings.read_bytes() == original
    assert machine.backup.read_bytes() == original
    assert sorted(p.name for p in machine.settings.parent.iterdir()) == ["settings.json", "settings.json.bak"]


def test_a_failed_backup_leaves_the_old_settings_and_no_staged_file(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    machine = Machine(tmp_path, monkeypatch, settings={"agents": {}})
    original = machine.settings.read_bytes()

    def refuse(*_args: object, **_kwargs: object) -> None:
        raise OSError("disk full")

    monkeypatch.setattr(shutil, "copy2", refuse)
    assert machine.run() == 1
    assert "not replaced" in capsys.readouterr().out
    assert machine.settings.read_bytes() == original
    assert [p.name for p in machine.settings.parent.iterdir()] == ["settings.json"]


def test_keys_setup_does_not_declare_are_unchanged_and_the_quiet_ones_stay_off(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    machine = Machine(tmp_path, monkeypatch, settings={"agents": {}, **QUIET})
    assert machine.run() == 0
    written = json.loads(machine.settings.read_text())
    assert {key: value for key, value in written.items() if key != "agents"} == QUIET
    assert written["agents"]


def test_a_fresh_settings_file_holds_only_the_agent_map_so_nothing_is_enabled(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    machine = Machine(tmp_path, monkeypatch)
    assert machine.run() == 0
    assert list(json.loads(machine.settings.read_text())) == ["agents"]


@pytest.mark.parametrize("existing", [False, True])
def test_the_data_directory_is_owner_only(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, existing: bool
) -> None:
    machine = Machine(tmp_path, monkeypatch, settings={"agents": {}} if existing else None)
    if existing:
        machine.settings.parent.chmod(0o755)
    assert machine.run() == 0
    assert machine.settings.parent.stat().st_mode & 0o777 == 0o700
    assert machine.settings.stat().st_mode & 0o777 == 0o600
    assert (not existing) or machine.backup.stat().st_mode & 0o777 == 0o600


def test_an_unchanged_run_still_makes_the_data_directory_owner_only(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    machine = Machine(tmp_path, monkeypatch)
    assert machine.run() == 0
    machine.settings.parent.chmod(0o755)
    assert machine.run() == 0
    assert machine.settings.parent.stat().st_mode & 0o777 == 0o700


LOUD = {
    "desktopMetricsEnabled": True,
    "desktopMetricsBaseUrl": "https://m.example",
    "desktopMulticaEnabled": True,
    "desktopUpdaterUrlOverride": "https://updates.example/latest.json",
    "imIntegrations": {
        "channels": [
            {"kind": "wecom", "enabled": True, "publicIdentity": "a"},
            {"kind": "slack", "enabled": False, "publicIdentity": "b"},
        ]
    },
    "logLevel": "debug",
}


def test_setup_turns_metrics_task_board_and_connectors_off_and_leaves_the_updater_override(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    machine = Machine(tmp_path, monkeypatch, settings={"agents": {}, **LOUD})
    assert machine.run() == 0
    written = json.loads(machine.settings.read_text())
    assert written["desktopMetricsEnabled"] is False
    assert written["desktopMulticaEnabled"] is False
    assert written["desktopUpdaterUrlOverride"] == "https://updates.example/latest.json"
    assert [c["enabled"] for c in written["imIntegrations"]["channels"]] == [False, False]
    assert written["desktopMetricsBaseUrl"] == "https://m.example"
    assert written["logLevel"] == "debug"
    assert written["imIntegrations"]["channels"] == [
        {**channel, "enabled": False} for channel in LOUD["imIntegrations"]["channels"]
    ]


def test_absent_quiet_keys_stay_absent_and_check_mode_writes_nothing(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    machine = Machine(tmp_path, monkeypatch, settings={"agents": {}, "logLevel": "info"})
    assert machine.run() == 0
    written = json.loads(machine.settings.read_text())
    assert not {k for k in written if k.startswith(("desktop", "im", "feedback"))}
    machine.settings.write_text(json.dumps({"agents": {}, **LOUD}))
    before = machine.settings.read_bytes()
    assert machine.run(check=True) == 0
    assert machine.settings.read_bytes() == before


def test_a_settings_file_that_only_needs_quieting_is_still_rewritten(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    first = Machine(tmp_path, monkeypatch)
    assert first.run() == 0
    doc = json.loads(first.settings.read_text())
    first.settings.write_text(json.dumps({**doc, "desktopMetricsEnabled": True}))
    assert first.run() == 0
    assert json.loads(first.settings.read_text())["desktopMetricsEnabled"] is False
    settled = first.settings.read_bytes()
    assert first.run() == 0
    assert first.settings.read_bytes() == settled


def test_odd_connector_shapes_are_left_alone(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    machine = Machine(tmp_path, monkeypatch, settings={"agents": {}, "imIntegrations": {"channels": ["x", {}]}})
    assert machine.run() == 0
    assert json.loads(machine.settings.read_text())["imIntegrations"] == {"channels": ["x", {}]}
    machine.settings.write_text(json.dumps({"agents": {}, "imIntegrations": 3}))
    assert machine.run() == 0
    assert json.loads(machine.settings.read_text())["imIntegrations"] == 3


# --- an agent whose adapter command is the installed binary's absolute path ---

PI = {
    "id": "pi",
    "binary": "pi-acp",
    "command_from_binary": True,
    "adapter": {"displayName": "Pi", "env": {}},
    "icon": "pi",
}


def test_command_from_binary_writes_the_installed_binarys_absolute_path(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    machine = Machine(
        tmp_path,
        monkeypatch,
        binaries=("pi-acp",),
        registry=_registry([PI], allowed=("pi",)),
    )
    assert machine.run() == 0
    entry = machine.agents()["pi"]
    assert entry["adapter"]["command"] == str(machine.bin / "pi-acp")
    assert Path(entry["adapter"]["command"]).is_absolute()
    assert "command_from_binary" not in entry and "binary" not in entry
    before = machine.settings.read_text()
    assert machine.run() == 0
    assert machine.settings.read_text() == before


def test_command_from_binary_with_the_binary_missing_is_skipped(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    machine = Machine(tmp_path, monkeypatch, binaries=(), registry=_registry([PI], allowed=("pi",)))
    assert machine.run() == 0
    assert "skipped: pi (pi-acp is not installed)" in capsys.readouterr().out
    assert "pi" not in machine.agents()


@pytest.mark.parametrize("bad", [{"command_from_binary": False}, {"command_from_binary": True, "binary": None}])
def test_command_from_binary_must_be_true_and_needs_a_binary(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str], bad: dict
) -> None:
    agent = {k: v for k, v in {**PI, **bad}.items() if v is not None}
    machine = Machine(tmp_path, monkeypatch, binaries=("pi-acp",), registry=_registry([agent], allowed=("pi",)))
    assert machine.run() == 1
    assert "pi: command_from_binary must be true and needs a binary" in capsys.readouterr().out


# --- a launch command declared per OS ---

AGY = {
    "id": "agy",
    "binary": "agy",
    "os": {"mac": {"command_from_binary": True, "adapter": {"args": ["acp"]}}},
    "adapter": {"displayName": "Antigravity", "env": {}},
    "icon": "antigravity",
}


def _agy_machine(tmp_path: Path, monkeypatch: pytest.MonkeyPatch, host: str, agent: dict = AGY) -> Machine:
    monkeypatch.setenv("AGENT_KIT_GOLD_BAND_HOST", host)
    return Machine(tmp_path, monkeypatch, binaries=("agy",), registry=_registry([agent], allowed=("agy",)))


def test_an_os_section_gives_the_launch_command_on_that_os(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    machine = _agy_machine(tmp_path, monkeypatch, "mac")
    assert machine.run() == 0
    adapter = machine.agents()["agy"]["adapter"]
    assert adapter == {"displayName": "Antigravity", "env": {}, "command": str(machine.bin / "agy"), "args": ["acp"]}
    assert "os" not in machine.agents()["agy"]
    before = machine.settings.read_text()
    assert machine.run() == 0
    assert machine.settings.read_text() == before


def test_an_agent_with_no_section_for_the_host_is_skipped_and_reported(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    machine = _agy_machine(tmp_path, monkeypatch, "linux")
    assert machine.run() == 0
    assert "skipped: agy (no launch command declared for linux)" in capsys.readouterr().out
    assert "agy" not in machine.agents()


def test_a_skipped_agents_existing_entry_is_left_alone(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("AGENT_KIT_GOLD_BAND_HOST", "linux")
    machine = Machine(
        tmp_path,
        monkeypatch,
        settings={"agents": {"agy": {"icon": "mine"}}},
        binaries=("agy",),
        registry=_registry([AGY], allowed=("agy",)),
    )
    assert machine.run() == 0
    assert machine.agents() == {"agy": {"icon": "mine"}}


def test_a_linux_section_overrides_the_binary_and_the_adapter(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    agent = {**AGY, "os": {**AGY["os"], "linux": {"binary": "agy-linux", "adapter": {"command": "agy-linux"}}}}
    monkeypatch.setenv("AGENT_KIT_GOLD_BAND_HOST", "linux")
    machine = Machine(tmp_path, monkeypatch, binaries=("agy-linux",), registry=_registry([agent], allowed=("agy",)))
    assert machine.run() == 0
    assert machine.agents()["agy"]["adapter"]["command"] == "agy-linux"


def test_the_host_comes_from_the_platform_when_not_forced(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.delenv("AGENT_KIT_GOLD_BAND_HOST", raising=False)
    monkeypatch.setattr(sys, "platform", "darwin")
    assert converge._gold_band_host() == "mac"
    monkeypatch.setattr(sys, "platform", "linux")
    assert converge._gold_band_host() == "linux"


@pytest.mark.parametrize(
    "launch",
    [
        {"binary": "/opt/homebrew/bin/agy"},
        {"adapter": {"command": "/opt/homebrew/bin/agy"}},
        {"adapter": {"command": "agy", "args": ["/Applications/Antigravity.app/Contents/MacOS/agy"]}},
        {"adapter": {"command": "/usr/local/Cellar/agy/1/bin/agy"}},
    ],
)
def test_a_macos_path_in_the_linux_launch_is_refused_on_any_host(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str], launch: dict
) -> None:
    agent = {**AGY, "os": {"linux": launch}}
    machine = _agy_machine(tmp_path, monkeypatch, "mac", agent)
    assert machine.run() == 1
    assert "agy: a macOS path (Homebrew or /Applications) is written on Linux" in capsys.readouterr().out
    assert not machine.settings.exists()


def test_a_macos_path_in_an_agent_with_no_os_section_is_refused(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    agent = {"id": "agy", "binary": "agy", "adapter": {"command": "/opt/homebrew/bin/agy"}}
    machine = _agy_machine(tmp_path, monkeypatch, "linux", agent)
    assert machine.run() == 1
    assert "agy: a macOS path (Homebrew or /Applications) is written on Linux" in capsys.readouterr().out


def test_a_macos_path_is_allowed_in_the_mac_section(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    agent = {**AGY, "os": {"mac": {"adapter": {"command": "/opt/homebrew/bin/agy"}}}}
    machine = _agy_machine(tmp_path, monkeypatch, "mac", agent)
    assert machine.run() == 0
    assert machine.agents()["agy"]["adapter"]["command"] == "/opt/homebrew/bin/agy"


@pytest.mark.parametrize(
    ("os_value", "message"),
    [
        ({}, "agy: os must map linux and/or mac to a launch declaration"),
        ("mac", "agy: os must map linux and/or mac to a launch declaration"),
        ({"windows": {}}, "agy: os.windows is not one of linux, mac"),
        ({"mac": "agy"}, "agy: os.mac must be a mapping"),
        ({"mac": {"icon": "x"}}, "agy: unknown field os.mac.icon"),
        (None, "agy: os must map linux and/or mac to a launch declaration"),
        ({"mac": {}}, "agy os.mac: must declare binary, command_from_binary or adapter"),
        ({"mac": {"command_from_binary": True}, "linux": {}}, "agy os.linux: must declare binary"),
        ({"mac": {"binary": 42}}, "agy os.mac: binary must be a non-empty string"),
        ({"mac": {"binary": None}}, "agy os.mac: binary must be a non-empty string"),
        ({"mac": {"command_from_binary": False}}, "agy os.mac: command_from_binary must be true"),
        ({"mac": {"adapter": {"comand": "agy"}}}, "agy os.mac: unknown adapter field comand"),
        ({"mac": {"adapter": "agy"}}, "agy os.mac: adapter must be a mapping"),
    ],
)
def test_a_malformed_os_declaration_is_refused(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str], os_value: object, message: str
) -> None:
    machine = _agy_machine(tmp_path, monkeypatch, "mac", {**AGY, "os": os_value})
    assert machine.run() == 1
    assert message in capsys.readouterr().out


def test_the_shipped_registry_declares_hermes_everywhere_and_antigravity_on_mac_only(
    monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str], tmp_path: Path
) -> None:
    registry = KIT_ROOT / "registry" / "estate-tooling.yaml"
    declared = {a["id"]: a for a in converge._gold_band_registry(registry)[0]}
    assert declared["hermes"]["binary"] == "hermes-acp" and declared["hermes"]["command_from_binary"] is True
    assert set(declared["agy"]["os"]) == {"mac"}
    for host in ("linux", "mac"):
        monkeypatch.setenv("AGENT_KIT_GOLD_BAND_HOST", host)
        monkeypatch.setenv("PATH", str(tmp_path))
        settings = tmp_path / f"{host}.json"
        argv = ["gold-band", "--registry", str(registry), "--settings", str(settings), "--keep", str(tmp_path / "k")]
        assert converge.main(argv) == 0
        out = capsys.readouterr().out
        assert "skipped: hermes (hermes-acp is not installed)" in out
        if host == "linux":
            assert "skipped: agy (no launch command declared for linux)" in out
        else:
            assert "skipped: agy (agy is not installed)" in out


def test_a_section_command_from_binary_needs_a_binary_somewhere(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    agent = {"id": "agy", "os": {"mac": {"command_from_binary": True}}, "adapter": {"env": {}}}
    machine = _agy_machine(tmp_path, monkeypatch, "mac", agent)
    assert machine.run() == 1
    assert "agy os.mac: command_from_binary needs a binary" in capsys.readouterr().out
