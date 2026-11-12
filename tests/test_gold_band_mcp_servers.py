"""Wave 6 MCP servers (B3-14): registered in the personal instance, absent from the work one.

Registry validation is driven through ``render_registry.validate`` and the
convergence through ``converge.main(["gold-band", ...])``, always against a
temporary registry, settings file and keep file.
"""

from __future__ import annotations

import json
import shlex
import sys
from pathlib import Path

import pytest
import yaml

KIT_ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(KIT_ROOT / "scripts"))

import converge  # noqa: E402
import render_registry  # noqa: E402

WAVE_6 = (
    "memory-api", "n8n", "n8n-mcp", "kubernetes", "github",
    "grimoire", "outline", "playwright", "drawio", "overleaf",
)
PERSONAL = "gold-band-personal"
WORK = "gold-band"


def _real() -> dict:
    return yaml.safe_load((KIT_ROOT / "registry" / "estate-tooling.yaml").read_text())


def _servers(data: dict) -> dict[str, dict]:
    return {server["name"]: server for server in data["mcp_servers"]}


def test_every_wave_6_server_is_registered_in_the_personal_instance_only() -> None:
    servers = _servers(_real())
    for name in WAVE_6:
        assert servers[name]["instances"] == [PERSONAL], name
    bound = {name for name, server in servers.items() if "instances" in server}
    assert bound == set(WAVE_6)


def test_every_wave_6_server_lists_reach_levels_and_none_takes_run_one_command() -> None:
    for name, server in _servers(_real()).items():
        if "instances" in server:
            assert server["reach"], name
            assert set(server["reach"]) <= set(render_registry.REACH_LEVELS), name
            assert "run-one-command" not in server["reach"], name


def test_the_real_registry_validates() -> None:
    render_registry.validate(_real())


@pytest.fixture
def small() -> dict:
    """The real registry with one more bound server, which the tests mutate."""
    data = _real()
    data["mcp_servers"] = [
        *data["mcp_servers"],
        {
            "name": "docs",
            "transport": "stdio",
            "command": "docs-mcp",
            "surfaces": ["workstation"],
            "instances": [PERSONAL],
            "reach": ["read", "write"],
        },
    ]
    return data


def _docs(data: dict) -> dict:
    return _servers(data)["docs"]


def _record(tmp_path: Path, ids: dict) -> Path:
    path = tmp_path / "reach-levels.yaml"
    path.write_text(yaml.safe_dump({"agents": {}, "ids": ids}))
    return path


def _refused(data: dict, reach: Path | None = None) -> str:
    with pytest.raises(render_registry.RegistryError) as raised:
        render_registry.validate(data, reach) if reach else render_registry.validate(data)
    return str(raised.value)


def test_a_small_registry_with_a_bound_server_validates(small: dict) -> None:
    render_registry.validate(small)


@pytest.mark.parametrize(
    ("mutate", "expected"),
    [
        (lambda s: s.update(instances=[WORK]), "may not name gold-band"),
        (lambda s: s.update(instances=["elsewhere"]), "does not declare"),
        (lambda s: s.update(instances=[]), "non-empty list"),
        (lambda s: s.update(instances="gold-band-personal"), "non-empty list"),
        (lambda s: s.update(instances=[PERSONAL, PERSONAL]), "twice"),
        (lambda s: s.update(session_only=True, transport="http", url_workstation="https://x"), "session-only"),
        (lambda s: s.update(transport="http", url_workstation=""), "url_workstation"),
        (lambda s: s.pop("reach"), "list the reach levels"),
        (lambda s: s.update(reach=["root"]), "not one of"),
        (lambda s: s.update(reach=["read", "read"]), "lists a level twice"),
        (lambda s: s.update(reach=["run-one-command"]), "run-one-command"),
        (lambda s: s.pop("instances"), "only apply to a server with instances"),
        (lambda s: s.update(agent_ids="claude"), "list of agent ids"),
        (lambda s: s.update(agent_ids=["a", "a"]), "lists an id twice"),
    ],
)
def test_a_server_that_crosses_an_instance_or_reach_rule_is_refused(small: dict, mutate, expected: str) -> None:
    mutate(_docs(small))
    assert expected in _refused(small)


def test_an_agent_id_must_be_declared_in_the_support_record(small: dict, tmp_path: Path) -> None:
    _docs(small)["agent_ids"] = ["claude-read"]
    assert "not declared in reach-levels.yaml" in _refused(small, _record(tmp_path, {}))


def test_an_agent_id_above_the_servers_reach_levels_is_refused(small: dict, tmp_path: Path) -> None:
    _docs(small)["agent_ids"] = ["claude-research"]
    record = _record(tmp_path, {"claude-research": {"agent": "claude", "reach": "research"}})
    message = _refused(small, record)
    assert "'research'" in message and "does not allow" in message


def test_an_agent_id_of_another_instances_agent_is_refused(small: dict, tmp_path: Path) -> None:
    _docs(small)["agent_ids"] = ["codex-read"]
    record = _record(tmp_path, {"codex-read": {"agent": "codex", "reach": "read"}})
    assert "not an agent of gold-band-personal" in _refused(small, record)


def test_agent_ids_at_an_allowed_reach_level_validate(small: dict, tmp_path: Path) -> None:
    _docs(small)["agent_ids"] = ["claude-read", "vibe-write"]
    record = _record(
        tmp_path,
        {"claude-read": {"agent": "claude", "reach": "read"}, "vibe-write": {"agent": "vibe", "reach": "write"}},
    )
    render_registry.validate(small, record)


def test_an_unreadable_or_malformed_support_record_is_refused(small: dict, tmp_path: Path) -> None:
    _docs(small)["agent_ids"] = ["claude-read"]
    assert "cannot be read" in _refused(small, tmp_path / "missing.yaml")
    bad = tmp_path / "bad.yaml"
    bad.write_text("ids: [1]\n")
    assert "ids must be a mapping" in _refused(small, bad)
    empty = tmp_path / "empty.yaml"
    empty.write_text("agents: {}\n")
    assert "not declared" in _refused(small, empty)
    odd = tmp_path / "odd.yaml"
    odd.write_text("ids: {claude-read: null}\n")
    assert "is at reach level None" in _refused(small, odd)


# --- convergence -----------------------------------------------------------------


def _registry(servers: list[dict]) -> dict:
    return {
        "gold_band": {"agents": []},
        "gold_band_instances": {
            PERSONAL: {"agents": ["opencode"]},
            WORK: {"agents": ["claude-work"]},
        },
        "mcp_servers": servers,
    }


HTTP = {"name": "wiki", "transport": "http", "url_workstation": "https://wiki.example/mcp", "instances": [PERSONAL]}
TOKEN = {
    "name": "notes", "transport": "http", "url_workstation": "https://notes.example/mcp",
    "credential": "NOTES_KEY", "instances": [PERSONAL],
}
STDIO = {
    "name": "docs", "transport": "stdio", "command": "docs-mcp", "args": ["--fast"],
    "env": {"BASE": "https://x"}, "instances": [PERSONAL],
}
LOOSE = {"name": "loose", "transport": "http", "url_workstation": "https://loose.example/mcp"}


class Run:
    def __init__(self, tmp_path: Path, monkeypatch: pytest.MonkeyPatch, servers: list[dict],
                 settings: dict | None = None, keep: list[str] | None = None) -> None:
        monkeypatch.setenv("PATH", str(tmp_path))
        monkeypatch.setenv("AGENT_KIT_GOLD_BAND_RUNNING", "0")
        monkeypatch.delenv("GOLD_BAND_HOME", raising=False)
        self.settings = tmp_path / "band" / "settings.json"
        if settings is not None:
            self.settings.parent.mkdir(parents=True)
            self.settings.write_text(json.dumps(settings))
        self.keep = tmp_path / "keep.yaml"
        self.keep.write_text(yaml.safe_dump({"keep": keep or []}))
        self.registry = tmp_path / "registry.yaml"
        self.registry.write_text(yaml.safe_dump(_registry(servers)))

    def go(self, instance: str = PERSONAL, *, check: bool = False) -> int:
        argv = ["gold-band", "--keep", str(self.keep), "--settings", str(self.settings),
                "--registry", str(self.registry), "--instance", instance]
        return converge.main(argv + (["--check"] if check else []))

    def servers(self) -> list[dict]:
        return json.loads(self.settings.read_text())["contextServers"]


def test_the_personal_instance_gets_every_bound_server_and_the_work_instance_none(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    run = Run(tmp_path, monkeypatch, [HTTP, TOKEN, STDIO, LOOSE], settings={})
    assert run.go() == 0
    out = capsys.readouterr().out
    assert [s["id"] for s in run.servers()] == ["wiki", "notes", "docs"]
    assert "added server wiki" in out and "added server docs" in out
    wiki, notes, docs = run.servers()
    assert wiki == {"id": "wiki", "name": "wiki", "enabled": True, "transport": "http",
                    "url": "https://wiki.example/mcp", "headers": {}}
    assert docs["command"] == "docs-mcp" and docs["args"] == ["--fast"] and docs["env"] == {"BASE": "https://x"}
    # The token is never written: the server is the proxy, named by the secret's name.
    assert notes["transport"] == "stdio" and notes["command"] == "/bin/sh"
    assert notes["args"][1].endswith("--url https://notes.example/mcp --secret NOTES_KEY")
    assert run.settings.stat().st_mode & 0o777 == 0o600


def test_the_work_instance_holds_no_wave_6_server(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    present = {"id": "wiki", "name": "wiki", "enabled": True, "transport": "http", "url": "u", "headers": {}}
    overlay = {"id": "jira", "name": "jira", "enabled": True, "transport": "stdio", "command": "jira"}
    managed = {"id": "builtin", "name": "builtin", "managed": True}
    run = Run(tmp_path, monkeypatch, [HTTP], settings={"contextServers": [present, overlay, managed, 7]})
    assert run.go(WORK) == 0
    assert "removed server wiki" in capsys.readouterr().out
    assert run.servers() == [overlay, managed, 7]


def test_the_keep_file_keeps_a_server_and_a_managed_one_stays(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    mine = {"id": "mine", "name": "mine", "enabled": True}
    managed = {"id": "builtin", "managed": True}
    stray = {"id": "stray", "name": "stray"}
    run = Run(tmp_path, monkeypatch, [HTTP], settings={"contextServers": [mine, managed, stray]}, keep=["mine"])
    assert run.go() == 0
    assert "removed server stray" in capsys.readouterr().out
    assert [s["id"] for s in run.servers()] == ["mine", "builtin", "wiki"]


def test_a_second_run_changes_nothing_and_an_app_added_field_is_not_a_change(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    run = Run(tmp_path, monkeypatch, [HTTP], settings={})
    assert run.go() == 0
    written = json.loads(run.settings.read_text())
    written["contextServers"][0]["managed"] = False
    run.settings.write_text(json.dumps(written))
    before = run.settings.read_bytes()
    capsys.readouterr()
    assert run.go() == 0
    assert "settings unchanged" in capsys.readouterr().out
    assert run.settings.read_bytes() == before


def test_a_server_whose_declaration_changed_is_rewritten(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    old = {"id": "wiki", "name": "wiki", "enabled": True, "transport": "http", "url": "https://old", "headers": {}}
    run = Run(tmp_path, monkeypatch, [HTTP], settings={"contextServers": [old]})
    assert run.go() == 0
    assert "changed server wiki" in capsys.readouterr().out
    assert run.servers()[0]["url"] == "https://wiki.example/mcp"


def test_a_server_the_owner_switched_off_stays_off(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    off = {"id": "wiki", "name": "wiki", "enabled": False, "transport": "http", "url": "https://old", "headers": {}}
    run = Run(tmp_path, monkeypatch, [HTTP], settings={"contextServers": [off]})
    assert run.go() == 0
    assert run.servers()[0]["enabled"] is False and run.servers()[0]["url"] == "https://wiki.example/mcp"
    assert run.go() == 0
    assert run.servers()[0]["enabled"] is False


def test_the_proxy_command_quotes_the_url_and_the_secret_name(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    odd = {**TOKEN, "url_workstation": "https://n.example/mcp?a=1&b='2'"}
    run = Run(tmp_path, monkeypatch, [odd], settings={})
    assert run.go() == 0
    command = run.servers()[0]["args"][1]
    assert shlex.split(command)[2:] == ["--url", "https://n.example/mcp?a=1&b='2'", "--secret", "NOTES_KEY"]


def test_check_writes_nothing(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    run = Run(tmp_path, monkeypatch, [HTTP], settings={"agents": {}})
    before = run.settings.read_bytes()
    assert run.go(check=True) == 0
    assert "added server wiki" in capsys.readouterr().out
    assert run.settings.read_bytes() == before


def test_a_registry_without_bound_servers_leaves_settings_without_the_key(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    run = Run(tmp_path, monkeypatch, [LOOSE], settings={"agents": {}})
    assert run.go() == 0
    assert "contextServers" not in json.loads(run.settings.read_text())


def test_a_registry_with_no_server_list_is_fine(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    run = Run(tmp_path, monkeypatch, [], settings={})
    data = yaml.safe_load(run.registry.read_text())
    del data["mcp_servers"]
    run.registry.write_text(yaml.safe_dump(data))
    assert run.go() == 0


@pytest.mark.parametrize(
    ("servers", "expected"),
    [
        ([{**HTTP, "name": "Bad_Name"}], "name must match"),
        ([{**HTTP, "url_workstation": ""}], "needs a url_workstation"),
        ([{**STDIO, "command": ""}], "needs a command"),
        ([{**STDIO, "transport": "sse"}], "transport must be http or stdio"),
        ([{**HTTP, "instances": "x"}], "instances must be a list"),
        ([{**HTTP, "instances": [WORK]}], "takes its servers from the overlay"),
        ([{**STDIO, "args": "--fast"}], "args must be a list"),
        ([{**STDIO, "env": ["A"]}], "env must be a mapping"),
    ],
)
def test_a_malformed_bound_server_is_refused_before_anything_is_written(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
    capsys: pytest.CaptureFixture[str],
    servers: list[dict],
    expected: str,
) -> None:
    run = Run(tmp_path, monkeypatch, servers, settings={"agents": {}})
    before = run.settings.read_bytes()
    assert run.go() == 1
    assert expected in capsys.readouterr().out
    assert run.settings.read_bytes() == before


def test_unreadable_registry_servers_cannot_converge(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    run = Run(tmp_path, monkeypatch, [HTTP], settings={})
    data = yaml.safe_load(run.registry.read_text())
    data["mcp_servers"] = "nope"
    run.registry.write_text(yaml.safe_dump(data))
    assert run.go() == 3
    assert "mcp_servers must be a list of mappings" in capsys.readouterr().out
    run.settings.write_text(json.dumps({"contextServers": {}}))
    run.registry.write_text(yaml.safe_dump(_registry([HTTP])))
    assert run.go() == 3
    assert "contextServers is not a list" in capsys.readouterr().out


def test_the_real_registry_gives_the_personal_instance_ten_servers_and_the_work_instance_none(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    run = Run(tmp_path, monkeypatch, [], settings={})
    real = _real()
    run.registry.write_text(yaml.safe_dump({**real, "gold_band": {"agents": []}}))
    assert run.go() == 0
    assert sorted(s["id"] for s in run.servers()) == sorted(WAVE_6)
    credentialed = {
        s["name"]: s["credential"]
        for s in real["mcp_servers"]
        if s.get("instances") and s.get("credential") and s["transport"] == "http"
    }
    assert set(credentialed) == {"memory-api", "github", "grimoire", "outline"}
    for server in run.servers():
        assert server.get("headers", {}) == {} and "Bearer" not in json.dumps(server)
        if server["id"] in credentialed:
            # The secret is only ever named, as the proxy's --secret argument.
            assert server["command"] == "/bin/sh"
            assert server["args"][1].endswith(f"--secret {credentialed[server['id']]}")
    run.settings.write_text(json.dumps({"contextServers": run.servers()}))
    assert run.go(WORK) == 0
    assert json.loads(run.settings.read_text())["contextServers"] == []
