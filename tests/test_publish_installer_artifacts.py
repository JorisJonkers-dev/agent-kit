"""End-to-end test of scripts/publish-installer-artifacts.sh against a local
fake S3/website endpoint. Never touches the real bucket: GARAGE_S3_ENDPOINT
and PUBLIC_URL both point at a throwaway http.server started for the test.
"""

from __future__ import annotations

import http.server
import subprocess
import threading
from collections.abc import Iterator
from pathlib import Path
from typing import ClassVar

import pytest

KIT_ROOT = Path(__file__).resolve().parent.parent
SCRIPT = KIT_ROOT / "scripts" / "publish-installer-artifacts.sh"


class _FakeObjectStore(http.server.BaseHTTPRequestHandler):
    """Stands in for both the signed S3 write API and the anonymous website
    read endpoint. PUT /<bucket>/<key> stores by key; GET /<key> serves it
    back, matching the real split between S3 API and website access closely
    enough to exercise this script's request shapes."""

    objects: ClassVar[dict[str, bytes]] = {}

    def do_PUT(self) -> None:
        length = int(self.headers.get("Content-Length", 0))
        body = self.rfile.read(length)
        key = self.path.split("/", 2)[-1]
        _FakeObjectStore.objects[key] = body
        self.send_response(200)
        self.end_headers()

    def do_GET(self) -> None:
        key = self.path.lstrip("/")
        body = _FakeObjectStore.objects.get(key)
        if body is None:
            self.send_response(404)
            self.end_headers()
            return
        self.send_response(200)
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, format: str, *args: object) -> None:
        pass  # keep test output quiet


@pytest.fixture
def fake_host() -> Iterator[str]:
    _FakeObjectStore.objects = {}
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), _FakeObjectStore)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        yield f"http://127.0.0.1:{server.server_port}"
    finally:
        server.shutdown()
        thread.join(timeout=5)


@pytest.fixture
def fake_repo(tmp_path: Path) -> Path:
    repo = tmp_path / "repo"
    (repo / "installer").mkdir(parents=True)
    (repo / "installer" / "setup-workstation.sh").write_text(
        "#!/usr/bin/env bash\n# GENERATED FROM registry/estate-tooling.yaml -- DO NOT EDIT.\nset -uo pipefail\n"
    )
    (repo / "skills" / "pr-composer").mkdir(parents=True)
    (repo / "skills" / "pr-composer" / "SKILL.md").write_text("---\nname: pr-composer\n---\nbody\n")
    return repo


def run_step(step: str, repo: Path, stage_dir: Path, host: str, tag: str = "v9.9.9") -> subprocess.CompletedProcess:
    env = {
        "PATH": "/usr/bin:/bin:/usr/local/bin:/opt/homebrew/bin:/sbin",
        "TAG_NAME": tag,
        "REPO_ROOT": str(repo),
        "STAGE_DIR": str(stage_dir),
        "PUBLIC_URL": host,
        "S3_ENDPOINT": host,
        "GARAGE_ACCESS_KEY_ID": "test-key-id",
        "GARAGE_SECRET_ACCESS_KEY": "test-secret",
        "BUCKET": "assets.jorisjonkers.dev",
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
    "upload-script",
    "upload-skills",
    "upload-checksum",
    "verify-script",
    "verify-skills",
    "verify-checksum",
    "verify-retired-absent",
)


def test_full_publish_and_verify_cycle_succeeds(fake_repo: Path, fake_host: str, tmp_path: Path):
    stage_dir = tmp_path / "stage"
    for step in ALL_STEPS:
        result = run_step(step, fake_repo, stage_dir, fake_host)
        assert result.returncode == 0, f"{step} failed:\n{result.stdout}\n{result.stderr}"


def test_rerun_on_the_same_release_is_a_no_op_that_still_verifies(fake_repo: Path, fake_host: str, tmp_path: Path):
    stage_dir = tmp_path / "stage"
    for _ in range(2):
        for step in ALL_STEPS:
            result = run_step(step, fake_repo, stage_dir, fake_host)
            assert result.returncode == 0, f"{step} failed on rerun:\n{result.stdout}\n{result.stderr}"


def test_skills_tarball_has_no_dot_slash_prefix_and_no_version_dir(fake_repo: Path, fake_host: str, tmp_path: Path):
    stage_dir = tmp_path / "stage"
    assert run_step("stage", fake_repo, stage_dir, fake_host).returncode == 0
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


def test_verify_script_fails_when_published_version_does_not_match_release(
    fake_repo: Path, fake_host: str, tmp_path: Path
):
    stage_dir = tmp_path / "stage"
    assert run_step("stage", fake_repo, stage_dir, fake_host, tag="v1.0.0").returncode == 0
    assert run_step("upload-script", fake_repo, stage_dir, fake_host, tag="v1.0.0").returncode == 0
    # A different release than the one actually published must fail, not pass.
    result = run_step("verify-script", fake_repo, stage_dir, fake_host, tag="v2.0.0")
    assert result.returncode != 0
    assert "version token" in result.stderr


def test_verify_skills_fails_when_published_tarball_is_corrupted(fake_repo: Path, fake_host: str, tmp_path: Path):
    stage_dir = tmp_path / "stage"
    assert run_step("stage", fake_repo, stage_dir, fake_host).returncode == 0
    assert run_step("upload-skills", fake_repo, stage_dir, fake_host).returncode == 0
    # Tamper with what the fake host serves back, as if the upload landed
    # truncated or corrupted in transit.
    _FakeObjectStore.objects["agent-kit-skills.tar.gz"] = b"corrupted"
    result = run_step("verify-skills", fake_repo, stage_dir, fake_host)
    assert result.returncode != 0
    assert "checksum" in result.stderr


def test_verify_retired_installers_absent_fails_if_one_is_served(fake_repo: Path, fake_host: str, tmp_path: Path):
    stage_dir = tmp_path / "stage"
    _FakeObjectStore.objects["install.sh"] = b"echo legacy"
    result = run_step("verify-retired-absent", fake_repo, stage_dir, fake_host)
    assert result.returncode != 0
    assert "install.sh" in result.stderr
