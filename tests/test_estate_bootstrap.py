"""The estate-bootstrap skill: its offline audit script, its named sources and its install."""

from __future__ import annotations

import contextlib
import importlib.util
import io
import json
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

SKILL = KIT_ROOT / "skills" / "workstation" / "estate-bootstrap"
SCRIPT = SKILL / "scripts" / "audit.py"
SKILL_MD = SKILL / "SKILL.md"
SOURCES = SKILL / "sources.yaml"
CHECKER = KIT_ROOT / "scripts" / "check_skill_sources.py"

LATEST = "a" * 40
STALE = "b" * 40
PIPELINE = "jobs:\n  done:\n    name: Pipeline Complete\n    runs-on: ubuntu-latest\n"


def _pin(ref: str, name: str = "go.yml") -> str:
    return f"      - uses: JorisJonkers-dev/github-workflows/.github/workflows/{name}@{ref}\n"


def _write(root: Path, rel: str, text: str) -> None:
    path = root / rel
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text)


def _clean(root: Path) -> Path:
    _write(root, ".github/workflows/ci.yml", PIPELINE + _pin(LATEST) + "      # uses: x/github-workflows@main\n")
    _write(root, "renovate.json", json.dumps({"extends": ["github>JorisJonkers-dev/renovate-config"]}))
    return root


def _run(*args: str) -> subprocess.CompletedProcess[str]:
    return subprocess.run([sys.executable, str(SCRIPT), *args], capture_output=True, text=True, check=False)


@pytest.fixture
def audit_module() -> ModuleType:
    spec = importlib.util.spec_from_file_location("estate_audit", SCRIPT)
    assert spec is not None
    assert spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


@pytest.fixture
def bad(tmp_path: Path) -> Path:
    root = tmp_path / "bad"
    body = "".join(
        [
            _pin("main", "a.yml"),
            _pin("v0.45.3", "b.yml"),
            _pin(STALE, "c.yml"),
            _pin(LATEST, "d.yml"),
        ]
    )
    _write(root, ".github/workflows/ci.yml", "jobs:\n  x:\n    steps:\n" + body)
    _write(root, ".github/workflows/notes.txt", _pin("main"))
    _write(root, "platform/fleet.yaml", "x: 1\n")
    _write(root, "deploy/app.yml", "token: packages-proxy-token x\nschema: deploy-config-schema\n")
    _write(root, "Taskfile.yml", "image: ExtraToast/app\n")
    _write(root, "node_modules/pkg/fleet.yml", "ignored\n")
    _write(root, ".github/ignored/vendor/x.yml", "ExtraToast/\n")
    return root


# ---------------------------------------------------------------------------
# The audit of a fixture repository
# ---------------------------------------------------------------------------


def test_a_clean_repository_prints_no_gaps(tmp_path: Path) -> None:
    result = _run(str(_clean(tmp_path / "ok")), "--latest-sha", LATEST)
    assert result.returncode == 0, result.stderr
    assert result.stdout == "no gaps\n"


def test_each_gap_is_listed_with_its_file_in_the_documented_line_shape(bad: Path) -> None:
    result = _run(str(bad), "--latest-sha", LATEST)
    assert result.returncode == 0, result.stderr
    lines = result.stdout.splitlines()
    ci = ".github/workflows/ci.yml"
    assert (
        f"{ci}: line 4: github-workflows .github/workflows/a.yml@main pinned to main "
        "(source: github-workflows README.md)"
    ) in lines
    assert any(line.startswith(f"{ci}: line 5:") and "v0.45.3 not pinned by 40-hex SHA" in line for line in lines)
    assert (
        f"{ci}: line 6: github-workflows .github/workflows/c.yml@{STALE} is not the latest release SHA {LATEST} "
        "(source: github-workflows latest release)"
    ) in lines
    assert not any("d.yml" in line for line in lines)
    assert not any("notes.txt" in line for line in lines)
    assert any(
        line.startswith(".github/workflows: no job named Pipeline Complete") and "ADR-0018" in line for line in lines
    )
    assert any(line.startswith("renovate.json: no renovate.json at the root") for line in lines)
    assert any(line.startswith("platform/: top-level platform/ directory") for line in lines)
    assert any(line.startswith("platform/fleet.yaml: fleet.yaml is flagged as retired (unverified") for line in lines)
    assert any(line.startswith("deploy/app.yml: line 1: retired reference 'packages-proxy-token'") for line in lines)
    assert any(line.startswith("deploy/app.yml: line 2: retired reference 'deploy-config-schema'") for line in lines)
    assert any(line.startswith("Taskfile.yml: line 1: retired reference 'ExtraToast/'") for line in lines)
    assert not any("node_modules" in line or "vendor" in line for line in lines)
    assert all(line.endswith(")") and " (source: " in line for line in lines)


def test_pins_are_not_compared_to_a_release_without_latest_sha(bad: Path) -> None:
    result = _run(str(bad))
    assert "is not the latest release SHA" not in result.stdout
    assert "pinned to main" in result.stdout


def test_json_output_is_a_list_of_file_gap_source(bad: Path) -> None:
    result = _run(str(bad), "--json")
    rows = json.loads(result.stdout)
    assert result.returncode == 0
    assert rows
    assert all(set(row) == {"file", "gap", "source"} for row in rows)
    platform = [r for r in rows if r["file"] == "platform/"]
    assert len(platform) == 1
    assert platform[0]["source"] == "workspace CLAUDE.md (Flux reads deploy/production)"
    fleet = [r for r in rows if r["file"] == "platform/fleet.yaml"]
    assert [r["source"] for r in fleet] == ["none (unverified)"]


def test_json_output_for_a_clean_repository_is_an_empty_list(tmp_path: Path) -> None:
    assert json.loads(_run(str(_clean(tmp_path / "ok")), "--json").stdout) == []


def test_missing_directory_exits_2(tmp_path: Path) -> None:
    result = _run(str(tmp_path / "nope"))
    assert result.returncode == 2
    assert "not a directory" in result.stderr
    assert result.stdout == ""


@pytest.mark.parametrize("sha", ["abc", "A" * 40, "g" * 40, "a" * 41])
def test_malformed_latest_sha_exits_2(tmp_path: Path, sha: str) -> None:
    result = _run(str(tmp_path), "--latest-sha", sha)
    assert result.returncode == 2
    assert "--latest-sha must be 40 lowercase hex characters" in result.stderr


# ---------------------------------------------------------------------------
# Edge cases of the individual checks
# ---------------------------------------------------------------------------


def test_two_pipeline_complete_jobs_are_a_gap(tmp_path: Path) -> None:
    root = _clean(tmp_path / "two")
    _write(root, ".github/workflows/other.yaml", PIPELINE.replace("Pipeline Complete", "'Pipeline Complete'"))
    out = _run(str(root)).stdout
    assert ".github/workflows: 2 jobs named Pipeline Complete; expected one" in out


def test_a_commented_pipeline_complete_does_not_count(tmp_path: Path) -> None:
    root = tmp_path / "c"
    _write(root, ".github/workflows/ci.yml", "# name: Pipeline Complete\n")
    _write(root, "renovate.json", json.dumps({"extends": ["github>JorisJonkers-dev/renovate-config"]}))
    assert "no job named Pipeline Complete" in _run(str(root)).stdout


def test_renovate_in_dot_github_is_found(tmp_path: Path) -> None:
    root = _clean(tmp_path / "r")
    (root / "renovate.json").rename(root / ".github" / "renovate.json")
    assert _run(str(root)).stdout == "no gaps\n"


def test_renovate_json_that_is_not_json_is_a_gap(tmp_path: Path) -> None:
    root = _clean(tmp_path / "r")
    _write(root, "renovate.json", "{nope")
    assert "renovate.json: could not confirm the preset" in _run(str(root)).stdout


@pytest.mark.parametrize("text", ['{"extends": ["config:base"]}', "[]", "{}"])
def test_renovate_without_the_preset_is_a_gap(tmp_path: Path, text: str) -> None:
    root = _clean(tmp_path / "r")
    _write(root, "renovate.json", text)
    assert 'renovate.json: does not extend "github>JorisJonkers-dev/renovate-config"' in _run(str(root)).stdout


def test_a_repository_without_workflows_is_missing_the_pipeline_job(tmp_path: Path) -> None:
    out = _run(str(tmp_path)).stdout
    assert ".github/workflows: no job named Pipeline Complete" in out


def _gaps(root: Path, *args: str) -> str:
    spec = importlib.util.spec_from_file_location("estate_audit_inproc", SCRIPT)
    assert spec is not None
    assert spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    buffer = io.StringIO()
    with contextlib.redirect_stdout(buffer):
        assert module.main([str(root), *args]) == 0
    return buffer.getvalue()


def test_pipeline_complete_counts_only_job_names_under_jobs(tmp_path: Path) -> None:
    root = _clean(tmp_path / "p")
    _write(
        root,
        ".github/workflows/other.yml",
        "name: Pipeline Complete\n"
        "jobs:\n"
        "  build:\n"
        "    name: Build\n"
        "    steps:\n"
        "      - uses: some/action@v1\n"
        "        with:\n"
        "          name: Pipeline Complete\n"
        "  done:\n"
        "    runs-on: x\n"
        "    name: Pipeline Complete # the check\n"
        "on: push\n"
        "env:\n"
        "  name: Pipeline Complete\n",
    )
    out = _gaps(root)
    assert "2 jobs named Pipeline Complete; expected one" in out


def test_a_top_level_or_input_name_is_not_a_pipeline_job(tmp_path: Path) -> None:
    root = tmp_path / "p"
    _write(
        root,
        ".github/workflows/ci.yml",
        "name: Pipeline Complete\njobs:\n  a:\n    steps:\n      - with:\n          name: Pipeline Complete\n",
    )
    _write(root, "renovate.json", json.dumps({"extends": ["github>JorisJonkers-dev/renovate-config"]}))
    assert ".github/workflows: no job named Pipeline Complete" in _gaps(root)


def test_a_pin_mentioned_only_in_a_trailing_comment_is_ignored(tmp_path: Path) -> None:
    root = _clean(tmp_path / "c")
    _write(
        root,
        ".github/workflows/more.yml",
        "jobs:\n  a:\n    steps:\n      - run: echo hi # uses JorisJonkers-dev/github-workflows/x.yml@main\n"
        + _pin(LATEST).rstrip("\n")
        + " # v1.2.3\n",
    )
    assert _gaps(root, "--latest-sha", LATEST) == "no gaps\n"


def test_pins_in_composite_actions_and_tmpl_templates_are_checked(tmp_path: Path) -> None:
    root = _clean(tmp_path / "t")
    _write(root, ".github/actions/x/action.yml", _pin("main"))
    _write(root, "templates/go/.github/workflows/ci.yml.tmpl", _pin("v1"))
    _write(root, "templates/go/readme.md", _pin("main"))
    _write(root, ".github/actions/node_modules/y.yml", _pin("main"))
    lines = _gaps(root).splitlines()
    assert any(ln.startswith(".github/actions/x/action.yml: line 1:") and "pinned to main" in ln for ln in lines)
    assert any(
        ln.startswith("templates/go/.github/workflows/ci.yml.tmpl: line 1:") and "not pinned" in ln for ln in lines
    )
    assert not any("readme.md" in ln or "node_modules" in ln for ln in lines)


def test_packages_proxy_token_is_found_without_a_colon_in_any_text_file(tmp_path: Path) -> None:
    root = _clean(tmp_path / "x")
    _write(root, "docs/notes.md", "uses packages-proxy-token here\n")
    _write(root, "src/blob.bin", "packages-proxy-token\0binary")
    _write(root, "src/huge.txt", "packages-proxy-token\n" + "x" * 1_100_000)
    _write(root, "docs/ExtraToast.md", "ExtraToast/ outside the scoped paths\n")
    (root / "src" / "dangling").symlink_to(root / "nowhere")
    out = _gaps(root)
    assert "docs/notes.md: line 1: retired reference 'packages-proxy-token'" in out
    assert "blob.bin" not in out
    assert "huge.txt" not in out
    assert "dangling" not in out
    assert "ExtraToast.md" not in out


def test_renovate_json_with_comments_is_read(tmp_path: Path) -> None:
    root = _clean(tmp_path / "r")
    _write(
        root,
        "renovate.json",
        '// header\n{\n  /* the preset */\n  "extends": ["github>JorisJonkers-dev/renovate-config"], // x\n'
        '  "note": "see https://example.com/a//b"\n}\n',
    )
    assert _gaps(root) == "no gaps\n"


def test_renovate_that_cannot_be_parsed_does_not_claim_invalid_json(tmp_path: Path) -> None:
    root = _clean(tmp_path / "r")
    _write(root, "renovate.json", "{nope")
    out = _gaps(root)
    assert "renovate.json: could not confirm the preset" in out
    assert "not valid JSON" not in out


@pytest.mark.parametrize(
    ("extends", "ok"),
    [
        ("github>JorisJonkers-dev/renovate-config", True),
        (["config:base", "github>JorisJonkers-dev/renovate-config#v1.2.3"], True),
        (["github>JorisJonkers-dev/renovate-config-other"], False),
        (["x github>JorisJonkers-dev/renovate-config"], False),
        ("github>JorisJonkers-dev/renovate-config-extra", False),
        ({"github>JorisJonkers-dev/renovate-config": 1}, False),
        ([5, None], False),
    ],
)
def test_renovate_extends_is_normalised_and_compared_by_entry(tmp_path: Path, extends: object, ok: bool) -> None:
    root = _clean(tmp_path / "r")
    _write(root, "renovate.json", json.dumps({"extends": extends}))
    assert (_gaps(root) == "no gaps\n") is ok


# ---------------------------------------------------------------------------
# In-process, so line coverage of audit.py is measurable
# ---------------------------------------------------------------------------


def test_main_runs_in_process(
    audit_module: ModuleType,
    bad: Path,
    tmp_path: Path,
    capsys: pytest.CaptureFixture[str],
) -> None:
    assert audit_module.main([str(_clean(tmp_path / "ok")), "--latest-sha", LATEST]) == 0
    assert capsys.readouterr().out == "no gaps\n"
    assert audit_module.main([str(bad), "--latest-sha", LATEST]) == 0
    assert "pinned to main" in capsys.readouterr().out
    assert audit_module.main([str(bad), "--json"]) == 0
    assert json.loads(capsys.readouterr().out)
    assert audit_module.main([str(tmp_path / "missing")]) == 2
    assert audit_module.main([str(bad), "--latest-sha", "x"]) == 2
    capsys.readouterr()


def test_edge_cases_in_process(
    audit_module: ModuleType,
    tmp_path: Path,
    capsys: pytest.CaptureFixture[str],
) -> None:
    def out(root: Path) -> str:
        assert audit_module.main([str(root)]) == 0
        return capsys.readouterr().out

    assert ".github/workflows: no job named Pipeline Complete" in out(tmp_path)
    two = _clean(tmp_path / "two")
    _write(two, ".github/workflows/other.yaml", PIPELINE)
    assert "2 jobs named Pipeline Complete; expected one" in out(two)
    broken = _clean(tmp_path / "broken")
    _write(broken, "renovate.json", "{nope")
    assert "renovate.json: could not confirm the preset" in out(broken)
    nopreset = _clean(tmp_path / "nopreset")
    _write(nopreset, "renovate.json", "[]")
    assert "does not extend" in out(nopreset)
    moved = _clean(tmp_path / "moved")
    (moved / "renovate.json").rename(moved / ".github" / "renovate.json")
    assert out(moved) == "no gaps\n"


def test_running_the_script_as_main_exits_with_the_status(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(sys, "argv", [str(SCRIPT), str(_clean(tmp_path / "ok"))])
    with pytest.raises(SystemExit) as raised:
        runpy.run_path(str(SCRIPT), run_name="__main__")
    assert raised.value.code == 0


# ---------------------------------------------------------------------------
# The skill names its disagreements, its rules and its sources
# ---------------------------------------------------------------------------


def _section(heading: str) -> str:
    text = SKILL_MD.read_text()
    start = text.index(heading)
    nxt = text.find("\n## ", start + 1)
    return text[start:] if nxt == -1 else text[start:nxt]


def _bullets() -> list[str]:
    section = _section("## 5. Where the docs and reality disagree")
    return [b for b in section.split("\n- ")[1:]]


def _bullet(key: str) -> str:
    found = [b for b in _bullets() if key in b]
    assert len(found) == 1, key
    return found[0]


@pytest.mark.parametrize(
    ("key", "followed", "audit"),
    [
        ("is not a GitHub template", "the live repository setting", "no,"),
        ("Workflow pins across the org", "the latest github-workflows release", "yes,"),
        ("migrates with goose", "`ADR-0021`", "no;"),
        ("run several CI jobs", "`ADR-0018`", "only the count"),
        ("recovery scripts", "what exists in `fleet-infra`", "no;"),
        ("deploy-config-schema trap", "deploy-kit", "yes,"),
        ("Coverage:", "`testing-standards` section 6 and `ADR-0022`", "no;"),
        ("under `inbox/`", "REPO_SETUP", "no."),
        ("`fleet.yaml` is flagged", "no document", "yes, with the source `none (unverified)`"),
    ],
)
def test_each_disagreement_bullet_names_the_side_followed_and_the_audit(key: str, followed: str, audit: str) -> None:
    bullet = _bullet(key)
    after_followed = bullet.split("Followed:", 1)[1]
    assert after_followed.split("The audit reports it:", 1)[0].count(followed) == 1
    assert "The audit reports it:" in bullet
    assert bullet.split("The audit reports it:", 1)[1].strip().startswith(audit)


def test_every_disagreement_bullet_is_covered_by_the_table() -> None:
    assert len(_bullets()) == 9
    assert all("Followed:" in b and "The audit reports it:" in b for b in _bullets())


def test_skill_md_points_at_docs_instead_of_restating_their_numbers() -> None:
    text = SKILL_MD.read_text()
    assert "14 days" not in text
    assert "80%" not in text
    assert "100%" not in text
    assert "v0.45" not in text
    assert "inside the window the standards give" in text
    assert "live" in text.split("## 1. Read first")[0]
    assert "the live read wins" in text


def test_skill_md_documents_the_audit_scope_and_the_platform_caveat() -> None:
    audit = _section("## 3. Audit")
    assert "`templates/**/*.tmpl`" in audit
    assert "`.github/actions/`" in audit
    assert "top-level `jobs:` key" in audit
    assert "could not confirm the preset" in audit
    assert "not meaningful for the `workspace` repository" in audit


def test_skill_md_labels_the_fleet_yaml_check_as_unverified() -> None:
    assert "treat it as unverified" in _bullet("`fleet.yaml` is flagged")
    assert "unverified" in SCRIPT.read_text()


def test_skill_md_files_issues_only_after_yes_and_never_opens_template_prs() -> None:
    text = SKILL_MD.read_text()
    rules = _section("## Rules")
    assert "File an issue only after the owner says yes" in rules
    assert "Never open a pull request against a template or library repository" in rules
    improve = _section("## 4. Improve")
    assert "Once the owner says yes, file it with `gh issue create`" in improve
    assert "Never open a pull request against a template or library repository" in improve
    assert "gh label list" in text


def test_skill_md_says_to_stop_and_name_the_doc_that_cannot_be_read() -> None:
    section = _section("## When a doc cannot be read")
    assert "stop" in section.lower()
    assert "Name the document" in section
    assert "Do not fall back to remembered content" in section


def test_sources_yaml_is_accepted_by_the_drift_checker() -> None:
    result = subprocess.run(
        [sys.executable, str(CHECKER), "--validate"],
        cwd=KIT_ROOT,
        capture_output=True,
        text=True,
        check=False,
    )
    assert result.returncode == 0, result.stdout + result.stderr


def test_sources_yaml_has_a_release_entry_for_github_workflows() -> None:
    sources = yaml.safe_load(SOURCES.read_text())["sources"]
    releases = [s for s in sources if s["kind"] == "release" and s["repo"] == "JorisJonkers-dev/github-workflows"]
    assert len(releases) == 1
    assert releases[0]["tag"].startswith("v")


# ---------------------------------------------------------------------------
# Registry and install
# ---------------------------------------------------------------------------

AGENTS = ["claude", "codex", "vibe", "antigravity", "opencode"]


def test_registry_lists_the_skill_as_a_local_skill_for_every_agent() -> None:
    skills = render_registry.load_registry()["local_skills"]
    entry = next(s for s in skills if s["name"] == "estate-bootstrap")
    assert entry["agents"] == AGENTS
    assert entry["surfaces"] == ["workstation"]
    assert entry["path"] == "skills/workstation/estate-bootstrap"
    assert (KIT_ROOT / entry["path"] / "SKILL.md").is_file()


@pytest.fixture(scope="module")
def fresh(tmp_path_factory: pytest.TempPathFactory) -> Run:
    return setup_run(tmp_path_factory.mktemp("bootstrap-fresh"), "Darwin")


def test_the_rendered_setup_installs_the_skill_with_its_script(fresh: Run) -> None:
    for agent in (".claude", ".codex"):
        installed = fresh.home / agent / "skills" / "estate-bootstrap"
        assert (installed / "SKILL.md").read_bytes() == SKILL_MD.read_bytes()
        assert (installed / "scripts" / "audit.py").read_bytes() == SCRIPT.read_bytes()
        assert (installed / "sources.yaml").read_bytes() == SOURCES.read_bytes()
