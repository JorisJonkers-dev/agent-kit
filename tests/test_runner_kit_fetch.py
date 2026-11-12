"""The in-cluster runner overlays the newest published kit at session start.

The entrypoint pulls the runtime-home artifact with oras, verifies it against
what the release published, overlays it, and reports the version it fetched.
Every failure falls back to the baked kit with a WARN line. The registry is a
fake ``oras`` that copies a fixture artifact; nothing touches the network.
"""

from __future__ import annotations

import hashlib
import io
import os
import subprocess
import tarfile
from pathlib import Path

import pytest

KIT_ROOT = Path(__file__).resolve().parent.parent
ENTRYPOINT = KIT_ROOT / "runner" / "manifests" / "runtime" / "entrypoint.sh"
VERSION = "v9.9.9"
REF = "registry.example/agent-kit/runtime-home:latest"

FAKE_ORAS = r"""#!/bin/sh
echo "$@" >> "$ORAS_LOG"
[ "$1" = pull ] || exit 2
[ -z "${ORAS_FAIL:-}" ] || { echo "$ORAS_FAIL" >&2; exit 1; }
out=""
while [ "$#" -gt 0 ]; do [ "$1" = -o ] && out="$2"; shift; done
cp "$ORAS_ARTIFACT"/* "$out"/
"""


def _sha(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def _tar(files: dict[str, bytes], extra_members: list[str] | None = None) -> bytes:
    buf = io.BytesIO()
    with tarfile.open(fileobj=buf, mode="w:gz") as tar:
        for name, data in files.items():
            info = tarfile.TarInfo(name)
            info.size = len(data)
            tar.addfile(info, io.BytesIO(data))
        for name in extra_members or []:
            info = tarfile.TarInfo(name)
            info.size = 1
            tar.addfile(info, io.BytesIO(b"x"))
    return buf.getvalue()


def _tar_with_link(kind: bytes) -> bytes:
    buf = io.BytesIO()
    with tarfile.open(fileobj=buf, mode="w:gz") as tar:
        info = tarfile.TarInfo("./home/x")
        info.type = kind
        info.linkname = "/etc"
        tar.addfile(info)
    return buf.getvalue()


def _package(extra_members: list[str] | None = None, bad_sums: bool = False) -> bytes:
    files = {
        "./home/.claude/skills/new/SKILL.md": b"new skill\n",
        "./home/.claude/.credentials.json": b"from-the-kit\n",
        "./mcp/claude-mcp-servers.minimal.json": b'{"fetched": {}}\n',
    }
    sums = "".join(f"{_sha(data)}  {name}\n" for name, data in files.items())
    if bad_sums:
        sums = sums.replace(sums[:4], "0000", 1)
    return _tar({**files, "./SHA256SUMS": sums.encode()}, extra_members)


class Session:
    def __init__(self, tmp: Path) -> None:
        self.tmp = tmp
        self.home = tmp / "home"
        self.artifact = tmp / "artifact"
        self.bin = tmp / "bin"
        for path in (self.home / ".claude", self.home / ".codex", self.artifact, self.bin):
            path.mkdir(parents=True)
        (self.home / ".claude" / ".knowledge-system-version").write_text("version=v1.0.0\n")
        (self.home / ".claude" / ".credentials.json").write_text("mounted\n")
        (self.bin / "oras").write_text(FAKE_ORAS)
        (self.bin / "oras").chmod(0o755)
        self.log = tmp / "oras.log"
        self.log.write_text("")
        self.publish(_package())

    def publish(self, tarball: bytes, published_sha: str | None = None, name: str | None = None) -> None:
        for old in self.artifact.iterdir():
            old.unlink()
        name = name or f"agent-kit-runtime-home-{VERSION}.tar.gz"
        (self.artifact / name).write_bytes(tarball)
        (self.artifact / f"{name}.sha256").write_text(f"{published_sha or _sha(tarball)}  {name}\n")

    def run(self, **env: str) -> subprocess.CompletedProcess[str]:
        base = {
            "PATH": f"{self.bin}:{os.environ['PATH']}",
            "HOME": str(self.home),
            "CLAUDE_CONFIG_DIR": str(self.home / ".claude"),
            "CODEX_HOME": str(self.home / ".codex"),
            "WORKSPACE_ROOT": str(self.tmp / "ws"),
            "AGENT_RUNNER_CONTAINER": "1",
            "AGENT_KIT_EXPECTED_VERSION": "",
            "ORAS_LOG": str(self.log),
            "ORAS_ARTIFACT": str(self.artifact),
            "AGENT_KIT_RUNTIME_REF": REF,
        }
        base.update(env)
        return subprocess.run(
            ["sh", str(ENTRYPOINT), "sh", "-c", 'echo "mcp=$AGENT_MCP_DIR fetched=${AGENT_KIT_FETCHED_VERSION:-}"'],
            env=base,
            capture_output=True,
            text=True,
            check=False,
        )

    def status(self) -> str:
        return (self.home / ".agent-kit-fetched" / "status").read_text()


@pytest.fixture
def session(tmp_path: Path) -> Session:
    return Session(tmp_path)


def _assert_baked(session: Session, result: subprocess.CompletedProcess[str], reason: str) -> None:
    assert result.returncode == 0, result.stderr
    assert f"WARN: kit fetch failed: {reason}" in result.stdout
    assert "falling back to the baked kit (version v1.0.0)" in result.stdout
    assert "source=baked" in session.status()
    assert not (session.home / ".claude" / "skills" / "new").exists()
    assert (session.home / ".claude" / ".knowledge-system-version").read_text() == "version=v1.0.0\n"
    assert "fetched=\n" in result.stdout


def test_a_session_reports_the_kit_version_it_fetched(session: Session) -> None:
    result = session.run()
    assert result.returncode == 0, result.stderr
    assert f"kit fetched: agent-kit {VERSION}" in result.stdout
    assert f"fetched={VERSION}" in result.stdout
    assert (session.home / ".claude" / "skills" / "new" / "SKILL.md").read_text() == "new skill\n"
    assert (session.home / ".claude" / ".knowledge-system-version").read_text() == f"version={VERSION}\n"
    assert (session.home / ".codex" / ".knowledge-system-version").read_text() == f"version={VERSION}\n"
    assert f"source=fetched\nversion={VERSION}\n" in session.status()
    assert f"mcp={session.home}/.agent-kit-fetched/mcp" in result.stdout
    assert (session.home / ".agent-kit-fetched" / "mcp" / "claude-mcp-servers.minimal.json").is_file()
    assert f"pull {REF} -o" in session.log.read_text()


def test_credentials_the_runner_mounted_are_not_overwritten(session: Session) -> None:
    session.run()
    assert (session.home / ".claude" / ".credentials.json").read_text() == "mounted\n"


def test_a_pinned_digest_that_matches_is_accepted(session: Session) -> None:
    digest = _sha((next(session.artifact.glob("*.tar.gz"))).read_bytes())
    result = session.run(AGENT_KIT_RUNTIME_DIGEST=f"sha256:{digest}")
    assert f"kit fetched: agent-kit {VERSION}" in result.stdout


def test_a_pinned_digest_that_differs_falls_back(session: Session) -> None:
    result = session.run(AGENT_KIT_RUNTIME_DIGEST="sha256:" + "0" * 64)
    assert result.returncode == 0
    assert "not the pinned sha256:" in result.stdout
    assert "falling back to the baked kit" in result.stdout
    assert not (session.home / ".claude" / "skills" / "new").exists()


def test_a_tarball_that_differs_from_the_published_sha256_falls_back(session: Session) -> None:
    session.publish(_package(), published_sha="0" * 64)
    result = session.run()
    _assert_baked(session, result, f"sha256 of {VERSION} is ")


def test_a_release_without_a_published_sha256_falls_back(session: Session) -> None:
    (next(session.artifact.glob("*.sha256"))).write_text("")
    result = session.run()
    _assert_baked(session, result, f"{REF} published no sha256 for {VERSION}")


def test_a_registry_failure_falls_back_with_the_reason(session: Session) -> None:
    result = session.run(ORAS_FAIL="denied: not found")
    _assert_baked(session, result, f"oras pull of {REF} failed: denied: not found")


def test_an_artifact_without_one_tarball_falls_back(session: Session) -> None:
    session.publish(_package(), name="unrelated.tar.gz")
    result = session.run()
    _assert_baked(session, result, f"{REF} did not hold exactly one runtime-home tarball")


def test_a_corrupt_tarball_falls_back(session: Session) -> None:
    session.publish(b"not a tarball")
    result = session.run()
    _assert_baked(session, result, f"{VERSION} is not a readable tarball")


@pytest.mark.parametrize("member", ["/etc/evil", "../evil", "home/../../evil"])
def test_paths_that_escape_the_overlay_fall_back(session: Session, member: str) -> None:
    session.publish(_package(extra_members=[member]))
    result = session.run()
    _assert_baked(session, result, f"{VERSION} holds an absolute or parent-relative path")


@pytest.mark.parametrize("kind", [tarfile.SYMTYPE, tarfile.LNKTYPE, tarfile.CHRTYPE])
def test_links_and_special_files_fall_back(session: Session, kind: bytes) -> None:
    session.publish(_tar_with_link(kind))
    result = session.run()
    _assert_baked(session, result, f"{VERSION} holds a link or special file")


def test_a_tarball_whose_own_sums_disagree_falls_back(session: Session) -> None:
    session.publish(_package(bad_sums=True))
    result = session.run()
    _assert_baked(session, result, f"SHA256SUMS inside {VERSION} does not match its files")


def test_a_tarball_without_a_home_falls_back(session: Session) -> None:
    session.publish(_tar({"./x": b"x\n", "./SHA256SUMS": f"{_sha(b'x' + chr(10).encode())}  ./x\n".encode()}))
    result = session.run()
    _assert_baked(session, result, f"{VERSION} has no home directory")


def test_missing_oras_falls_back(session: Session) -> None:
    tools = session.tmp / "tools"
    tools.mkdir()
    for directory in os.environ["PATH"].split(os.pathsep):
        for tool in Path(directory).glob("*") if Path(directory).is_dir() else []:
            if tool.name != "oras" and not (tools / tool.name).exists():
                (tools / tool.name).symlink_to(tool)
    result = session.run(PATH=str(tools))
    _assert_baked(session, result, "oras is not on PATH")


def test_without_a_ref_the_baked_kit_is_used_and_said_so(session: Session) -> None:
    result = session.run(AGENT_KIT_RUNTIME_REF="")
    assert "kit fetch not configured" in result.stdout
    assert session.log.read_text() == ""


def test_the_fetch_can_be_switched_off(session: Session) -> None:
    result = session.run(AGENT_KIT_FETCH="0")
    assert "kit fetch disabled" in result.stdout
    assert session.log.read_text() == ""


def test_outside_the_runner_container_nothing_is_fetched(session: Session) -> None:
    result = session.run(AGENT_RUNNER_CONTAINER="")
    if Path("/.dockerenv").exists() or Path("/run/.containerenv").exists():
        pytest.skip("the test host is itself a container")
    assert "kit fetch" not in result.stdout
    assert session.log.read_text() == ""
