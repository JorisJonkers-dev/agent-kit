"""The Gold Band profiles: setup converges the critique roles onto them (#345).

Gold Band calls a critic a profile: a Markdown file with YAML front matter
it reads from ~/.gold-band/context/profiles (v0.19.1, src/app/profiles.rs --
never ~/.gold-band/presets/profiles). Setup's gold_band_roles step runs
converge.py's gold-band-roles surface, which runs the gold-band-roles
crate's converge on the critique skill's own files in this kit and that
directory: the six rendered profiles matching their sources rule for
rule, a hand-made profile (no front matter id) and a Gold Band built-in
never touched, a stale generated one removed, nothing written while Gold
Band runs and nothing at all where the app is not installed.

Each case runs the whole checked-in setup script against a temporary home
(tests/setup_harness.py), with a ``uv`` stub that runs converge.py for
real for every Gold Band surface, so the crate's own converge -- not the
stub's -- decides the directory the way it does on a workstation. The
crate builds the way the canary tests build it, and the run matches a
workstation's: the agent map is converged first, which is what makes
~/.gold-band owner-only before the profiles step creates its directory.
"""

from __future__ import annotations

import shutil
import subprocess
import sys
from pathlib import Path

import pytest

KIT_ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(KIT_ROOT / "scripts"))
sys.path.insert(0, str(Path(__file__).resolve().parent))

import converge  # noqa: E402
from setup_harness import setup_run  # noqa: E402


@pytest.fixture(scope="module")
def built_crate() -> None:
    """Build the workspace's crates, the way setup's own skip line asks for (docs/RUST.md)."""
    if shutil.which("cargo") is None:
        pytest.skip("Rust toolchain (cargo) is absent: gold-band-roles cannot be built and run")
    build = subprocess.run(
        ["cargo", "build", "--release", "--manifest-path", str(KIT_ROOT / "scripts" / "Cargo.toml")],
        capture_output=True,
        text=True,
        check=False,
    )
    assert build.returncode == 0, build.stderr


# The six rendered profiles, file name to front matter id, in the order the
# crate's ROLES table lists them (scripts/crates/gold-band-roles/src/lib.rs):
# the decision-gap critic's id is pf-gap, the role it names -- the crate is
# the record of the exact ids.
ROLES: dict[str, str] = {
    "decision-gap-critic.md": "pf-gap",
    "spec-red-team.md": "pf-spec-redteam",
    "ticket-linter.md": "pf-ticket-linter",
    "diff-reviewer.md": "pf-diff-reviewer",
    "feature-reviewer.md": "pf-feature-reviewer",
    "adversarial-verifier.md": "pf-adversarial-verifier",
}

# A hand-made profile: no front matter id, so convergence never touches it.
HAND_MADE = "# My own role\n\nNo front matter id, so convergence leaves this file alone.\n"


def _gate(root: Path, running: str = "0") -> dict[str, str]:
    """The env of an installed app and a not-running-or-running Gold Band.

    The app the gate looks for points at a stub under ``root``, created
    here: the step checks existence only, the way the agent map's does.
    """
    app = root / "Gold Band.app"
    app.mkdir(parents=True, exist_ok=True)
    return {converge.GOLD_BAND_APP_ENV: str(app),
            converge.GOLD_BAND_RUNNING_ENV: running}


def _keep_file(home: Path) -> None:
    """The converge keep file every surface reads, with nothing kept."""
    agent_kit = home / ".config" / "agent-kit"
    agent_kit.mkdir(parents=True)
    (agent_kit / "keep.yaml").write_text("keep:\n")


def _uv_stub(root: Path) -> str:
    """A uv stub that runs converge.py for real for every Gold Band surface.

    Every other call is recorded like the harness's default stub.
    """
    return (
        f'echo "uv $*" >> "{root / "calls.log"}"\n'
        'while [ "$#" -gt 0 ] && [ "$1" != gold-band ] && [ "$1" != gold-band-validate ] '
        '&& [ "$1" != gold-band-roles ]; do shift; done\n'
        'if [ "$#" = 0 ]; then\n'
        "  exit 0\n"
        "fi\n"
        'sub="$1"\n'
        "shift\n"
        f'exec "{sys.executable}" "{KIT_ROOT / "scripts" / "converge.py"}" "$sub" "$@"\n'
    )


def _profiles(home: Path) -> Path:
    """The directory Gold Band reads its profiles from."""
    return home / ".gold-band" / "context" / "profiles"


def _rendered(home: Path) -> dict[str, bytes]:
    """The profiles directory's files as a name-to-bytes map, sorted by name."""
    assert _profiles(home).is_dir()
    return {path.name: path.read_bytes() for path in sorted(_profiles(home).iterdir())}


def _assert_the_six(home: Path, extra: set[str] = set()) -> None:
    """The six generated profiles and nothing else but ``extra``, each carrying
    its crate-declared front matter id, in a directory the step made 0700."""
    profiles = _profiles(home)
    assert {path.name for path in profiles.iterdir()} == set(ROLES) | extra
    for name, role_id in ROLES.items():
        lines = (profiles / name).read_text().splitlines()
        assert lines[0] == "---", name
        assert lines[1] == f"id: {role_id}", name
    assert profiles.stat().st_mode & 0o777 == 0o700


def test_a_setup_run_with_the_app_present_writes_the_six_profiles(tmp_path: Path, built_crate: None) -> None:
    run = setup_run(tmp_path, "Linux", prepare_home=_keep_file,
                    stubs={"uv": _uv_stub(tmp_path)}, extra_env=_gate(tmp_path))
    _assert_the_six(run.home)
    assert "wrote decision-gap-critic" in run.stdout
    assert "wrote adversarial-verifier" in run.stdout


def test_a_second_run_changes_nothing(tmp_path: Path, built_crate: None) -> None:
    run = setup_run(tmp_path, "Linux", prepare_home=_keep_file,
                    stubs={"uv": _uv_stub(tmp_path)}, extra_env=_gate(tmp_path))
    before = _rendered(run.home)
    assert set(before) == set(ROLES)
    again = setup_run(tmp_path / "again", "Linux", home=run.home,
                      stubs={"uv": _uv_stub(tmp_path / "again")}, extra_env=_gate(tmp_path))
    assert "unchanged" in again.stdout
    assert _rendered(run.home) == before


def test_a_hand_made_profile_survives_and_a_stale_generated_one_goes(
    tmp_path: Path, built_crate: None
) -> None:
    def seeded(home: Path) -> None:
        """The keep file, a hand-made profile and a stale generated one."""
        _keep_file(home)
        profiles = _profiles(home)
        profiles.mkdir(parents=True)
        (profiles / "my-own-role.md").write_text(HAND_MADE)
        (profiles / "old-critic.md").write_text(
            "---\nid: pf-old\n---\nA profile that is no longer one of the six.\n")

    run = setup_run(tmp_path, "Linux", prepare_home=seeded,
                    stubs={"uv": _uv_stub(tmp_path)}, extra_env=_gate(tmp_path))
    assert (run.home / ".gold-band" / "context" / "profiles" / "my-own-role.md").read_text() == HAND_MADE
    assert not (run.home / ".gold-band" / "context" / "profiles" / "old-critic.md").exists()
    _assert_the_six(run.home, extra={"my-own-role.md"})


def test_without_the_app_nothing_is_created(tmp_path: Path) -> None:
    missing_app = tmp_path / "not-installed"
    run = setup_run(tmp_path, "Linux", prepare_home=_keep_file,
                    extra_env={converge.GOLD_BAND_APP_ENV: str(missing_app)})
    assert f"Gold Band is not installed ({missing_app})" in run.stdout
    assert "its profiles are left alone" in run.stdout
    assert not (run.home / ".gold-band").exists()


def test_a_running_gold_band_is_refused_and_writes_nothing(tmp_path: Path, built_crate: None) -> None:
    def seeded(home: Path) -> None:
        _keep_file(home)
        profiles = _profiles(home)
        profiles.mkdir(parents=True)
        (profiles / "my-own-role.md").write_text(HAND_MADE)

    run = setup_run(tmp_path, "Linux", prepare_home=seeded,
                    stubs={"uv": _uv_stub(tmp_path)}, extra_env=_gate(tmp_path, running="1"))
    profiles = _profiles(run.home)
    assert "Gold Band is running" in run.stdout
    assert "quit it and re-run setup" in run.stdout
    assert "the Gold Band profiles did not converge" in run.stdout + run.stderr
    assert run.returncode != 0
    assert {path.name: path.read_bytes() for path in profiles.iterdir()} == {
        "my-own-role.md": HAND_MADE.encode()}
