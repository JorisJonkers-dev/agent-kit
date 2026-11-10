"""Keep the test suite out of the developer's real home directory.

Setup and its helpers write files under the home directory: kubeconfig, netrc,
npmrc, Claude and Codex settings. A test that reaches the real home overwrites
the developer's own copies. Two guards stop that:

* Every test runs with HOME in its own temporary directory, and with the XDG
  directory variables and KUBECONFIG unset so they fall back under it. Code that
  finds the home through the environment, and every subprocess it starts, stays
  there.
* The session records the real home's sensitive files (found through passwd,
  which the environment cannot redirect) before the first test, and fails the
  run if any of them was created, changed or removed. That catches code that
  finds the home another way, such as ``getent passwd``.

Only file metadata is read, never contents.
"""

from __future__ import annotations

import os
import pwd
from pathlib import Path

import pytest

REAL_HOME = Path(pwd.getpwuid(os.getuid()).pw_dir)

# Files setup writes, and nothing else on a workstation changes during a test
# run. ~/.claude and ~/.vibe hold more than these, but running agents write to
# them all the time, so only the files setup owns are watched there.
WATCHED = (
    ".kube/config",
    ".netrc",
    ".npmrc",
    ".gitconfig",
    ".config/git/config",
    ".config/go/env",
    ".config/pip/pip.conf",
    ".config/uv/uv.toml",
    ".config/jorisjonkers/packages-token",
    ".cargo/config.toml",
    ".cargo/credentials.toml",
    ".gradle/gradle.properties",
    ".gradle/init.d/packages-proxy.init.gradle.kts",
    ".m2/settings.xml",
    ".claude/CLAUDE.md",
    ".codex/AGENTS.md",
    ".codex/hooks.json",
    ".codex/config.toml",
    ".hermes/SOUL.md",
    ".config/opencode/AGENTS.md",
    ".hindsight/coding-agent.json",
)


_BEFORE = pytest.StashKey[dict]()


def _snapshot() -> dict[str, tuple[int, int] | None]:
    state: dict[str, tuple[int, int] | None] = {}
    for rel in WATCHED:
        try:
            st = (REAL_HOME / rel).stat()
        except FileNotFoundError:
            state[rel] = None
        else:
            state[rel] = (st.st_mtime_ns, st.st_size)
    return state


def pytest_sessionstart(session: pytest.Session) -> None:
    session.config.stash[_BEFORE] = _snapshot()


def pytest_sessionfinish(session: pytest.Session, exitstatus: int) -> None:
    before = session.config.stash.get(_BEFORE, None)
    if before is None:
        return
    after = _snapshot()
    touched = sorted(rel for rel in WATCHED if before[rel] != after[rel])
    if touched:
        reporter = session.config.pluginmanager.get_plugin("terminalreporter")
        lines = [f"the test run changed files in the real home {REAL_HOME}:"]
        lines += [f"  ~/{rel}" for rel in touched]
        if reporter is not None:
            reporter.write_line("\n".join(lines), red=True, bold=True)
        session.exitstatus = pytest.ExitCode.TESTS_FAILED


@pytest.fixture(autouse=True)
def _isolated_home(tmp_path_factory: pytest.TempPathFactory, monkeypatch: pytest.MonkeyPatch) -> Path:
    home = tmp_path_factory.mktemp("home")
    # Toolchains stay where they are: rustup finds its toolchains and cargo its registry
    # config under the home, and both would be gone in an empty one. The watched files above
    # still fail the run if a test writes cargo's config or credentials.
    for name, default in (("RUSTUP_HOME", ".rustup"), ("CARGO_HOME", ".cargo")):
        if name not in os.environ and (REAL_HOME / default).is_dir():
            monkeypatch.setenv(name, str(REAL_HOME / default))
    monkeypatch.setenv("HOME", str(home))
    # Unset rather than set: each falls back to a path under HOME, and a test that sets one
    # itself still can. Some scripts refuse to run while KUBECONFIG is set at all.
    for name in ("XDG_CONFIG_HOME", "XDG_DATA_HOME", "XDG_CACHE_HOME", "XDG_STATE_HOME", "KUBECONFIG"):
        monkeypatch.delenv(name, raising=False)
    return home
