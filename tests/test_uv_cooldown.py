"""uv resolves nothing published in the last seven days (ticket B6-08).

The window is one setting, `[tool.uv] exclude-newer = "7 days"` in
pyproject.toml. It is relative, so it moves forward on every `uv lock`, and the
lock records it as `exclude-newer-span = "P7D"`. `uv run --no-project` does not
read pyproject.toml, so every such call in the scripts and the generated
installer passes the same flag.

The proof is hermetic: a local index serves a package whose newest version was
uploaded two days ago next to one uploaded thirty days ago. With the window the
old version is resolved; without it the new one is, so the control shows the
index itself is not what does the work.
"""

from __future__ import annotations

import hashlib
import io
import json
import os
import re
import subprocess
import sys
import threading
import tomllib
import zipfile
from collections.abc import Iterator
from datetime import UTC, datetime, timedelta
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

import pytest
import yaml

KIT_ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(KIT_ROOT / "scripts"))

import render_registry  # noqa: E402

WINDOW = "7 days"
# Any `uv run` that resolves packages with --with must carry the window, written
# as --exclude-newer "7 days" in bash (or \"7 days\" inside a double-quoted
# python string of the generator).
WINDOW_FLAG = re.compile(r'--exclude-newer \\?"7 days\\?"')
PROBE = "cooldown-probe"
SIMPLE_JSON = "application/vnd.pypi.simple.v1+json"


def _pyproject_uv() -> dict:
    return tomllib.loads((KIT_ROOT / "pyproject.toml").read_text())["tool"]["uv"]


def test_pyproject_sets_the_seven_day_window_and_a_uv_that_records_it() -> None:
    uv = _pyproject_uv()
    assert uv["exclude-newer"] == WINDOW
    # 0.9.17 is the oldest uv that applies the relative window (older uv ignores it).
    assert uv["required-version"] == ">=0.9.17"


def test_lock_records_the_relative_window_and_no_entry_is_younger_than_seven_days() -> None:
    lock = tomllib.loads((KIT_ROOT / "uv.lock").read_text())
    assert lock["options"]["exclude-newer-span"] == "P7D"
    cutoff = datetime.now(UTC) - timedelta(days=7)
    uploads: list[datetime] = []  # uv writes upload-time as an RFC 3339 string
    for package in lock["package"]:
        for entry in [package.get("sdist"), *package.get("wheels", [])]:
            if entry is not None:
                uploads.append(datetime.fromisoformat(entry["upload-time"]))
    assert len(uploads) > 0
    # A lock made at time T with the window holds only uploads at or before T-7d,
    # and T is never later than now, so this holds for every later day too.
    younger = [when for when in uploads if when > cutoff]
    assert younger == []


def _uv_call_sites() -> Iterator[tuple[str, int, str]]:
    """Every line that adds packages through uv: `uv run ... --with` or `uv tool install/upgrade`."""
    paths = [
        *sorted((KIT_ROOT / "scripts").rglob("*")),
        *sorted((KIT_ROOT / "installer").rglob("*")),
        *sorted((KIT_ROOT / "registry").glob("*.yaml")),
        *sorted((KIT_ROOT / "tests").glob("*.py")),
    ]
    for path in paths:
        if not path.is_file() or path.name == "test_uv_cooldown.py" or path.suffix in {".pyc", ".png", ".gz"}:
            continue
        try:
            lines = path.read_text().splitlines()
        except UnicodeDecodeError:
            continue
        for number, line in enumerate(lines, start=1):
            adds = ("uv run" in line and "--with" in line) or "uv tool install" in line or "uv tool upgrade" in line
            if adds:
                yield path.relative_to(KIT_ROOT).as_posix(), number, line


def test_every_uv_run_that_adds_packages_passes_the_window() -> None:
    sites = list(_uv_call_sites())
    # The installer and the scripts carry about a dozen such calls; a near-empty
    # scan would mean the pattern above stopped matching anything.
    assert len(sites) >= 10
    missing = [f"{path}:{number}: {line.strip()}" for path, number, line in sites if not WINDOW_FLAG.search(line)]
    assert missing == []


def test_generated_setup_script_passes_the_window_on_every_uv_run_with() -> None:
    data = yaml.safe_load((KIT_ROOT / "registry" / "estate-tooling.yaml").read_text())
    script = render_registry.render_setup_script(data)
    calls = [line for line in script.splitlines() if "uv run" in line and "--with" in line]
    assert len(calls) >= 10
    assert [line for line in calls if not WINDOW_FLAG.search(line)] == []


# --- the proof: a version younger than seven days is not resolved ---


def _wheel(version: str) -> bytes:
    dist = f"cooldown_probe-{version}.dist-info"
    buf = io.BytesIO()
    with zipfile.ZipFile(buf, "w") as zf:
        zf.writestr("cooldown_probe/__init__.py", f"VERSION = {version!r}\n")
        zf.writestr(f"{dist}/METADATA", f"Metadata-Version: 2.1\nName: {PROBE}\nVersion: {version}\n")
        zf.writestr(f"{dist}/WHEEL", "Wheel-Version: 1.0\nGenerator: test\nRoot-Is-Purelib: true\nTag: py3-none-any\n")
        zf.writestr(f"{dist}/RECORD", "")
    return buf.getvalue()


class _Index:
    """A PEP 691 simple index on 127.0.0.1 with one probe package and two uploads."""

    def __init__(self) -> None:
        now = datetime.now(UTC)
        self.versions = {
            "1.0.0": (now - timedelta(days=30), _wheel("1.0.0")),
            "2.0.0": (now - timedelta(days=2), _wheel("2.0.0")),
        }
        self.server = ThreadingHTTPServer(("127.0.0.1", 0), self._handler())
        self.url = f"http://127.0.0.1:{self.server.server_address[1]}/simple/"
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)

    def _files(self) -> list[dict]:
        files = []
        for version, (uploaded, body) in self.versions.items():
            name = f"cooldown_probe-{version}-py3-none-any.whl"
            files.append(
                {
                    "filename": name,
                    "url": f"/files/{name}",
                    "hashes": {"sha256": hashlib.sha256(body).hexdigest()},
                    "requires-python": ">=3.11",
                    "size": len(body),
                    "upload-time": uploaded.strftime("%Y-%m-%dT%H:%M:%S.%fZ"),
                }
            )
        return files

    def _handler(self) -> type[BaseHTTPRequestHandler]:
        index = self

        class Handler(BaseHTTPRequestHandler):
            def do_GET(self) -> None:
                if self.path == f"/simple/{PROBE}/":
                    body = json.dumps(
                        {
                            "meta": {"api-version": "1.1"},
                            "name": PROBE,
                            "files": index._files(),
                            "versions": list(index.versions),
                        }
                    ).encode()
                    self._send(body, SIMPLE_JSON)
                    return
                for version, (_, data) in index.versions.items():
                    if self.path == f"/files/cooldown_probe-{version}-py3-none-any.whl":
                        self._send(data, "application/octet-stream")
                        return
                self.send_error(404)

            do_HEAD = do_GET  # uv probes a distribution with HEAD before it downloads it

            def _send(self, body: bytes, content_type: str) -> None:
                self.send_response(200)
                self.send_header("Content-Type", content_type)
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                if self.command != "HEAD":
                    self.wfile.write(body)

            def log_message(self, *args: object) -> None:  # silence the test output
                return

        return Handler

    def __enter__(self) -> _Index:
        self.thread.start()
        return self

    def __exit__(self, *exc: object) -> None:
        self.server.shutdown()
        self.server.server_close()


def _uv(args: list[str], cwd: Path, home: Path, index: _Index) -> subprocess.CompletedProcess[str]:
    # XDG_CONFIG_HOME moves the workstation's uv.toml (the packages proxy) out of
    # the way, and UV_DEFAULT_INDEX points the resolver at the local index.
    env = {
        **os.environ,
        "XDG_CONFIG_HOME": str(home / "config"),
        "UV_CACHE_DIR": str(home / "cache"),
        "UV_DEFAULT_INDEX": index.url,
        "UV_PYTHON_DOWNLOADS": "never",
    }
    return subprocess.run(["uv", *args], cwd=cwd, env=env, capture_output=True, text=True, check=False)


def _locked_probe_version(project: Path, home: Path, index: _Index, pyproject_extra: str) -> str:
    project.mkdir()
    (project / "pyproject.toml").write_text(
        f'[project]\nname = "window-probe"\nversion = "0"\nrequires-python = ">=3.11"\n'
        f'dependencies = ["{PROBE}"]\n[tool.uv]\npackage = false\n{pyproject_extra}'
    )
    run = _uv(["lock"], project, home, index)
    assert run.returncode == 0, run.stderr
    lock = tomllib.loads((project / "uv.lock").read_text())
    versions = [pkg["version"] for pkg in lock["package"] if pkg["name"] == PROBE]
    assert len(versions) == 1
    return versions[0]


@pytest.fixture
def index() -> Iterator[_Index]:
    with _Index() as served:
        yield served


def test_a_version_uploaded_inside_the_window_is_not_resolved(tmp_path: Path, index: _Index) -> None:
    home = tmp_path / "home"
    # The control: without the window the newest upload is resolved.
    assert _locked_probe_version(tmp_path / "open", home, index, "") == "2.0.0"
    # With the window the two-day-old upload is refused and the thirty-day-old one wins.
    assert _locked_probe_version(tmp_path / "windowed", home, index, f'exclude-newer = "{WINDOW}"\n') == "1.0.0"


def test_a_no_project_run_with_the_flag_does_not_resolve_the_young_version(tmp_path: Path, index: _Index) -> None:
    home = tmp_path / "home"
    run = _uv(
        [
            "run",
            "--no-project",
            "--exclude-newer",
            WINDOW,
            "--with",
            PROBE,
            "python",
            "-c",
            "import cooldown_probe; print(repr(cooldown_probe.VERSION))",
        ],
        tmp_path,
        home,
        index,
    )
    assert run.returncode == 0, run.stderr
    assert run.stdout.strip() == "'1.0.0'"
