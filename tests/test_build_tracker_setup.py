"""The build tracker's install: its skill in every agent home, its runner
pinned in manifest.yaml the way the critique runner is, and runnable with
`node` as it ships.

The setup assertions run the checked-in setup script against a temp HOME with
every external command stubbed (tests/setup_harness.py).
"""

from __future__ import annotations

import copy
import hashlib
import importlib.util
import json
import os
import re
import shutil
import subprocess
import sys
from pathlib import Path

import pytest
import yaml

KIT_ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(KIT_ROOT / "scripts"))

from setup_harness import Run, setup_run  # noqa: E402

SKILL = KIT_ROOT / "skills" / "workstation" / "build-tracker"
RUNNER = SKILL / "build-tracker.mjs"
HOMES = (".claude", ".codex", ".vibe", ".vibe-private", ".gemini/config", ".gemini/antigravity-cli")


def _registry() -> dict:
    return yaml.safe_load((KIT_ROOT / "registry" / "estate-tooling.yaml").read_text())


def _manifest() -> dict:
    return yaml.safe_load((KIT_ROOT / "manifest.yaml").read_text())


def _validator():
    spec = importlib.util.spec_from_file_location("validate_manifest", KIT_ROOT / "scripts" / "validate_manifest.py")
    assert spec and spec.loader
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


@pytest.fixture(scope="module")
def macos_run(tmp_path_factory: pytest.TempPathFactory) -> Run:
    return setup_run(tmp_path_factory.mktemp("build-tracker-macos"), "Darwin")


def test_the_build_tracker_skill_is_declared_for_claude_codex_and_the_other_agents() -> None:
    skill = next(s for s in _registry()["local_skills"] if s["name"] == "build-tracker")
    assert skill["path"] == "skills/workstation/build-tracker"
    assert {"claude", "codex"} <= set(skill["agents"])
    assert set(skill["agents"]) == {"claude", "codex", "vibe", "antigravity", "opencode"}
    assert skill["surfaces"] == ["workstation"]
    assert "pending" not in skill


def test_setup_installs_the_whole_skill_into_every_agent_home(macos_run: Run) -> None:
    want = sorted(p.relative_to(SKILL).as_posix() for p in SKILL.rglob("*") if p.is_file())
    assert want == ["SKILL.md", "build-tracker", "build-tracker.mjs"]
    for home in HOMES:
        dest = macos_run.home / home / "skills" / "build-tracker"
        got = sorted(p.relative_to(dest).as_posix() for p in dest.rglob("*") if p.is_file())
        assert got == want, home
        assert (dest / "build-tracker.mjs").read_bytes() == RUNNER.read_bytes(), home


def test_the_runner_is_pinned_in_the_manifest_like_the_critique_runner() -> None:
    files = {f["path"]: f for f in _manifest()["build_tracker"]["files"]}
    assert files["skills/workstation/build-tracker/build-tracker.mjs"]["mode"] == "0755"
    for path, entry in files.items():
        digest = hashlib.sha256((KIT_ROOT / path).read_bytes()).hexdigest()
        assert entry["sha256"] == digest, f"{path}: rebuild council/ts and update manifest.yaml build_tracker.files"


def test_the_manifest_pins_every_file_the_skill_ships_but_its_skill_md() -> None:
    _validator().validate_build_tracker(_manifest())


def test_a_runner_missing_from_the_pin_fails_validation() -> None:
    manifest = copy.deepcopy(_manifest())
    manifest["build_tracker"]["files"] = []
    with pytest.raises(AssertionError, match=re.escape("build_tracker.files mismatch")):
        _validator().validate_build_tracker(manifest)


@pytest.mark.skipif(shutil.which("node") is None, reason="node is not installed")
def test_the_shipped_runner_renders_a_page_with_node(tmp_path: Path) -> None:
    state = tmp_path / "state.json"
    state.write_text(
        json.dumps(
            {
                "version": 1,
                "spec": "acme/widgets#10",
                "title": "Widgets build",
                "tickets": {"1": {"title": "widget store", "blocked_by": [], "status": "todo"}},
            }
        )
    )
    page = tmp_path / "tracker.html"

    rendered = subprocess.run(
        ["node", str(RUNNER), "render", "--state", str(state), "--out", str(page)],
        capture_output=True,
        text=True,
        check=False,
    )
    assert rendered.returncode == 0, rendered.stderr
    assert json.loads(rendered.stdout)["status"] == "rendered"
    assert '<a href="https://github.com/acme/widgets/issues/1">#1</a>' in page.read_text()

    changed = subprocess.run(
        ["node", str(RUNNER), "set", "--state", str(state), "1=merged:abc1234"],
        capture_output=True,
        text=True,
        check=False,
    )
    assert changed.returncode == 0, changed.stderr
    assert json.loads(state.read_text())["tickets"]["1"] == {
        "title": "widget store",
        "blocked_by": [],
        "status": "merged",
        "note": "abc1234",
    }


@pytest.mark.skipif(shutil.which("node") is None, reason="node is not installed")
def test_the_shipped_runner_names_init_for_a_missing_state(tmp_path: Path) -> None:
    result = subprocess.run(
        ["node", str(RUNNER), "set", "--state", str(tmp_path / "state.json"), "1=merged"],
        capture_output=True,
        text=True,
        check=False,
    )
    assert result.returncode == 1
    assert json.loads(result.stdout)["code"] == "missing-state"
    assert "init" in result.stderr


@pytest.mark.skipif(shutil.which("node") is None or shutil.which("git") is None, reason="node or git is not installed")
def test_the_shipped_runner_refreshes_a_local_spec_as_flow_runs_it(tmp_path: Path) -> None:
    """#273: one refresh seeds the state, marks the landed tickets merged and renders the page."""
    repo = tmp_path / "widgets"
    (repo / "specs" / "issues").mkdir(parents=True)
    (repo / "specs" / "spec.md").write_text("# Local widgets\n")
    (repo / "specs" / "issues" / "01-store.md").write_text("# Widget store\n")
    (repo / "specs" / "issues" / "02-list.md").write_text("# Widget list\n\n## Blocked by\n\n- 01\n")
    subprocess.run(["git", "init", "-q", str(repo)], check=True)
    env = {"PATH": os.environ["PATH"], "HOME": str(tmp_path), "XDG_STATE_HOME": str(tmp_path / "state")}

    def refresh() -> dict:
        result = subprocess.run(
            ["node", str(RUNNER), "refresh", "--spec", "specs/spec.md", "--landed", "01"],
            capture_output=True,
            text=True,
            check=False,
            cwd=repo,
            env=env,
        )
        assert result.returncode == 0, result.stderr
        return json.loads(result.stdout)

    first = refresh()
    assert (first["status"], first["ran"], first["merged"]) == ("refreshed", "init", [1])
    tickets = json.loads(Path(first["state"]).read_text())["tickets"]
    assert {n: t["status"] for n, t in tickets.items()} == {"1": "merged", "2": "todo"}
    assert "Widget list" in Path(first["page"]).read_text()
    assert first["page"].startswith(str(tmp_path / "state"))

    second = refresh()
    assert (second["ran"], second["merged"], second["page"]) == ("sync", [], first["page"])


# ---------------------------------------------------------------------------
# The skill text: what makes any orchestrator keep the page current (#272)
# ---------------------------------------------------------------------------

GLOSSARY = KIT_ROOT / "CONTEXT.md"


def _skill_text() -> str:
    return (SKILL / "SKILL.md").read_text()


def _flat(text: str) -> str:
    return " ".join(text.split())


def _front() -> dict:
    return yaml.safe_load(_skill_text().split("---")[1])


def _section(text: str, heading: str) -> str:
    """From `heading` to the next heading of the same or a higher level."""
    start = text.index(heading)
    level = len(heading) - len(heading.lstrip("#"))
    rest = text[start + len(heading) :]
    end = re.search(rf"\n#{{1,{level}}} ", rest)
    return text[start:] if end is None else text[start : start + len(heading) + end.start()]


def _glossary_entries(section: str) -> dict[str, list[str]]:
    """Each **Term** of one CONTEXT.md section with its _Avoid_ words."""
    text = _section(GLOSSARY.read_text(), f"## {section}\n")
    entries: dict[str, list[str]] = {}
    for block in text.split("\n\n"):
        term = re.match(r"\*\*(.+?)\*\*:", block.strip())
        if not term:
            continue
        avoid = re.search(r"_Avoid_: (.+)", block)
        words = [] if not avoid else [w.strip() for w in re.sub(r"\(.*?\)", "", avoid.group(1)).split(",")]
        entries[term.group(1)] = [w for w in words if w]
    return entries


def test_the_skill_says_it_applies_at_the_start_of_every_multi_ticket_build() -> None:
    front = _front()
    assert set(front) == {"name", "description", "disable-model-invocation"}
    assert front["name"] == "build-tracker"
    assert front["description"].startswith("Use at the start of any multi-ticket build")
    for build in ("/implement-spec", "council run", "/flow"):
        assert build in front["description"], build


def test_the_skill_starts_the_build_tracker_with_init_and_the_specs_milestones() -> None:
    start = _flat(_section(_skill_text(), "### 1. "))
    for build in ("/implement-spec", "council run", "/flow"):
        assert build in start, build
    assert "`init --spec <ref>`" in start
    assert '`--milestone "<name>=<ticket>"`' in start
    assert "`sync --spec <ref>`" in start, "a build that already has a state continues with sync"
    assert "`path --spec <ref>`" in start


def test_the_skill_lists_every_command_the_runner_has() -> None:
    commands = _section(_skill_text(), "## Commands")
    for command in ("init", "sync", "path", "refresh", "render", "set"):
        assert f"build-tracker {command} " in commands, command
        assert f"- `{command}`" in commands, command


EVENTS = [
    ("An implementer started", "running", None),
    ("A merger started", "review", None),
    ("merged on the integration branch", "merged", "the merge commit"),
    ("waits on the owner", "owner", "what is needed"),
    ("paused or stopped", "todo", "why"),
]


@pytest.mark.parametrize(("event", "status", "note"), EVENTS)
def test_the_skill_maps_each_build_event_to_its_status(event: str, status: str, note: str | None) -> None:
    table = _section(_skill_text(), "### 3. ")
    row = next((line for line in table.splitlines() if line.startswith("|") and event in line), None)
    assert row is not None, event
    assert f"`{status}`" in row, row
    if note is not None:
        assert note in row, row


def test_only_the_orchestrator_writes_the_state() -> None:
    text = _flat(_section(_skill_text(), "## Who writes the state"))
    assert "Only the orchestrator writes the state file." in text
    assert "Implementers and mergers report back to it" in text
    assert "never runs `init`, `sync` or `set`" in text


def test_every_status_change_is_set_rendered_and_republished_at_once() -> None:
    step = _flat(_section(_skill_text(), "### 3. "))
    assert "run `set`, then `render`, then republish" in step
    assert "one change at a time" in step
    assert "Never batch status changes" in step
    publish = _flat(_section(_skill_text(), "## Republish"))
    assert "In Claude Code, the republish is the Artifact tool with the same file path every time" in publish
    assert "give the owner the page path" in publish
    assert "When a republish fails, tell the owner" in publish
    assert "the local page is still current" in publish


def test_the_worked_example_runs_from_init_to_a_first_render_and_one_status_change() -> None:
    example = _section(_skill_text(), "## Worked example")
    block = re.search(r"```sh\n(.*?)```", example, re.S)
    assert block, "the example is one sh block"
    commands = re.findall(r"^sh \S.*?build-tracker (\w+)", block.group(1), re.M)
    assert commands == ["init", "path", "render", "set", "render"], commands
    assert "--milestone " in block.group(1)
    assert "=running" in block.group(1)
    assert "Artifact tool" in example


def test_every_step_ends_on_a_done_when_line() -> None:
    text = _skill_text()
    headings = [line for line in text.splitlines() if line.startswith("### ")]
    numbers = [int(re.match(r"### (\d+)\. ", h).group(1)) for h in headings]
    assert numbers == list(range(1, len(headings) + 1)), headings
    for heading in headings:
        assert re.search(r"^Done when\b", _section(text, heading), re.M), heading


def test_the_glossary_defines_the_build_tracker_and_milestone() -> None:
    entries = _glossary_entries("Build tracker")
    assert {"Build tracker", "Milestone"} <= set(entries), entries


def test_the_skill_uses_the_glossarys_words_and_none_it_avoids() -> None:
    text = _skill_text()
    for term, avoided in _glossary_entries("Build tracker").items():
        assert term.lower() in text.lower(), term
        for word in avoided:
            assert not re.search(rf"\b{re.escape(word)}\b", text, re.I), f"{term}: the glossary avoids {word!r}"
    for term in ("orchestrator", "issue tracker"):
        assert term in text, term


def test_the_skill_never_says_tracker_alone_which_means_the_issue_tracker() -> None:
    bare = re.findall(r"(?<!build )(?<!issue )(?<!build-)\btracker\b", _skill_text(), re.I)
    assert bare == []


def test_the_skill_carries_no_assistant_attribution() -> None:
    text = _skill_text().lower()
    for mark in ("Co-Authored-By", "Generated with", "noreply@anthropic.com", "\U0001f916"):
        assert mark.lower() not in text, mark


POINTER_LINE = (
    "Multi-ticket builds (/implement-spec, a council run, a /flow feature in implementation):"
    " keep the build tracker current with the build-tracker skill."
)


def test_setup_names_the_build_tracker_in_the_agent_instructions_it_installs(macos_run: Run) -> None:
    """The AGENTS.md pointer setup writes into both Vibe homes and Antigravity."""
    for pointer in (".vibe/AGENTS.md", ".vibe-private/AGENTS.md", ".gemini/config/AGENTS.md"):
        lines = (macos_run.home / pointer).read_text().splitlines()
        assert POINTER_LINE in lines, pointer
        assert sum("build tracker" in line for line in lines) == 1, pointer


def test_the_running_and_review_rows_clear_the_note_the_table_calls_none() -> None:
    table = _section(_skill_text(), "### 3. ")
    for event, status in (("An implementer started", "running"), ("A merger started", "review")):
        row = next(line for line in table.splitlines() if line.startswith("|") and event in line)
        assert f"={status}:" in row, f"{event}: {row} keeps the previous note"


def test_every_running_and_review_example_in_the_skill_clears_the_note() -> None:
    """A `set` without a note keeps the old one, so a ticket that starts again would show a stale note."""
    kept = [m.group(0) for m in re.finditer(r"\b\S+=(?:running|review)(?!:)\S*", _skill_text())]
    assert kept == [], kept


def test_the_glossary_helper_reads_an_avoid_list_with_a_comma_inside_parentheses() -> None:
    assert _glossary_entries("Critique pipeline")["Critique stage"] == ["step", "phase"]


def test_continuing_an_earlier_build_says_what_happens_to_its_milestones() -> None:
    step = _section(_skill_text(), "### 1. ")
    assert "keeps the earlier state's milestones" in step
    assert "run `init` again" in step


# ---------------------------------------------------------------------------
# The Rust swap: the launcher prefers ~/.local/bin/build-tracker and falls back to
# the node bundle beside it; scripts/install-build-tracker-rust.sh builds it.
# Everything runs against temp directories with stub programs.
# ---------------------------------------------------------------------------

LAUNCHER = SKILL / "build-tracker"
INSTALLER = KIT_ROOT / "scripts" / "install-build-tracker-rust.sh"

TOOLS = ("bash", "sh", "env", "dirname", "cmp", "mkdir", "mktemp", "cp", "chmod", "mv", "rm", "cat", "printf", "sleep")


def _tools(directory: Path) -> str:
    """A directory holding only the basic tools (links to the host's), so a test's PATH is exactly what it names."""
    directory.mkdir(parents=True, exist_ok=True)
    for name in TOOLS:
        found = shutil.which(name)
        if found and not (directory / name).exists():
            (directory / name).symlink_to(found)
    return str(directory)


def _program(path: Path, body: str) -> Path:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(f"#!/usr/bin/env bash\n{body}\n")
    path.chmod(0o755)
    return path


@pytest.fixture
def home(tmp_path: Path) -> Path:
    root = tmp_path / "home"
    root.mkdir()
    _program(tmp_path / "bin" / "node", 'echo "node $*"')
    skill = root / ".claude" / "skills" / "build-tracker"
    skill.mkdir(parents=True)
    shutil.copy(LAUNCHER, skill / "build-tracker")
    (skill / "build-tracker.mjs").write_text("// bundle\n")
    return root


def _tracker(home: Path, *args: str, **env: str) -> subprocess.CompletedProcess[str]:
    full = {"HOME": str(home), "PATH": f"{home.parent / 'bin'}:{_tools(home.parent / 'tools')}", **env}
    launcher = home / ".claude" / "skills" / "build-tracker" / "build-tracker"
    return subprocess.run(["sh", str(launcher), *args], capture_output=True, text=True, check=False, env=full)


def _install_rust(home: Path) -> Path:
    return _program(home / ".local" / "bin" / "build-tracker", 'echo "rust $*"')


def test_without_the_rust_binary_the_launcher_runs_the_node_bundle_beside_it(home: Path) -> None:
    result = _tracker(home, "render", "--tickets", "t.yaml")
    assert result.returncode == 0, result.stderr
    assert result.stdout == f"node {home}/.claude/skills/build-tracker/build-tracker.mjs render --tickets t.yaml\n"


def test_with_the_rust_binary_installed_the_launcher_runs_it(home: Path) -> None:
    _install_rust(home)
    assert _tracker(home, "render", "--tickets", "t.yaml").stdout == "rust render --tickets t.yaml\n"


def test_an_unexecutable_rust_binary_falls_back_to_node(home: Path) -> None:
    _install_rust(home).chmod(0o644)
    assert _tracker(home, "path").stdout.startswith("node ")


def test_a_directory_where_the_binary_should_be_is_not_executed(home: Path) -> None:
    (home / ".local" / "bin" / "build-tracker").mkdir(parents=True)
    assert _tracker(home, "path").stdout.startswith("node ")


def test_the_environment_rolls_one_call_back_to_node(home: Path) -> None:
    _install_rust(home)
    assert _tracker(home, "path", AGENT_KIT_BUILD_TRACKER="node").stdout.startswith("node ")
    assert _tracker(home, "path", AGENT_KIT_BUILD_TRACKER="rust").stdout.startswith("rust path")


def test_the_pin_file_rolls_every_call_back_until_it_is_removed(home: Path) -> None:
    _install_rust(home)
    pin = home / ".config" / "agent-kit" / "build-tracker-node"
    pin.parent.mkdir(parents=True)
    pin.write_text("")
    assert _tracker(home, "path").stdout.startswith("node ")
    pin.unlink()
    assert _tracker(home, "path").stdout.startswith("rust path")


def test_another_rust_binary_can_be_named(home: Path, tmp_path: Path) -> None:
    other = _program(tmp_path / "other" / "build-tracker", 'echo "other $*"')
    assert _tracker(home, "path", AGENT_KIT_BUILD_TRACKER_BIN=str(other)).stdout == "other path\n"


def test_the_skills_run_the_launcher() -> None:
    for name in ("build-tracker", "flow"):
        text = (KIT_ROOT / "skills" / "workstation" / name / "SKILL.md").read_text()
        assert "node <this folder>/build-tracker.mjs" not in text
        assert "node <build-tracker dir>/build-tracker.mjs" not in text
    assert "sh <this folder>/build-tracker render --tickets" in (SKILL / "SKILL.md").read_text()


# ---------------------------------------------------------------------------
# The installer
# ---------------------------------------------------------------------------


class Kit:
    """A kit directory with a workspace manifest, and a stub cargo that 'builds' the binary."""

    def __init__(self, tmp_path: Path, home: Path) -> None:
        self.root = tmp_path / "kit"
        (self.root / "scripts").mkdir(parents=True)
        (self.root / "scripts" / "Cargo.toml").write_text("[workspace]\n")
        self.home = home
        self.tools = _tools(tmp_path / "tools")
        self.bin = tmp_path / "cargo-bin"
        self.dest = home / ".local" / "bin" / "build-tracker"
        self.cargo(0, "echo built")

    def cargo(self, code: int, binary_body: str | None, smoke: int = 0) -> None:
        build = f"mkdir -p {self.root}/scripts/target/release\n"
        if binary_body is not None:
            build += (
                f"printf '#!/usr/bin/env bash\\n[ \"$1\" = --help ] && exit {smoke}\\n{binary_body}\\n' "
                f"> {self.root}/scripts/target/release/build-tracker\n"
                f"chmod 755 {self.root}/scripts/target/release/build-tracker\n"
            )
        _program(self.bin / "cargo", f'echo "$@" > {self.root}/cargo-args\n{build}exit {code}')

    def run(self, *args: str, path: bool = True, **env: str) -> subprocess.CompletedProcess[str]:
        full = {
            "HOME": str(self.home),
            "KIT_ROOT": str(self.root),
            "PATH": f"{self.bin}:{self.tools}" if path else self.tools,
            **env,
        }
        return subprocess.run(["bash", str(INSTALLER), *args], capture_output=True, text=True, check=False, env=full)


@pytest.fixture
def kit(tmp_path: Path, home: Path) -> Kit:
    return Kit(tmp_path, home)


def test_the_installer_builds_and_installs_the_binary(kit: Kit) -> None:
    result = kit.run()
    assert result.returncode == 0, result.stderr
    assert f"installed {kit.dest}" in result.stdout
    args = (kit.root / "cargo-args").read_text().split()
    assert args[:2] == ["build", "--release"] and "--locked" in args
    assert args[args.index("-p") + 1] == "build-tracker"
    assert args[args.index("--manifest-path") + 1] == f"{kit.root}/scripts/Cargo.toml"
    assert os.access(kit.dest, os.X_OK)
    assert kit.run().stdout.count("already current") == 1


def test_the_installer_honours_xdg_bin_home(kit: Kit, tmp_path: Path) -> None:
    elsewhere = tmp_path / "elsewhere"
    assert kit.run(XDG_BIN_HOME=str(elsewhere)).returncode == 0
    assert os.access(elsewhere / "build-tracker", os.X_OK) and not kit.dest.exists()


def test_a_failed_build_leaves_the_installed_binary_alone(kit: Kit) -> None:
    kit.run()
    before = kit.dest.read_bytes()
    kit.cargo(101, "echo new")
    result = kit.run()
    assert result.returncode == 1
    assert "the installed build tracker is unchanged" in result.stdout
    assert kit.dest.read_bytes() == before


def test_a_binary_that_does_not_answer_help_is_not_installed(kit: Kit) -> None:
    kit.cargo(0, "echo new", smoke=1)
    result = kit.run()
    assert result.returncode == 1
    assert not kit.dest.exists()


def test_a_build_that_leaves_no_binary_is_a_failure(kit: Kit) -> None:
    kit.cargo(0, None)
    assert kit.run().returncode == 1


def test_an_install_that_cannot_land_is_reported(kit: Kit) -> None:
    kit.dest.parent.parent.mkdir(parents=True)
    kit.dest.parent.write_text("a file where the directory should be")
    result = kit.run()
    assert result.returncode == 1
    assert "could not create" in result.stdout


def test_an_unknown_argument_builds_nothing(kit: Kit) -> None:
    result = kit.run("--chekc")
    assert result.returncode == 2
    assert not (kit.root / "cargo-args").exists()


def test_without_cargo_a_workspace_or_with_the_pin_the_step_is_skipped(kit: Kit, home: Path) -> None:
    assert "cargo is not on PATH" in kit.run(path=False).stdout
    (kit.root / "scripts" / "Cargo.toml").unlink()
    assert "no cargo workspace" in kit.run().stdout
    (kit.root / "scripts" / "Cargo.toml").write_text("[workspace]\n")
    pin = home / ".config" / "agent-kit" / "build-tracker-node"
    pin.parent.mkdir(parents=True)
    pin.write_text("")
    result = kit.run()
    assert result.returncode == 0 and "the node bundle is pinned" in result.stdout
    assert not kit.dest.exists()


def test_check_builds_nothing(kit: Kit) -> None:
    result = kit.run("--check")
    assert result.returncode == 0 and "would build and install" in result.stdout
    assert not kit.dest.exists()
    assert not (kit.root / "scripts" / "target").exists()


def test_setup_runs_the_installer_and_the_bundle_ships_it() -> None:
    setup = (KIT_ROOT / "installer" / "setup-workstation.sh").read_text()
    assert "build_tracker_rust_setup" in setup
    assert "scripts/install-build-tracker-rust.sh" in setup
    assert (
        "scripts/install-build-tracker-rust.sh" in (KIT_ROOT / "scripts" / "publish-installer-artifacts.sh").read_text()
    )
