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
        pytest.skip("Rust toolchain (cargo) is absent: gold-band-agents cannot be built and run")
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


def _gate(root: Path, running: str = "0") -> dict[str, str]:
    """The env of an installed-or-missing app and a not-running-or-running Gold Band.

    The app the gate looks for points at a stub under ``root``, created
    here: the gate checks existence only.
    """
    (root / "Gold Band.app").mkdir(exist_ok=True)
    return {"AGENT_KIT_GOLD_BAND_APP": str(root / "Gold Band.app"),
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
