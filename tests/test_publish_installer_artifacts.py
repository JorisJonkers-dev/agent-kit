"""End-to-end test of scripts/publish-installer-artifacts.sh against a fake
`gh` CLI. Never touches a real GitHub release: a stub `gh` script on PATH
intercepts `gh release upload`/`gh release download` and stores/serves
assets from a local directory instead.
"""

from __future__ import annotations

import stat
import subprocess
from pathlib import Path

import pytest

KIT_ROOT = Path(__file__).resolve().parent.parent
SCRIPT = KIT_ROOT / "scripts" / "publish-installer-artifacts.sh"
KIT_FILES = (
    "registry/estate-tooling.yaml",
    "registry/generated/hermes/mcp-servers.local.yaml",
    "installer/port-forward-agent.sh",
    "installer/cloud-session.sh",
    "scripts/hermes-merge-mcp.py",
)

FAKE_GH = """#!/usr/bin/env python3
# Fake `gh` for tests: `gh release upload <tag> <file> [--clobber]` copies the
# file into $FAKE_RELEASE_STORE/<tag>/; `gh release download <tag> --pattern
# <name> --dir <dir> [--clobber]` copies it back out. Anything else is an error.
import os
import shutil
import sys
from pathlib import Path

def main() -> int:
    args = sys.argv[1:]
    if len(args) < 2 or args[0] != "release":
        print(f"fake gh: unsupported invocation: {args}", file=sys.stderr)
        return 1
    store = Path(os.environ["FAKE_RELEASE_STORE"])
    subcommand, rest = args[1], args[2:]
    if subcommand == "upload":
        tag, local_file = rest[0], rest[1]
        dest_dir = store / tag
        dest_dir.mkdir(parents=True, exist_ok=True)
        shutil.copy(local_file, dest_dir / Path(local_file).name)
        return 0
    if subcommand == "download":
        tag = rest[0]
        pattern = out_dir = None
        flags = iter(rest[1:])
        for flag in flags:
            if flag == "--pattern":
                pattern = next(flags)
            elif flag == "--dir":
                out_dir = next(flags)
        assert pattern and out_dir, "test fake gh needs --pattern and --dir"
        source = store / tag / pattern
        if not source.exists():
            print(f"fake gh: no asset {pattern!r} on release {tag!r}", file=sys.stderr)
            return 1
        Path(out_dir).mkdir(parents=True, exist_ok=True)
        shutil.copy(source, Path(out_dir) / pattern)
        return 0
    print(f"fake gh: unsupported subcommand {subcommand!r}", file=sys.stderr)
    return 1

if __name__ == "__main__":
    sys.exit(main())
"""


@pytest.fixture
def fake_gh_bin(tmp_path: Path) -> Path:
    bin_dir = tmp_path / "fake-bin"
    bin_dir.mkdir()
    gh_path = bin_dir / "gh"
    gh_path.write_text(FAKE_GH)
    gh_path.chmod(gh_path.stat().st_mode | stat.S_IEXEC)
    return bin_dir


@pytest.fixture
def fake_repo(tmp_path: Path) -> Path:
    repo = tmp_path / "repo"
    (repo / "installer").mkdir(parents=True)
    (repo / "installer" / "setup-workstation.sh").write_text(
        "#!/usr/bin/env bash\n# GENERATED FROM registry/estate-tooling.yaml -- DO NOT EDIT.\nset -uo pipefail\n"
    )
    (repo / "skills" / "pr-composer").mkdir(parents=True)
    (repo / "skills" / "pr-composer" / "SKILL.md").write_text("---\nname: pr-composer\n---\nbody\n")
    for rel in KIT_FILES:
        (repo / rel).parent.mkdir(parents=True, exist_ok=True)
        (repo / rel).write_text("# fixture\n")
    return repo


def run_step(
    step: str,
    repo: Path,
    stage_dir: Path,
    fake_gh_bin: Path,
    release_store: Path,
    tag: str = "v9.9.9",
) -> subprocess.CompletedProcess:
    env = {
        "PATH": f"{fake_gh_bin}:/usr/bin:/bin:/usr/local/bin:/opt/homebrew/bin:/sbin",
        "TAG_NAME": tag,
        "REPO_ROOT": str(repo),
        "STAGE_DIR": str(stage_dir),
        "FAKE_RELEASE_STORE": str(release_store),
        "GH_TOKEN": "test-token",
        "GH_REPO": "JorisJonkers-dev/agent-kit",
    }
    return subprocess.run(
        [str(SCRIPT), step],
        env=env,
        cwd=repo,
        capture_output=True,
        text=True,
        check=False,
    )


ALL_STEPS = (
    "stage",
    "verify-retired-excluded",
    "attach-script",
    "attach-skills",
    "attach-checksum",
    "verify-script",
    "verify-skills",
    "verify-checksum",
)


def test_full_publish_and_verify_cycle_succeeds(fake_repo: Path, fake_gh_bin: Path, tmp_path: Path):
    stage_dir = tmp_path / "stage"
    release_store = tmp_path / "release-store"
    for step in ALL_STEPS:
        result = run_step(step, fake_repo, stage_dir, fake_gh_bin, release_store)
        assert result.returncode == 0, f"{step} failed:\n{result.stdout}\n{result.stderr}"


def test_rerun_on_the_same_release_is_a_no_op_that_still_verifies(fake_repo: Path, fake_gh_bin: Path, tmp_path: Path):
    stage_dir = tmp_path / "stage"
    release_store = tmp_path / "release-store"
    for _ in range(2):
        for step in ALL_STEPS:
            result = run_step(step, fake_repo, stage_dir, fake_gh_bin, release_store)
            assert result.returncode == 0, f"{step} failed on rerun:\n{result.stdout}\n{result.stderr}"


def test_skills_tarball_has_no_dot_slash_prefix_and_no_version_dir(fake_repo: Path, fake_gh_bin: Path, tmp_path: Path):
    stage_dir = tmp_path / "stage"
    release_store = tmp_path / "release-store"
    assert run_step("stage", fake_repo, stage_dir, fake_gh_bin, release_store).returncode == 0
    names = subprocess.run(
        ["tar", "-tzf", str(stage_dir / "agent-kit-skills.tar.gz")],
        capture_output=True,
        text=True,
        check=True,
    ).stdout.splitlines()
    assert names, "tarball is empty"
    for name in names:
        assert not name.startswith("./"), name
        assert not name.startswith("v9.9.9/"), name
    assert "skills/pr-composer/SKILL.md" in names


def test_the_bundle_carries_what_the_setup_script_reads_beside_itself(
    fake_repo: Path, fake_gh_bin: Path, tmp_path: Path,
):
    """A setup-workstation.sh run outside a checkout re-runs the copy in this bundle."""
    stage_dir = tmp_path / "stage"
    release_store = tmp_path / "release-store"
    assert run_step("stage", fake_repo, stage_dir, fake_gh_bin, release_store).returncode == 0
    names = subprocess.run(
        ["tar", "-tzf", str(stage_dir / "agent-kit-skills.tar.gz")],
        capture_output=True,
        text=True,
        check=True,
    ).stdout.splitlines()
    for rel in ("installer/setup-workstation.sh", *KIT_FILES):
        assert rel in names, rel


def test_stage_refuses_a_checkout_missing_a_kit_file(fake_repo: Path, fake_gh_bin: Path, tmp_path: Path):
    (fake_repo / "installer" / "cloud-session.sh").unlink()
    result = run_step("stage", fake_repo, tmp_path / "stage", fake_gh_bin, tmp_path / "release-store")
    assert result.returncode != 0
    assert "installer/cloud-session.sh" in result.stderr


def test_verify_script_fails_when_asset_version_does_not_match_release(
    fake_repo: Path, fake_gh_bin: Path, tmp_path: Path
):
    stage_dir = tmp_path / "stage"
    release_store = tmp_path / "release-store"
    tag = "v1.0.0"
    assert run_step("stage", fake_repo, stage_dir, fake_gh_bin, release_store, tag=tag).returncode == 0
    assert run_step("attach-script", fake_repo, stage_dir, fake_gh_bin, release_store, tag=tag).returncode == 0
    # Tamper with what the release actually holds, as if a stale asset from a
    # different tag had been attached by hand.
    attached = release_store / tag / "setup-workstation.sh"
    attached.write_text(attached.read_text().replace(tag, "v0.0.1-stale"))
    result = run_step("verify-script", fake_repo, stage_dir, fake_gh_bin, release_store, tag=tag)
    assert result.returncode != 0
    assert "version token" in result.stderr


def test_verify_skills_fails_when_asset_is_corrupted(fake_repo: Path, fake_gh_bin: Path, tmp_path: Path):
    stage_dir = tmp_path / "stage"
    release_store = tmp_path / "release-store"
    tag = "v9.9.9"
    assert run_step("stage", fake_repo, stage_dir, fake_gh_bin, release_store, tag=tag).returncode == 0
    assert run_step("attach-skills", fake_repo, stage_dir, fake_gh_bin, release_store, tag=tag).returncode == 0
    # Tamper with what the release actually holds, as if the attach landed
    # truncated or corrupted in transit.
    (release_store / tag / "agent-kit-skills.tar.gz").write_bytes(b"corrupted")
    result = run_step("verify-skills", fake_repo, stage_dir, fake_gh_bin, release_store, tag=tag)
    assert result.returncode != 0
    assert "checksum" in result.stderr


def test_verify_retired_excluded_fails_if_installer_file_present(fake_repo: Path, fake_gh_bin: Path, tmp_path: Path):
    stage_dir = tmp_path / "stage"
    release_store = tmp_path / "release-store"
    assert run_step("stage", fake_repo, stage_dir, fake_gh_bin, release_store).returncode == 0
    (fake_repo / "installer" / "install.sh").write_text("#!/usr/bin/env bash\necho legacy\n")
    result = run_step("verify-retired-excluded", fake_repo, stage_dir, fake_gh_bin, release_store)
    assert result.returncode != 0
    assert "install.sh" in result.stderr


def test_verify_retired_excluded_fails_if_tarball_contains_retired_file(
    fake_repo: Path, fake_gh_bin: Path, tmp_path: Path
):
    stage_dir = tmp_path / "stage"
    release_store = tmp_path / "release-store"
    (fake_repo / "skills" / "pr-composer" / "install-agents.sh").write_text("#!/usr/bin/env bash\necho legacy\n")
    assert run_step("stage", fake_repo, stage_dir, fake_gh_bin, release_store).returncode == 0
    result = run_step("verify-retired-excluded", fake_repo, stage_dir, fake_gh_bin, release_store)
    assert result.returncode != 0
    assert "install-agents.sh" in result.stderr
