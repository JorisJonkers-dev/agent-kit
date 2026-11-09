"""The Gold Band agent map: setup converges settings.json onto the registry (#333).

The registry declares Gold Band agents and a setup run makes the ``agents``
map of ``~/.gold-band/settings.json`` match them: each declared agent is
written with only the fields Gold Band knows, a built-in id without
``adapter.command``/``adapter.args`` (the app strips those two when
it saves and re-injects them from its catalog when it loads, #328, so writing
them would make the app's own save look like drift to the next run); an
existing agent survives only when the keep file names it -- never a codex id,
never a retired one -- and every other top-level key of the document is
preserved with its order. The settings map REPLACES the app's runtime agent
list; it is not merged with the built-in catalog. A second run changes
nothing: the crate reports ``settings unchanged`` and the file stays
byte-identical. Setup runs the step only while Gold Band is installed
(``/Applications/Gold Band.app``, or the test override
``AGENT_KIT_GOLD_BAND_APP``), never under a running app (the
``AGENT_KIT_GOLD_BAND_RUNNING`` override answers that check in tests);
a ``--check`` run writes nothing, so it is never refused for a running app.

Safe writes are #334: an app that starts mid-run is caught too, because the
crate re-checks a guard command (``AGENT_KIT_GOLD_BAND_GUARD`` overrides it
for these tests) immediately before its rename and replaces nothing -- the
step reports each group it did not write (``not written: settings (Gold Band
started)``) and a second run finishes. The data directory comes out
owner-only (0700, the settings file 0600), the report shows the installed
version and build commit from the app's Info.plist, and no key but ``agents``
and no file but settings.json (plus its one backup) is ever added or changed.

Each case runs the whole checked-in setup script against a temporary home
(tests/setup_harness.py), with a ``uv`` stub that runs converge.py for
real against the registry the test names, so the crate's own converge -- not
the stub's -- decides the settings the way it does on a workstation. The
crate builds the way the canary tests build it, and the run skips with
setup's own reason where no Rust toolchain is there.
"""

from __future__ import annotations

import argparse
import json
import plistlib
import shutil
import subprocess
import sys
from pathlib import Path
from typing import Any

import pytest
import yaml

KIT_ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(KIT_ROOT / "scripts"))
sys.path.insert(0, str(Path(__file__).resolve().parent))

import converge  # noqa: E402
from setup_harness import setup_run  # noqa: E402


@pytest.fixture(scope="module")
def built_crate() -> None:
    """Build the workspace's crates, the way setup's own skip line asks for (docs/RUST.md)."""
    if shutil.which("cargo") is None:
        pytest.skip("Rust toolchain (cargo) is absent: gold-band-registry cannot be built and run")
    build = subprocess.run(
        ["cargo", "build", "--release", "--manifest-path", str(KIT_ROOT / "scripts" / "Cargo.toml")],
        capture_output=True,
        text=True,
        check=False,
    )
    assert build.returncode == 0, build.stderr


# The settings a machine could hold before any converge: extra top-level
# keys, an undeclared agent, a kept one, a retired one and a codex one, and
# the app's own injected command inside the built-in's entry (#328).
SEEDED_SETTINGS: dict[str, Any] = {
    "theme": "dark",
    "agents": {
        "stray-agent": {"adapter": {"command": "/usr/local/bin/stray"}},
        "mine": {"adapter": {"displayName": "Mine"}, "icon": "sparkles"},
        "old-friend": {"adapter": {"displayName": "An Old Friend"}},
        "codex-acp": {"adapter": {"displayName": "Codex"}},
        "opencode": {
            "adapter": {"command": "/usr/local/bin/opencode", "displayName": "OpenCode"},
            "icon": "opencode",
            "primaryAgentDir": ".opencode",
            "compatibleAgentDirs": [".agents"],
            "systemPromptDelivery": "none",
            "externalSessionSyncSupported": False,
            "externalSessionSyncEnabled": False,
        },
    },
    "window": {"width": 1280},
}

# What the registry's declared OpenCode agent becomes in settings.json: the
# Gold Band fields only, typed, and no command or args for the built-in id.
OPENCODE_SETTINGS: dict[str, Any] = {
    "adapter": {"displayName": "OpenCode"},
    "icon": "opencode",
    "primaryAgentDir": ".opencode",
    "compatibleAgentDirs": [".agents"],
    "systemPromptDelivery": "none",
    "externalSessionSyncSupported": False,
    "externalSessionSyncEnabled": False,
}

# The registry's declared agents, in declaration order (#338). Their
# adapter commands are declared with a leading `~/` and written absolute,
# expanded to the home of the run, because Gold Band reads settings.json
# without shell expansion; no declaration carries a key.
DECLARED_AGENT_IDS = ["opencode", "vibe", "vibe-study", "agy", "hermes"]


def _declared_adapters(home: Path) -> dict[str, Any]:
    """Each declared agent's adapter block as converge writes it: home-expanded."""
    return {
        "opencode": OPENCODE_SETTINGS["adapter"],
        "vibe": {"command": f"{home}/.local/bin/vibe-acp", "displayName": "Vibe"},
        "vibe-study": {"command": f"{home}/.local/bin/vibe-study-acp", "displayName": "Vibe (Study)"},
        "agy": {"command": "/opt/homebrew/bin/agy", "args": ["acp"], "displayName": "Antigravity"},
        "hermes": {"command": f"{home}/.local/bin/hermes-acp", "displayName": "Hermes"},
    }


def _seeded_settings(home: Path, keep: tuple[str, ...] = ("mine", "codex-acp")) -> None:
    """A settings.json with extra keys and undeclared, kept, retired and codex agents."""
    settings = home / ".gold-band" / "settings.json"
    settings.parent.mkdir(parents=True)
    settings.write_text(json.dumps(SEEDED_SETTINGS, indent=2) + "\n")
    agent_kit = home / ".config" / "agent-kit"
    agent_kit.mkdir(parents=True)
    (agent_kit / "keep.yaml").write_text("keep:\n" + "".join(f"  - {name}\n" for name in keep))


# The updater, metrics, feedback, chat-connector and task-board keys of
# settings.json, seeded off or pointed at dead endpoints: setup never
# enables or changes any of them, because no key other than ``agents`` is
# ever added or changed (#334).
GOLD_BAND_UNTOUCHED: dict[str, Any] = {
    "desktopUpdaterUrlOverride": "https://invalid.invalid/latest.json",
    "desktopMetricsEnabled": False,
    "desktopMetricsBaseUrl": "https://invalid.invalid/metrics",
    "desktopMetricsApiKey": "${DESKTOP_METRICS_API_KEY}",
    "desktopFeedbackEnabled": False,
    "imIntegrations": {"wecom": {"enabled": False}, "feishu": {"enabled": False}},
    "desktopMulticaEnabled": False,
    "desktopMulticaServerUrl": "https://invalid.invalid/multica",
    "desktopMulticaApiKey": "${DESKTOP_MULTICA_API_KEY}",
}

# The settings the hardened seeding writes: the keys setup must leave alone
# joined onto the seeded map itself.
GOLD_BAND_SEEDED_DOCUMENT = {**GOLD_BAND_UNTOUCHED, **SEEDED_SETTINGS}

# The app's own files under ~/.gold-band besides settings.json -- its
# workflows, templates, preferences and databases -- which setup never
# writes and never changes (#334).
GOLD_BAND_NEIGHBOURS: dict[str, bytes] = {
    "workflows.json": b'{"version": "0.1", "templates": []}\n',
    "context/auto-templates.json": b'{"templates": []}\n',
    "context/profiles/grill.md": b"---\nid: pf-builtin-grill\n---\nThe relentless one.\n",
    "desktop/agent-model-catalogs.json": b"{}\n",
    "prefs.json": b'{"browser": {"searchEngine": "baidu"}}\n',
    "core.db": b"SQLite format 3\x00",
    "logs/runtime.log": b"started\n",
}


def _seeded_hardened_settings(home: Path) -> None:
    """The seeded settings.json plus the keys setup must leave alone, and
    the app's own workflows, templates, preferences and databases beside it."""
    _seeded_settings(home)
    data_dir = home / ".gold-band"
    (data_dir / "settings.json").write_text(json.dumps(GOLD_BAND_SEEDED_DOCUMENT, indent=2) + "\n")
    for name, body in GOLD_BAND_NEIGHBOURS.items():
        path = data_dir / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(body)


def _uv_stub(root: Path, registry: Path | None = None) -> str:
    """A uv stub that runs converge.py for real for both Gold Band steps.

    Every other call is recorded like the harness's default stub. ``--registry``
    appears twice when the test overrides it; argparse takes the last one, so
    the test's registry decides the run.
    """
    extra = f" --registry \"{registry}\"" if registry else ""
    return (
        f'echo "uv $*" >> "{root / "calls.log"}"\n'
        'while [ "$#" -gt 0 ] && [ "$1" != gold-band ] && [ "$1" != gold-band-validate ]; do shift; done\n'
        'if [ "$#" = 0 ]; then\n'
        "  exit 0\n"
        "fi\n"
        'sub="$1"\n'
        "shift\n"
        f'exec "{sys.executable}" "{KIT_ROOT / "scripts" / "converge.py"}" "$sub" "$@"{extra}\n'
    )


def _gate(root: Path, running: str = "0", plist: dict[str, Any] | None = None) -> dict[str, str]:
    """The env of an installed-or-missing app and a not-running-or-running Gold Band.

    The app the gate looks for points at a stub under ``root``, created
    here: the gate checks existence only. ``plist`` stamps the stub's
    ``Contents/Info.plist``, so a run's build report names the version and
    commit the test chooses, the app path being the overridable one (#334).
    """
    app = root / "Gold Band.app"
    if plist is None:
        app.mkdir(parents=True, exist_ok=True)
    else:
        contents = app / "Contents"
        contents.mkdir(parents=True, exist_ok=True)
        with (contents / "Info.plist").open("wb") as handle:
            plistlib.dump(plist, handle)
    return {"AGENT_KIT_GOLD_BAND_APP": str(app),
            "AGENT_KIT_GOLD_BAND_RUNNING": running}


def test_a_first_run_holds_the_declared_agents_and_the_kept_agents(tmp_path: Path, built_crate: None) -> None:
    run = setup_run(tmp_path, "Linux", prepare_home=_seeded_settings,
                    stubs={"uv": _uv_stub(tmp_path)}, extra_env=_gate(tmp_path))
    settings = run.home / ".gold-band" / "settings.json"
    document = json.loads(settings.read_text())
    # Every other key is unchanged, in its order.
    assert list(document) == ["theme", "agents", "window"]
    assert document["theme"] == "dark"
    assert document["window"] == {"width": 1280}
    # The five declared agents in declaration order, then the kept one.
    assert list(document["agents"]) == [*DECLARED_AGENT_IDS, "mine"]
    adapters = _declared_adapters(run.home)
    for agent_id in DECLARED_AGENT_IDS:
        assert document["agents"][agent_id]["adapter"] == adapters[agent_id], agent_id
    assert document["agents"]["opencode"] == OPENCODE_SETTINGS
    assert document["agents"]["mine"] == SEEDED_SETTINGS["agents"]["mine"]
    # The old document was backed up before anything was written.
    backup = settings.with_name("settings.json.bak")
    assert json.loads(backup.read_text())["agents"]["stray-agent"]["adapter"]["command"] == (
        "/usr/local/bin/stray"
    )


def test_no_value_in_settings_equals_the_study_key(tmp_path: Path, built_crate: None) -> None:
    """No declared Gold Band agent holds a key: the launcher reads it at start (#336)."""
    stub_key = "study-stub-key-336"

    def seeded_with_a_key(home: Path) -> None:
        """The seeded settings plus a secret store that holds the Study key."""
        _seeded_settings(home)
        env_sh = home / ".config" / "agent-kit" / "env.sh"
        env_sh.write_text(
            '_agent_kit_secret() { [ "$1" = MISTRAL_STUDY_API_KEY ] '
            f"&& printf '%s' \"{stub_key}\"; }}\n"
        )

    run = setup_run(
        tmp_path, "Linux", prepare_home=seeded_with_a_key,
        stubs={"uv": _uv_stub(tmp_path)},
        extra_env={**_gate(tmp_path), "MISTRAL_STUDY_API_KEY": stub_key})
    document = json.loads((run.home / ".gold-band" / "settings.json").read_text())
    assert list(document["agents"]) == [*DECLARED_AGENT_IDS, "mine"]
    # No value, and no name, of the converged document is the stub key.
    assert stub_key not in json.dumps(document)
    assert "MISTRAL_STUDY_API_KEY" not in json.dumps(document)


def test_a_second_run_changes_nothing(tmp_path: Path, built_crate: None) -> None:
    run = setup_run(tmp_path, "Linux", prepare_home=_seeded_settings,
                    stubs={"uv": _uv_stub(tmp_path)}, extra_env=_gate(tmp_path))
    settings = run.home / ".gold-band" / "settings.json"
    backup = settings.with_name("settings.json.bak")
    after_first, backup_first = settings.read_bytes(), backup.read_bytes()
    again = setup_run(tmp_path / "again", "Linux", home=run.home,
                      stubs={"uv": _uv_stub(tmp_path / "again")}, extra_env=_gate(tmp_path))
    assert "settings unchanged" in again.stdout
    assert "unchanged: settings" in again.stdout
    assert "written: settings" not in again.stdout
    assert settings.read_bytes() == after_first
    assert backup.read_bytes() == backup_first


def test_a_codex_id_in_the_keep_file_is_removed_anyway(tmp_path: Path, built_crate: None) -> None:
    run = setup_run(tmp_path, "Linux",
                    prepare_home=lambda home: _seeded_settings(home, keep=("codex-acp",)),
                    stubs={"uv": _uv_stub(tmp_path)}, extra_env=_gate(tmp_path))
    document = json.loads((run.home / ".gold-band" / "settings.json").read_text())
    assert list(document["agents"]) == DECLARED_AGENT_IDS


def test_a_retired_id_is_removed_even_when_kept(tmp_path: Path, built_crate: None) -> None:
    registry = tmp_path / "gold-band-retired.yaml"
    section = yaml.safe_load((KIT_ROOT / "registry" / "estate-tooling.yaml").read_text())["gold_band"]
    registry.write_text(yaml.safe_dump(
        {"gold_band": {**section, "retired": ["old-friend"]}}, sort_keys=False))
    run = setup_run(tmp_path, "Linux",
                    prepare_home=lambda home: _seeded_settings(home, keep=("mine", "old-friend")),
                    stubs={"uv": _uv_stub(tmp_path, registry)}, extra_env=_gate(tmp_path))
    document = json.loads((run.home / ".gold-band" / "settings.json").read_text())
    assert list(document["agents"]) == [*DECLARED_AGENT_IDS, "mine"]
    assert "old-friend" not in document["agents"]


def test_gold_band_not_installed_is_skipped(tmp_path: Path) -> None:
    missing_app = tmp_path / "not-installed"
    run = setup_run(tmp_path, "Linux", prepare_home=_seeded_settings,
                    extra_env={"AGENT_KIT_GOLD_BAND_APP": str(missing_app)})
    assert f"Gold Band is not installed ({missing_app})" in run.stdout
    # Nothing was converged: the seeded map is left exactly as it was.
    settings = run.home / ".gold-band" / "settings.json"
    assert json.loads(settings.read_text()) == SEEDED_SETTINGS


def test_gold_band_not_installed_creates_nothing_and_costs_no_exit_code(tmp_path: Path) -> None:
    """A home with no ~/.gold-band keeps none: the step is skipped whole, and
    the run's exit code is the same as one whose app is installed (#334)."""

    def only_the_keep_file(home: Path) -> None:
        agent_kit = home / ".config" / "agent-kit"
        agent_kit.mkdir(parents=True)
        (agent_kit / "keep.yaml").write_text("keep:\n")

    missing_app = tmp_path / "not-installed"
    bare = setup_run(tmp_path, "Linux", prepare_home=only_the_keep_file,
                     stubs={"uv": _uv_stub(tmp_path)},
                     extra_env={"AGENT_KIT_GOLD_BAND_APP": str(missing_app)})
    assert f"Gold Band is not installed ({missing_app})" in bare.stdout
    assert not (bare.home / ".gold-band").exists()
    installed = setup_run(tmp_path / "installed", "Linux", prepare_home=_seeded_settings,
                          stubs={"uv": _uv_stub(tmp_path / "installed")},
                          extra_env=_gate(tmp_path / "installed"))
    assert bare.returncode == installed.returncode


def test_an_app_starting_mid_run_writes_nothing_and_a_second_run_finishes(
    tmp_path: Path, built_crate: None
) -> None:
    """The guard re-check the crate runs immediately before the rename (#334):
    an app that starts mid-run replaces nothing, and re-running finishes."""
    started = {**_gate(tmp_path), "AGENT_KIT_GOLD_BAND_GUARD": "exit 1"}
    run = setup_run(tmp_path, "Linux", prepare_home=_seeded_settings,
                    stubs={"uv": _uv_stub(tmp_path)}, extra_env=started)
    settings = run.home / ".gold-band" / "settings.json"
    # The guard vetoed the rename: nothing was replaced, and the step reports
    # the group it did not write, its refusal up, so the run stops.
    assert converge.GOLD_BAND_GUARD_REFUSED in run.stdout + run.stderr
    assert "not written: settings (Gold Band started)" in run.stdout
    assert "the Gold Band agent map did not converge" in run.stdout + run.stderr
    assert run.returncode != 0
    assert settings.read_text() == json.dumps(SEEDED_SETTINGS, indent=2) + "\n"
    assert not settings.with_name("settings.json.bak").exists()
    # A second run, with the app quit, finishes the convergence.
    again = setup_run(tmp_path / "again", "Linux", home=run.home,
                      stubs={"uv": _uv_stub(tmp_path / "again")}, extra_env=_gate(tmp_path / "again"))
    assert "written: settings" in again.stdout
    document = json.loads(settings.read_text())
    assert list(document["agents"]) == [*DECLARED_AGENT_IDS, "mine"]


def test_the_report_shows_the_installed_version_and_build_commit(tmp_path: Path, built_crate: None) -> None:
    """The step's report reads the installed Gold Band's Info.plist, with the
    app path overridable for tests, and says unknown when it cannot (#334)."""
    committed = _gate(tmp_path, plist={"CFBundleShortVersionString": "0.19.1",
                                      "GitCommit": "987fc3e298b15d725a7f0e9b609b0f77dba63a74"})
    run = setup_run(tmp_path, "Linux", prepare_home=_seeded_settings,
                    stubs={"uv": _uv_stub(tmp_path)}, extra_env=committed)
    assert "Gold Band 0.19.1 (commit 987fc3e298b15d725a7f0e9b609b0f77dba63a74)" in run.stdout
    # A plist without a commit stamped on it says so.
    uncommitted = _gate(tmp_path / "again", plist={"CFBundleShortVersionString": "0.20.0"})
    run = setup_run(tmp_path / "again", "Linux", home=run.home,
                    stubs={"uv": _uv_stub(tmp_path / "again")}, extra_env=uncommitted)
    assert "Gold Band 0.20.0 (commit unknown)" in run.stdout
    # An app with no plist at all cannot name either.
    bare = _gate(tmp_path / "third")
    run = setup_run(tmp_path / "third", "Linux", prepare_home=_seeded_settings,
                    stubs={"uv": _uv_stub(tmp_path / "third")}, extra_env=bare)
    assert "Gold Band unknown (commit unknown)" in run.stdout


def test_the_data_directory_is_owner_only_after_a_write(tmp_path: Path, built_crate: None) -> None:
    """After a write ~/.gold-band is 0700 and settings.json 0600 (#334)."""
    run = setup_run(tmp_path, "Linux", prepare_home=_seeded_settings,
                    stubs={"uv": _uv_stub(tmp_path)}, extra_env=_gate(tmp_path))
    data_dir = run.home / ".gold-band"
    settings = data_dir / "settings.json"
    assert data_dir.stat().st_mode & 0o777 == 0o700
    assert settings.stat().st_mode & 0o777 == 0o600
    assert "owner-only" in run.stdout


def test_no_key_but_agents_and_no_file_but_settings_is_written(tmp_path: Path, built_crate: None) -> None:
    """The updater, metrics, feedback, chat-connector and task-board keys are
    byte-identical after setup, and no file but settings.json (plus its one
    backup) appears or changes under ~/.gold-band (#334)."""
    run = setup_run(tmp_path, "Linux", prepare_home=_seeded_hardened_settings,
                    stubs={"uv": _uv_stub(tmp_path)}, extra_env=_gate(tmp_path))
    data_dir = run.home / ".gold-band"
    document = json.loads((data_dir / "settings.json").read_text())
    # No key added, dropped or reordered; every seeded one byte-identical.
    assert list(document) == list(GOLD_BAND_SEEDED_DOCUMENT)
    for key, value in GOLD_BAND_SEEDED_DOCUMENT.items():
        if key == "agents":
            continue  # converge owns this key
        assert json.dumps(document[key]) == json.dumps(value), key
    assert list(document["agents"]) == [*DECLARED_AGENT_IDS, "mine"]
    # The one backup of the old document, and nothing else beside it.
    backup = data_dir / "settings.json.bak"
    assert backup.read_text() == json.dumps(GOLD_BAND_SEEDED_DOCUMENT, indent=2) + "\n"
    tree = {str(path.relative_to(data_dir)): path.read_bytes()
            for path in sorted(data_dir.rglob("*")) if path.is_file()}
    assert set(tree) == {"settings.json", "settings.json.bak"} | set(GOLD_BAND_NEIGHBOURS)
    for name, body in GOLD_BAND_NEIGHBOURS.items():
        assert tree[name] == body, name


def test_a_running_gold_band_is_never_written_under(tmp_path: Path, built_crate: None) -> None:
    run = setup_run(tmp_path, "Linux", prepare_home=_seeded_settings,
                    stubs={"uv": _uv_stub(tmp_path)}, extra_env=_gate(tmp_path, running="1"))
    settings = run.home / ".gold-band" / "settings.json"
    assert "Gold Band is running" in run.stdout
    assert "quit it and re-run setup" in run.stdout
    assert "the Gold Band agent map did not converge" in run.stdout + run.stderr
    assert json.loads(settings.read_text()) == SEEDED_SETTINGS
    assert not settings.with_name("settings.json.bak").exists()


def test_a_check_run_is_never_refused_for_a_running_gold_band(tmp_path: Path, built_crate: None) -> None:
    """A check run writes nothing, so an open app never refuses it (report only)."""
    run = setup_run(tmp_path, "Linux", args=("--check",), prepare_home=_seeded_settings,
                    stubs={"uv": _uv_stub(tmp_path)}, extra_env=_gate(tmp_path, running="1"))
    settings = run.home / ".gold-band" / "settings.json"
    # The step ran the crate's check over the seeded map instead of refusing.
    assert "Gold Band is running" not in run.stdout
    assert "the Gold Band agent map did not converge" not in run.stdout + run.stderr
    assert "removed stray-agent" in run.stdout
    # A check writes nothing: the seeded map and no backup of it.
    assert json.loads(settings.read_text()) == SEEDED_SETTINGS
    assert not settings.with_name("settings.json.bak").exists()


def test_the_running_check_reads_the_override_env(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv(converge.GOLD_BAND_RUNNING_ENV, "1")
    assert converge._gold_band_running() is True
    monkeypatch.setenv(converge.GOLD_BAND_RUNNING_ENV, "0")
    monkeypatch.setenv("AGENT_KIT_GOLD_BAND_APP", "/nonexistent/test/Gold Band.app")
    assert converge._gold_band_running() is False


def test_the_build_report_reads_the_plist_or_says_unknown(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    """(version, commit) straight from the app's Info.plist: the version key,
    the commit from any key whose name names one, unknown whenever the plist
    cannot be read (#334)."""
    monkeypatch.setenv(converge.GOLD_BAND_APP_ENV, str(tmp_path / "nothing-here"))
    assert converge._gold_band_build() == ("unknown", "unknown")
    app = tmp_path / "Gold Band.app"
    contents = app / "Contents"
    contents.mkdir(parents=True)
    monkeypatch.setenv(converge.GOLD_BAND_APP_ENV, str(app))
    with (contents / "Info.plist").open("wb") as handle:
        handle.write(b"not a plist at all")
    assert converge._gold_band_build() == ("unknown", "unknown")
    with (contents / "Info.plist").open("wb") as handle:
        plistlib.dump({"CFBundleShortVersionString": "0.19.1"}, handle)
    assert converge._gold_band_build() == ("0.19.1", "unknown")
    with (contents / "Info.plist").open("wb") as handle:
        plistlib.dump({"CFBundleShortVersionString": "0.20.0",
                       "BuildCommit": "987fc3e298b15d725a7f0e9b609b0f77dba63a74"}, handle)
    assert converge._gold_band_build() == ("0.20.0", "987fc3e298b15d725a7f0e9b609b0f77dba63a74")


def test_the_guard_command_reuses_this_scripts_own_running_answer(
    monkeypatch: pytest.MonkeyPatch
) -> None:
    """The guard the crate re-checks with is this script's own answer, over
    the same overrides, and a test override stands in for a mid-run start."""
    assert "gold-band-running" in converge._gold_band_guard()
    monkeypatch.setenv(converge.GOLD_BAND_GUARD_ENV, "exit 1")
    assert converge._gold_band_guard() == "exit 1"
    monkeypatch.delenv(converge.GOLD_BAND_GUARD_ENV, raising=False)
    monkeypatch.setenv(converge.GOLD_BAND_RUNNING_ENV, "1")
    assert converge.gold_band_running() == 1
    assert converge.main(["gold-band-running"]) == 1
    monkeypatch.setenv(converge.GOLD_BAND_RUNNING_ENV, "0")
    monkeypatch.setenv(converge.GOLD_BAND_APP_ENV, "/nonexistent/test/Gold Band.app")
    assert converge.main(["gold-band-running"]) == 0


def test_the_owner_only_hardening_reports_a_chmod_it_cannot_do(
    tmp_path: Path, capsys: pytest.CaptureFixture[str]
) -> None:
    """The data directory and the settings file come out owner-only after a
    write, and a chmod that cannot happen is reported (#334)."""
    assert converge._gold_band_owner_only(tmp_path / "no-such-dir" / "settings.json") is False
    assert "cannot make" in capsys.readouterr().out
    settings = tmp_path / "data" / "settings.json"
    settings.parent.mkdir()
    settings.write_text("{}")
    assert converge._gold_band_owner_only(settings) is True
    assert "owner-only" in capsys.readouterr().out
    assert settings.parent.stat().st_mode & 0o777 == 0o700
    assert settings.stat().st_mode & 0o777 == 0o600


def test_a_registry_that_is_not_a_mapping_cannot_converge(
    tmp_path: Path, built_crate: None, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setenv(converge.GOLD_BAND_RUNNING_ENV, "0")
    monkeypatch.setenv("AGENT_KIT_GOLD_BAND_APP", "/nonexistent/test/Gold Band.app")
    registry = tmp_path / "estate-tooling.yaml"
    registry.write_text("- just\n- a\n- list\n")
    args = argparse.Namespace(keep=tmp_path / "keep.yaml", settings=tmp_path / "settings.json",
                              registry=registry, check=True)
    assert converge.gold_band(args, cargo=True) == converge.GOLD_BAND_CANNOT_VALIDATE_EXIT


def test_regression_the_env_cannot_hide_a_running_gold_band(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    """Outside test mode (no fake app path), only pgrep decides "not running" (#333 review)."""
    bin_dir = tmp_path / "bin"
    bin_dir.mkdir()
    pgrep = bin_dir / "pgrep"
    pgrep.write_text("#!/usr/bin/env bash\nexit 0\n")
    pgrep.chmod(0o755)
    monkeypatch.setenv("PATH", f"{bin_dir}:/usr/bin:/bin")
    monkeypatch.setenv("AGENT_KIT_GOLD_BAND_RUNNING", "0")
    monkeypatch.delenv("AGENT_KIT_GOLD_BAND_APP", raising=False)
    assert converge._gold_band_running() is True

