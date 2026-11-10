"""The estate-delivery-status skill: its verdict script, its worked examples, its sources and its install.

`verdict.py` reads the five signals SKILL.md gathers and prints `VERDICT: landed`,
`mixed` or `not landed`, then one line per signal with its link. Landed needs the
compose row clean and `estate/production` success; the Deployment does not gate it.
"""

from __future__ import annotations

import copy
import importlib.util
import json
import re
import runpy
import subprocess
import sys
from pathlib import Path
from types import ModuleType
from typing import Any

import pytest
import yaml
from setup_harness import Run, setup_run

KIT_ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(KIT_ROOT / "scripts"))

import render_registry  # noqa: E402

SKILL = KIT_ROOT / "skills" / "workstation" / "estate-delivery-status"
SCRIPT = SKILL / "scripts" / "verdict.py"
SKILL_MD = SKILL / "SKILL.md"
SOURCES = SKILL / "sources.yaml"
CHECKER = KIT_ROOT / "scripts" / "check_skill_sources.py"

SHA = "a" * 40
OTHER = "b" * 40
EXAMPLES = ("landed", "mixed", "not landed")


def signals(
    compose: str = "clean",
    status: str = "success",
    deployment: str = "success",
    publish: str = "success",
    issues: list[dict[str, str]] | None = None,
    codes: list[str] | None = None,
) -> dict[str, Any]:
    return {
        "project": "home-portal",
        "release": "v1.2.3",
        "sha": SHA,
        "signals": {
            "publish": {"state": publish, "url": "https://example.test/publish/1", "headSha": SHA},
            "compose": {
                "state": compose,
                "url": "https://example.test/compose/2",
                "headSha": SHA,
                "codes": codes or [],
            },
            "status": {"state": status, "url": "https://example.test/status/3", "headSha": SHA},
            "deployment": {"state": deployment, "url": "https://example.test/deploy/4", "headSha": SHA},
            "issues": issues or [],
        },
    }


def _run(data: Any, *args: str, raw: str | None = None) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [sys.executable, "-I", str(SCRIPT), *args],
        input=raw if raw is not None else json.dumps(data),
        capture_output=True,
        text=True,
        check=False,
    )


def _verdict(data: Any) -> str:
    result = _run(data)
    assert result.returncode == 0, result.stderr
    first = result.stdout.splitlines()[0]
    assert first.startswith("VERDICT: ")
    return first.removeprefix("VERDICT: ")


@pytest.fixture
def verdict_module() -> ModuleType:
    spec = importlib.util.spec_from_file_location("estate_verdict", SCRIPT)
    assert spec is not None
    assert spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


# ---------------------------------------------------------------------------
# The verdict
# ---------------------------------------------------------------------------


def test_all_signals_agreeing_is_landed() -> None:
    assert _verdict(signals()) == "landed"


def test_landed_does_not_wait_for_the_deployment() -> None:
    assert _verdict(signals(deployment="failure")) == "landed"
    assert _verdict(signals(deployment="missing")) == "landed"
    assert _verdict(signals(deployment="in_progress")) == "landed"


def test_nothing_good_is_not_landed() -> None:
    data = signals(compose="errors", status="failure", deployment="missing", codes=["E_UNKNOWN_SURFACE"])
    assert _verdict(data) == "not landed"


def test_a_pending_status_with_a_clean_compose_is_mixed_with_every_link() -> None:
    result = _run(signals(status="pending", deployment="success"))
    lines = result.stdout.splitlines()
    assert lines[0] == "VERDICT: mixed"
    assert "status: pending  https://example.test/status/3" in lines
    assert "deployment: success  https://example.test/deploy/4" in lines
    assert "compose: clean  https://example.test/compose/2" in lines
    assert "publish: success  https://example.test/publish/1" in lines


def test_a_failed_status_with_a_clean_compose_and_a_good_deployment_is_mixed() -> None:
    result = _run(signals(status="failure", deployment="success"))
    assert result.stdout.splitlines()[0] == "VERDICT: mixed"
    assert "https://example.test/status/3" in result.stdout


def test_the_opposite_disagreement_is_mixed_too() -> None:
    data = signals(compose="errors", status="success", deployment="success", codes=["E_X", "E_Y"])
    result = _run(data)
    assert result.stdout.splitlines()[0] == "VERDICT: mixed"
    assert "compose: errors  https://example.test/compose/2" in result.stdout
    assert "codes: E_X, E_Y" in result.stdout
    assert "https://example.test/status/3" in result.stdout


def test_only_a_good_deployment_is_mixed() -> None:
    data = signals(compose="errors", status="failure", deployment="success")
    assert _verdict(data) == "mixed"


def test_a_head_sha_mismatch_counts_as_missing() -> None:
    data = signals()
    data["signals"]["status"]["headSha"] = OTHER
    result = _run(data)
    assert result.stdout.splitlines()[0] == "VERDICT: mixed"
    note = f"(counted as missing: headSha {OTHER} differs from {SHA})"
    assert f"status: missing  https://example.test/status/3  {note}" in result.stdout


def test_a_null_or_absent_head_sha_is_not_a_mismatch() -> None:
    data = signals()
    data["signals"]["status"]["headSha"] = None
    del data["signals"]["compose"]["headSha"]
    assert _verdict(data) == "landed"


def test_open_issues_are_listed_with_links_and_do_not_change_the_verdict() -> None:
    issues = [{"title": "home-portal/isolated", "url": "https://example.test/issues/9"}]
    result = _run(signals(issues=issues))
    lines = result.stdout.splitlines()
    assert lines[0] == "VERDICT: landed"
    assert "issues: 1 open" in lines
    assert "  home-portal/isolated  https://example.test/issues/9" in lines


def test_no_issues_says_none_open_and_a_missing_issues_key_is_none() -> None:
    assert "issues: none open" in _run(signals()).stdout
    data = signals()
    del data["signals"]["issues"]
    assert "issues: none open" in _run(data).stdout


def test_a_signal_without_a_url_prints_an_empty_link() -> None:
    data = signals()
    del data["signals"]["status"]["url"]
    assert "status: success  \n" in _run(data).stdout + "\n"


def test_the_signals_can_come_from_a_file(tmp_path: Path) -> None:
    path = tmp_path / "signals.json"
    path.write_text(json.dumps(signals()))
    result = _run(None, "--signals", str(path), raw="")
    assert result.returncode == 0
    assert result.stdout.startswith("VERDICT: landed")


# ---------------------------------------------------------------------------
# Malformed input exits 2
# ---------------------------------------------------------------------------


def _mutated(mutate: Any) -> dict[str, Any]:
    data = copy.deepcopy(signals())
    mutate(data)
    return data


MALFORMED = {
    "not an object": lambda d: ["list"],
    "empty project": lambda d: d.update(project=""),
    "no release": lambda d: d.pop("release"),
    "sha not a string": lambda d: d.update(sha=5),
    "signals not an object": lambda d: d.update(signals=[]),
    "missing signal": lambda d: d["signals"].pop("status"),
    "signal not an object": lambda d: d["signals"].update(deployment="success"),
    "unknown state": lambda d: d["signals"]["status"].update(state="green"),
    "url not a string": lambda d: d["signals"]["status"].update(url=3),
    "head sha not a string": lambda d: d["signals"]["status"].update(headSha=3),
    "codes not a list": lambda d: d["signals"]["compose"].update(codes="E_X"),
    "codes not strings": lambda d: d["signals"]["compose"].update(codes=[1]),
    "issues not a list": lambda d: d["signals"].update(issues="none"),
    "issue without url": lambda d: d["signals"].update(issues=[{"title": "t"}]),
    "issue not an object": lambda d: d["signals"].update(issues=["t"]),
}


@pytest.mark.parametrize("name", MALFORMED)
def test_malformed_input_exits_2_with_a_message_and_no_verdict(name: str) -> None:
    mutate = MALFORMED[name]
    data = copy.deepcopy(signals())
    replaced = mutate(data)
    result = _run(replaced if isinstance(replaced, list) else data)
    assert result.returncode == 2
    assert result.stderr.startswith("verdict.py: malformed input: ")
    assert result.stdout == ""


def test_invalid_json_and_a_missing_file_exit_2(tmp_path: Path) -> None:
    assert _run(None, raw="{nope").returncode == 2
    result = _run(None, "--signals", str(tmp_path / "absent.json"), raw="")
    assert result.returncode == 2
    assert "malformed input" in result.stderr


# ---------------------------------------------------------------------------
# In-process, so line coverage of verdict.py is measurable
# ---------------------------------------------------------------------------


def test_main_runs_in_process(
    verdict_module: ModuleType,
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
    capsys: pytest.CaptureFixture[str],
) -> None:
    good = tmp_path / "good.json"
    good.write_text(json.dumps(signals(issues=[{"title": "t", "url": "u"}])))
    mismatch = signals()
    mismatch["signals"]["deployment"]["headSha"] = OTHER
    mixed = tmp_path / "mixed.json"
    mixed.write_text(json.dumps(mismatch))
    bad = tmp_path / "bad.json"
    bad.write_text("{")
    monkeypatch.setattr(sys, "argv", ["verdict.py", "--signals", str(good)])
    assert verdict_module.main() == 0
    monkeypatch.setattr(sys, "argv", ["verdict.py", "--signals", str(mixed)])
    assert verdict_module.main() == 0
    monkeypatch.setattr(sys, "argv", ["verdict.py", "--signals", str(bad)])
    assert verdict_module.main() == 2
    out = capsys.readouterr().out
    assert "VERDICT: landed" in out
    assert "counted as missing" in out
    monkeypatch.setattr(sys, "argv", ["verdict.py"])
    monkeypatch.setattr(sys, "stdin", __import__("io").StringIO(json.dumps(signals(compose="errors"))))
    assert verdict_module.main() == 0
    assert "VERDICT: mixed" in capsys.readouterr().out


@pytest.mark.parametrize("name", MALFORMED)
def test_evaluate_refuses_malformed_input_in_process(verdict_module: ModuleType, name: str) -> None:
    data = copy.deepcopy(signals())
    replaced = MALFORMED[name](data)
    with pytest.raises(verdict_module.Malformed):
        verdict_module.evaluate(replaced if isinstance(replaced, list) else data)


@pytest.mark.parametrize(
    ("compose", "status", "deployment", "expected"),
    [
        ("clean", "success", "missing", "landed"),
        ("errors", "failure", "missing", "not landed"),
        ("clean", "pending", "missing", "mixed"),
        ("errors", "success", "missing", "mixed"),
        ("errors", "failure", "success", "mixed"),
    ],
)
def test_verdict_rule_in_process(
    verdict_module: ModuleType, compose: str, status: str, deployment: str, expected: str
) -> None:
    states = {"compose": compose, "status": status, "deployment": deployment}
    assert verdict_module.verdict(states) == expected


def test_running_the_script_as_main_exits_with_the_status(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    path = tmp_path / "s.json"
    path.write_text(json.dumps(signals()))
    monkeypatch.setattr(sys, "argv", [str(SCRIPT), "--signals", str(path)])
    with pytest.raises(SystemExit) as raised:
        runpy.run_path(str(SCRIPT), run_name="__main__")
    assert raised.value.code == 0


# ---------------------------------------------------------------------------
# SKILL.md: the verdict rule and the three worked examples
# ---------------------------------------------------------------------------


def missing_examples(text: str) -> list[str]:
    """The worked-example headings that `text` lacks."""
    return [name for name in EXAMPLES if f"### Worked example: {name}\n" not in text]


def example_section(text: str, name: str) -> str:
    heading = f"### Worked example: {name}\n"
    start = text.index(heading)
    rest = text[start + len(heading) :]
    following = re.search(r"^#{1,3} ", rest, re.MULTILINE)
    return rest[: following.start()] if following else rest


def test_skill_md_has_all_three_worked_examples() -> None:
    assert missing_examples(SKILL_MD.read_text()) == []


@pytest.mark.parametrize("name", EXAMPLES)
def test_the_checker_fails_when_a_worked_example_is_removed(name: str) -> None:
    text = SKILL_MD.read_text().replace(f"### Worked example: {name}\n", "")
    assert missing_examples(text) == [name]


def test_skill_md_states_the_verdict_rule_and_that_the_deployment_does_not_gate_landed() -> None:
    text = SKILL_MD.read_text()
    assert "**landed** only when the compose row is clean and `estate/production` is success" in text
    assert "The Deployment does not gate landed" in text
    assert "**not landed** when none of the three is good" in text
    assert "**mixed** otherwise" in text
    assert "headSha" in text
    assert "differs from `sha`" in text


# The fixed values each worked example states, fed to verdict.py.
EXAMPLE_INPUTS = {
    "landed": signals(),
    "mixed": signals(
        status="pending",
        deployment="in_progress",
        issues=[{"title": "home-portal/isolated", "url": "https://example.test/issues/1"}],
    ),
    "not landed": signals(
        compose="errors",
        status="failure",
        deployment="missing",
        codes=["E_UNKNOWN_SURFACE"],
        issues=[{"title": "home-portal/isolated", "url": "https://example.test/issues/1"}],
    ),
}

# Phrases from each example's section that the input above encodes.
EXAMPLE_FACTS = {
    "landed": ["compose `clean`", "status `success`", "deployment `success`", "no open issues"],
    "mixed": ["compose `clean`", "status `pending`", "deployment `in_progress`", "`home-portal/isolated`"],
    "not landed": ["compose `errors`", "`E_UNKNOWN_SURFACE`", "status `failure`", "deployment `missing`"],
}


@pytest.mark.parametrize("name", EXAMPLES)
def test_each_worked_example_gives_the_verdict_it_states(name: str) -> None:
    section = example_section(SKILL_MD.read_text(), name)
    for fact in EXAMPLE_FACTS[name]:
        assert fact in section, fact
    stated = re.search(r"Result: `VERDICT: ([a-z ]+)`", section)
    assert stated is not None
    assert stated.group(1) == name
    assert _verdict(EXAMPLE_INPUTS[name]) == name


def test_the_mixed_example_names_the_disagreeing_signals_with_links() -> None:
    result = _run(EXAMPLE_INPUTS["mixed"])
    assert "status: pending  https://example.test/status/3" in result.stdout
    assert "deployment: in_progress  https://example.test/deploy/4" in result.stdout
    assert "home-portal/isolated  https://example.test/issues/1" in result.stdout


# ---------------------------------------------------------------------------
# Sources and the stop rule
# ---------------------------------------------------------------------------


def test_skill_md_names_the_docs_from_sources_yaml() -> None:
    text = SKILL_MD.read_text()
    sources = yaml.safe_load(SOURCES.read_text())["sources"]
    assert sources
    for source in sources:
        assert source["kind"] == "file"
        assert Path(source["path"]).name in text, source["path"]
        assert source["repo"].split("/")[-1] in text, source["repo"]


def test_sources_yaml_is_accepted_by_the_drift_checker() -> None:
    result = subprocess.run(
        [sys.executable, str(CHECKER), "--validate"],
        cwd=KIT_ROOT,
        capture_output=True,
        text=True,
        check=False,
    )
    assert result.returncode == 0, result.stdout + result.stderr


def test_skill_md_says_to_stop_and_name_the_doc_that_cannot_be_read() -> None:
    text = SKILL_MD.read_text()
    section = text[text.index("## When a doc cannot be read") :]
    assert "STOP" in section
    assert "Name the doc" in section
    assert "repository and path" in section
    assert "Do not answer from memory" in section


# ---------------------------------------------------------------------------
# Registry and install
# ---------------------------------------------------------------------------

AGENTS = ["claude", "codex", "vibe", "antigravity", "opencode"]


def test_registry_lists_the_skill_as_a_local_skill_for_every_agent() -> None:
    skills = render_registry.load_registry()["local_skills"]
    entry = next(s for s in skills if s["name"] == "estate-delivery-status")
    assert entry["agents"] == AGENTS
    assert entry["surfaces"] == ["workstation"]
    assert entry["path"] == "skills/workstation/estate-delivery-status"
    assert (KIT_ROOT / entry["path"] / "SKILL.md").is_file()


@pytest.fixture(scope="module")
def fresh(tmp_path_factory: pytest.TempPathFactory) -> Run:
    return setup_run(tmp_path_factory.mktemp("delivery-fresh"), "Darwin")


def test_the_rendered_setup_installs_the_skill_with_its_script(fresh: Run) -> None:
    for agent in (".claude", ".codex"):
        installed = fresh.home / agent / "skills" / "estate-delivery-status"
        assert (installed / "SKILL.md").read_bytes() == SKILL_MD.read_bytes()
        assert (installed / "scripts" / "verdict.py").read_bytes() == SCRIPT.read_bytes()
        assert (installed / "sources.yaml").read_bytes() == SOURCES.read_bytes()
