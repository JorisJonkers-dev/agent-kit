"""code-nav's work roots come from the work overlay, on a work machine only.

The roots here are placeholders in a temp HOME: no real work path or name is
used anywhere. The overlay rules live in the gold-band-registry crate, so the
unit tests drive the real crate (skipped on a machine with neither cargo nor
GOLD_BAND_REGISTRY_BIN); the setup tests run the checked-in script with a stub
crate binary and the real helper.
"""

from __future__ import annotations

import os
import shutil
import sys
from pathlib import Path
from typing import Any

import pytest
import yaml

KIT_ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(KIT_ROOT / "scripts"))
sys.path.insert(0, str(Path(__file__).resolve().parent))

import gold_band_instances as gbi  # noqa: E402
import render_registry  # noqa: E402
from setup_harness import setup_run  # noqa: E402

needs_crate = pytest.mark.skipif(
    shutil.which("cargo") is None and not os.environ.get(gbi.BIN_ENV),
    reason="neither cargo nor GOLD_BAND_REGISTRY_BIN is available",
)

PERSONAL = "~/Projects/JorisJonkers-dev"
WORK = "~/Projects/fixture-work-root"


def _registry() -> dict[str, Any]:
    return yaml.safe_load(render_registry.REGISTRY_PATH.read_text())  # type: ignore[no-any-return]


def _overlay(tmp_path: Path, mcp_servers: str, instance: str = "gold-band") -> Path:
    path = tmp_path / "overlay.yaml"
    path.write_text(f"instances:\n  {instance}:\n    mcp_servers:\n      {mcp_servers}\n")
    return path


@needs_crate
def test_the_work_overlay_adds_its_roots_beyond_the_registrys(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("HOME", str(tmp_path))
    overlay = _overlay(
        tmp_path,
        f'code-nav: {{command: code-nav, args: ["--root", "{WORK}", "--root", "{PERSONAL}", "--root", "{WORK}"]}}',
    )

    assert gbi.code_nav_roots(_registry(), overlay) == [f"{tmp_path}/Projects/fixture-work-root"]


def test_no_overlay_and_no_code_nav_entry_add_nothing(tmp_path: Path) -> None:
    assert gbi.code_nav_roots(_registry(), tmp_path / "missing.yaml") == []


@needs_crate
def test_an_overlay_without_a_code_nav_entry_adds_nothing(tmp_path: Path) -> None:
    overlay = _overlay(tmp_path, "jira: {command: npx, args: [-y, jira]}")

    assert gbi.code_nav_roots(_registry(), overlay) == []


@needs_crate
def test_the_personal_instance_never_takes_mcp_servers_from_the_overlay(tmp_path: Path) -> None:
    overlay = _overlay(tmp_path, f'code-nav: {{command: code-nav, args: ["--root", "{WORK}"]}}', "gold-band-personal")

    with pytest.raises(gbi.InstanceError, match="work instance gold-band only"):
        gbi.code_nav_roots(_registry(), overlay)


@needs_crate
@pytest.mark.parametrize(
    "entry",
    [
        "code-nav: {command: npx, args: [--root, /x]}",
        'code-nav: {command: code-nav, args: ["--root"]}',
        'code-nav: {command: code-nav, args: ["--allow-write", "/x"]}',
        'code-nav: {command: code-nav, args: ["--root", ""]}',
        'code-nav: {command: code-nav, args: ["--root", "--allow-write"]}',
        "code-nav: {command: code-nav}",
        "code-nav: placeholder",
    ],
)
def test_a_code_nav_entry_that_is_not_root_pairs_is_refused(tmp_path: Path, entry: str) -> None:
    with pytest.raises(gbi.InstanceError, match=r"only --root PATH pairs|unknown field|must be"):
        gbi.code_nav_roots(_registry(), _overlay(tmp_path, entry))


def test_the_public_registry_holds_the_personal_root_alone() -> None:
    (server,) = [s for s in _registry()["mcp_servers"] if s["name"] == "code-nav"]

    assert server["args"] == ["--root", PERSONAL]
    assert server["overlay_roots"] is True
    assert "fixture-work-root" not in render_registry.render_setup_script(_registry())


def test_overlay_roots_on_an_http_server_are_refused() -> None:
    data = _registry()
    server = next(s for s in data["mcp_servers"] if s["name"] == "memory-api")
    server["overlay_roots"] = True

    with pytest.raises(render_registry.RegistryError, match="overlay_roots only applies to a stdio server"):
        render_registry.render_setup_script(data)


def _prepare(overlay: str | None):
    def prepare(home: Path) -> None:
        if overlay is not None:
            path = home / ".config" / "agent-kit" / "overlay.yaml"
            path.parent.mkdir(parents=True)
            path.write_text(overlay)

    return prepare


# Real helper, real overlay parsing; only the crate binary is a stub, since the
# setup PATH holds no cargo. `uv run ... python SCRIPT args` runs the helper.
_UV = (
    f'echo "uv $*" >> "$CALLS"\n'
    'echo "warning: noise on stderr" >&2\n'
    'while [ $# -gt 0 ] && [ "${1%gold_band_instances.py}" = "$1" ]; do shift; done\n'
    f'[ $# -gt 0 ] && exec "{sys.executable}" "$@"\n'
    "exit 0"
)


def _run(tmp_path: Path, overlay: str | None, crate: str = "exit 0"):
    binary = tmp_path / "crate-stub"
    binary.write_text(f"#!/bin/sh\n{crate}\n")
    binary.chmod(0o755)
    calls = tmp_path / "calls.log"
    return setup_run(
        tmp_path / "run",
        "Linux",
        prepare_home=_prepare(overlay),
        stubs={"uv": _UV.replace("$CALLS", str(calls))},
        extra_env={gbi.BIN_ENV: str(binary), "PYTHONPATH": ""},
    )


def _code_nav_adds(calls: list[str]) -> list[str]:
    return [c for c in calls if " mcp add " in c and "code-nav" in c and "code-nav --root" in c]


WORK_OVERLAY = (
    f'instances:\n  gold-band:\n    mcp_servers:\n      code-nav: {{command: code-nav, args: ["--root", "{WORK}"]}}\n'
)


def test_a_machine_without_an_overlay_registers_the_personal_root_alone(tmp_path: Path) -> None:
    run = _run(tmp_path, None)

    adds = _code_nav_adds(run.calls)
    assert {a.split()[0] for a in adds} == {"claude", "codex"}
    assert all(add.endswith(f"code-nav --root {run.home}/Projects/JorisJonkers-dev") for add in adds), adds
    assert not any("gold_band_instances.py" in call for call in run.calls)


def test_a_work_overlay_adds_its_roots_to_the_claude_and_codex_registrations(tmp_path: Path) -> None:
    run = _run(tmp_path, WORK_OVERLAY)

    adds = _code_nav_adds(run.calls)
    assert {a.split()[0] for a in adds} == {"claude", "codex"}
    for add in adds:
        assert add.endswith(
            f"code-nav --root {run.home}/Projects/JorisJonkers-dev --root {run.home}/Projects/fixture-work-root"
        ), add
    assert "code-nav: 1 work overlay root(s) applied" in run.stdout


def test_check_mode_registers_nothing_but_still_reads_the_overlay(tmp_path: Path) -> None:
    binary = tmp_path / "crate-stub"
    binary.write_text("#!/bin/sh\nexit 0\n")
    binary.chmod(0o755)
    run = setup_run(
        tmp_path / "run",
        "Linux",
        args=("--check",),
        prepare_home=_prepare(WORK_OVERLAY),
        stubs={"uv": _UV.replace("$CALLS", str(tmp_path / "calls.log"))},
        extra_env={gbi.BIN_ENV: str(binary)},
    )

    assert _code_nav_adds(run.calls) == []
    assert any("gold_band_instances.py code-nav-roots" in call for call in run.calls)
    assert "would run: claude mcp add code-nav" in run.stdout


def test_a_refused_overlay_leaves_the_personal_root_alone_and_says_why(tmp_path: Path) -> None:
    run = _run(tmp_path, WORK_OVERLAY, crate='echo "overlay gold-band: refused" >&2; exit 1')

    adds = _code_nav_adds(run.calls)
    assert {a.split()[0] for a in adds} == {"claude", "codex"}
    assert all(add.endswith(f"code-nav --root {run.home}/Projects/JorisJonkers-dev") for add in adds), adds
    assert "code-nav: the work overlay was not applied" in run.stdout + run.stderr


@needs_crate
def test_the_cli_prints_the_roots_and_refuses_with_a_status(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    monkeypatch.setenv("HOME", str(tmp_path))
    good = _overlay(tmp_path, f'code-nav: {{command: code-nav, args: ["--root", "{WORK}"]}}')

    assert gbi.main(["code-nav-roots", "--overlay", str(good)]) == 0
    assert capsys.readouterr().out == f"{tmp_path}/Projects/fixture-work-root\n"

    bad = _overlay(tmp_path, f'code-nav: {{command: code-nav, args: ["--root", "{WORK}"]}}', "gold-band-personal")
    assert gbi.main(["code-nav-roots", "--overlay", str(bad)]) == 1
    assert "code-nav roots:" in capsys.readouterr().err
    assert gbi.main(["code-nav-roots", "--registry", str(tmp_path / "none.yaml")]) == 1
