"""Gold Band's ``context/profiles`` directory is converged onto the registry's declared roles.

Each test drives ``converge.main(["gold-band-roles", ...])`` against a temporary
critique directory, registry, closing file, keep file and profiles directory, so
nothing here depends on the real critique skill. The running guard is driven
through ``AGENT_KIT_GOLD_BAND_RUNNING``. One test renders the real six roles from
the kit's own registry and critique directory.
"""

from __future__ import annotations

import sys
from pathlib import Path

import pytest
import yaml

KIT_ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(KIT_ROOT / "scripts"))

import converge  # noqa: E402

# Every source carries the braces a template engine would eat: a JSON object, a
# doubled brace, a shell reference and a placeholder. The renderer must copy all
# four through byte for byte.
ALPHA = {
    "file": "alpha-critic",
    "id": "pf-alpha",
    "name": "Alpha critic",
    "summary": "Asks about {id} and {{x}}.",
    "source": "alpha.md",
    "lenses": ["first-lens", "second-lens"],
}

BETA = {
    "file": "beta-critic",
    "id": "pf-beta",
    "name": "Beta critic",
    "summary": "Checks ${HOME}.",
    "source": "beta.md",
    "lenses": [],
}

# beta.md has no trailing newline, so both branches of the section separator run.
SOURCES = {
    "roles/alpha.md": 'Alpha role: {"a": {1}} {{x}} ${HOME} {id}\n',
    "roles/beta.md": "Beta role: {id}",
    "prompts/preamble.md": "Preamble {{x}}\n",
    "prompts/output-contract.md": "Contract ${HOME}\n",
    "schemas/finding.schema.json": '{\n  "a": {1}\n}\n\n',
    "lenses/first-lens.md": "First lens {id}\n",
    "lenses/second-lens.md": "Second lens {{x}}\n",
}

CLOSING = "Closing {id} {{x}}\n"

EXPECTED_ALPHA = (
    "---\n"
    "id: pf-alpha\n"
    "name: Alpha critic\n"
    "summary: Asks about {id} and {{x}}.\n"
    "createdAt: 2025-01-01T00:00:00Z\n"
    "updatedAt: 2025-01-01T00:00:00Z\n"
    "dynamicTemplate: false\n"
    "---\n"
    "\n"
    'Alpha role: {"a": {1}} {{x}} ${HOME} {id}\n'
    "\n"
    "Preamble {{x}}\n"
    "\n"
    "Contract ${HOME}\n"
    "\n"
    '```json\n{\n  "a": {1}\n}\n```\n'
    "\n"
    "First lens {id}\n"
    "\n"
    "Second lens {{x}}\n"
    "\n"
    "Closing {id} {{x}}\n"
    "\n"
)

EXPECTED_BETA = (
    "---\n"
    "id: pf-beta\n"
    "name: Beta critic\n"
    "summary: Checks ${HOME}.\n"
    "createdAt: 2025-01-01T00:00:00Z\n"
    "updatedAt: 2025-01-01T00:00:00Z\n"
    "dynamicTemplate: false\n"
    "---\n"
    "\n"
    "Beta role: {id}\n"
    "\n"
    "Preamble {{x}}\n"
    "\n"
    "Contract ${HOME}\n"
    "\n"
    '```json\n{\n  "a": {1}\n}\n```\n'
    "\n"
    "Closing {id} {{x}}\n"
    "\n"
)


def _registry(roles: list[dict] | None = None, retired: list[str] | None = None) -> dict:
    section: dict = {"roles_source": "critique", "roles": [ALPHA, BETA] if roles is None else roles}
    if retired is not None:
        section["retired_roles"] = retired
    return {"gold_band": section}


class Machine:
    """A temporary critique directory, registry, closing file, keep file and profiles directory."""

    def __init__(
        self,
        tmp_path: Path,
        monkeypatch: pytest.MonkeyPatch,
        *,
        roles: list[dict] | None = None,
        retired: list[str] | None = None,
        keep: list[str] | None = None,
        registry: dict | None = None,
        sources: dict[str, str] | None = None,
        closing: str = CLOSING,
    ) -> None:
        self.critique = tmp_path / "critique"
        for relative, text in (SOURCES if sources is None else sources).items():
            path = self.critique / relative
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(text)
        self.closing = tmp_path / "closing.md"
        self.closing.write_text(closing)
        self.profiles = tmp_path / "profiles"
        self.keep = tmp_path / "keep.yaml"
        self.keep.write_text(yaml.safe_dump({"keep": keep or []}))
        self.registry = tmp_path / "registry.yaml"
        self.registry.write_text(yaml.safe_dump(_registry(roles, retired) if registry is None else registry))

    def profile(self, stem: str) -> Path:
        return self.profiles / f"{stem}.md"

    def hand_made(self, stem: str, text: str) -> Path:
        """A profile the registry does not declare, as a person would leave one."""
        self.profiles.mkdir(parents=True, exist_ok=True)
        path = self.profile(stem)
        path.write_text(text)
        return path

    def run(
        self,
        *,
        check: bool = False,
        profiles: bool = True,
        registry: bool = True,
        critique: bool = True,
        closing: bool = True,
    ) -> int:
        argv = ["gold-band-roles", "--keep", str(self.keep)]
        if profiles:
            argv += ["--profiles", str(self.profiles)]
        if registry:
            argv += ["--registry", str(self.registry)]
        if critique:
            argv += ["--critique", str(self.critique)]
        if closing:
            argv += ["--closing", str(self.closing)]
        if check:
            argv.append("--check")
        return converge.main(argv)


@pytest.fixture(autouse=True)
def _not_running(monkeypatch: pytest.MonkeyPatch) -> None:
    # The guard reads this first, so no test depends on the machine's processes.
    monkeypatch.setenv("AGENT_KIT_GOLD_BAND_RUNNING", "0")
    monkeypatch.delenv("GOLD_BAND_HOME", raising=False)


def test_a_role_renders_its_sources_verbatim_and_in_order(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    machine = Machine(tmp_path, monkeypatch)
    assert machine.run() == 0
    # Byte for byte: no templating, so every brace survives as written.
    assert machine.profile("alpha-critic").read_text() == EXPECTED_ALPHA
    assert machine.profile("beta-critic").read_text() == EXPECTED_BETA


def test_the_first_run_adds_every_declared_role(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    machine = Machine(tmp_path, monkeypatch)
    assert machine.run() == 0
    out = capsys.readouterr().out
    assert "added alpha-critic" in out
    assert "added beta-critic" in out
    assert "removed" not in out


def test_a_second_run_changes_nothing(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    machine = Machine(tmp_path, monkeypatch)
    assert machine.run() == 0
    capsys.readouterr()
    before = {path.name: path.read_bytes() for path in machine.profiles.iterdir()}
    assert machine.run() == 0
    assert "profiles unchanged" in capsys.readouterr().out
    assert {path.name: path.read_bytes() for path in machine.profiles.iterdir()} == before


def test_a_declared_profile_whose_content_differs_is_rewritten(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    machine = Machine(tmp_path, monkeypatch)
    machine.hand_made("alpha-critic", "stale\n")
    assert machine.run() == 0
    out = capsys.readouterr().out
    assert "changed alpha-critic" in out
    assert "added alpha-critic" not in out
    assert machine.profile("alpha-critic").read_text() == EXPECTED_ALPHA


def test_an_undeclared_profile_is_removed(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    machine = Machine(tmp_path, monkeypatch)
    machine.hand_made("notes", "hand made\n")
    assert machine.run() == 0
    assert "removed notes (undeclared)" in capsys.readouterr().out
    assert not machine.profile("notes").exists()


def test_an_undeclared_profile_the_keep_file_names_by_stem_stays(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    machine = Machine(tmp_path, monkeypatch, keep=["notes"])
    machine.hand_made("notes", "hand made\n")
    assert machine.run() == 0
    assert "removed" not in capsys.readouterr().out
    assert machine.profile("notes").read_text() == "hand made\n"


def test_an_undeclared_profile_the_keep_file_names_with_its_extension_stays(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    machine = Machine(tmp_path, monkeypatch, keep=["notes.md"])
    machine.hand_made("notes", "hand made\n")
    assert machine.run() == 0
    assert "removed" not in capsys.readouterr().out
    assert machine.profile("notes").read_text() == "hand made\n"


def test_a_retired_stem_is_removed_even_when_the_keep_file_names_it(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    machine = Machine(tmp_path, monkeypatch, retired=["notes"], keep=["notes"])
    machine.hand_made("notes", "hand made\n")
    assert machine.run() == 0
    assert "removed notes (retired)" in capsys.readouterr().out
    assert not machine.profile("notes").exists()


def test_a_retired_id_is_removed_even_when_the_keep_file_names_it(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    machine = Machine(tmp_path, monkeypatch, retired=["pf-old"], keep=["notes"])
    machine.hand_made("notes", "---\nid: pf-old\n---\n")
    assert machine.run() == 0
    assert "removed notes (retired)" in capsys.readouterr().out
    assert not machine.profile("notes").exists()


def test_a_builtin_profile_is_never_touched(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    machine = Machine(tmp_path, monkeypatch)
    assert machine.run() == 0
    capsys.readouterr()
    body = "---\nid: pf-builtin-summariser\n---\n"
    builtin = machine.hand_made("pf-builtin-summariser", body)
    assert machine.run() == 0
    assert "profiles unchanged" in capsys.readouterr().out
    assert builtin.read_text() == body


def test_check_prints_the_changes_and_creates_no_directory(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    machine = Machine(tmp_path, monkeypatch)
    assert machine.run(check=True) == 0
    out = capsys.readouterr().out
    assert "added alpha-critic" in out
    assert "added beta-critic" in out
    assert not machine.profiles.exists()


def test_check_reports_a_removal_and_keeps_the_file(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    machine = Machine(tmp_path, monkeypatch)
    machine.hand_made("notes", "hand made\n")
    assert machine.run(check=True) == 0
    assert "removed notes (undeclared)" in capsys.readouterr().out
    assert machine.profile("notes").read_text() == "hand made\n"


def test_the_running_guard_refuses_to_write(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    monkeypatch.setenv("AGENT_KIT_GOLD_BAND_RUNNING", "1")
    machine = Machine(tmp_path, monkeypatch)
    assert machine.run() == converge.GOLD_BAND_RUNNING_EXIT
    out = capsys.readouterr().out
    assert "gold-band-roles: Gold Band instance gold-band-personal" in out
    assert "quit it and re-run setup (nothing was written)" in out
    assert not machine.profiles.exists()


def test_check_is_not_refused_while_gold_band_runs(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    monkeypatch.setenv("AGENT_KIT_GOLD_BAND_RUNNING", "1")
    machine = Machine(tmp_path, monkeypatch)
    assert machine.run(check=True) == 0
    assert "added alpha-critic" in capsys.readouterr().out
    assert not machine.profiles.exists()


def test_a_missing_source_file_exits_3_and_removes_nothing(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    machine = Machine(tmp_path, monkeypatch)
    machine.hand_made("notes", "hand made\n")
    missing = machine.critique / "roles" / "beta.md"
    missing.unlink()
    assert machine.run() == 3
    assert f"gold-band-roles: cannot converge: {missing}:" in capsys.readouterr().out
    assert machine.profile("notes").read_text() == "hand made\n"
    assert not machine.profile("alpha-critic").exists()


def test_an_undecodable_source_file_exits_3(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    machine = Machine(tmp_path, monkeypatch)
    (machine.critique / "roles" / "beta.md").write_bytes(b"\xff\xfe")
    assert machine.run() == 3
    assert "gold-band-roles: cannot converge:" in capsys.readouterr().out
    assert not machine.profiles.exists()


@pytest.mark.parametrize(
    ("roles", "retired", "line"),
    [
        ([{**ALPHA, "file": "Alpha"}], None, "Alpha: file must match ^[a-z0-9]+(-[a-z0-9]+)*$"),
        ([{key: value for key, value in ALPHA.items() if key != "file"}], None, "?: file must match"),
        ([{**ALPHA, "id": "alpha"}], None, "alpha: id must start with pf-"),
        ([{key: value for key, value in ALPHA.items() if key != "id"}], None, "alpha-critic: id must start with pf-"),
        ([{**ALPHA, "id": "pf-builtin-alpha"}], None, "pf-builtin-alpha: id must not name a built-in profile"),
        ([{**ALPHA, "file": "pf-builtin-alpha"}], None, "pf-builtin-alpha: file must not name a built-in profile"),
        ([{**ALPHA, "summary": "Asks: why #1"}], None, "pf-alpha: summary must be a plain single-line scalar"),
        ([{**ALPHA, "name": "[x]"}], None, "pf-alpha: name must be a plain single-line scalar"),
        ([{**ALPHA, "name": 3}], None, "pf-alpha: name must be a string"),
        (
            [{key: value for key, value in ALPHA.items() if key != "summary"}],
            None,
            "pf-alpha: summary must be a string",
        ),
        ([{**ALPHA, "source": None}], None, "pf-alpha: source must be a string"),
        ([{**ALPHA, "lenses": "first-lens"}], None, "pf-alpha: lenses must be a list of names"),
        ([{**ALPHA, "lenses": [1]}], None, "pf-alpha: lenses must be a list of names"),
        ([{**ALPHA, "colour": "red"}], None, "pf-alpha: unknown field colour"),
        ([ALPHA, {**BETA, "file": "alpha-critic"}], None, "alpha-critic: duplicate file"),
        ([ALPHA, {**BETA, "id": "pf-alpha"}], None, "pf-alpha: duplicate id"),
        ([ALPHA], ["alpha-critic"], "pf-alpha: declared and retired at once"),
        ([ALPHA], ["pf-alpha"], "pf-alpha: declared and retired at once"),
    ],
)
def test_a_declared_role_that_breaks_a_rule_is_refused(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
    capsys: pytest.CaptureFixture[str],
    roles: list[dict],
    retired: list[str] | None,
    line: str,
) -> None:
    machine = Machine(tmp_path, monkeypatch, roles=roles, retired=retired)
    assert machine.run() == 1
    assert f"gold-band-roles: {line}" in capsys.readouterr().out
    assert not machine.profiles.exists()


def test_an_unreadable_registry_exits_3(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    machine = Machine(tmp_path, monkeypatch)
    machine.registry.unlink()
    assert machine.run() == 3
    out = capsys.readouterr().out
    assert "gold-band-roles: cannot converge:" in out
    assert "cannot be read" in out
    assert not machine.profiles.exists()


@pytest.mark.parametrize(
    ("body", "suffix"),
    [
        ("- 1\n", " is not a mapping"),
        ("gold_band_instances: {}\n", " has no gold_band mapping"),
        ("gold_band:\n  roles: x\n", ": gold_band.roles must be a list of mappings"),
        ("gold_band:\n  roles: [1]\n", ": gold_band.roles must be a list of mappings"),
        ("gold_band:\n  roles: []\n  retired_roles: x\n", ": gold_band.retired_roles must be a list of stems or ids"),
        ("gold_band:\n  roles: []\n  roles_source: 3\n", ": gold_band.roles_source must be a path"),
        ("gold_band: [\n", " cannot be read"),
    ],
)
def test_a_malformed_registry_exits_3(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str], body: str, suffix: str
) -> None:
    machine = Machine(tmp_path, monkeypatch)
    machine.registry.write_text(body)
    assert machine.run() == 3
    assert f"gold-band-roles: cannot converge: {machine.registry}{suffix}" in capsys.readouterr().out
    assert not machine.profiles.exists()


def test_a_registry_without_a_roles_source_cannot_converge(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    machine = Machine(tmp_path, monkeypatch, registry={"gold_band": {"roles": [ALPHA]}})
    assert machine.run(critique=False) == 3
    assert (
        f"gold-band-roles: cannot converge: {machine.registry}: gold_band.roles_source must be a path"
        in capsys.readouterr().out
    )


def test_a_directory_named_like_a_profile_and_a_non_markdown_file_are_ignored(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    machine = Machine(tmp_path, monkeypatch)
    assert machine.run() == 0
    capsys.readouterr()
    (machine.profiles / "notes.md").mkdir()
    (machine.profiles / "notes.txt").write_text("hand made\n")
    assert machine.run() == 0
    assert "profiles unchanged" in capsys.readouterr().out
    assert (machine.profiles / "notes.md").is_dir()
    assert (machine.profiles / "notes.txt").read_text() == "hand made\n"


def test_an_undecodable_profile_has_no_id_and_is_removed(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    machine = Machine(tmp_path, monkeypatch)
    machine.profiles.mkdir()
    machine.profile("notes").write_bytes(b"\xff\xfe")
    assert machine.run() == 0
    assert "removed notes (undeclared)" in capsys.readouterr().out
    assert not machine.profile("notes").exists()


@pytest.mark.parametrize(
    "body",
    [
        "",
        "no front matter\n",
        "---\nname: x\n---\n",
        "---\nname: x\n",
        "---\nid:\n---\n",
        "---\nid\n---\n",
        "---\nname: x\nid: pf-other\n---\n",
    ],
)
def test_a_profile_without_a_usable_id_is_undeclared(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str], body: str
) -> None:
    machine = Machine(tmp_path, monkeypatch)
    machine.hand_made("notes", body)
    assert machine.run() == 0
    assert "removed notes (undeclared)" in capsys.readouterr().out
    assert not machine.profile("notes").exists()


def test_an_oserror_on_write_exits_3(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    machine = Machine(tmp_path, monkeypatch)

    def refuse(*_args: object, **_kwargs: object) -> None:
        raise OSError("disk full")

    monkeypatch.setattr(converge.os, "replace", refuse)
    assert machine.run() == 3
    assert "gold-band-roles: cannot converge: disk full" in capsys.readouterr().out


def test_the_profiles_directory_is_created_private(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    machine = Machine(tmp_path, monkeypatch)
    assert machine.run() == 0
    assert machine.profiles.stat().st_mode & 0o777 == 0o700


def test_the_profiles_path_defaults_to_gold_band_home(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    home = tmp_path / "gb"
    monkeypatch.setenv("GOLD_BAND_HOME", str(home))
    machine = Machine(tmp_path, monkeypatch)
    assert machine.run(profiles=False) == 0
    assert (home / "context" / "profiles" / "alpha-critic.md").read_text() == EXPECTED_ALPHA


def test_the_real_registry_renders_the_six_declared_roles(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    home = tmp_path / "gb"
    monkeypatch.setenv("GOLD_BAND_HOME", str(home))
    keep = tmp_path / "keep.yaml"
    keep.write_text("keep: []\n")
    # No --registry, --critique or --closing: the kit's own files are the defaults.
    assert converge.main(["gold-band-roles", "--keep", str(keep)]) == 0
    profiles = home / "context" / "profiles"
    section = yaml.safe_load((KIT_ROOT / "registry" / "estate-tooling.yaml").read_text())["gold_band"]
    critique = KIT_ROOT / section["roles_source"]
    closing = (KIT_ROOT / "scripts" / "gold-band" / "roles-closing.md").read_text()
    roles = section["roles"]
    assert len(roles) == 6
    for role in roles:
        text = (profiles / f"{role['file']}.md").read_text()
        assert "dynamicTemplate: false" in text
        assert f"id: {role['id']}\n" in text
        fragments = [
            (critique / "roles" / role["source"]).read_text(),
            (critique / "prompts" / "preamble.md").read_text(),
            (critique / "prompts" / "output-contract.md").read_text(),
            (critique / "schemas" / "finding.schema.json").read_text().rstrip(),
            *[(critique / "lenses" / f"{lens}.md").read_text() for lens in role["lenses"]],
            closing,
        ]
        position = -1
        for fragment in fragments:
            found = text.find(fragment, position + 1)
            assert found > position, f"{role['file']}: {fragment[:40]!r} is missing or out of order"
            position = found
    capsys.readouterr()
    before = {path.name: path.read_bytes() for path in profiles.iterdir()}
    assert converge.main(["gold-band-roles", "--keep", str(keep)]) == 0
    assert "profiles unchanged" in capsys.readouterr().out
    assert {path.name: path.read_bytes() for path in profiles.iterdir()} == before


def test_a_declared_stem_holding_a_builtin_profile_is_refused(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    machine = Machine(tmp_path, monkeypatch)
    body = "---\nid: pf-builtin-summariser\n---\n"
    builtin = machine.hand_made("alpha-critic", body)
    assert machine.run() == 1
    assert "alpha-critic: the existing profile is a built-in; nothing was written" in capsys.readouterr().out
    assert builtin.read_text() == body
    assert not machine.profile("beta-critic").exists()


def test_a_failed_write_leaves_no_staged_file(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    machine = Machine(tmp_path, monkeypatch)

    def refuse(*_args: object, **_kwargs: object) -> None:
        raise OSError("disk full")

    monkeypatch.setattr(converge.os, "replace", refuse)
    assert machine.run() == 3
    assert list(machine.profiles.glob(".*agent-kit-tmp")) == []
