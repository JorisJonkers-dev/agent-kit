"""The estate-rollback skill: plan.py, its named sources and its install.

plan.py prints the exact `gh workflow run` command for a pause, rollback or
resume and never runs it. A fake `gh` on PATH serves the Project's pin file and
logs every invocation, so the tests can prove the only call ever made is a
read-only GET of the contents endpoint.
"""

from __future__ import annotations

import importlib.util
import os
import re
import runpy
import shlex
import subprocess
import sys
from pathlib import Path
from types import ModuleType

import pytest
import yaml
from setup_harness import Run, setup_run

KIT_ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(KIT_ROOT / "scripts"))

import render_registry  # noqa: E402

SKILL = KIT_ROOT / "skills" / "workstation" / "estate-rollback"
SCRIPT = SKILL / "scripts" / "plan.py"
SKILL_MD = SKILL / "SKILL.md"
SOURCES = SKILL / "sources.yaml"
CHECKER = KIT_ROOT / "scripts" / "check_skill_sources.py"

REPO = "JorisJonkers-dev/estate"
PIN_PATH = f"repos/{REPO}/contents/projects/grimoire/source.yaml"

SOURCE_YAML = """\
# GENERATED. Never hand-edit.
---
apiVersion: source.toolkit.fluxcd.io/v1
kind: OCIRepository
metadata:
  name: project-grimoire
  namespace: flux-system
spec:
  interval: 10m
---
apiVersion: kustomize.toolkit.fluxcd.io/v1
kind: Kustomization
metadata:
  name: apps-grimoire
  namespace: flux-system
  annotations:
    event.toolkit.fluxcd.io/repository: JorisJonkers-dev/grimoire
    event.toolkit.fluxcd.io/environment: production
    event.toolkit.fluxcd.io/version: {pin}
    event.toolkit.fluxcd.io/url: https://grimoire.jorisjonkers.dev
spec:
  interval: 10m
"""

PAUSED = """\
    estate.jorisjonkers.dev/paused-by: alice
    estate.jorisjonkers.dev/paused-at: "2026-11-01T10:00:00Z"
    estate.jorisjonkers.dev/paused-reason: "bad deploy"
"""
ROLLED = """\
    estate.jorisjonkers.dev/rollback-version: 1.3.0
    estate.jorisjonkers.dev/rollback-fragment: sha256:abc
"""

FAKE_GH = f"""#!{sys.executable}
import os
import sys
from pathlib import Path

args = sys.argv[1:]
with open(os.environ["FAKE_GH_LOG"], "a") as log:
    log.write(" ".join(args) + "\\n")
mode = os.environ.get("FAKE_GH_MODE", "ok")
if mode == "fail":
    print("HTTP 404: Not Found", file=sys.stderr)
    sys.exit(1)
sys.stdout.write(Path(os.environ["FAKE_GH_BODY"]).read_text())
"""


def _fake_gh(tmp_path: Path, body: str, mode: str = "ok") -> dict[str, str]:
    bin_dir = tmp_path / "bin"
    bin_dir.mkdir(exist_ok=True)
    tool = bin_dir / "gh"
    tool.write_text(FAKE_GH)
    tool.chmod(0o755)
    (tmp_path / "body.yaml").write_text(body)
    return {
        "PATH": f"{bin_dir}:{os.environ['PATH']}",
        "FAKE_GH_LOG": str(tmp_path / "gh.log"),
        "FAKE_GH_BODY": str(tmp_path / "body.yaml"),
        "FAKE_GH_MODE": mode,
    }


def _calls(env: dict[str, str]) -> list[str]:
    log = Path(env["FAKE_GH_LOG"])
    return log.read_text().splitlines() if log.exists() else []


def _run(*args: str, env: dict[str, str]) -> subprocess.CompletedProcess[str]:
    return subprocess.run([sys.executable, str(SCRIPT), *args], capture_output=True, text=True, check=False, env=env)


def _env(tmp_path: Path, pin: str = "1.5.0", extra: str = "", mode: str = "ok") -> dict[str, str]:
    body = SOURCE_YAML.format(pin=pin)
    if extra:
        body = body.replace("    event.toolkit.fluxcd.io/url", extra + "    event.toolkit.fluxcd.io/url")
    return _fake_gh(tmp_path, body, mode)


def _commands(stdout: str) -> list[str]:
    return [line.strip() for line in stdout.splitlines() if line.strip().startswith("gh workflow run")]


PAUSE_CMD = (
    "gh workflow run pause.yml --repo JorisJonkers-dev/estate --ref main -f project=grimoire -f 'reason=bad deploy'"
)
ROLLBACK_CMD = (
    "gh workflow run rollback.yml --repo JorisJonkers-dev/estate --ref main"
    " -f project=grimoire -f version=1.3.0 -f 'reason=bad deploy'"
)
PIN_NOTE = "pin given by hand: the pin file was not read, so a recorded rollback is not checked"
NOT_STATED = (
    "Migration Proof not stated: pass --proof-names <release the proof names>"
    " or --no-proof when the pinned release carries none"
)
RESUME_CMD = "gh workflow run resume.yml --repo JorisJonkers-dev/estate --ref main -f project=grimoire"


@pytest.fixture
def plan_module() -> ModuleType:
    spec = importlib.util.spec_from_file_location("estate_plan", SCRIPT)
    assert spec is not None
    assert spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


# ---------------------------------------------------------------------------
# The exact commands
# ---------------------------------------------------------------------------


def test_pause_prints_the_exact_command(tmp_path: Path) -> None:
    env = _env(tmp_path)
    result = _run("--project", "grimoire", "--action", "pause", "--reason", "bad deploy", env=env)
    assert result.returncode == 0, result.stderr
    assert _commands(result.stdout) == [PAUSE_CMD]
    assert "pin:      1.5.0 (projects/grimoire/source.yaml, event.toolkit.fluxcd.io/version)" in result.stdout


def test_rollback_prints_the_rollback_and_the_later_resume(tmp_path: Path) -> None:
    env = _env(tmp_path)
    result = _run(
        "--project", "grimoire", "--action", "rollback", "--target", "1.3.0", "--no-proof", "--reason", "bad deploy",
        env=env,
    )  # fmt: skip
    assert result.returncode == 0, result.stderr
    assert _commands(result.stdout) == [ROLLBACK_CMD, RESUME_CMD]
    assert "target:   1.3.0 (earlier than the pin 1.5.0: ok)" in result.stdout
    assert "Migration Proof: --no-proof says release 1.5.0 carries none" in result.stdout
    assert "pin given by hand" not in result.stdout


def test_rollback_with_the_proofs_release_says_ok(tmp_path: Path) -> None:
    env = _env(tmp_path)
    result = _run(
        "--project", "grimoire", "--action", "rollback", "--target", "1.3.0",
        "--proof-names", "1.3.0", "--reason", "bad deploy", env=env,
    )  # fmt: skip
    assert result.returncode == 0, result.stderr
    assert _commands(result.stdout)[0] == ROLLBACK_CMD
    assert "target 1.3.0 is the release --proof-names gives: ok" in result.stdout


def test_resume_prints_the_exact_command(tmp_path: Path) -> None:
    env = _env(tmp_path)
    result = _run("--project", "grimoire", "--action", "resume", env=env)
    assert result.returncode == 0, result.stderr
    assert _commands(result.stdout) == [RESUME_CMD]
    assert "WARNING" not in result.stdout


def test_resume_reports_the_paused_and_rolled_back_state_and_warns(tmp_path: Path) -> None:
    env = _env(tmp_path, extra=PAUSED + ROLLED)
    result = _run("--project", "grimoire", "--action", "resume", env=env)
    assert result.returncode == 0, result.stderr
    assert "state:    paused by alice at 2026-11-01T10:00:00Z: bad deploy" in result.stdout
    assert "state:    rolled back to 1.3.0" in result.stdout
    assert "WARNING: this Project is rolled back" in result.stdout
    assert _commands(result.stdout) == [RESUME_CMD]


def test_a_reason_is_quoted_so_it_cannot_inject_a_second_command(tmp_path: Path) -> None:
    env = _env(tmp_path)
    result = _run("--project", "grimoire", "--action", "pause", "--reason", "x; rm -rf ~", env=env)
    assert result.returncode == 0
    assert "-f 'reason=x; rm -rf ~'" in result.stdout


# ---------------------------------------------------------------------------
# The fake gh proves no dispatch
# ---------------------------------------------------------------------------

CASES = [
    ["--action", "pause", "--reason", "bad deploy"],
    ["--action", "resume"],
    ["--action", "rollback", "--target", "1.3.0", "--no-proof", "--reason", "bad deploy"],
    ["--action", "rollback", "--target", "1.3.0", "--reason", "bad deploy"],
    ["--action", "rollback", "--target", "1.3.0", "--no-proof", "--proof-names", "1.3.0", "--reason", "bad deploy"],
    ["--action", "rollback", "--target", "1.6.0", "--no-proof", "--reason", "bad deploy"],
    ["--action", "rollback", "--target", "1.5.0", "--no-proof", "--reason", "bad deploy"],
    ["--action", "rollback", "--target", "1.3.0", "--proof-names", "1.4.2", "--reason", "bad deploy"],
    ["--action", "rollback", "--reason", "bad deploy"],
    ["--action", "rollback", "--target", "1.3.0"],
    ["--action", "pause"],
]


@pytest.mark.parametrize("case", CASES)
def test_the_only_gh_call_is_a_read_only_get_of_the_contents_endpoint(tmp_path: Path, case: list[str]) -> None:
    env = _env(tmp_path)
    _run("--project", "grimoire", *case, env=env)
    expected = f"api {PIN_PATH} -H Accept: application/vnd.github.raw"
    for call in _calls(env):
        assert call == expected


def test_a_refusal_made_before_reading_never_calls_gh(tmp_path: Path) -> None:
    env = _env(tmp_path)
    result = _run("--project", "grimoire", "--action", "pause", env=env)
    assert result.returncode == 1
    assert _calls(env) == []


def test_a_pause_reads_the_pin_with_exactly_one_get(tmp_path: Path) -> None:
    env = _env(tmp_path)
    _run("--project", "grimoire", "--action", "pause", "--reason", "r", env=env)
    assert _calls(env) == [f"api {PIN_PATH} -H Accept: application/vnd.github.raw"]


@pytest.mark.parametrize("action", ["pause", "rollback", "resume"])
def test_with_pin_given_gh_is_not_called_at_all(tmp_path: Path, action: str) -> None:
    env = _env(tmp_path)
    extra = ["--target", "1.3.0"] if action == "rollback" else []
    reason = [] if action == "resume" else ["--reason", "bad deploy"]
    extra += ["--no-proof"] if action == "rollback" else []
    result = _run("--project", "grimoire", "--action", action, *extra, *reason, "--pin", "1.5.0", env=env)
    assert result.returncode == 0, result.stderr
    assert "(given with --pin (file not read))" in result.stdout
    assert PIN_NOTE in result.stdout
    assert _calls(env) == []


# ---------------------------------------------------------------------------
# Refusals
# ---------------------------------------------------------------------------


def _refused(tmp_path: Path, args: list[str], message: str, **env_kwargs: str) -> None:
    env = _env(tmp_path, **env_kwargs)
    result = _run(*args, env=env)
    assert result.returncode == 1, result.stdout + result.stderr
    assert message in result.stderr
    assert result.stderr.startswith("REFUSED: ")
    assert "gh workflow run" not in result.stdout
    assert result.stdout == ""


RB = ["--project", "grimoire", "--action", "rollback", "--reason", "bad deploy"]
NP = ["--no-proof"]


def test_a_target_later_than_the_pin_is_refused(tmp_path: Path) -> None:
    _refused(tmp_path, [*RB, *NP, "--target", "1.6.0"], "target 1.6.0 is not earlier than the pin 1.5.0")


def test_a_target_equal_to_the_pin_is_refused(tmp_path: Path) -> None:
    _refused(tmp_path, [*RB, *NP, "--target", "1.5.0"], "target 1.5.0 is not earlier than the pin 1.5.0")


def test_versions_compare_numerically_not_as_text(tmp_path: Path) -> None:
    _refused(tmp_path, [*RB, *NP, "--target", "1.10.0"], "not earlier than the pin 1.9.0", pin="1.9.0")


def test_a_target_the_proof_does_not_name_is_refused(tmp_path: Path) -> None:
    _refused(
        tmp_path,
        [*RB, "--target", "1.3.0", "--proof-names", "1.4.2"],
        "target 1.3.0 is not the release the pinned release 1.5.0's Migration Proof names (1.4.2)",
    )


def test_a_rollback_stating_neither_proof_flag_is_refused(tmp_path: Path) -> None:
    _refused(tmp_path, [*RB, "--target", "1.3.0"], NOT_STATED)


def test_a_rollback_stating_both_proof_flags_is_refused(tmp_path: Path) -> None:
    _refused(tmp_path, [*RB, "--target", "1.3.0", *NP, "--proof-names", "1.3.0"], NOT_STATED)


def test_pin_given_by_hand_prints_the_note_on_every_plan(tmp_path: Path) -> None:
    env = _env(tmp_path)
    result = _run(*RB, "--target", "1.3.0", *NP, "--pin", "1.5.0", env=env)
    assert result.stdout.count(PIN_NOTE) == 1


@pytest.mark.parametrize("version", ["01.4.0", "1.04.0", "1.4.00"])
def test_leading_zeros_in_a_version_are_refused(tmp_path: Path, version: str) -> None:
    _refused(tmp_path, [*RB, *NP, "--target", version], f"--target '{version}' is not a release version")


def test_a_missing_target_is_refused(tmp_path: Path) -> None:
    _refused(tmp_path, RB, "--action rollback needs --target")


@pytest.mark.parametrize("action", ["pause", "rollback"])
@pytest.mark.parametrize("reason", [None, "", "   ", "two\nlines", "a\x85b", "a\u2028b", "a\u2029b"])
def test_a_missing_blank_or_multiline_reason_is_refused(tmp_path: Path, action: str, reason: str | None) -> None:
    args = ["--project", "grimoire", "--action", action]
    if action == "rollback":
        args += ["--target", "1.3.0", "--no-proof"]
    if reason is not None:
        args += ["--reason", reason]
    message = "--reason must be one line" if reason and reason.strip() else "needs --reason"
    _refused(tmp_path, args, message)


def test_an_already_rolled_back_pin_is_refused(tmp_path: Path) -> None:
    _refused(tmp_path, [*RB, *NP, "--target", "1.2.0"], "grimoire is already rolled back to 1.3.0", extra=ROLLED)


def test_an_already_rolled_back_pin_without_a_version_is_refused(tmp_path: Path) -> None:
    extra = "    estate.jorisjonkers.dev/rollback-fragment: sha256:abc\n"
    _refused(tmp_path, [*RB, *NP, "--target", "1.2.0"], "already rolled back to ?", extra=extra)


@pytest.mark.parametrize(
    ("args", "message"),
    [
        (["--project", "Grimoire", "--action", "resume"], "'Grimoire' is not a Project's name"),
        (["--project", "grimoire", "--action", "resume", "--target", "1.0.0"], "only apply to --action rollback"),
        (["--project", "grimoire", "--action", "resume", "--no-proof"], "only apply to --action rollback"),
        (["--project", "grimoire", "--action", "pause", "--reason", "r", "--proof-names", "1.0.0"], "only apply"),
        ([*RB, *NP, "--target", "1.x"], "--target '1.x' is not a release version"),
        ([*RB, "--target", "1.3.0", "--proof-names", "v1"], "--proof-names 'v1' is not a release version"),
        (["--project", "grimoire", "--action", "resume", "--pin", "latest"], "--pin 'latest' is not a release version"),
    ],
)
def test_malformed_input_is_refused(tmp_path: Path, args: list[str], message: str) -> None:
    _refused(tmp_path, args, message)


def test_an_unknown_project_or_gh_failure_exits_1_with_no_command(tmp_path: Path) -> None:
    env = _env(tmp_path, mode="fail")
    result = _run("--project", "nosuch", "--action", "resume", env=env)
    assert result.returncode == 1
    assert "could not read projects/nosuch/source.yaml" in result.stderr
    assert "HTTP 404: Not Found" in result.stderr
    assert result.stdout == ""
    assert len(_calls(env)) == 1


def test_gh_missing_from_path_exits_1_and_says_stop(tmp_path: Path) -> None:
    result = _run("--project", "grimoire", "--action", "resume", env={"PATH": "/nonexistent"})
    assert result.returncode == 1
    assert "gh is not on PATH" in result.stderr
    assert result.stdout == ""


@pytest.mark.parametrize(
    ("body", "message"),
    [
        ("kind: Kustomization\n", "found 0"),
        ("    event.toolkit.fluxcd.io/version: 1.0.0\n    event.toolkit.fluxcd.io/version: 1.1.0\n", "found 2"),
        ("    event.toolkit.fluxcd.io/version: latest\n", "the pin 'latest'"),
    ],
)
def test_an_unusable_pin_file_is_refused(tmp_path: Path, body: str, message: str) -> None:
    env = _fake_gh(tmp_path, body)
    result = _run("--project", "grimoire", "--action", "resume", env=env)
    assert result.returncode == 1
    assert message in result.stderr
    assert result.stdout == ""


def test_a_quoted_pin_with_a_trailing_comment_is_read(tmp_path: Path) -> None:
    env = _fake_gh(tmp_path, '    event.toolkit.fluxcd.io/version: "2.0.1"  # tag\n')
    result = _run("--project", "grimoire", "--action", "resume", env=env)
    assert result.returncode == 0
    assert "pin:      2.0.1" in result.stdout


# ---------------------------------------------------------------------------
# In-process, so line coverage of plan.py is measurable
# ---------------------------------------------------------------------------


class _Done:
    def __init__(self, code: int, out: str = "", err: str = "") -> None:
        self.returncode, self.stdout, self.stderr = code, out, err


def test_main_runs_in_process(
    plan_module: ModuleType, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    calls: list[list[str]] = []
    body = SOURCE_YAML.format(pin="1.5.0").replace(
        "    event.toolkit.fluxcd.io/url", PAUSED + "    event.toolkit.fluxcd.io/url"
    )

    def fake_run(cmd: list[str], **_kw: object) -> _Done:
        calls.append(cmd)
        return _Done(0, body)

    monkeypatch.setattr(plan_module.subprocess, "run", fake_run)
    assert plan_module.main(["--project", "grimoire", "--action", "resume"]) == 0
    out = capsys.readouterr().out
    assert RESUME_CMD in out
    assert "state:    paused by alice at 2026-11-01T10:00:00Z: bad deploy" in out
    assert plan_module.main([*RB, "--target", "1.3.0", "--no-proof", "--pin", "1.5.0"]) == 0
    assert PIN_NOTE in capsys.readouterr().out
    assert plan_module.main([*RB, "--target", "1.6.0", *NP, "--pin", "1.5.0"]) == 1
    assert plan_module.main([*RB, "--target", "1.3.0", "--pin", "1.5.0"]) == 1
    assert plan_module.main([*RB, "--target", "1.3.0", "--proof-names", "1.4.2", "--pin", "1.5.0"]) == 1
    assert plan_module.main([*RB, "--target", "1.3.0", "--proof-names", "1.3.0", "--pin", "1.5.0"]) == 0
    assert len(calls) == 1
    assert calls[0][:3] == ["gh", "api", PIN_PATH]
    capsys.readouterr()


def test_main_reports_gh_failures_in_process(
    plan_module: ModuleType, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    def missing(*_a: object, **_k: object) -> _Done:
        raise FileNotFoundError

    def slow(*_a: object, **_k: object) -> _Done:
        raise subprocess.TimeoutExpired("gh", 120)

    argv = ["--project", "grimoire", "--action", "resume"]
    for fake, text in ((missing, "gh is not on PATH"), (slow, "did not answer within 120 seconds")):
        monkeypatch.setattr(plan_module.subprocess, "run", fake)
        assert plan_module.main(argv) == 1
        assert text in capsys.readouterr().err
    monkeypatch.setattr(plan_module.subprocess, "run", lambda *_a, **_k: _Done(1, "", ""))
    assert plan_module.main(argv) == 1
    assert "no detail" in capsys.readouterr().err
    monkeypatch.setattr(plan_module.subprocess, "run", lambda *_a, **_k: _Done(0, "nothing"))
    assert plan_module.main(argv) == 1
    assert plan_module.main(["--project", "grimoire", "--action", "pause"]) == 1
    monkeypatch.setattr(
        plan_module.subprocess, "run", lambda *_a, **_k: _Done(0, "  event.toolkit.fluxcd.io/version: latest\n")
    )
    assert plan_module.main(["--project", "grimoire", "--action", "resume"]) == 1
    assert "the pin 'latest'" in capsys.readouterr().err
    monkeypatch.setattr(plan_module.subprocess, "run", lambda *_a, **_k: _Done(0, SOURCE_YAML.format(pin="1.5.0")))
    assert plan_module.main(["--project", "grimoire", "--action", "pause", "--reason", "why"]) == 0
    assert "-f reason=why" in capsys.readouterr().out
    assert plan_module.main(["--project", "grimoire", "--action", "pause", "--reason", "a\tb"]) == 1
    assert plan_module.main(["--project", "grimoire", "--action", "resume", "--pin", "x"]) == 1
    assert plan_module.main(["--project", "Bad", "--action", "resume"]) == 1
    assert plan_module.main([*RB, "--pin", "1.5.0"]) == 1
    assert plan_module.main([*RB, "--target", "x", "--pin", "1.5.0"]) == 1
    assert plan_module.main([*RB, "--target", "1.3.0", "--proof-names", "x", "--pin", "1.5.0"]) == 1
    assert plan_module.main(["--project", "grimoire", "--action", "resume", "--target", "1.0.0"]) == 1
    capsys.readouterr()


def test_in_process_rolled_back_state_is_refused(
    plan_module: ModuleType, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    body = SOURCE_YAML.format(pin="1.5.0").replace(
        "    event.toolkit.fluxcd.io/url", ROLLED + "    event.toolkit.fluxcd.io/url"
    )
    monkeypatch.setattr(plan_module.subprocess, "run", lambda *_a, **_k: _Done(0, body))
    assert plan_module.main([*RB, *NP, "--target", "1.2.0"]) == 1
    assert plan_module.main(["--project", "grimoire", "--action", "resume"]) == 0
    capsys.readouterr()


def test_running_the_script_as_main_exits_with_the_status(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(sys, "argv", [str(SCRIPT), "--project", "grimoire", "--action", "resume", "--pin", "1.5.0"])
    with pytest.raises(SystemExit) as raised:
        runpy.run_path(str(SCRIPT), run_name="__main__")
    assert raised.value.code == 0


def test_the_script_never_names_a_dispatching_gh_call() -> None:
    code = SCRIPT.read_text()
    runs = re.findall(r"subprocess\.run\(", code)
    assert len(runs) == 1
    assert '"gh", "api"' in code
    assert "--method" not in code
    assert '"-X"' not in code
    assert "-XPOST" not in code
    assert '"-f"' not in code.split("def gh_run_command")[0]


# ---------------------------------------------------------------------------
# The skill text
# ---------------------------------------------------------------------------

REFUSED_HEADINGS = (
    "### Worked example: target later than the pin (refused)",
    "### Worked example: target with no Migration Proof (refused)",
)


def _example_commands() -> list[tuple[str, list[str], str]]:
    """Each worked example's command line and its REFUSED line, parsed from SKILL.md."""
    text = SKILL_MD.read_text()
    found = []
    for heading in REFUSED_HEADINGS:
        assert heading in text, heading
        block = text[text.index(heading) :].split("```")[1]
        command, refused = block.strip().splitlines()[:2]
        assert command.startswith("python3 scripts/plan.py ")
        found.append((heading, shlex.split(command)[2:], refused))
    return found


@pytest.mark.parametrize("heading", REFUSED_HEADINGS)
def test_skill_md_has_each_refused_worked_example(heading: str) -> None:
    assert heading in SKILL_MD.read_text()


def test_the_worked_examples_values_really_are_refused_by_plan_py(plan_module: ModuleType) -> None:
    examples = _example_commands()
    assert len(examples) == 2
    for _heading, argv, refused in examples:
        result = subprocess.run(
            [sys.executable, str(SCRIPT), *argv],
            capture_output=True,
            text=True,
            check=False,
            env={"PATH": "/nonexistent"},
        )
        assert result.returncode == 1
        assert result.stdout == ""
        assert result.stderr.strip() == refused


def test_skill_md_names_the_readme_and_workflow_disagreement() -> None:
    text = SKILL_MD.read_text()
    section = text[text.index("## Where the docs and reality disagree") :]
    assert "README" in section
    assert "two commits" in section
    assert "rollback.yml" in section
    assert "The skill follows the estate workflows" in section


def test_skill_md_says_to_stop_and_name_the_doc_that_cannot_be_read() -> None:
    text = SKILL_MD.read_text()
    section = text[text.index("## When a doc cannot be read") :]
    assert "stop" in section.lower()
    assert "Name the doc" in section


def test_skill_md_never_dispatches_and_names_its_sources() -> None:
    text = SKILL_MD.read_text()
    assert "never dispatches a workflow" in text
    sources = yaml.safe_load(SOURCES.read_text())["sources"]
    for source in sources:
        if source["path"].startswith(".github/workflows/"):
            assert source["path"].split("/")[-1] in text


def test_sources_yaml_is_accepted_by_the_drift_checker() -> None:
    result = subprocess.run(
        [sys.executable, str(CHECKER), "--validate"], cwd=KIT_ROOT, capture_output=True, text=True, check=False
    )
    assert result.returncode == 0, result.stdout + result.stderr


# ---------------------------------------------------------------------------
# Registry and install
# ---------------------------------------------------------------------------

AGENTS = ["claude", "codex", "vibe", "antigravity", "opencode"]


def test_registry_lists_the_skill_as_a_local_skill_for_every_agent() -> None:
    skills = render_registry.load_registry()["local_skills"]
    entry = next(s for s in skills if s["name"] == "estate-rollback")
    assert entry["agents"] == AGENTS
    assert entry["surfaces"] == ["workstation"]
    assert entry["path"] == "skills/workstation/estate-rollback"
    assert (KIT_ROOT / entry["path"] / "SKILL.md").is_file()


@pytest.fixture(scope="module")
def fresh(tmp_path_factory: pytest.TempPathFactory) -> Run:
    return setup_run(tmp_path_factory.mktemp("rollback-fresh"), "Darwin")


def test_the_rendered_setup_installs_the_skill_with_its_script(fresh: Run) -> None:
    for agent in (".claude", ".codex"):
        installed = fresh.home / agent / "skills" / "estate-rollback"
        assert (installed / "SKILL.md").read_bytes() == SKILL_MD.read_bytes()
        assert (installed / "scripts" / "plan.py").read_bytes() == SCRIPT.read_bytes()
        assert (installed / "sources.yaml").read_bytes() == SOURCES.read_bytes()
