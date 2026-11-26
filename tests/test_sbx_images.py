"""The Docker Sandboxes template images build from images/sbx/<client>/Dockerfile.

``scripts/build-sbx-images.sh`` tags each image ``agent-kit/<client>:<version>``
(the Dockerfile's ``ARG VERSION=``) and ``:current``. The Pi image's pins must
equal the ones installer/setup-workstation.sh installs, so a pin bump in setup
without the image fails here. Docker is replaced by a stub that records its
arguments; no image is built.
"""

from __future__ import annotations

import os
import re
import subprocess
from pathlib import Path

import pytest

KIT_ROOT = Path(__file__).resolve().parent.parent
SCRIPT = KIT_ROOT / "scripts" / "build-sbx-images.sh"
SETUP = KIT_ROOT / "installer" / "setup-workstation.sh"
PI_DOCKERFILE = KIT_ROOT / "images" / "sbx" / "pi" / "Dockerfile"
OPENCODE_DOCKERFILE = KIT_ROOT / "images" / "sbx" / "opencode" / "Dockerfile"

DOCKER_STUB = """#!/bin/sh
printf '%s\\n' "$*" >> "$SBX_TEST_DOCKER_LOG"
"""


def _arg(dockerfile: Path, name: str) -> str:
    match = re.search(rf"^ARG {name}=(\S+)$", dockerfile.read_text(), re.MULTILINE)
    assert match, f"{dockerfile} has no ARG {name}="
    return match.group(1)


def _stub_path(tmp_path: Path) -> tuple[str, Path]:
    bin_dir = tmp_path / "bin"
    bin_dir.mkdir()
    docker = bin_dir / "docker"
    docker.write_text(DOCKER_STUB)
    docker.chmod(0o755)
    log = tmp_path / "docker.log"
    return f"{bin_dir}:/usr/bin:/bin", log


def _run(args: list[str], path: str, log: Path) -> subprocess.CompletedProcess:
    env = {"PATH": path, "SBX_TEST_DOCKER_LOG": str(log), "HOME": str(log.parent)}
    return subprocess.run(
        ["/bin/sh", str(SCRIPT), *args],
        capture_output=True,
        text=True,
        check=False,
        env=env,
    )


def test_dry_run_prints_tags_and_contexts_and_builds_nothing(tmp_path: Path) -> None:
    path, log = _stub_path(tmp_path)
    result = _run(["--dry-run"], path, log)

    assert result.returncode == 0, result.stderr
    assert not log.exists(), "dry run must not call docker"
    lines = result.stdout.splitlines()
    pi_version = _arg(PI_DOCKERFILE, "VERSION")
    oc_version = _arg(OPENCODE_DOCKERFILE, "VERSION")
    pi_line = next(line for line in lines if "agent-kit/pi:" in line)
    oc_line = next(line for line in lines if "agent-kit/opencode:" in line)
    assert f"-t agent-kit/pi:{pi_version} -t agent-kit/pi:current" in pi_line
    assert pi_line.endswith(str(PI_DOCKERFILE.parent))
    assert f"-f {PI_DOCKERFILE}" in pi_line
    assert f"-t agent-kit/opencode:{oc_version} -t agent-kit/opencode:current" in oc_line
    assert oc_line.endswith(str(OPENCODE_DOCKERFILE.parent))


def test_client_flag_builds_only_that_client(tmp_path: Path) -> None:
    path, log = _stub_path(tmp_path)
    result = _run(["--dry-run", "--client", "pi"], path, log)

    assert result.returncode == 0, result.stderr
    assert "agent-kit/pi:" in result.stdout
    assert "agent-kit/opencode:" not in result.stdout


def test_real_run_calls_docker_build_with_both_tags(tmp_path: Path) -> None:
    path, log = _stub_path(tmp_path)
    result = _run(["--client", "opencode"], path, log)

    assert result.returncode == 0, result.stderr
    oc_version = _arg(OPENCODE_DOCKERFILE, "VERSION")
    calls = log.read_text().splitlines()
    assert calls == [
        f"build -f {OPENCODE_DOCKERFILE} -t agent-kit/opencode:{oc_version} "
        f"-t agent-kit/opencode:current {OPENCODE_DOCKERFILE.parent}"
    ]


def test_missing_docker_fails_with_message(tmp_path: Path) -> None:
    empty = tmp_path / "empty"
    empty.mkdir()
    result = _run([], str(empty), tmp_path / "docker.log")

    assert result.returncode != 0
    assert "docker is not on PATH" in result.stderr


def test_unknown_client_is_bad_usage(tmp_path: Path) -> None:
    path, log = _stub_path(tmp_path)
    result = _run(["--client", "nope"], path, log)

    assert result.returncode == 2
    assert "no image for client 'nope'" in result.stderr


def test_pi_pins_match_setup() -> None:
    setup = SETUP.read_text()
    pi_pins = set(re.findall(r'"@earendil-works/pi-coding-agent@([^"\s]+)"', setup))
    acp_pins = set(re.findall(r'"pi-acp@([^"\s]+)"', setup))
    assert pi_pins == {_arg(PI_DOCKERFILE, "VERSION")}
    assert acp_pins == {_arg(PI_DOCKERFILE, "PI_ACP_VERSION")}


@pytest.mark.parametrize("dockerfile", [PI_DOCKERFILE, OPENCODE_DOCKERFILE], ids=["pi", "opencode"])
def test_image_is_pinned_and_runs_as_non_root(dockerfile: Path) -> None:
    text = dockerfile.read_text()
    assert re.findall(r"^FROM\s+(\S+)", text, re.MULTILINE) == ["${NODE_IMAGE}"]
    node_image = _arg(dockerfile, "NODE_IMAGE")
    assert node_image.startswith("node:") and not node_image.endswith(":latest")
    users = re.findall(r"^USER\s+(\S+)", text, re.MULTILINE)
    assert users and users[-1] not in ("root", "0")
    assert ":latest" not in text


def test_build_script_is_posix_sh() -> None:
    assert SCRIPT.read_text().splitlines()[0] == "#!/bin/sh"
    assert os.access(SCRIPT, os.X_OK)
