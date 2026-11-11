"""Pi installs pinned, scripts off, behind a 7-day release cutoff (issue #337).

The registry records Pi under ``clis:`` with ``pinned_npm:`` (the owner's
exact pins) and ``throwaway_only: true``, because Pi has no permission
prompts and may only run inside a throwaway repository. Setup generates the
whole step from that block: one ``npm install -g --ignore-scripts
--before=<cutoff>`` line for both pins (the cutoff is computed at setup time
as now minus 7 days, UTC), a publish-time age check per pin that fails the
step and installs nothing when a pin is younger than the cutoff, and the
installed binary's absolute path printed at the end -- the command the Gold
Band declaration placeholder in the registry names. Each harness case runs
the whole checked-in setup script against a temporary home with a stubbed
npm (tests/setup_harness.py).
"""

from __future__ import annotations

import copy
import json
import re
import sys
from datetime import UTC, datetime, timedelta
from pathlib import Path
from typing import Any

import pytest

KIT_ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(KIT_ROOT / "scripts"))
sys.path.insert(0, str(Path(__file__).resolve().parent))

import render_registry  # noqa: E402
import yaml  # noqa: E402
from setup_harness import setup_run  # noqa: E402

PINNED_PACKAGES = [("@earendil-works/pi-coding-agent", "1.5.0"), ("pi-acp", "0.0.34")]
PINS = [f"{package}@{version}" for package, version in PINNED_PACKAGES]
INSTALL = "npm install -g --ignore-scripts"
EXPECTED_PATH = "npm-prefix/bin/pi-acp"


def _registry() -> dict[str, Any]:
    return yaml.safe_load((KIT_ROOT / "registry" / "estate-tooling.yaml").read_text())


def _pi(data: dict[str, Any] | None = None) -> dict[str, Any]:
    return next(cli for cli in (data or _registry())["clis"] if cli["name"] == "pi")


def _iso(days_old: int) -> str:
    stamp = datetime.now(UTC) - timedelta(days=days_old)
    return stamp.strftime("%Y-%m-%dT%H:%M:%S.000Z")


def _times(ages: list[int]) -> dict[str, dict[str, str]]:
    """Each pin's publish time, as `npm view <spec> time --json` answers it."""
    return {
        pin: {version: _iso(age)}
        for (_package, version), pin, age in zip(PINNED_PACKAGES, PINS, ages, strict=True)
    }


# The stub's Python: `ls -g` answers from a state file under HOME, `view
# <spec> time --json` answers from FAKE_NPM_TIMES, `prefix -g` names a
# directory under HOME, and `install -g` records its non-flag arguments as
# installed. FAKE_NPM runs it with the test run's own interpreter.
NPM_STUB_PY = """
import json, os, sys

args = sys.argv[1:]
home = os.environ["HOME"]
state = os.path.join(home, "npm-installed")
installed = open(state).read().split() if os.path.exists(state) else []
open(os.environ["FAKE_NPM_LOG"], "a").write("npm " + " ".join(args) + "\\n")
if args[:2] == ["ls", "-g"]:
    sys.exit(0 if args[-1] in installed else 1)
if args[:2] == ["prefix", "-g"]:
    print(os.path.join(home, "npm-prefix"))
elif args[0] == "view":
    print(json.dumps(json.loads(os.environ["FAKE_NPM_TIMES"]).get(args[1], {})))
elif args[:2] == ["install", "-g"]:
    for arg in args[2:]:
        if not arg.startswith("-"):
            open(state, "a").write(arg + "\\n")
"""
FAKE_NPM = f'exec "{sys.executable}" - "$@" <<\'PY\'\n' + NPM_STUB_PY + "PY"


def _pi_run(
    root: Path,
    times: dict[str, dict[str, str]],
    *,
    installed: list[str] | None = None,
):
    def prepare(home: Path) -> None:
        if installed is not None:
            (home / "npm-installed").write_text("\n".join(installed) + "\n")

    return setup_run(
        root,
        "Linux",
        prepare_home=prepare,
        stubs={"npm": FAKE_NPM},
        extra_env={
            "FAKE_NPM_TIMES": json.dumps(times),
            "FAKE_NPM_LOG": str(root / "calls.log"),
        },
    )


def _pi_installs(calls: list[str]) -> list[str]:
    return [call for call in calls if call.startswith(INSTALL)]


# --- through setup, against a stubbed npm ---


def test_setup_installs_both_pins_scripts_off_behind_a_cutoff_seven_days_back(tmp_path: Path) -> None:
    # PIN_CUTOFF_PY prints whole seconds (npm's publish-time format), so the
    # comparison runs on whole seconds too: a `started` that kept its
    # microseconds would fail the lower bound whenever the run reaches the
    # Pi block inside the second it started, because the printed cutoff is
    # that second floored while `started` never went below its fraction.
    started = datetime.now(UTC).replace(microsecond=0)
    run = _pi_run(tmp_path, _times([30, 30]))
    installs = _pi_installs(run.calls)
    assert len(installs) == 1, run.calls
    line = installs[0]
    assert PINS[0] in line and PINS[1] in line
    found = re.search(r"--before=(\S+)", line)
    assert found, line
    cutoff = datetime.fromisoformat(found.group(1).replace("Z", "+00:00"))
    # Computed at setup time: now minus 7 days, UTC, so nothing published in
    # the last 7 days can resolve while npm honours it.
    assert started - timedelta(days=7) <= cutoff <= datetime.now(UTC) - timedelta(days=7)
    assert "pi: installed at its pins" in run.stdout
    assert f"pi: pi-acp -> {run.home}/{EXPECTED_PATH}" in run.stdout


def test_publish_times_are_checked_before_anything_installs(tmp_path: Path) -> None:
    run = _pi_run(tmp_path, _times([30, 30]))
    views = [call for call in run.calls if call.startswith("npm view") and "time --json" in call]
    assert views == [f"npm view {pin} time --json" for pin in PINS]
    assert run.calls.index(views[0]) < run.calls.index(_pi_installs(run.calls)[0])


def test_a_pin_published_two_days_ago_fails_the_step_and_installs_nothing(tmp_path: Path) -> None:
    run = _pi_run(tmp_path, _times([30, 2]))
    assert run.returncode != 0
    # The old pin passed its check; the young one refused the whole step.
    assert f"pi: {PINS[0]} was published" in run.stdout
    assert f"FAIL  pi: {PINS[1]} was published" in run.stderr
    assert "inside the 7-day release cutoff; nothing installed" in run.stderr
    assert _pi_installs(run.calls) == []
    state = run.home / "npm-installed"
    installed = state.read_text() if state.exists() else ""
    assert not any(pin in installed for pin in PINS)


def test_a_publish_time_that_cannot_be_proven_fails_closed(tmp_path: Path) -> None:
    times = _times([30, 30])
    del times[PINS[1]]
    run = _pi_run(tmp_path, times)
    assert f"FAIL  pi: {PINS[1]}" in run.stderr
    assert "<no time proven>" in run.stderr
    assert _pi_installs(run.calls) == []


def test_pins_already_installed_leave_the_path_still_printed(tmp_path: Path) -> None:
    run = _pi_run(tmp_path, _times([30, 30]), installed=PINS)
    assert _pi_installs(run.calls) == []
    assert "pi: pinned packages already installed" in run.stdout
    assert f"pi: pi-acp -> {run.home}/{EXPECTED_PATH}" in run.stdout


# --- the registry record and the renderer's rules ---


def test_the_registry_records_pi_pinned_and_throwaway_only() -> None:
    pi = _pi()
    assert pi["binary"] == "pi-acp"
    assert pi["surfaces"] == ["workstation"]
    assert pi["throwaway_only"] is True
    assert [f"{p['package']}@{p['version']}" for p in pi["pinned_npm"]["packages"]] == PINS


def test_the_pi_entry_declares_no_install_command_of_its_own() -> None:
    """The whole step is generated from the pin block, and nothing floats."""
    assert "install" not in _pi()


def test_a_pi_entry_without_throwaway_only_is_refused() -> None:
    data = copy.deepcopy(_registry())
    del _pi(data)["throwaway_only"]
    with pytest.raises(render_registry.RegistryError, match="throwaway_only"):
        render_registry.validate(data)


def test_a_pinned_npm_entry_must_list_its_packages() -> None:
    data = copy.deepcopy(_registry())
    _pi(data)["pinned_npm"]["packages"] = []
    with pytest.raises(render_registry.RegistryError, match="must list its pinned packages"):
        render_registry.validate(data)


def test_a_pinned_npm_entry_must_pin_exact_versions() -> None:
    data = copy.deepcopy(_registry())
    _pi(data)["pinned_npm"]["packages"][0]["version"] = "latest"
    with pytest.raises(render_registry.RegistryError, match="pin an exact version"):
        render_registry.validate(data)


@pytest.mark.parametrize("bad", ["^1.5.0", "1.x", ">=1.0.0", 1.5, "1.5.0.1", "1.5.0-"])
def test_a_pinned_npm_entry_must_pin_an_exact_version_not_a_range(bad: Any) -> None:
    """A range, x-version, float or sloppy pin renders a floating install."""
    data = copy.deepcopy(_registry())
    _pi(data)["pinned_npm"]["packages"][0]["version"] = bad
    with pytest.raises(render_registry.RegistryError, match="pin an exact version"):
        render_registry.validate(data)


def test_a_prerelease_pin_is_still_an_exact_version() -> None:
    data = copy.deepcopy(_registry())
    _pi(data)["pinned_npm"]["packages"][0]["version"] = "1.5.0-beta.1"
    render_registry.validate(data)


def test_the_rendered_step_installs_with_scripts_off_behind_the_cutoff() -> None:
    script = render_registry.render_setup_script(_registry())
    assert f'{INSTALL} --before="${{pi_cutoff}}" "{PINS[0]}" "{PINS[1]}"' in script
    assert 'pi_bin="$(npm prefix -g)/bin/pi-acp"' in script


def test_the_gold_band_section_holds_the_pi_declaration_placeholder() -> None:
    """The Pi agent's Gold Band declaration lands with the converge ticket."""
    text = (KIT_ROOT / "registry" / "estate-tooling.yaml").read_text()
    section = text.split("gold_band:", 1)[1].split("\n\n")[0]
    assert "pi-acp" in section
    assert "reach: full" in section
    assert "throwawayOnly: true" in section


def test_regression_a_pinned_npm_value_that_is_not_a_mapping_is_a_registry_error() -> None:
    """A broken `pinned_npm:` fails with the renderer's own error, not a traceback (#337 review)."""
    data = yaml.safe_load((Path(__file__).resolve().parent.parent / "registry" / "estate-tooling.yaml").read_text())
    pi = next(cli for cli in data["clis"] if cli.get("name") == "pi")
    pi["pinned_npm"] = "oops"
    with pytest.raises(render_registry.RegistryError):
        render_registry.validate(data)
