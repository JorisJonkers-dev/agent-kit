"""Gold Band's per-agent skill directories are converged onto the registry's skill sets.

Each test drives ``converge.main(["gold-band", ...])`` against a temporary
registry, settings file, keep file and kit of fake skills, with fake binaries on
PATH. The running guard is driven through ``AGENT_KIT_GOLD_BAND_RUNNING``.
"""

from __future__ import annotations

import json
import os
import shutil
import sys
from pathlib import Path

import pytest
import yaml

KIT_ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(KIT_ROOT / "scripts"))

import converge  # noqa: E402

# The fake kit: two skills in one set and one in another, so a set is never the
# whole kit and one id's directory can be told apart from its neighbour's.
SKILLS = {
    "estate-bootstrap": {"SKILL.md": "# Bootstrap\n", "notes.md": "bootstrap notes\n"},
    "estate-rollback": {"SKILL.md": "# Rollback\n"},
    "council": {"SKILL.md": "# Council\n", "council.mjs": "export {};\n"},
}

LOCAL_SKILLS = [
    {"name": "estate-bootstrap", "path": "skills/estate-bootstrap"},
    {"name": "estate-rollback", "path": "skills/estate-rollback"},
    {"name": "council", "path": "skills/council"},
]

SETS = {"estate": ["estate-bootstrap", "estate-rollback"], "writing": ["council"]}


def _agent(name: str, skill_set: str | list[str] | None, **extra: object) -> dict:
    agent: dict = {"id": name, "binary": name, "adapter": {"displayName": name.title()}}
    if skill_set is not None:
        agent["skill_set"] = skill_set
    agent.update(extra)
    return agent


def _registry(
    agents: list[dict] | None = None,
    skill_sets: dict[str, list[str]] | None = None,
    local_skills: list[dict] | None = None,
    retired: list[str] | None = None,
    allowed: tuple[str, ...] = ("opencode", "vibe", "claude"),
) -> dict:
    section: dict = {
        "agents": [_agent("opencode", "estate"), _agent("vibe", "writing")] if agents is None else agents,
        "skill_sets": {name: list(skills) for name, skills in (SETS if skill_sets is None else skill_sets).items()},
    }
    if retired is not None:
        section["retired_agents"] = retired
    return {
        "gold_band": section,
        "gold_band_instances": {"gold-band-personal": {"agents": list(allowed)}},
        "local_skills": [dict(skill) for skill in (LOCAL_SKILLS if local_skills is None else local_skills)],
    }


class Machine:
    """A temporary registry, settings file, keep file, kit and a PATH of fake binaries."""

    def __init__(
        self,
        tmp_path: Path,
        monkeypatch: pytest.MonkeyPatch,
        *,
        settings: dict | None = None,
        keep: list[str] | None = None,
        registry: dict | None = None,
        binaries: tuple[str, ...] = ("opencode", "vibe"),
        skills: dict[str, dict[str, str]] | None = None,
    ) -> None:
        self.bin = tmp_path / "bin"
        self.bin.mkdir()
        for name in binaries:
            self._fake(name, "#!/bin/sh\nexit 0\n")
        # Only the fakes: nothing else on the machine decides what is installed.
        monkeypatch.setenv("PATH", str(self.bin))
        self.kit = tmp_path / "kit"
        self.kit.mkdir()
        for name, files in (SKILLS if skills is None else skills).items():
            for relative, text in files.items():
                path = self.kit / "skills" / name / relative
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(text)
        self.settings = tmp_path / "gold-band" / "settings.json"
        if settings is not None:
            self.settings.parent.mkdir(parents=True, exist_ok=True)
            # Compact, so a run that rewrites it is visible in the bytes.
            self.settings.write_text(json.dumps(settings))
        self.keep = tmp_path / "keep.yaml"
        if keep is not None:
            self.keep.write_text(yaml.safe_dump({"keep": keep}))
        self.registry = tmp_path / "registry.yaml"
        self.registry.write_text(yaml.safe_dump(_registry() if registry is None else registry))

    def _fake(self, name: str, body: str) -> Path:
        path = self.bin / name
        path.write_text(body)
        path.chmod(0o755)
        return path

    @property
    def skills_root(self) -> Path:
        return self.settings.parent / "skill-sets"

    def directory(self, name: str) -> Path:
        return self.skills_root / name

    def run(self, *, check: bool = False, kit: bool = True, skills_root: bool = True) -> int:
        argv = ["gold-band", "--keep", str(self.keep)]
        if kit:
            argv += ["--kit", str(self.kit)]
        argv += ["--settings", str(self.settings), "--registry", str(self.registry)]
        if skills_root:
            argv += ["--skills-root", str(self.skills_root)]
        if check:
            argv.append("--check")
        return converge.main(argv)

    def agents(self) -> dict:
        return json.loads(self.settings.read_text())["agents"]


@pytest.fixture(autouse=True)
def _not_running(monkeypatch: pytest.MonkeyPatch) -> None:
    # The guard reads this first, so no test depends on the machine's processes.
    monkeypatch.setenv("AGENT_KIT_GOLD_BAND_RUNNING", "0")
    monkeypatch.delenv("GOLD_BAND_HOME", raising=False)


def test_exactly_the_declared_set_lands(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    machine = Machine(tmp_path, monkeypatch, settings={"agents": {}})
    assert machine.run() == 0
    target = machine.directory("opencode")
    assert sorted(child.name for child in target.iterdir()) == ["estate-bootstrap", "estate-rollback"]
    for name in ("estate-bootstrap", "estate-rollback"):
        for relative, text in SKILLS[name].items():
            assert (target / name / relative).read_text() == text


def test_an_id_naming_several_sets_gets_their_skills_merged(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    registry = _registry(agents=[_agent("opencode", ["estate", "writing", "estate"]), _agent("vibe", "writing")])
    machine = Machine(tmp_path, monkeypatch, settings={"agents": {}}, registry=registry)
    assert machine.run() == 0
    assert sorted(child.name for child in machine.directory("opencode").iterdir()) == [
        "council",
        "estate-bootstrap",
        "estate-rollback",
    ]
    assert sorted(child.name for child in machine.directory("vibe").iterdir()) == ["council"]
    assert machine.agents()["opencode"]["primaryAgentDir"] == str(machine.directory("opencode"))


def test_two_ids_with_different_sets_get_different_directories(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    machine = Machine(tmp_path, monkeypatch, settings={"agents": {}})
    assert machine.run() == 0
    assert sorted(child.name for child in machine.directory("opencode").iterdir()) == [
        "estate-bootstrap",
        "estate-rollback",
    ]
    assert sorted(child.name for child in machine.directory("vibe").iterdir()) == ["council"]


def test_the_entry_points_at_the_ids_own_directory(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    machine = Machine(tmp_path, monkeypatch, settings={"agents": {}})
    assert machine.run() == 0
    entry = machine.agents()["opencode"]
    assert entry["primaryAgentDir"] == str(machine.directory("opencode"))
    assert entry["compatibleAgentDirs"] == []
    # skill_set is registry-only: it is never written to settings.json.
    assert "skill_set" not in entry


def test_no_two_entries_share_a_primary_agent_dir(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    machine = Machine(tmp_path, monkeypatch, settings={"agents": {}})
    assert machine.run() == 0
    dirs = [entry["primaryAgentDir"] for entry in machine.agents().values() if "primaryAgentDir" in entry]
    assert len(dirs) == 2
    assert len(set(dirs)) == 2


def test_an_id_without_a_set_gets_no_directory_and_keeps_its_default_dirs(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    agent = {
        "id": "opencode",
        "binary": "opencode",
        "primaryAgentDir": ".opencode",
        "compatibleAgentDirs": [".agents"],
    }
    registry = _registry(agents=[agent])
    # A registry that declares no sets at all: the key may be absent.
    del registry["gold_band"]["skill_sets"]
    machine = Machine(tmp_path, monkeypatch, settings={"agents": {}}, registry=registry)
    assert machine.run() == 0
    assert machine.agents()["opencode"] == {"primaryAgentDir": ".opencode", "compatibleAgentDirs": [".agents"]}
    assert not machine.directory("opencode").exists()


def test_removing_a_skill_from_a_set_removes_it_from_the_directory(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    machine = Machine(tmp_path, monkeypatch, settings={"agents": {}})
    assert machine.run() == 0
    capsys.readouterr()
    machine.registry.write_text(
        yaml.safe_dump(_registry(skill_sets={"estate": ["estate-bootstrap"], "writing": ["council"]}))
    )
    assert machine.run() == 0
    assert "removed skills opencode" in capsys.readouterr().out
    assert sorted(child.name for child in machine.directory("opencode").iterdir()) == ["estate-bootstrap"]


def test_a_stray_file_in_the_directory_is_removed(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    machine = Machine(tmp_path, monkeypatch, settings={"agents": {}})
    assert machine.run() == 0
    capsys.readouterr()
    stray = machine.directory("opencode") / "stray.txt"
    stray.write_text("not a skill\n")
    assert machine.run() == 0
    assert "removed skills opencode" in capsys.readouterr().out
    assert not stray.exists()


def test_a_changed_file_is_rewritten(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    machine = Machine(tmp_path, monkeypatch, settings={"agents": {}})
    assert machine.run() == 0
    capsys.readouterr()
    tampered = machine.directory("opencode") / "estate-bootstrap" / "SKILL.md"
    tampered.write_text("tampered\n")
    assert machine.run() == 0
    assert "changed skills opencode" in capsys.readouterr().out
    assert tampered.read_text() == "# Bootstrap\n"


def test_a_second_run_writes_nothing(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    machine = Machine(tmp_path, monkeypatch, settings={"agents": {}})
    assert machine.run() == 0
    capsys.readouterr()
    stamps = {path: path.stat().st_mtime_ns for path in machine.skills_root.rglob("*")}
    assert machine.run() == 0
    out = capsys.readouterr().out
    assert "skills" not in out
    assert "settings unchanged" in out
    assert {path: path.stat().st_mtime_ns for path in machine.skills_root.rglob("*")} == stamps


def test_an_unchanged_settings_map_still_converges_the_skills(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    machine = Machine(tmp_path, monkeypatch, settings={"agents": {}})
    assert machine.run() == 0
    capsys.readouterr()
    shutil.rmtree(machine.directory("opencode"))
    assert machine.run() == 0
    out = capsys.readouterr().out
    assert "settings unchanged" in out
    assert "added skills opencode" in out
    assert (machine.directory("opencode") / "estate-bootstrap" / "SKILL.md").is_file()


def test_check_reports_the_work_and_writes_nothing(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    machine = Machine(tmp_path, monkeypatch, settings={"agents": {}})
    before = machine.settings.read_bytes()
    assert machine.run(check=True) == 0
    out = capsys.readouterr().out
    assert "added skills opencode" in out
    assert "added skills vibe" in out
    assert machine.settings.read_bytes() == before
    assert not machine.skills_root.exists()


def test_a_stale_id_directory_is_removed(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    machine = Machine(tmp_path, monkeypatch, settings={"agents": {}})
    stale = machine.directory("pycharm")
    stale.mkdir(parents=True)
    (stale / "SKILL.md").write_text("old\n")
    assert machine.run() == 0
    assert "removed skills pycharm" in capsys.readouterr().out
    assert not stale.exists()


def test_a_skipped_ids_directory_stays(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    machine = Machine(tmp_path, monkeypatch, settings={"agents": {}}, binaries=())
    kept = machine.directory("opencode")
    kept.mkdir(parents=True)
    (kept / "SKILL.md").write_text("mine\n")
    assert machine.run() == 0
    out = capsys.readouterr().out
    assert "skipped: opencode (opencode is not installed)" in out
    assert "removed skills opencode" not in out
    assert (kept / "SKILL.md").read_text() == "mine\n"


def test_a_kept_ids_directory_stays(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    machine = Machine(tmp_path, monkeypatch, settings={"agents": {}}, keep=["pycharm"])
    kept = machine.directory("pycharm")
    kept.mkdir(parents=True)
    (kept / "SKILL.md").write_text("mine\n")
    assert machine.run() == 0
    assert "removed skills pycharm" not in capsys.readouterr().out
    assert (kept / "SKILL.md").read_text() == "mine\n"


def test_a_symlink_in_a_skill_is_copied_as_a_symlink(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    machine = Machine(tmp_path, monkeypatch, settings={"agents": {}})
    (machine.kit / "skills" / "estate-bootstrap" / "shared.md").symlink_to("notes.md")
    assert machine.run() == 0
    copied = machine.directory("opencode") / "estate-bootstrap" / "shared.md"
    assert copied.is_symlink()
    assert os.readlink(copied) == "notes.md"
    # A link is compared as a link, so the second run finds nothing to do.
    capsys.readouterr()
    assert machine.run() == 0
    assert "changed skills" not in capsys.readouterr().out


def test_the_directories_are_created_0o700(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    machine = Machine(tmp_path, monkeypatch, settings={"agents": {}})
    assert machine.run() == 0
    assert machine.skills_root.stat().st_mode & 0o777 == 0o700
    assert machine.directory("opencode").stat().st_mode & 0o777 == 0o700


def test_the_skills_root_defaults_to_the_settings_directory(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    machine = Machine(tmp_path, monkeypatch, settings={"agents": {}})
    assert machine.run(skills_root=False) == 0
    assert (machine.settings.parent / "skill-sets" / "opencode" / "estate-bootstrap" / "SKILL.md").is_file()


def test_the_kit_defaults_to_the_kits_own_root(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    machine = Machine(
        tmp_path,
        monkeypatch,
        settings={"agents": {}},
        registry=_registry(
            agents=[_agent("opencode", "estate")],
            skill_sets={"estate": ["estate-bootstrap"]},
            local_skills=[{"name": "estate-bootstrap", "path": "skills/workstation/estate-bootstrap"}],
        ),
    )
    assert machine.run(kit=False) == 0
    assert (machine.directory("opencode") / "estate-bootstrap" / "SKILL.md").is_file()


def test_the_running_guard_writes_no_skill_directory(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    monkeypatch.setenv("AGENT_KIT_GOLD_BAND_RUNNING", "1")
    machine = Machine(tmp_path, monkeypatch, settings={"agents": {}})
    before = machine.settings.read_bytes()
    assert machine.run() == converge.GOLD_BAND_RUNNING_EXIT
    assert "is running" in capsys.readouterr().out
    assert machine.settings.read_bytes() == before
    assert not machine.skills_root.exists()


def test_skill_sets_that_are_not_a_mapping_are_refused(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    registry = _registry()
    registry["gold_band"]["skill_sets"] = ["estate"]
    machine = Machine(tmp_path, monkeypatch, settings={"agents": {}}, registry=registry)
    before = machine.settings.read_bytes()
    assert machine.run() == 1
    assert "gold-band-agents: skill_sets must be a mapping of set name to skills" in capsys.readouterr().out
    assert machine.settings.read_bytes() == before
    assert not machine.skills_root.exists()


def test_local_skills_that_are_not_a_list_cannot_converge(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    registry = _registry()
    registry["local_skills"] = "nope"
    machine = Machine(tmp_path, monkeypatch, settings={"agents": {}}, registry=registry)
    before = machine.settings.read_bytes()
    assert machine.run() == 3
    assert (
        f"gold-band-agents: cannot converge: {machine.registry}: local_skills must be a list of mappings"
        in capsys.readouterr().out
    )
    assert machine.settings.read_bytes() == before


NO_SKILL_MD = {
    "estate-bootstrap": {"notes.md": "no skill here\n"},
    "estate-rollback": {"SKILL.md": "# Rollback\n"},
    "council": {"SKILL.md": "# Council\n"},
}


@pytest.mark.parametrize(
    ("mutate", "line", "skills"),
    [
        (
            lambda r: r["gold_band"]["agents"][0].update(skill_set=3),
            "opencode: skill_set must be a set name or a list of set names",
            None,
        ),
        (
            lambda r: r["gold_band"]["agents"][0].update(skill_set=[]),
            "opencode: skill_set must be a set name or a list of set names",
            None,
        ),
        (
            lambda r: r["gold_band"]["agents"][0].update(skill_set=["estate", 3]),
            "opencode: skill_set must be a set name or a list of set names",
            None,
        ),
        (
            lambda r: r["gold_band"]["agents"][0].update(skill_set=["estate", "nope"]),
            "opencode: skill_set nope is not a declared set",
            None,
        ),
        (
            lambda r: (
                r["gold_band"]["skill_sets"].update(again=["estate-rollback"]),
                r["gold_band"]["agents"][0].update(skill_set=["estate", "again"]),
            ),
            "opencode: skill sets overlap on estate-rollback",
            None,
        ),
        (
            lambda r: r["gold_band"]["agents"][0].update(skill_set="nope"),
            "opencode: skill_set nope is not a declared set",
            None,
        ),
        (
            lambda r: r["gold_band"].update(skill_sets={}),
            "opencode: skill_set estate is not a declared set",
            None,
        ),
        (
            lambda r: r["gold_band"].update(skill_sets={"estate": "estate-bootstrap"}),
            "estate: skill set must be a list of skill names",
            None,
        ),
        (
            lambda r: r["gold_band"].update(skill_sets={"estate": ["estate-bootstrap", 1]}),
            "estate: skill set must be a list of skill names",
            None,
        ),
        (
            lambda r: r["gold_band"].update(skill_sets={"estate": ["estate-bootstrap", "estate-bootstrap"]}),
            "estate: duplicate skill estate-bootstrap",
            None,
        ),
        (
            lambda r: r["gold_band"].update(skill_sets={"estate": ["nope"]}),
            "estate: nope is not in local_skills",
            None,
        ),
        (
            lambda r: r["gold_band"].update(skill_sets={"Estate": ["estate-bootstrap"]}),
            "Estate: skill set name must match ^[a-z0-9]+(-[a-z0-9]+)*$",
            None,
        ),
        (
            lambda r: r["gold_band"].update(skill_sets={"estate": ["estate-bootstrap"]}),
            "estate: estate-bootstrap has no SKILL.md at skills/estate-bootstrap",
            NO_SKILL_MD,
        ),
        (
            lambda r: r["gold_band"]["agents"][0].update(primaryAgentDir=".opencode"),
            "opencode: skill_set replaces primaryAgentDir",
            None,
        ),
        (
            lambda r: r["gold_band"]["agents"][0].update(
                primaryAgentDir=".opencode", compatibleAgentDirs=[".agents"]
            ),
            "opencode: skill_set replaces primaryAgentDir, compatibleAgentDirs",
            None,
        ),
        (
            lambda r: r["gold_band"]["agents"][0].update(projectPrimaryAgentDir=".opencode"),
            "opencode: skill_set replaces projectPrimaryAgentDir",
            None,
        ),
        (
            lambda r: r["gold_band"].update(agents=[{"id": "OpenCode", "skill_set": "estate"}]),
            "OpenCode: id must match ^[a-z0-9]+(-[a-z0-9]+)*$",
            None,
        ),
    ],
)
def test_a_skill_set_that_breaks_a_rule_is_refused(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
    capsys: pytest.CaptureFixture[str],
    mutate,
    line: str,
    skills: dict | None,
) -> None:
    registry = _registry()
    mutate(registry)
    machine = Machine(tmp_path, monkeypatch, settings={"agents": {}}, registry=registry, skills=skills)
    before = machine.settings.read_bytes()
    assert machine.run() == 1
    assert f"gold-band-agents: {line}" in capsys.readouterr().out
    assert machine.settings.read_bytes() == before
    assert not machine.skills_root.exists()


def test_a_nested_directory_in_a_skill_is_copied_and_compared(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    skills = {**SKILLS, "council": {"SKILL.md": "# Council\n", "prompts/one.md": "one\n"}}
    machine = Machine(tmp_path, monkeypatch, settings={"agents": {}}, skills=skills)
    assert machine.run() == 0
    capsys.readouterr()
    nested = machine.directory("vibe") / "council" / "prompts" / "one.md"
    assert nested.read_text() == "one\n"
    nested.write_text("edited\n")
    assert machine.run() == 0
    assert "changed skills vibe" in capsys.readouterr().out
    assert nested.read_text() == "one\n"


def test_a_symlinked_id_directory_is_replaced_and_its_target_left_alone(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    machine = Machine(tmp_path, monkeypatch, settings={"agents": {}})
    outside = tmp_path / "documents"
    outside.mkdir()
    (outside / "precious.txt").write_text("keep me\n")
    machine.skills_root.mkdir(parents=True)
    machine.directory("opencode").symlink_to(outside)
    assert machine.run(check=True) == 0
    assert machine.directory("opencode").is_symlink()
    assert machine.run() == 0
    assert not machine.directory("opencode").is_symlink()
    assert (machine.directory("opencode") / "estate-bootstrap" / "SKILL.md").is_file()
    assert (outside / "precious.txt").read_text() == "keep me\n"
    assert not (outside / "estate-bootstrap").exists()


def test_a_file_where_an_id_directory_belongs_is_replaced(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    machine = Machine(tmp_path, monkeypatch, settings={"agents": {}})
    machine.skills_root.mkdir(parents=True)
    machine.directory("vibe").write_text("in the way\n")
    assert machine.run() == 0
    assert (machine.directory("vibe") / "council" / "SKILL.md").is_file()


def test_a_retired_ids_directory_goes_even_when_kept(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    registry = _registry(retired=["old"])
    machine = Machine(tmp_path, monkeypatch, settings={"agents": {}}, keep=["old"], registry=registry)
    (machine.directory("old")).mkdir(parents=True)
    assert machine.run() == 0
    assert not machine.directory("old").exists()


def test_a_skill_that_is_a_link_to_an_identical_directory_is_replaced_by_a_copy(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    machine = Machine(tmp_path, monkeypatch, settings={"agents": {}})
    assert machine.run() == 0
    skill = machine.directory("vibe") / "council"
    outside = tmp_path / "outside"
    shutil.copytree(skill, outside)
    shutil.rmtree(skill)
    skill.symlink_to(outside)
    assert machine.run() == 0
    assert not skill.is_symlink()
    assert (skill / "SKILL.md").is_file()
