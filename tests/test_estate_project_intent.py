"""The estate-project-intent skill: its scaffold/validate script, its named sources and its install.

`intent.py scaffold` writes a Project Intent, and `intent.py validate` runs
`deploy-kit validate` and mirrors its exit status. deploy-kit is a Node CLI, so a
fake `deploy-kit` on PATH (a small python script) stands in for it: it accepts a
file that has every required top-level field, refuses one that lacks a field with
an `E_...` line and exit 1, and exits 2 when called without paths.
"""

from __future__ import annotations

import importlib.util
import os
import runpy
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

SKILL = KIT_ROOT / "skills" / "workstation" / "estate-project-intent"
SCRIPT = SKILL / "scripts" / "intent.py"
SKILL_MD = SKILL / "SKILL.md"
SOURCES = SKILL / "sources.yaml"
CHECKER = KIT_ROOT / "scripts" / "check_skill_sources.py"

REQUIRED = ("apiVersion", "kind", "schemaVersion", "project", "owner", "applications")

FAKE_DEPLOY_KIT = f"""#!{sys.executable}
import sys
from pathlib import Path

import yaml

REQUIRED = {REQUIRED!r}
args = sys.argv[1:]
if len(args) < 2 or args[0] != "validate":
    print("E_USAGE: deploy-kit validate <paths>", file=sys.stderr)
    sys.exit(2)
refused = False
for path in args[1:]:
    p = Path(path)
    if not p.is_file() or not p.name.endswith(".project.yml"):
        continue
    doc = yaml.safe_load(p.read_text())
    for field in REQUIRED:
        if field not in doc:
            print(f"E_REQUIRED_FIELD: {{p}}: missing required field {{field!r}}")
            refused = True
sys.exit(1 if refused else 0)
"""

SCAFFOLD = {
    "--project": "notes",
    "--owner": "team-notes",
    "--host": "notes.example.dev",
    "--audience": "authenticated",
    "--image": "notes-api",
    "--process": "notes-api",
    "--port": "8080",
}


def _scaffold_args(out: Path, **override: str) -> list[str]:
    values = {**SCAFFOLD, **override, "--out": str(out)}
    return [item for pair in values.items() for item in pair]


def _bin_with_deploy_kit(tmp_path: Path) -> Path:
    bin_dir = tmp_path / "bin"
    bin_dir.mkdir()
    tool = bin_dir / "deploy-kit"
    tool.write_text(FAKE_DEPLOY_KIT)
    tool.chmod(0o755)
    return bin_dir


def _run(*args: str, path: str | None = None) -> subprocess.CompletedProcess[str]:
    env = {"PATH": path if path is not None else os.environ["PATH"]}
    return subprocess.run(
        [sys.executable, str(SCRIPT), *args],
        capture_output=True,
        text=True,
        check=False,
        env=env,
    )


@pytest.fixture
def intent_module() -> ModuleType:
    spec = importlib.util.spec_from_file_location("estate_intent", SCRIPT)
    assert spec is not None
    assert spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


@pytest.fixture
def scaffolded(tmp_path: Path) -> Path:
    result = _run("scaffold", *_scaffold_args(tmp_path / "repo"))
    assert result.returncode == 0, result.stderr
    return tmp_path / "repo"


# ---------------------------------------------------------------------------
# scaffold
# ---------------------------------------------------------------------------


def test_scaffold_writes_the_project_file_and_base_env(scaffolded: Path) -> None:
    project = scaffolded / "deploy" / "notes.project.yml"
    env = scaffolded / "deploy" / "env" / "notes-api" / "base.env"
    doc = yaml.safe_load(project.read_text())
    assert set(REQUIRED) <= set(doc)
    assert (doc["project"], doc["owner"]) == ("notes", "team-notes")
    process = doc["applications"][0]["processes"][0]
    assert (process["name"], process["image"], process["provides"]["http"]) == ("notes-api", "notes-api", 8080)
    assert doc["applications"][0]["exposure"][0]["audience"] == "authenticated"
    assert env.is_file()


def test_scaffold_marks_the_author_choices_with_check(scaffolded: Path) -> None:
    text = (scaffolded / "deploy" / "notes.project.yml").read_text()
    assert text.count("CHECK") >= 6
    assert "runtime: node    # CHECK" in text


def test_scaffold_prints_what_it_wrote(tmp_path: Path) -> None:
    result = _run("scaffold", *_scaffold_args(tmp_path))
    assert result.stdout.splitlines() == [
        f"wrote {tmp_path}/deploy/notes.project.yml",
        f"wrote {tmp_path}/deploy/env/notes-api/base.env",
    ]


def test_scaffold_refuses_to_overwrite_and_leaves_the_files_alone(scaffolded: Path) -> None:
    project = scaffolded / "deploy" / "notes.project.yml"
    project.write_text("edited by the owner\n")
    result = _run("scaffold", *_scaffold_args(scaffolded))
    assert result.returncode == 2
    assert f"refusing to overwrite {project}" in result.stderr
    assert "base.env" in result.stderr
    assert project.read_text() == "edited by the owner\n"


@pytest.mark.parametrize(
    ("override", "message"),
    [
        ({"--project": "Notes_App"}, "--project must be a DNS-1123 label"),
        ({"--process": "Bad"}, "--process must be a DNS-1123 label"),
        ({"--audience": "everyone"}, "--audience must be one of anonymous, authenticated, internal, lan"),
        ({"--port": "0"}, "--port must be 1-65535"),
        ({"--port": "70000"}, "--port must be 1-65535"),
    ],
)
def test_scaffold_refuses_bad_values_and_writes_nothing(tmp_path: Path, override: dict[str, str], message: str) -> None:
    result = _run("scaffold", *_scaffold_args(tmp_path / "repo", **override))
    assert result.returncode == 2
    assert message in result.stderr
    assert not (tmp_path / "repo").exists()


# ---------------------------------------------------------------------------
# validate, against a fake deploy-kit
# ---------------------------------------------------------------------------


def test_a_scaffolded_intent_is_accepted_by_deploy_kit(tmp_path: Path, scaffolded: Path) -> None:
    path = f"{_bin_with_deploy_kit(tmp_path)}:{os.environ['PATH']}"
    result = _run(
        "validate", str(scaffolded / "deploy" / "notes.project.yml"), str(scaffolded / "deploy" / "env"), path=path
    )
    assert result.returncode == 0, result.stdout + result.stderr
    assert "deploy-kit validate: accepted (exit 0)" in result.stdout


@pytest.mark.parametrize("field", REQUIRED)
def test_a_missing_required_field_is_refused_with_deploy_kits_text(
    tmp_path: Path, scaffolded: Path, field: str
) -> None:
    project = scaffolded / "deploy" / "notes.project.yml"
    doc = yaml.safe_load(project.read_text())
    del doc[field]
    broken = scaffolded / "deploy" / "broken.project.yml"
    broken.write_text(yaml.safe_dump(doc))
    path = f"{_bin_with_deploy_kit(tmp_path)}:{os.environ['PATH']}"
    result = _run("validate", str(broken), path=path)
    assert result.returncode == 1
    assert f"E_REQUIRED_FIELD: {broken}: missing required field '{field}'" in result.stdout
    assert "REFUSED (exit 1)" in result.stdout


def test_validate_without_deploy_kit_on_path_exits_2_and_validates_nothing(scaffolded: Path) -> None:
    result = _run("validate", str(scaffolded / "deploy"), path="/nonexistent")
    assert result.returncode == 2
    assert "deploy-kit is not installed on PATH" in result.stderr
    assert "Nothing was validated" in result.stderr


def test_validate_names_a_path_that_does_not_exist(tmp_path: Path) -> None:
    result = _run("validate", str(tmp_path / "nope.project.yml"), path="/nonexistent")
    assert result.returncode == 2
    assert f"no such path: {tmp_path / 'nope.project.yml'}" in result.stderr


def test_validate_names_an_extra_path_that_does_not_exist(tmp_path: Path, scaffolded: Path) -> None:
    result = _run("validate", str(scaffolded / "deploy"), str(tmp_path / "gone"), path="/nonexistent")
    assert result.returncode == 2
    assert f"no such path: {tmp_path / 'gone'}" in result.stderr


def test_the_fake_deploy_kit_exits_2_without_paths(tmp_path: Path) -> None:
    tool = _bin_with_deploy_kit(tmp_path) / "deploy-kit"
    bare = subprocess.run([str(tool), "validate"], capture_output=True, text=True, check=False)
    assert bare.returncode == 2
    assert "E_USAGE" in bare.stderr


def _stub_deploy_kit(tmp_path: Path, code: int) -> str:
    bin_dir = tmp_path / "stubbin"
    bin_dir.mkdir()
    tool = bin_dir / "deploy-kit"
    tool.write_text(f"#!/bin/sh\necho deploy-kit-said-this\nexit {code}\n")
    tool.chmod(0o755)
    return str(bin_dir)


@pytest.mark.parametrize(
    ("code", "returned", "line"),
    [
        (0, 0, "accepted (exit 0)"),
        (1, 1, "REFUSED (exit 1)"),
        (2, 2, "called wrongly (exit 2)"),
        (3, 2, "unexpected exit 3"),
    ],
)
def test_validate_mirrors_the_exit_status_and_passes_the_output_through(
    tmp_path: Path,
    scaffolded: Path,
    code: int,
    returned: int,
    line: str,
) -> None:
    result = _run("validate", str(scaffolded / "deploy"), path=_stub_deploy_kit(tmp_path, code))
    assert result.returncode == returned
    assert result.stdout.splitlines()[0] == "deploy-kit-said-this"
    assert line in result.stdout


# ---------------------------------------------------------------------------
# In-process, so line coverage of intent.py is measurable
# ---------------------------------------------------------------------------


def test_main_runs_in_process_for_both_commands(
    intent_module: ModuleType,
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
    capsys: pytest.CaptureFixture[str],
) -> None:
    assert intent_module.main(["scaffold", *_scaffold_args(tmp_path / "repo")]) == 0
    assert intent_module.main(["scaffold", *_scaffold_args(tmp_path / "repo")]) == 2
    bad = [("--project", "A"), ("--audience", "x"), ("--port", "0")]
    for flag, value in bad:
        assert intent_module.main(["scaffold", *_scaffold_args(tmp_path / "other", **{flag: value})]) == 2
    deploy = str(tmp_path / "repo" / "deploy")
    monkeypatch.setenv("PATH", "/nonexistent")
    assert intent_module.main(["validate", deploy]) == 2
    assert intent_module.main(["validate", str(tmp_path / "missing")]) == 2
    assert intent_module.main(["validate", deploy, str(tmp_path / "missing")]) == 2
    for code, returned in ((0, 0), (1, 1), (2, 2), (3, 2)):
        (tmp_path / f"c{code}").mkdir()
        monkeypatch.setenv("PATH", _stub_deploy_kit(tmp_path / f"c{code}", code))
        assert intent_module.main(["validate", deploy]) == returned
    capsys.readouterr()


def test_running_the_script_as_main_exits_with_the_command_status(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setattr(sys, "argv", [str(SCRIPT), "scaffold", *_scaffold_args(tmp_path)])
    with pytest.raises(SystemExit) as raised:
        runpy.run_path(str(SCRIPT), run_name="__main__")
    assert raised.value.code == 0


# ---------------------------------------------------------------------------
# The skill names its sources and its two disagreements
# ---------------------------------------------------------------------------


def test_skill_md_names_the_deploy_kit_spec_paths_from_sources_yaml() -> None:
    text = SKILL_MD.read_text()
    sources = yaml.safe_load(SOURCES.read_text())["sources"]
    spec_paths = [s["path"] for s in sources if s["kind"] == "file" and s["repo"] == "JorisJonkers-dev/deploy-kit"]
    assert "spec/v1/10-project-intent.md" in spec_paths
    for path in spec_paths:
        assert path.split("/")[-1] in text, path
    assert "spec/v1/10-project-intent.md" in text
    assert "JorisJonkers-dev/github-workflows" in text


def test_sources_yaml_is_accepted_by_the_drift_checker() -> None:
    result = subprocess.run(
        [sys.executable, str(CHECKER), "--validate"],
        cwd=KIT_ROOT,
        capture_output=True,
        text=True,
        check=False,
    )
    assert result.returncode == 0, result.stdout + result.stderr


def test_skill_md_states_the_two_disagreements() -> None:
    text = SKILL_MD.read_text()
    assert "`platform/`" in text
    assert "`deploy/`" in text
    assert "1.0.0" in text
    assert "1.10.0" in text
    assert "Where the docs and reality disagree" in text


def test_skill_md_says_to_stop_and_name_the_doc_that_cannot_be_read() -> None:
    text = SKILL_MD.read_text()
    section = text[text.index("## When a doc cannot be read") :]
    assert "STOP" in section
    assert "Name the doc you could not read" in section


# ---------------------------------------------------------------------------
# Registry and install
# ---------------------------------------------------------------------------

AGENTS = ["claude", "codex", "vibe", "antigravity", "opencode"]


def test_registry_lists_the_skill_as_a_local_skill_for_every_agent() -> None:
    skills = render_registry.load_registry()["local_skills"]
    entry = next(s for s in skills if s["name"] == "estate-project-intent")
    assert entry["agents"] == AGENTS
    assert entry["surfaces"] == ["workstation"]
    assert entry["path"] == "skills/workstation/estate-project-intent"
    assert (KIT_ROOT / entry["path"] / "SKILL.md").is_file()


@pytest.fixture(scope="module")
def fresh(tmp_path_factory: pytest.TempPathFactory) -> Run:
    return setup_run(tmp_path_factory.mktemp("intent-fresh"), "Darwin")


def test_the_rendered_setup_installs_the_skill_with_its_script(fresh: Run) -> None:
    for agent in (".claude", ".codex"):
        installed = fresh.home / agent / "skills" / "estate-project-intent"
        assert (installed / "SKILL.md").read_bytes() == SKILL_MD.read_bytes()
        assert (installed / "scripts" / "intent.py").read_bytes() == SCRIPT.read_bytes()
        assert (installed / "sources.yaml").read_bytes() == SOURCES.read_bytes()
