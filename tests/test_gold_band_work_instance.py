"""The work Mac converges the work instance, and only that one, from the overlay.

Placeholder values only: every name, path and URL below is a fixture. The crate
that checks the overlay is a stub whose exit status each test chooses; its own
rules are tested in the crate. Setup runs against a temp HOME with a stubbed
``uname`` reporting Darwin.
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

import pytest
import yaml

KIT_ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(KIT_ROOT / "scripts"))
sys.path.insert(0, str(Path(__file__).resolve().parent))

import converge  # noqa: E402
import gold_band_instances as gbi  # noqa: E402
from setup_harness import setup_run  # noqa: E402

REGISTRY = KIT_ROOT / "registry" / "estate-tooling.yaml"

OVERLAY = {
    "instances": {
        "gold-band": {
            "agents": ["codex"],
            "mcp_servers": {
                "fixture-tracker": {
                    "command": "npx",
                    "args": ["-y", "fixture-tracker-mcp"],
                    "env": {"FIXTURE_TOKEN": "$FIXTURE_TOKEN"},
                },
                "fixture-wiki": {"url": "https://wiki.fixture.invalid/mcp"},
            },
            "settings": {"fixtureEditor": {"theme": "dark"}, "fixtureFlag": True},
        }
    }
}


def _crate(tmp_path: Path, monkeypatch: pytest.MonkeyPatch, body: str = "exit 0") -> None:
    binary = tmp_path / "crate-stub"
    binary.write_text(f"#!/bin/sh\n{body}\n")
    binary.chmod(0o755)
    monkeypatch.setenv(gbi.BIN_ENV, str(binary))


class Mac:
    """A work Mac: fake claude-work and codex launchers, a settings file and an overlay."""

    def __init__(self, tmp_path: Path, monkeypatch: pytest.MonkeyPatch, overlay: dict | None = OVERLAY) -> None:
        bin_dir = tmp_path / "bin"
        bin_dir.mkdir()
        for name in ("npx", "codex"):
            (bin_dir / name).write_text("#!/bin/sh\nexit 0\n")
            (bin_dir / name).chmod(0o755)
        monkeypatch.setenv("PATH", str(bin_dir))
        monkeypatch.setenv("AGENT_KIT_GOLD_BAND_HOST", "mac")
        monkeypatch.setenv("AGENT_KIT_GOLD_BAND_RUNNING", "0")
        monkeypatch.delenv("GOLD_BAND_HOME", raising=False)
        _crate(tmp_path, monkeypatch)
        self.settings = tmp_path / "work" / ".gold-band" / "settings.json"
        self.keep = tmp_path / "keep.yaml"
        self.overlay = tmp_path / "overlay.yaml"
        if overlay is not None:
            self.overlay.write_text(yaml.safe_dump(overlay))

    def seed(self, document: dict) -> None:
        self.settings.parent.mkdir(parents=True, exist_ok=True)
        self.settings.write_text(json.dumps(document))

    def run(self, *extra: str, overlay: bool = True, instance: str = "gold-band") -> int:
        argv = ["gold-band", "--instance", instance, "--keep", str(self.keep), "--settings", str(self.settings)]
        argv += ["--registry", str(REGISTRY)]
        if overlay:
            argv += ["--overlay", str(self.overlay)]
        return converge.main([*argv, *extra])

    def document(self) -> dict:
        return json.loads(self.settings.read_text())


def test_the_work_instance_holds_exactly_claude_work_and_codex(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    mac = Mac(tmp_path, monkeypatch)
    mac.seed({"agents": {"opencode": {"icon": "opencode"}, "vibe": {"icon": "vibe"}, "codex-acp": {}}})

    assert mac.run() == 0

    agents = mac.document()["agents"]
    assert sorted(agents) == ["claude-work", "codex"]
    assert agents["claude-work"]["adapter"]["env"]["CLAUDE_CONFIG_DIR"].endswith("/.claude")
    assert agents["codex"]["adapter"]["command"] == "npx"
    assert agents["codex"]["adapter"]["args"][-1].startswith("@agentclientprotocol/codex-acp@")
    out = capsys.readouterr().out
    assert "removed opencode" in out and "removed vibe" in out


def test_the_keep_file_does_not_keep_a_personal_agent_on_the_work_instance(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    mac = Mac(tmp_path, monkeypatch)
    mac.keep.write_text(yaml.safe_dump({"keep": ["vibe", "codex-acp"]}))
    mac.seed({"agents": {"vibe": {"icon": "vibe"}, "codex-acp": {"icon": "codex"}}})

    assert mac.run() == 0
    assert sorted(mac.document()["agents"]) == ["claude-work", "codex"]


def test_the_overlay_settings_and_mcp_servers_are_applied(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    mac = Mac(tmp_path, monkeypatch)
    mac.seed({"fixtureEditor": {"fontSize": 12}, "contextServers": [{"id": "other", "name": "other"}]})

    assert mac.run() == 0

    doc = mac.document()
    assert doc["fixtureEditor"] == {"fontSize": 12, "theme": "dark"}
    assert doc["fixtureFlag"] is True
    by_id = {server["id"]: server for server in doc["contextServers"]}
    assert set(by_id) == {"other", "fixture-tracker", "fixture-wiki"}
    assert by_id["fixture-tracker"] == {
        "id": "fixture-tracker",
        "name": "fixture-tracker",
        "enabled": True,
        "transport": "stdio",
        "command": "npx",
        "args": ["-y", "fixture-tracker-mcp"],
        "env": {"FIXTURE_TOKEN": "$FIXTURE_TOKEN"},
        "managed": False,
    }
    assert by_id["fixture-wiki"]["transport"] == "http"
    assert by_id["fixture-wiki"]["url"] == "https://wiki.fixture.invalid/mcp"
    out = capsys.readouterr().out
    assert "overlay: set fixtureEditor.theme" in out
    assert "overlay: added mcp server fixture-tracker" in out


def test_a_second_run_changes_nothing_and_a_changed_overlay_updates_one_server(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    mac = Mac(tmp_path, monkeypatch)
    assert mac.run() == 0
    first = mac.settings.read_bytes()
    capsys.readouterr()

    assert mac.run() == 0
    assert mac.settings.read_bytes() == first
    assert "settings unchanged" in capsys.readouterr().out

    changed = json.loads(json.dumps(OVERLAY))
    changed["instances"]["gold-band"]["mcp_servers"]["fixture-wiki"]["url"] = "https://wiki2.fixture.invalid/mcp"
    mac.overlay.write_text(yaml.safe_dump(changed))
    assert mac.run() == 0
    assert "overlay: changed mcp server fixture-wiki" in capsys.readouterr().out
    urls = {s["id"]: s.get("url") for s in mac.document()["contextServers"]}
    assert urls["fixture-wiki"] == "https://wiki2.fixture.invalid/mcp"


def test_check_mode_reports_the_overlay_and_writes_nothing(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    mac = Mac(tmp_path, monkeypatch)

    assert mac.run("--check") == 0

    assert not mac.settings.exists()
    assert "overlay: added mcp server fixture-wiki" in capsys.readouterr().out


def test_without_an_overlay_the_agents_alone_are_converged(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    mac = Mac(tmp_path, monkeypatch, overlay=None)

    assert mac.run() == 0
    assert set(mac.document()) == {"agents"}

    assert mac.run(overlay=False) == 0


def test_an_overlay_without_servers_or_settings_adds_no_keys(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    mac = Mac(tmp_path, monkeypatch, overlay={"instances": {"gold-band": {"agents": ["codex"]}}})

    assert mac.run() == 0
    assert set(mac.document()) == {"agents"}


def test_an_overlay_the_crate_refuses_writes_nothing(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    mac = Mac(tmp_path, monkeypatch)
    _crate(tmp_path, monkeypatch, 'echo "overlay gold-band: unknown field fixture" >&2; exit 1')

    assert mac.run() == 1
    assert "gold-band-agents: overlay: overlay gold-band: unknown field fixture" in capsys.readouterr().out
    assert not mac.settings.exists()


def test_an_overlay_that_cannot_be_checked_writes_nothing(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    mac = Mac(tmp_path, monkeypatch)
    mac.overlay.write_text("- not\n- a mapping\n")

    assert mac.run() == 3
    assert "cannot converge" in capsys.readouterr().out
    assert not mac.settings.exists()


@pytest.mark.parametrize("head", ["agents", "contextServers"])
def test_the_overlay_cannot_set_the_agent_map_or_the_server_list_as_a_setting(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str], head: str
) -> None:
    mac = Mac(tmp_path, monkeypatch, overlay={"instances": {"gold-band": {"settings": {head: {"x": "1"}}}}})

    assert mac.run() == 3
    assert f"{head} is not set from the overlay" in capsys.readouterr().out
    assert not mac.settings.exists()


def test_a_setting_below_a_value_that_is_not_a_mapping_is_refused(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    mac = Mac(tmp_path, monkeypatch)
    mac.seed({"fixtureEditor": "plain"})

    assert mac.run() == 3
    assert "fixtureEditor holds a value that is not a mapping" in capsys.readouterr().out
    assert mac.document() == {"fixtureEditor": "plain"}


def test_a_server_list_that_is_not_a_list_is_refused(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    mac = Mac(tmp_path, monkeypatch)
    mac.seed({"contextServers": {"a": 1}})

    assert mac.run() == 3
    assert "contextServers is not a list" in capsys.readouterr().out


def test_the_overlay_applies_to_the_work_instance_only(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    mac = Mac(tmp_path, monkeypatch)

    assert mac.run(instance="gold-band-personal") == 3
    assert "--overlay applies to the work instance gold-band only" in capsys.readouterr().out
    assert not mac.settings.exists()


def test_nothing_from_the_overlay_enters_the_public_registry(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    before = REGISTRY.read_bytes()
    mac = Mac(tmp_path, monkeypatch)

    assert mac.run() == 0

    assert REGISTRY.read_bytes() == before
    for value in ("fixture-tracker", "fixture-wiki", "fixtureEditor", "FIXTURE_TOKEN"):
        assert value.encode() not in before
    work = next(a for a in yaml.safe_load(before)["gold_band"]["agents"] if a["id"] == "claude-work")
    assert work["instance"] == "gold-band"


def test_the_registry_declares_the_work_pair_for_the_work_instance_only() -> None:
    data = yaml.safe_load(REGISTRY.read_text())
    work = [a["id"] for a in data["gold_band"]["agents"] if a.get("instance") == "gold-band"]
    assert work == ["claude-work", "codex"]
    assert data["gold_band_instances"]["gold-band"]["agents"] == work
    assert "codex" not in data["gold_band_instances"]["gold-band-personal"]["agents"]


# ---- the overlay helper ----------------------------------------------------


def test_the_helper_says_whether_the_overlay_names_the_work_instance(tmp_path: Path) -> None:
    path = tmp_path / "overlay.yaml"
    assert gbi.names_work_instance(path) is False
    path.write_text("")
    assert gbi.names_work_instance(path) is False
    path.write_text("instances:\n")
    assert gbi.names_work_instance(path) is False
    path.write_text("instances:\n  gold-band-personal:\n    agents: [vibe]\n")
    assert gbi.names_work_instance(path) is False
    path.write_text("instances:\n  gold-band:\n")
    assert gbi.names_work_instance(path) is True
    path.write_text("instances: [gold-band]\n")
    with pytest.raises(gbi.InstanceError, match="must be a mapping"):
        gbi.names_work_instance(path)


def test_the_helper_command_prints_yes_no_or_the_reason(tmp_path: Path, capsys: pytest.CaptureFixture[str]) -> None:
    path = tmp_path / "overlay.yaml"
    assert gbi.main(["names-work-instance", "--overlay", str(path)]) == 0
    assert capsys.readouterr().out == "no\n"
    path.write_text("instances:\n  gold-band:\n    agents: [codex]\n")
    assert gbi.main(["names-work-instance", "--overlay", str(path)]) == 0
    assert capsys.readouterr().out == "yes\n"
    path.write_text("stray: 1\n")
    assert gbi.main(["names-work-instance", "--overlay", str(path)]) == 1
    assert "unknown top-level keys" in capsys.readouterr().err


# ---- setup, on a stubbed Mac -----------------------------------------------

# Real helpers and a real converge under `uv run ... python SCRIPT args`; the
# crate is the stub, since the setup PATH holds no cargo.
_UV = (
    'echo "uv $*" >> "$CALLS"\n'
    'while [ $# -gt 0 ] && [ "${1%.py}" = "$1" ]; do shift; done\n'
    f'[ $# -gt 0 ] && exec "{sys.executable}" "$@"\n'
    "exit 0"
)


def _setup(
    tmp_path: Path,
    overlay: str | None,
    platform: str = "Darwin",
    crate: str = "exit 0",
    env: dict[str, str] | None = None,
    **kwargs,
):
    tmp_path.mkdir(parents=True, exist_ok=True)
    binary = tmp_path / "crate-stub"
    binary.write_text(f"#!/bin/sh\n{crate}\n")
    binary.chmod(0o755)
    calls = tmp_path / "uv-calls.log"

    def prepare(home: Path) -> None:
        if overlay is not None:
            path = home / ".config" / "agent-kit" / "overlay.yaml"
            path.parent.mkdir(parents=True)
            path.write_text(overlay)

    run = setup_run(
        tmp_path / "run",
        platform,
        prepare_home=prepare,
        stubs={"uv": _UV.replace("$CALLS", str(calls))},
        extra_env={
            gbi.BIN_ENV: str(binary),
            "PYTHONPATH": "",
            "AGENT_KIT_GOLD_BAND_HOST": "mac",
            "AGENT_KIT_GOLD_BAND_RUNNING": "0",
            **(env or {}),
        },
        **kwargs,
    )
    return run, (calls.read_text().splitlines() if calls.exists() else [])


def _work_calls(lines: list[str]) -> list[str]:
    return [line for line in lines if "converge.py gold-band " in line and "--instance gold-band " in line]


WORK_OVERLAY = yaml.safe_dump(OVERLAY)


def test_setup_converges_the_work_instance_on_the_mac_when_the_overlay_names_it(tmp_path: Path) -> None:
    run, uv = _setup(tmp_path, WORK_OVERLAY)

    (call,) = _work_calls(uv)
    assert f"--settings {run.home}/.gold-band/settings.json" in call
    assert f"--overlay {run.home}/.config/agent-kit/overlay.yaml" in call
    assert f"--keep {run.home}/.config/agent-kit/keep.yaml" in call
    document = json.loads((run.home / ".gold-band" / "settings.json").read_text())
    assert sorted(document["agents"]) == ["claude-work", "codex"]
    assert document["fixtureFlag"] is True
    assert {s["id"] for s in document["contextServers"]} == {"fixture-tracker", "fixture-wiki"}
    assert "overlay: added mcp server fixture-wiki" in run.stdout


def test_the_work_instance_follows_a_moved_data_directory(tmp_path: Path) -> None:
    data = tmp_path / "instance"
    run, _ = _setup(tmp_path, WORK_OVERLAY, env={"GOLD_BAND_HOME": str(data)})

    assert (data / ".gold-band" / "settings.json").is_file()
    assert not (run.home / ".gold-band").exists()


def test_a_machine_without_an_overlay_leaves_the_work_instance_alone(tmp_path: Path) -> None:
    run, uv = _setup(tmp_path, None)

    assert _work_calls(uv) == []
    assert "gold band work instance: no overlay, the work instance is left alone" in run.stdout
    assert not (run.home / ".gold-band").exists()


def test_an_overlay_that_does_not_name_the_work_instance_leaves_it_alone(tmp_path: Path) -> None:
    run, uv = _setup(tmp_path, "instances:\n  gold-band-personal:\n    agents: [vibe]\n")

    assert _work_calls(uv) == []
    assert "the overlay does not name it, left alone" in run.stdout
    assert not (run.home / ".gold-band").exists()


def test_an_unreadable_overlay_fails_setup(tmp_path: Path) -> None:
    run, uv = _setup(tmp_path, "- not\n- a mapping\n")

    assert _work_calls(uv) == []
    assert "the overlay cannot be read" in run.stdout + run.stderr
    assert run.returncode != 0


def test_a_refused_overlay_fails_setup_and_writes_nothing(tmp_path: Path) -> None:
    run, _ = _setup(tmp_path, WORK_OVERLAY, crate='echo "overlay gold-band: unknown field fixture" >&2; exit 1')

    assert "the Gold Band work instance did not converge (see above)" in run.stdout + run.stderr
    assert run.returncode != 0
    assert not (run.home / ".gold-band" / "settings.json").exists()


def test_check_mode_asks_for_a_report_only(tmp_path: Path) -> None:
    run, uv = _setup(tmp_path, WORK_OVERLAY, args=("--check",))

    (call,) = _work_calls(uv)
    assert " --check " in f"{call} "
    assert not (run.home / ".gold-band" / "settings.json").exists()


def test_a_running_work_instance_stops_setup(tmp_path: Path) -> None:
    run, _ = _setup(tmp_path, WORK_OVERLAY, env={"AGENT_KIT_GOLD_BAND_RUNNING": "1"})

    assert "Gold Band is running: setup stopped" in run.stdout + run.stderr
    assert run.returncode != 0
    assert not (run.home / ".gold-band" / "settings.json").exists()


def test_linux_and_a_cloud_session_do_not_converge_the_work_instance(tmp_path: Path) -> None:
    run, uv = _setup(tmp_path / "linux", WORK_OVERLAY, platform="Linux")
    assert _work_calls(uv) == []
    assert "only the work Mac is converged here, skipped" in run.stdout
    run, uv = _setup(tmp_path / "cloud", WORK_OVERLAY, args=("--cloud",))
    assert _work_calls(uv) == []
